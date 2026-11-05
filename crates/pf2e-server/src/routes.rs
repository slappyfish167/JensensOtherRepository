//! HTTP routes. See `docs/API.md` for the full specification.

use crate::app::{CombatView, SharedApp};
use crate::error::{ApiError, ApiResult};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use pf2e_ai::queries::{describe_action, rank_spells, rank_targets, recommend_turn, side_strategy};
use pf2e_ai::{simulate, HeuristicEvaluator, SearchConfig, SimulationConfig};
use pf2e_core::combat::{Action, Command};
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

// ----- combats ------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateCombat {
    encounter_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default = "manual")]
    dice: DiceMode,
}

fn manual() -> DiceMode {
    DiceMode::Manual
}

async fn create_combat(State(app): State<SharedApp>, Json(req): Json<CreateCombat>) -> J {
    let e = app.store().get_encounter(&req.encounter_id)?.ok_or_else(|| ApiError::not_found(format!("encounter '{}'", req.encounter_id)))?;
    let state = app.combat_state_for(&e)?;
    ok(app.create_combat(Some(e.id.clone()), req.name.unwrap_or(e.name.clone()), state, req.dice)?)
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

async fn post_command(State(app): State<SharedApp>, Path(id): Path<String>, Json(cmd): Json<Command>) -> ApiResult<Json<CombatView>> {
    Ok(Json(app.command(&id, cmd)?))
}

async fn auto_roll(State(app): State<SharedApp>, Path(id): Path<String>) -> ApiResult<Json<CombatView>> {
    Ok(Json(app.auto_roll(&id)?))
}

async fn undo(State(app): State<SharedApp>, Path(id): Path<String>) -> ApiResult<Json<CombatView>> {
    Ok(Json(app.undo(&id)?))
}

async fn combat_log(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    ok(app.log(&id)?)
}

/// Re-derives the state from the stored log and compares fingerprints.
async fn verify_replay(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let log = app.log(&id)?;
    let live = app.snapshot(&id)?;
    let (replayed, events) = pf2e_core::combat::replay(&log.initial, &log.commands).map_err(|e| ApiError::conflict(e.to_string()))?;
    ok(json!({
        "commands": log.commands.len(),
        "events": events.len(),
        "live_fingerprint": live.fingerprint(),
        "replayed_fingerprint": replayed.fingerprint(),
        "identical": replayed == live,
    }))
}

async fn legal_actions(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let s = app.snapshot(&id)?;
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
    let s = app.snapshot(&id)?;
    let cid = CombatantId(combatant);
    if s.combatant(cid).is_none() {
        return Err(ApiError::not_found(format!("combatant {combatant}")));
    }
    let squares: Vec<Value> = s.movement(cid).destinations().map(|(p, cost)| json!({ "x": p.x, "y": p.y, "cost": cost })).collect();
    ok(squares)
}

// ----- analysis --------------------------------------------------------------------

async fn evaluation(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let s = app.snapshot(&id)?;
    ok(blocking(move || HeuristicEvaluator::default().evaluate_detailed(&s)).await?)
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
    let s = app.snapshot(&id)?;
    let req = body.map(|b| b.0).unwrap_or_default();
    let cfg = SearchConfig { node_budget: req.node_budget.unwrap_or(40_000).min(400_000), focus: req.focus, allow_end_turn: req.allow_end_turn, ..Default::default() };
    let rec = blocking(move || recommend_turn(&s, cfg)).await?;
    match rec {
        Some(r) => ok(r),
        None => Err(ApiError::conflict("no creature is ready to act (combat not running or input pending)")),
    }
}

#[derive(Deserialize)]
struct TargetsQuery {
    attacker: Option<u32>,
}

async fn targets(State(app): State<SharedApp>, Path(id): Path<String>, Query(q): Query<TargetsQuery>) -> J {
    let s = app.snapshot(&id)?;
    let attacker = q.attacker.map(CombatantId).or(s.current_actor()).ok_or_else(|| ApiError::bad_request("specify ?attacker="))?;
    ok(blocking(move || rank_targets(&s, attacker)).await?)
}

async fn spells(State(app): State<SharedApp>, Path(id): Path<String>) -> J {
    let s = app.snapshot(&id)?;
    ok(blocking(move || rank_spells(&s)).await?)
}

#[derive(Deserialize, Default)]
struct TpkReq {
    runs: Option<u32>,
    seed: Option<u64>,
    max_rounds: Option<u32>,
}

async fn tpk(State(app): State<SharedApp>, Path(id): Path<String>, body: Option<Json<TpkReq>>) -> J {
    let s = app.snapshot(&id)?;
    let r = body.map(|b| b.0).unwrap_or_default();
    let cfg = SimulationConfig { runs: r.runs.unwrap_or(200).clamp(1, 2000), seed: r.seed.unwrap_or(1), max_rounds: r.max_rounds.unwrap_or(20).clamp(1, 100) };
    ok(blocking(move || simulate(&s, cfg)).await?)
}

#[derive(Deserialize)]
struct StrategyQuery {
    side: Option<Side>,
    node_budget: Option<usize>,
}

async fn strategy(State(app): State<SharedApp>, Path(id): Path<String>, Query(q): Query<StrategyQuery>) -> J {
    let s = app.snapshot(&id)?;
    let cfg = SearchConfig { node_budget: q.node_budget.unwrap_or(15_000).min(200_000), ..Default::default() };
    let side = q.side.unwrap_or(Side::Enemy);
    ok(blocking(move || side_strategy(&s, side, cfg)).await?)
}
