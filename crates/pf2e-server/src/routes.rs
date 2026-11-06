//! HTTP routes. See `docs/API.md` for the full specification.

use crate::app::{AnySession, CombatView, SharedApp, Snapshot};
use crate::error::{ApiError, ApiResult};
use crate::{with_session, with_snapshot};
use cpred_ai::CyberpunkRed;
use pf2e_ai::Pf2e;
use tactics_core::GameSystem;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use pf2e_ai::queries::{describe_action, rank_spells};
use tactics_ai::{SearchConfig, SimulationConfig};
use pf2e_core::combat::Action;
use pf2e_core::models::{Character, Encounter, Monster, MonsterTemplate, Statblock};
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use pf2e_store::DiceMode;
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router(app: SharedApp) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/content", get(content))
        // game systems
        .route("/api/systems", get(systems))
        .route("/api/systems/{system}/content", get(system_content))
        .route("/api/systems/{system}/{kind}", get(list_documents).post(create_document))
        .route("/api/systems/{system}/{kind}/{id}", get(get_document).put(put_document).delete(delete_document))
        // characters
        .route("/api/characters", get(list_characters).post(create_character))
        .route("/api/characters/validate", post(validate_character))
        .route("/api/characters/import", post(import_character))
        .route("/api/characters/{id}", get(get_character).put(put_character).delete(delete_character))
        .route("/api/characters/{id}/statblock", get(character_statblock))
        .route("/api/characters/{id}/export", get(export_character))
        // monsters
        .route("/api/monsters", get(list_monsters).post(create_monster))
        .route("/api/monsters/import", post(import_monsters))
        .route("/api/monsters/{id}", get(get_monster).put(put_monster).delete(delete_monster))
        .route("/api/monsters/{id}/template", post(template_monster))
        // encounters
        .route("/api/encounters", get(list_encounters).post(create_encounter))
        .route("/api/encounters/budget", post(budget_unsaved))
        .route("/api/encounters/{id}", get(get_encounter).put(put_encounter).delete(delete_encounter))
        .route("/api/encounters/{id}/budget", get(encounter_budget))
        // combats
        .route("/api/combats", get(list_combats).post(create_combat))
        .route("/api/combats/{id}", get(get_combat).delete(delete_combat))
        .route("/api/combats/{id}/commands", post(post_command))
        .route("/api/combats/{id}/auto-roll", post(auto_roll))
        .route("/api/combats/{id}/undo", post(undo))
        .route("/api/combats/{id}/log", get(combat_log))
        .route("/api/combats/{id}/replay", get(verify_replay))
        .route("/api/combats/{id}/legal-actions", get(legal_actions))
        .route("/api/combats/{id}/actions", get(actions))
        .route("/api/combats/{id}/movement/{combatant}", get(movement))
        // analysis (optional AI layer)
        .route("/api/combats/{id}/analysis/evaluation", get(evaluation))
        .route("/api/combats/{id}/analysis/best-turn", post(best_turn))
        .route("/api/combats/{id}/analysis/targets", get(targets))
        .route("/api/combats/{id}/analysis/spells", get(spells))
        .route("/api/combats/{id}/analysis/tpk", post(tpk))
        .route("/api/combats/{id}/analysis/strategy", get(strategy))
        .with_state(app)
}

type J = ApiResult<Json<Value>>;

fn ok<T: serde::Serialize>(v: T) -> J {
    Ok(Json(serde_json::to_value(v).map_err(ApiError::from)?))
}

/// Runs CPU-heavy analysis off the async executor.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn health(State(app): State<SharedApp>) -> J {
    ok(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION"), "spells": app.content.spells.len() }))
}

async fn content(State(app): State<SharedApp>) -> J {
    let c = &app.content;
    ok(json!({
        "ancestries": c.ancestries, "backgrounds": c.backgrounds, "classes": c.classes, "feats": c.feats,
        "weapons": c.weapons, "armor": c.armor, "shields": c.shields, "spells": c.spells,
    }))
}

// ----- characters ------------------------------------------------------------

fn fresh_id(id: &str) -> String {
    if id.trim().is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        id.to_string()
    }
}

async fn list_characters(State(app): State<SharedApp>) -> J {
    ok(app.store().list_characters()?)
}

fn save_character(app: &SharedApp, mut c: Character) -> J {
    c.id = fresh_id(&c.id);
    let build = app.build_character(&c);
    app.store().put_character(&c, build.report.is_legal())?;
    ok(json!({ "character": c, "build": build }))
}

async fn create_character(State(app): State<SharedApp>, Json(c): Json<Character>) -> J {
    if !c.id.is_empty() && app.store().get_character(&c.id)?.is_some() {
        return Err(ApiError::conflict(format!("character '{}' already exists", c.id)));
    }
    save_character(&app, c)
}

/// Like create, but never overwrites: a colliding id gets a new one.
async fn import_character(State(app): State<SharedApp>, Json(mut c): Json<Character>) -> J {
    if c.id.is_empty() || app.store().get_character(&c.id)?.is_some() {
        c.id = uuid::Uuid::new_v4().to_string();
    }
    save_character(&app, c)
}

async fn put_character(State(app): State<SharedApp>, Path(id): Path<String>, Json(mut c): Json<Character>) -> J {
    c.id = id;
    save_character(&app, c)
}

async fn get_character(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(app.store().get_character(&id)?.ok_or_else(|| ApiError::not_found(format!("character '{id}'")))?)
}

async fn export_character(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    get_character(State(app), Path(id)).await
}

async fn delete_character(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(json!({ "deleted": app.store().delete_character(&id)? }))
}

async fn validate_character(State(app): State<SharedApp>, Json(c): Json<Character>) -> J {
    ok(app.build_character(&c))
}

async fn character_statblock(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let c = app.store().get_character(&id)?.ok_or_else(|| ApiError::not_found(format!("character '{id}'")))?;
    ok(app.build_character(&c))
}

// ----- monsters ----------------------------------------------------------------

async fn list_monsters(State(app): State<SharedApp>) -> J {
    ok(app.store().list_monsters()?)
}

fn save_monster(app: &SharedApp, mut m: Monster) -> J {
    m.id = fresh_id(&m.id);
    if m.statblock.name.trim().is_empty() {
        return Err(ApiError::bad_request("monster needs a name"));
    }
    if m.statblock.max_hp <= 0 {
        return Err(ApiError::bad_request("max_hp must be positive"));
    }
    app.store().put_monster(&m)?;
    ok(m)
}

async fn create_monster(State(app): State<SharedApp>, Json(m): Json<Monster>) -> J {
    if !m.id.is_empty() && app.store().get_monster(&m.id)?.is_some() {
        return Err(ApiError::conflict(format!("monster '{}' already exists", m.id)));
    }
    save_monster(&app, m)
}

async fn put_monster(State(app): State<SharedApp>, Path(id): Path<String>, Json(mut m): Json<Monster>) -> J {
    m.id = id;
    save_monster(&app, m)
}

async fn get_monster(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(app.store().get_monster(&id)?.ok_or_else(|| ApiError::not_found(format!("monster '{id}'")))?)
}

async fn delete_monster(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(json!({ "deleted": app.store().delete_monster(&id)? }))
}

/// Accepts a monster, an array of monsters, or a bare stat block.
async fn import_monsters(State(app): State<SharedApp>, Json(body): Json<Value>) -> J {
    let items: Vec<Value> = match body {
        Value::Array(v) => v,
        other => vec![other],
    };
    let mut saved = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        let m: Monster = if item.get("statblock").is_some() {
            serde_json::from_value(item).map_err(|e| ApiError::bad_request(format!("item {i}: {e}")))?
        } else {
            let sb: Statblock = serde_json::from_value(item).map_err(|e| ApiError::bad_request(format!("item {i}: {e}")))?;
            Monster { id: String::new(), source: "imported".into(), tags: vec![], description: String::new(), applied_templates: vec![], statblock: sb }
        };
        let mut m = m;
        if m.id.is_empty() || app.store().get_monster(&m.id)?.is_some() {
            m.id = uuid::Uuid::new_v4().to_string();
        }
        app.store().put_monster(&m)?;
        saved.push(m);
    }
    ok(saved)
}

#[derive(Deserialize)]
struct TemplateReq {
    template: MonsterTemplate,
    #[serde(default)]
    save: bool,
}

async fn template_monster(State(app): State<SharedApp>, Path(id): Path<String>, Json(req): Json<TemplateReq>) -> J {
    let m = app.store().get_monster(&id)?.ok_or_else(|| ApiError::not_found(format!("monster '{id}'")))?;
    let t = m.with_template(req.template);
    if req.save {
        app.store().put_monster(&t)?;
    }
    ok(t)
}

// ----- encounters ------------------------------------------------------------------

async fn list_encounters(State(app): State<SharedApp>) -> J {
    ok(app.store().list_encounters()?)
}

fn save_encounter(app: &SharedApp, mut e: Encounter) -> J {
    e.id = fresh_id(&e.id);
    e.battlefield.validate().map_err(|err| ApiError::bad_request(err.to_string()))?;
    let budget = app.budget(&e)?;
    app.store().put_encounter(&e)?;
    ok(json!({ "encounter": e, "budget": budget }))
}

async fn create_encounter(State(app): State<SharedApp>, Json(e): Json<Encounter>) -> J {
    save_encounter(&app, e)
}

async fn put_encounter(State(app): State<SharedApp>, Path(id): Path<String>, Json(mut e): Json<Encounter>) -> J {
    e.id = id;
    save_encounter(&app, e)
}

async fn get_encounter(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(app.store().get_encounter(&id)?.ok_or_else(|| ApiError::not_found(format!("encounter '{id}'")))?)
}

async fn delete_encounter(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(json!({ "deleted": app.store().delete_encounter(&id)? }))
}

async fn encounter_budget(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let e = app.store().get_encounter(&id)?.ok_or_else(|| ApiError::not_found(format!("encounter '{id}'")))?;
    ok(app.budget(&e)?)
}

async fn budget_unsaved(State(app): State<SharedApp>, Json(e): Json<Encounter>) -> J {
    ok(app.budget(&e)?)
}

// ----- systems ------------------------------------------------------------------------

async fn systems(State(app): State<SharedApp>) -> J {
    ok(json!([
        {
            "id": Pf2e::ID, "name": Pf2e::NAME,
            "features": ["characters", "monsters", "encounters", "combat", "analysis", "spells"],
        },
        {
            "id": CyberpunkRed::ID, "name": CyberpunkRed::NAME,
            "features": ["actors", "encounters", "combat", "analysis"],
            "sample_encounters": app.cpred.encounters.iter().map(|e| json!({"id": e.id, "name": e.name})).collect::<Vec<_>>(),
        },
    ]))
}

async fn system_content(State(app): State<SharedApp>, Path(system): Path<String>) -> J {
    match system.as_str() {
        "pf2e" => content(State(app)).await,
        "cpred" => ok(json!({ "weapons": app.cpred.weapons, "armor": app.cpred.armor, "rules": app.cpred.rules })),
        other => Err(ApiError::not_found(format!("game system '{other}'"))),
    }
}

/// Validates a document of a document-backed system and returns (id, name, normalised JSON).
fn check_document(app: &SharedApp, system: &str, kind: &str, mut doc: Value) -> ApiResult<(String, String, Value)> {
    if system != CyberpunkRed::ID {
        return Err(ApiError::not_found(format!("'{system}' has no generic {kind} store (use its dedicated endpoints)")));
    }
    let id = doc.get("id").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    doc["id"] = json!(id);
    match kind {
        "actors" => {
            let a: cpred_core::model::Actor = serde_json::from_value(doc).map_err(|e| ApiError::bad_request(format!("not a Cyberpunk RED actor: {e}")))?;
            let name = a.name.clone();
            Ok((id, name, serde_json::to_value(a)?))
        }
        "encounters" => {
            let e: cpred_core::model::Encounter = serde_json::from_value(doc).map_err(|e| ApiError::bad_request(format!("not a Cyberpunk RED encounter: {e}")))?;
            app.cpred_state_for(&e)?; // must build a legal combat
            let name = e.name.clone();
            Ok((id, name, serde_json::to_value(e)?))
        }
        other => Err(ApiError::not_found(format!("document kind '{other}'"))),
    }
}

fn doc_kind(kind: &str) -> ApiResult<&'static str> {
    match kind {
        "actors" => Ok("actor"),
        "encounters" => Ok("encounter"),
        other => Err(ApiError::not_found(format!("document kind '{other}'"))),
    }
}

async fn list_documents(State(app): State<SharedApp>, Path((system, kind)): Path<(String, String)>) -> J {
    ok(app.store().list_documents(&system, doc_kind(&kind)?)?)
}

async fn get_document(State(app): State<SharedApp>, Path((system, kind, id)): Path<(String, String, String)>) -> J {
    ok(app.store().get_document(&system, doc_kind(&kind)?, &id)?.ok_or_else(|| ApiError::not_found(format!("{kind} '{id}'")))?)
}

async fn create_document(State(app): State<SharedApp>, Path((system, kind)): Path<(String, String)>, Json(doc): Json<Value>) -> J {
    let (id, name, doc) = check_document(&app, &system, &kind, doc)?;
    let k = doc_kind(&kind)?;
    if app.store().get_document(&system, k, &id)?.is_some() {
        return Err(ApiError::conflict(format!("{kind} '{id}' already exists")));
    }
    app.store().put_document(&system, k, &id, &name, &doc)?;
    ok(doc)
}

async fn put_document(State(app): State<SharedApp>, Path((system, kind, id)): Path<(String, String, String)>, Json(mut doc): Json<Value>) -> J {
    doc["id"] = json!(id);
    let (id, name, doc) = check_document(&app, &system, &kind, doc)?;
    app.store().put_document(&system, doc_kind(&kind)?, &id, &name, &doc)?;
    ok(doc)
}

async fn delete_document(State(app): State<SharedApp>, Path((system, kind, id)): Path<(String, String, String)>) -> J {
    ok(json!({ "deleted": app.store().delete_document(&system, doc_kind(&kind)?, &id)? }))
}

// ----- combats ------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateCombat {
    #[serde(default = "pf2e_id")]
    system: String,
    #[serde(default)]
    encounter_id: Option<String>,
    /// An unsaved encounter of the system (instead of `encounter_id`).
    #[serde(default)]
    encounter: Option<Value>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "manual")]
    dice: DiceMode,
}

fn pf2e_id() -> String {
    Pf2e::ID.into()
}

fn manual() -> DiceMode {
    DiceMode::Manual
}

async fn create_combat(State(app): State<SharedApp>, Json(req): Json<CreateCombat>) -> J {
    match req.system.as_str() {
        "pf2e" => {
            let e: Encounter = match (&req.encounter_id, req.encounter) {
                (Some(id), _) => app.store().get_encounter(id)?.ok_or_else(|| ApiError::not_found(format!("encounter '{id}'")))?,
                (None, Some(v)) => serde_json::from_value(v)?,
                (None, None) => return Err(ApiError::bad_request("give encounter_id or encounter")),
            };
            let state = app.combat_state_for(&e)?;
            let saved = req.encounter_id.as_ref().map(|_| e.id.clone());
            ok(app.create_combat::<Pf2e>(saved, req.name.unwrap_or(e.name.clone()), state, req.dice, |s| AnySession::Pf2e(Box::new(s)))?)
        }
        "cpred" => {
            let e = app.cpred_encounter(req.encounter_id.as_deref(), req.encounter)?;
            let state = app.cpred_state_for(&e)?;
            let saved = req.encounter_id.as_ref().map(|_| e.id.clone());
            ok(app.create_combat::<CyberpunkRed>(saved, req.name.unwrap_or(e.name.clone()), state, req.dice, |s| AnySession::Cpred(Box::new(s)))?)
        }
        other => Err(ApiError::not_found(format!("game system '{other}'"))),
    }
}

async fn list_combats(State(app): State<SharedApp>) -> J {
    ok(app.store().list_combats()?)
}

async fn get_combat(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(app.view(&id)?)
}

async fn delete_combat(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    app.forget_session(&id);
    ok(json!({ "deleted": app.store().delete_combat(&id)? }))
}

async fn post_command(State(app): State<SharedApp>, Path(id): Path<String>, Json(cmd): Json<Value>) -> ApiResult<Json<CombatView>> {
    Ok(Json(app.command(&id, cmd)?))
}

async fn auto_roll(State(app): State<SharedApp>, Path(id): Path<String>) -> ApiResult<Json<CombatView>> {
    Ok(Json(app.auto_roll(&id)?))
}

async fn undo(State(app): State<SharedApp>, Path(id): Path<String>) -> ApiResult<Json<CombatView>> {
    Ok(Json(app.undo(&id)?))
}

async fn combat_log(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(app.with(&id, |s, _| Ok(with_session!(s, s => s.log())))?)
}

/// Re-derives the state from the stored log and compares fingerprints.
async fn verify_replay(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(app.with(&id, |s, _| with_session!(s, s => s.replay_check()))?)
}

/// System-neutral list of legal actions (`ActionView`s).
async fn actions(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let snap = app.snapshot(&id)?;
    with_snapshot!(snap, S, s => ok(S::action_views(&s)))
}

/// PF2e legal actions with their action cost (Strides are listed by `movement`).
async fn legal_actions(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let s = match app.snapshot(&id)? {
        Snapshot::Pf2e(s) => s,
        other => return with_snapshot!(other, S, s => ok(S::action_views(&s))),
    };
    let Some(actor) = s.current_actor() else { return ok(json!([])) };
    let list: Vec<Value> = s
        .legal_actions()
        .into_iter()
        .filter(|a| !matches!(a, Action::Stride { .. }))
        .map(|a| {
            let cost = s.check_action(actor, &a).unwrap_or(1);
            json!({ "action": a, "description": describe_action(&s, actor, &a), "cost": cost })
        })
        .collect();
    ok(list)
}

async fn movement(State(app): State<SharedApp>, Path((id, combatant)): Path<(String, u32)>) -> J {
    let cid = CombatantId(combatant);
    match app.snapshot(&id)? {
        Snapshot::Pf2e(s) => {
            if s.combatant(cid).is_none() {
                return Err(ApiError::not_found(format!("combatant {combatant}")));
            }
            ok(s.movement(cid).destinations().map(|(p, cost)| json!({ "x": p.x, "y": p.y, "cost": cost })).collect::<Vec<_>>())
        }
        Snapshot::Cpred(s) => {
            if s.combatant(cid).is_none() {
                return Err(ApiError::not_found(format!("combatant {combatant}")));
            }
            ok(s.movement(cid).destinations.iter().map(|(p, cost)| json!({ "x": p.x, "y": p.y, "cost": cost })).collect::<Vec<_>>())
        }
    }
}

// ----- analysis (system-neutral; optional AI layer) -----------------------------------

async fn evaluation(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let snap = app.snapshot(&id)?;
    blocking(move || with_snapshot!(snap, S, s => ok(tactics_ai::HeuristicEvaluator::<S>::default().evaluate_detailed(&s)))).await?
}

#[derive(Deserialize, Default)]
struct BestTurnReq {
    #[serde(default)]
    node_budget: Option<usize>,
    #[serde(default)]
    focus: Option<CombatantId>,
    /// Advisory mode: allow "end the turn" to compete with acting.
    #[serde(default)]
    allow_end_turn: bool,
}

async fn best_turn(State(app): State<SharedApp>, Path(id): Path<String>, body: Option<Json<BestTurnReq>>) -> J {
    let snap = app.snapshot(&id)?;
    let req = body.map(|b| b.0).unwrap_or_default();
    let cfg = SearchConfig { node_budget: req.node_budget.unwrap_or(40_000).min(400_000), focus: req.focus, allow_end_turn: req.allow_end_turn, ..Default::default() };
    blocking(move || {
        with_snapshot!(snap, S, s => match tactics_ai::recommend_turn::<S>(&s, cfg) {
            Some(r) => ok(r),
            None => Err(ApiError::conflict("no creature is ready to act (combat not running or input pending)")),
        })
    })
    .await?
}

#[derive(Deserialize)]
struct TargetsQuery {
    attacker: Option<u32>,
}

async fn targets(State(app): State<SharedApp>, Path(id): Path<String>, Query(q): Query<TargetsQuery>) -> J {
    let snap = app.snapshot(&id)?;
    with_snapshot!(snap, S, s => {
        let attacker = q.attacker.map(CombatantId).or(S::current_actor(&s)).ok_or_else(|| ApiError::bad_request("specify ?attacker="))?;
        ok(tactics_ai::rank_targets::<S>(&s, attacker))
    })
}

async fn spells(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    match app.snapshot(&id)? {
        Snapshot::Pf2e(s) => ok(blocking(move || rank_spells(&s)).await?),
        // Systems without spellcasting have nothing to rank.
        _ => ok(json!([])),
    }
}

#[derive(Deserialize, Default)]
struct TpkReq {
    runs: Option<u32>,
    seed: Option<u64>,
    max_rounds: Option<u32>,
}

async fn tpk(State(app): State<SharedApp>, Path(id): Path<String>, body: Option<Json<TpkReq>>) -> J {
    let snap = app.snapshot(&id)?;
    let r = body.map(|b| b.0).unwrap_or_default();
    let cfg = SimulationConfig { runs: r.runs.unwrap_or(200).clamp(1, 2000), seed: r.seed.unwrap_or(1), max_rounds: r.max_rounds.unwrap_or(20).clamp(1, 100) };
    blocking(move || with_snapshot!(snap, S, s => ok(tactics_ai::simulate::<S>(&s, cfg)))).await?
}

#[derive(Deserialize)]
struct StrategyQuery {
    side: Option<Side>,
    node_budget: Option<usize>,
}

async fn strategy(State(app): State<SharedApp>, Path(id): Path<String>, Query(q): Query<StrategyQuery>) -> J {
    let snap = app.snapshot(&id)?;
    let cfg = SearchConfig { node_budget: q.node_budget.unwrap_or(15_000).min(200_000), ..Default::default() };
    let side = q.side.unwrap_or(Side::Enemy);
    blocking(move || with_snapshot!(snap, S, s => ok(tactics_ai::side_strategy::<S>(&s, side, cfg)))).await?
}
