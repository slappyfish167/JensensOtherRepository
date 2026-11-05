//! Application services: the glue between HTTP, storage, rules and AI.
//! Handlers stay thin; every rule decision is delegated to `pf2e-core`.

use crate::error::{ApiError, ApiResult};
use pf2e_core::builder::content::Content;
use pf2e_core::builder::{build, BuildResult};
use pf2e_core::combat::{
    replay, CombatLog, CombatSetup, CombatState, CombatantKind, Command, Event, PendingInput, SetupParticipant,
};
use pf2e_core::models::encounter::{encounter_budget, EncounterBudget};
use pf2e_core::models::{Encounter, ParticipantSource, Statblock};
use pf2e_core::rng::SeededDice;
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use pf2e_store::{now, CombatRecord, DiceMode, SqliteStore, Store};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct CombatSession {
    pub record: CombatRecord,
    pub log: CombatLog,
    pub state: CombatState,
    pub last_events: Vec<Event>,
}

pub struct App {
    pub store: Mutex<Box<dyn Store>>,
    pub content: Content,
    sessions: Mutex<HashMap<String, CombatSession>>,
}

pub type SharedApp = Arc<App>;

#[derive(Debug, Serialize)]
pub struct CombatView {
    pub id: String,
    pub name: String,
    pub encounter_id: Option<String>,
    pub dice: DiceMode,
    pub status: String,
    pub state: CombatState,
    pub pending: Option<PendingInput>,
    pub current_actor: Option<CombatantId>,
    pub command_count: usize,
    pub fingerprint: String,
    pub events: Vec<Event>,
}

impl App {
    pub fn new(store: SqliteStore, content: Content) -> Self {
        App { store: Mutex::new(Box::new(store)), content, sessions: Mutex::new(HashMap::new()) }
    }

    pub fn store(&self) -> std::sync::MutexGuard<'_, Box<dyn Store>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, CombatSession>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ----- characters ------------------------------------------------------

    pub fn build_character(&self, c: &pf2e_core::models::Character) -> BuildResult {
        build(c, &self.content)
    }

    // ----- encounters -------------------------------------------------------

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
        // Disambiguate duplicate names: "Bone Sentinel 1", "Bone Sentinel 2".
        let mut counts: HashMap<String, usize> = HashMap::new();
        for p in &parts {
            *counts.entry(p.name.clone()).or_default() += 1;
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        for p in &mut parts {
            if counts[&p.name] > 1 {
                let n = seen.entry(p.name.clone()).or_default();
                *n += 1;
                p.name = format!("{} {}", p.name, n);
            }
        }
        CombatSetup::default()
            .build(e.battlefield.clone(), parts, &self.content)
            .map_err(|err| ApiError::bad_request(err.to_string()))
    }

    // ----- combats ------------------------------------------------------------

    pub fn create_combat(&self, encounter_id: Option<String>, name: String, state: CombatState, dice: DiceMode) -> ApiResult<CombatView> {
        let id = uuid::Uuid::new_v4().to_string();
        let rec = CombatRecord { id: id.clone(), encounter_id, name, dice, status: "active".into(), created_at: now(), updated_at: now() };
        self.store().create_combat(&rec, &state)?;
        let session = CombatSession { record: rec, log: CombatLog::new(state.clone()), state, last_events: vec![] };
        let view = view_of(&session);
        self.sessions().insert(id, session);
        Ok(view)
    }

    fn with_session<T>(&self, id: &str, f: impl FnOnce(&mut CombatSession, &dyn Store) -> ApiResult<T>) -> ApiResult<T> {
        let mut sessions = self.sessions();
        if !sessions.contains_key(id) {
            let (record, log) = self.store().get_combat(id)?.ok_or_else(|| ApiError::not_found(format!("combat '{id}'")))?;
            let (state, _) = replay(&log.initial, &log.commands)
                .map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("stored log does not replay: {e}")))?;
            sessions.insert(id.to_string(), CombatSession { record, log, state, last_events: vec![] });
        }
        let session = sessions.get_mut(id).expect("inserted above");
        let store = self.store();
        f(session, store.as_ref())
    }

    pub fn view(&self, id: &str) -> ApiResult<CombatView> {
        self.with_session(id, |s, _| Ok(view_of(s)))
    }

    pub fn snapshot(&self, id: &str) -> ApiResult<CombatState> {
        self.with_session(id, |s, _| Ok(s.state.clone()))
    }

    pub fn log(&self, id: &str) -> ApiResult<CombatLog> {
        self.with_session(id, |s, _| Ok(s.log.clone()))
    }

    /// Applies a command, persists it, and returns the new view.
    pub fn command(&self, id: &str, cmd: Command) -> ApiResult<CombatView> {
        self.with_session(id, |s, store| {
            let events = s.state.apply(&cmd)?;
            let seq = s.log.commands.len();
            store.append_command(&s.record.id, seq, &cmd)?;
            s.log.commands.push(cmd);
            s.last_events = events;
            if s.state.is_over() && s.record.status != "ended" {
                store.set_combat_status(&s.record.id, "ended")?;
                s.record.status = "ended".into();
            }
            Ok(view_of(s))
        })
    }

    /// Rolls the pending request with the combat's seeded dice. Only allowed
    /// when the combat was created with `dice: {mode: "seeded"}`.
    pub fn auto_roll(&self, id: &str) -> ApiResult<CombatView> {
        let cmd = self.with_session(id, |s, _| {
            let DiceMode::Seeded { seed } = s.record.dice else {
                return Err(ApiError::conflict("this combat uses manual dice; enable seeded dice to auto-roll"));
            };
            let Some(PendingInput::Roll(r)) = s.state.pending() else {
                return Err(ApiError::conflict("no roll is pending"));
            };
            // One fresh generator per log position: deterministic and stateless.
            let mut dice = SeededDice::new(seed ^ (s.log.commands.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            Ok(Command::ProvideRoll { request_id: r.request_id, value: dice.answer(&r), extra_modifier: 0 })
        })?;
        self.command(id, cmd)
    }

    pub fn undo(&self, id: &str) -> ApiResult<CombatView> {
        self.with_session(id, |s, store| {
            if s.log.commands.is_empty() {
                return Err(ApiError::conflict("nothing to undo"));
            }
            let mut commands = s.log.commands.clone();
            commands.pop();
            let (state, _) = replay(&s.log.initial, &commands).map_err(|e| ApiError::conflict(e.to_string()))?;
            store.truncate_commands(&s.record.id, commands.len())?;
            if s.record.status == "ended" && !state.is_over() {
                store.set_combat_status(&s.record.id, "active")?;
                s.record.status = "active".into();
            }
            s.log.commands = commands;
            s.state = state;
            s.last_events.clear();
            Ok(view_of(s))
        })
    }

    pub fn forget_session(&self, id: &str) {
        self.sessions().remove(id);
    }
}

fn view_of(s: &CombatSession) -> CombatView {
    CombatView {
        id: s.record.id.clone(),
        name: s.record.name.clone(),
        encounter_id: s.record.encounter_id.clone(),
        dice: s.record.dice,
        status: s.record.status.clone(),
        pending: s.state.pending(),
        current_actor: s.state.current_actor(),
        command_count: s.log.commands.len(),
        fingerprint: s.state.fingerprint(),
        state: s.state.clone(),
        events: s.last_events.clone(),
    }
}
