//! Application services: the glue between HTTP, storage, rules and AI.
//! Handlers stay thin; every rule decision is delegated to a rules engine.
//!
//! Combats of every game system run through the same code: a
//! [`Session<S>`] is generic over `S: GameSystem`, and [`AnySession`] is the
//! one place that lists the supported systems.

use crate::error::{ApiError, ApiResult};
use pf2e_ai::Pf2e;
use pf2e_core::builder::content::Content;
use pf2e_core::builder::{build, BuildResult};
use pf2e_core::combat::{CombatSetup, CombatState, CombatantKind, SetupParticipant};
use pf2e_core::models::encounter::{encounter_budget, EncounterBudget};
use pf2e_core::models::{Encounter, ParticipantSource, Statblock};
use pf2e_core::stats::Side;
use pf2e_store::{now, CombatRecord, DiceMode, SqliteStore, Store, StoredLog};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tactics_core::view::{PromptView, TableView};
use tactics_core::{CombatantId, GameSystem, SeededDice};

/// One live combat of system `S`: its log and the state it replays to.
pub struct Session<S: GameSystem> {
    pub record: CombatRecord,
    pub initial: S::State,
    pub commands: Vec<S::Command>,
    pub state: S::State,
    pub last_events: Vec<S::Event>,
}

/// Every supported game system. Add a variant (and an arm to [`with_session!`])
/// to support another one.
pub enum AnySession {
    Pf2e(Session<Pf2e>),
    Cpred(Session<cpred_ai::CyberpunkRed>),
}

/// Runs `$body` with `$s` bound to the typed session, whatever its system.
#[macro_export]
macro_rules! with_session {
    ($session:expr, $s:ident => $body:expr) => {
        match $session {
            $crate::app::AnySession::Pf2e($s) => $body,
            $crate::app::AnySession::Cpred($s) => $body,
        }
    };
}

/// A copy of a combat's current state, typed by system (for analysis, which
/// runs off the request thread on its own copy).
pub enum Snapshot {
    Pf2e(CombatState),
    Cpred(cpred_core::combat::CombatState),
}

/// Runs `$body` with `$s` bound to the typed state and `$S` to its system type.
#[macro_export]
macro_rules! with_snapshot {
    ($snap:expr, $S:ident, $s:ident => $body:expr) => {
        match $snap {
            $crate::app::Snapshot::Pf2e($s) => {
                type $S = pf2e_ai::Pf2e;
                $body
            }
            $crate::app::Snapshot::Cpred($s) => {
                type $S = cpred_ai::CyberpunkRed;
                $body
            }
        }
    };
}

pub struct App {
    pub store: Mutex<Box<dyn Store>>,
    pub content: Content,
    pub cpred: cpred_core::content::Content,
    sessions: Mutex<HashMap<String, AnySession>>,
}

pub type SharedApp = Arc<App>;

/// What every combat looks like over HTTP. `state`, `pending` and `events`
/// are the system's own JSON; `table`, `prompt` and `log` are system-neutral
/// so one UI can run any system.
#[derive(Debug, Serialize)]
pub struct CombatView {
    pub id: String,
    pub name: String,
    pub system: String,
    pub system_name: String,
    pub encounter_id: Option<String>,
    pub dice: DiceMode,
    pub status: String,
    pub state: Value,
    pub pending: Option<Value>,
    pub current_actor: Option<CombatantId>,
    pub command_count: usize,
    pub fingerprint: String,
    pub events: Vec<Value>,
    /// The events above, described in words.
    pub log: Vec<String>,
    pub table: TableView,
    pub prompt: Option<PromptView>,
}

#[derive(Debug, Serialize)]
pub struct ReplayCheck {
    pub commands: usize,
    pub events: usize,
    pub live_fingerprint: String,
    pub replayed_fingerprint: String,
    pub identical: bool,
}

fn internal(msg: impl Into<String>) -> ApiError {
    ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg)
}

fn illegal(msg: String) -> ApiError {
    ApiError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, msg)
}

impl<S: GameSystem> Session<S> {
    pub fn new(record: CombatRecord, initial: S::State) -> Self {
        Session { record, state: initial.clone(), initial, commands: vec![], last_events: vec![] }
    }

    /// Rebuilds a session from its stored JSON log.
    pub fn from_log(record: CombatRecord, log: StoredLog) -> ApiResult<Self> {
        let initial: S::State = serde_json::from_value(log.initial).map_err(|e| internal(format!("stored state does not decode as {}: {e}", S::NAME)))?;
        let commands = log
            .commands
            .into_iter()
            .map(|c| serde_json::from_value::<S::Command>(c).map_err(|e| internal(format!("stored command does not decode: {e}"))))
            .collect::<ApiResult<Vec<_>>>()?;
        let (state, _) = S::replay(&initial, &commands).map_err(|(i, e)| internal(format!("stored log does not replay (command #{i}): {e}")))?;
        Ok(Session { record, initial, commands, state, last_events: vec![] })
    }

    pub fn view(&self) -> CombatView {
        CombatView {
            id: self.record.id.clone(),
            name: self.record.name.clone(),
            system: S::ID.into(),
            system_name: S::NAME.into(),
            encounter_id: self.record.encounter_id.clone(),
            dice: self.record.dice,
            status: self.record.status.clone(),
            state: serde_json::to_value(&self.state).unwrap_or_default(),
            pending: S::pending(&self.state).and_then(|p| serde_json::to_value(p).ok()),
            current_actor: S::current_actor(&self.state),
            command_count: self.commands.len(),
            fingerprint: S::fingerprint(&self.state),
            events: self.last_events.iter().filter_map(|e| serde_json::to_value(e).ok()).collect(),
            log: self.last_events.iter().filter_map(|e| S::describe_event(&self.state, e)).collect(),
            table: S::table(&self.state),
            prompt: S::prompt(&self.state),
        }
    }

    /// Applies a command, persists it, and returns the new view.
    pub fn command(&mut self, store: &dyn Store, cmd: S::Command) -> ApiResult<CombatView> {
        let events = S::apply(&mut self.state, &cmd).map_err(illegal)?;
        let seq = self.commands.len();
        store.append_command(&self.record.id, seq, &serde_json::to_value(&cmd)?)?;
        self.commands.push(cmd);
        self.last_events = events;
        if S::is_over(&self.state) && self.record.status != "ended" {
            store.set_combat_status(&self.record.id, "ended")?;
            self.record.status = "ended".into();
        }
        Ok(self.view())
    }

    pub fn command_json(&mut self, store: &dyn Store, cmd: Value) -> ApiResult<CombatView> {
        let cmd: S::Command = serde_json::from_value(cmd).map_err(|e| ApiError::bad_request(format!("not a {} command: {e}", S::NAME)))?;
        self.command(store, cmd)
    }

    /// Rolls the pending request with the combat's seeded dice. Only allowed
    /// when the combat was created with `dice: {mode: "seeded"}`.
    pub fn auto_roll(&mut self, store: &dyn Store) -> ApiResult<CombatView> {
        let DiceMode::Seeded { seed } = self.record.dice else {
            return Err(ApiError::conflict("this combat uses manual dice; enable seeded dice to auto-roll"));
        };
        // One fresh generator per log position: deterministic and stateless.
        let mut dice = SeededDice::new(seed ^ (self.commands.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let cmd = S::auto_answer(&self.state, &mut dice).ok_or_else(|| ApiError::conflict("no roll is pending"))?;
        self.command(store, cmd)
    }

    pub fn undo(&mut self, store: &dyn Store) -> ApiResult<CombatView> {
        if self.commands.is_empty() {
            return Err(ApiError::conflict("nothing to undo"));
        }
        let mut commands = self.commands.clone();
        commands.pop();
        let (state, _) = S::replay(&self.initial, &commands).map_err(|(_, e)| ApiError::conflict(e))?;
        store.truncate_commands(&self.record.id, commands.len())?;
        if self.record.status == "ended" && !S::is_over(&state) {
            store.set_combat_status(&self.record.id, "active")?;
            self.record.status = "active".into();
        }
        self.commands = commands;
        self.state = state;
        self.last_events.clear();
        Ok(self.view())
    }

    pub fn replay_check(&self) -> ApiResult<ReplayCheck> {
        let (replayed, events) = S::replay(&self.initial, &self.commands).map_err(|(_, e)| ApiError::conflict(e))?;
        Ok(ReplayCheck {
            commands: self.commands.len(),
            events: events.len(),
            live_fingerprint: S::fingerprint(&self.state),
            replayed_fingerprint: S::fingerprint(&replayed),
            identical: replayed == self.state,
        })
    }

    pub fn log(&self) -> Value {
        serde_json::json!({ "system": S::ID, "initial": self.initial, "commands": self.commands })
    }
}

impl AnySession {
    pub fn system(&self) -> &'static str {
        match self {
            AnySession::Pf2e(_) => Pf2e::ID,
            AnySession::Cpred(_) => cpred_ai::CyberpunkRed::ID,
        }
    }

    fn load(record: CombatRecord, log: StoredLog) -> ApiResult<Self> {
        Ok(match record.system.as_str() {
            Pf2e::ID => AnySession::Pf2e(Session::from_log(record, log)?),
            cpred_ai::CyberpunkRed::ID => AnySession::Cpred(Session::from_log(record, log)?),
            other => return Err(internal(format!("combat uses unknown game system '{other}'"))),
        })
    }
}

impl App {
    pub fn new(store: SqliteStore, content: Content) -> Self {
        let cpred = cpred_core::content::Content::bundled().expect("bundled Cyberpunk RED content is valid");
        App { store: Mutex::new(Box::new(store)), content, cpred, sessions: Mutex::new(HashMap::new()) }
    }

    pub fn store(&self) -> std::sync::MutexGuard<'_, Box<dyn Store>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, AnySession>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ----- PF2e characters ----------------------------------------------------

    pub fn build_character(&self, c: &pf2e_core::models::Character) -> BuildResult {
        build(c, &self.content)
    }

    // ----- PF2e encounters ----------------------------------------------------

    fn resolve_statblock(&self, src: &ParticipantSource) -> ApiResult<(Statblock, CombatantKind)> {
        match src {
            ParticipantSource::Character(id) => {
                let ch = self.store().get_character(id)?.ok_or_else(|| ApiError::not_found(format!("character '{id}'")))?;
                let r = self.build_character(&ch);
                let sb = r.statblock.ok_or_else(|| {
                    ApiError::bad_request(format!("character '{}' is not a legal build", ch.name))
                        .with_details(serde_json::to_value(&r.report).unwrap_or_default())
                })?;
                Ok((sb, CombatantKind::Character))
            }
            ParticipantSource::Monster(id) => {
                let m = self.store().get_monster(id)?.ok_or_else(|| ApiError::not_found(format!("monster '{id}'")))?;
                let kind = if m.is_hazard() { CombatantKind::Hazard } else { CombatantKind::Monster };
                Ok((m.statblock, kind))
            }
            ParticipantSource::Inline(sb) => {
                let kind = if sb.hazard.is_some() { CombatantKind::Hazard } else { CombatantKind::Monster };
                Ok(((**sb).clone(), kind))
            }
        }
    }

    pub fn budget(&self, e: &Encounter) -> ApiResult<EncounterBudget> {
        let mut party = 0;
        let mut opponents = Vec::new();
        for p in &e.participants {
            let (sb, kind) = self.resolve_statblock(&p.source)?;
            match p.side {
                Side::Party => party += 1,
                Side::Enemy => {
                    let simple = kind == CombatantKind::Hazard && !sb.hazard.as_ref().map(|h| h.complex).unwrap_or(false);
                    opponents.push((sb.level, simple));
                }
                Side::Neutral => {}
            }
        }
        Ok(encounter_budget(e.party_level, party, &opponents))
    }

    pub fn combat_state_for(&self, e: &Encounter) -> ApiResult<CombatState> {
        let mut parts = Vec::new();
        for p in &e.participants {
            let (sb, kind) = self.resolve_statblock(&p.source)?;
            parts.push(SetupParticipant {
                name: p.name.clone().unwrap_or_else(|| sb.name.clone()),
                side: p.side,
                kind,
                statblock: sb,
                position: p.position,
                current_hp: p.current_hp,
            });
        }
        let mut names: Vec<String> = parts.iter().map(|p| p.name.clone()).collect();
        disambiguate(&mut names);
        for (p, n) in parts.iter_mut().zip(names) {
            p.name = n;
        }
        CombatSetup::default().build(e.battlefield.clone(), parts, &self.content).map_err(|err| ApiError::bad_request(err.to_string()))
    }

    // ----- combats --------------------------------------------------------------

    fn record(&self, system: &str, encounter_id: Option<String>, name: String, dice: DiceMode) -> CombatRecord {
        CombatRecord { id: uuid::Uuid::new_v4().to_string(), system: system.into(), encounter_id, name, dice, status: "active".into(), created_at: now(), updated_at: now() }
    }

    /// Persists a new combat of system `S` and opens a session for it.
    pub fn create_combat<S: GameSystem>(&self, encounter_id: Option<String>, name: String, state: S::State, dice: DiceMode, wrap: fn(Session<S>) -> AnySession) -> ApiResult<CombatView> {
        let rec = self.record(S::ID, encounter_id, name, dice);
        self.store().create_combat(&rec, &serde_json::to_value(&state)?)?;
        let session = Session::<S>::new(rec.clone(), state);
        let view = session.view();
        self.sessions().insert(rec.id, wrap(session));
        Ok(view)
    }

    /// Runs `f` on the (cached or replayed) session of combat `id`.
    pub fn with<T>(&self, id: &str, f: impl FnOnce(&mut AnySession, &dyn Store) -> ApiResult<T>) -> ApiResult<T> {
        let mut sessions = self.sessions();
        if !sessions.contains_key(id) {
            let (record, log) = self.store().get_combat(id)?.ok_or_else(|| ApiError::not_found(format!("combat '{id}'")))?;
            sessions.insert(id.to_string(), AnySession::load(record, log)?);
        }
        let session = sessions.get_mut(id).expect("inserted above");
        let store = self.store();
        f(session, store.as_ref())
    }

    pub fn view(&self, id: &str) -> ApiResult<CombatView> {
        self.with(id, |s, _| Ok(with_session!(s, s => s.view())))
    }

    pub fn command(&self, id: &str, cmd: Value) -> ApiResult<CombatView> {
        self.with(id, |s, store| with_session!(s, s => s.command_json(store, cmd)))
    }

    pub fn auto_roll(&self, id: &str) -> ApiResult<CombatView> {
        self.with(id, |s, store| with_session!(s, s => s.auto_roll(store)))
    }

    pub fn undo(&self, id: &str) -> ApiResult<CombatView> {
        self.with(id, |s, store| with_session!(s, s => s.undo(store)))
    }

    pub fn forget_session(&self, id: &str) {
        self.sessions().remove(id);
    }

    pub fn snapshot(&self, id: &str) -> ApiResult<Snapshot> {
        self.with(id, |s, _| {
            Ok(match s {
                AnySession::Pf2e(s) => Snapshot::Pf2e(s.state.clone()),
                AnySession::Cpred(s) => Snapshot::Cpred(s.state.clone()),
            })
        })
    }

    /// A PF2e combat's state (for PF2e-only endpoints).
    pub fn pf2e_snapshot(&self, id: &str) -> ApiResult<CombatState> {
        self.with(id, |s, _| match s {
            AnySession::Pf2e(s) => Ok(s.state.clone()),
            other => Err(ApiError::conflict(format!("this endpoint is for Pathfinder 2e combats; combat '{id}' is {}", other.system()))),
        })
    }
}

/// "Bone Sentinel", "Bone Sentinel" → "Bone Sentinel 1", "Bone Sentinel 2".
pub fn disambiguate(names: &mut [String]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for n in names.iter() {
        *counts.entry(n.clone()).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for n in names.iter_mut() {
        if counts[n.as_str()] > 1 {
            let k = seen.entry(n.clone()).or_default();
            *k += 1;
            *n = format!("{n} {k}");
        }
    }
}
