//! End-to-end API tests through the real router (no network).

use crate::app::App;
use crate::{routes, seed};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pf2e_core::builder::content::Content;
use pf2e_store::SqliteStore;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn app() -> axum::Router {
    let app = Arc::new(App::new(SqliteStore::in_memory().unwrap(), Content::bundled().unwrap()));
    seed::seed(&app).unwrap();
    routes::router(app)
}

async fn call(r: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty))
        .unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test]
async fn health_and_catalogues() {
    let r = app();
    let (s, v) = call(&r, "GET", "/api/health", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["status"], "ok");
    let (_, chars) = call(&r, "GET", "/api/characters", None).await;
    assert_eq!(chars.as_array().unwrap().len(), 4);
    assert!(chars.as_array().unwrap().iter().all(|c| c["is_legal"] == true));
    let (_, monsters) = call(&r, "GET", "/api/monsters", None).await;
    assert!(monsters.as_array().unwrap().len() >= 6);
}

#[tokio::test]
async fn validate_reports_errors() {
    let r = app();
    let (_, ch) = call(&r, "GET", "/api/characters/sample-fighter", None).await;
    let mut bad = ch.clone();
    bad["class_id"] = json!("bard");
    let (s, v) = call(&r, "POST", "/api/characters/validate", Some(bad)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["report"]["issues"].as_array().unwrap().iter().any(|i| i["code"] == "unknown_class"));
    assert!(v["statblock"].is_null());
}

#[tokio::test]
async fn elite_template_and_import() {
    let r = app();
    let (s, v) = call(&r, "POST", "/api/monsters/bandit-thug/template", Some(json!({"template": "elite", "save": true}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["statblock"]["level"], 2);
    assert_eq!(v["statblock"]["ac"], 18);
    let (s, _) = call(&r, "GET", "/api/monsters/bandit-thug-elite", None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, base) = call(&r, "GET", "/api/monsters/goblin-cutter", None).await;
    let (s, imported) = call(&r, "POST", "/api/monsters/import", Some(base["statblock"].clone())).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(imported[0]["statblock"]["name"], "Goblin Cutter");
}

#[tokio::test]
async fn full_combat_flow_with_replay_and_undo() {
    let r = app();
    let (s, b) = call(&r, "GET", "/api/encounters/sample-crypt-ambush/budget", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["party_size"], 4);
    assert_eq!(b["threat"], "moderate", "{b}");

    let (s, view) = call(&r, "POST", "/api/combats", Some(json!({"encounter_id": "sample-crypt-ambush", "dice": {"mode": "seeded", "seed": 7}}))).await;
    assert_eq!(s, StatusCode::OK, "{view}");
    let id = view["id"].as_str().unwrap().to_string();
    let names: Vec<_> = view["state"]["combatants"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"Goblin Cutter 1".to_string()) && names.contains(&"Goblin Cutter 2".to_string()));

    let (s, v) = call(&r, "POST", &format!("/api/combats/{id}/commands"), Some(json!({"type": "start_combat"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["pending"]["type"], "roll");
    assert!(v["pending"]["prompt"].as_str().unwrap().starts_with("Roll 1d20+"));

    // Manual roll for the first initiative, seeded auto-roll for the rest.
    let rid = v["pending"]["request_id"].as_u64().unwrap();
    let (s, _) = call(&r, "POST", &format!("/api/combats/{id}/commands"), Some(json!({"type": "provide_roll", "request_id": rid, "value": 25}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "natural 25 is impossible");
    call(&r, "POST", &format!("/api/combats/{id}/commands"), Some(json!({"type": "provide_roll", "request_id": rid, "value": 14}))).await;
    let mut view = Value::Null;
    for _ in 0..6 {
        let (s, v) = call(&r, "POST", &format!("/api/combats/{id}/auto-roll"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        view = v;
    }
    assert_eq!(view["state"]["phase"]["kind"], "active");
    assert!(view["current_actor"].is_number());

    let (s, la) = call(&r, "GET", &format!("/api/combats/{id}/legal-actions"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!la.as_array().unwrap().is_empty());

    let (_, ev) = call(&r, "GET", &format!("/api/combats/{id}/analysis/evaluation"), None).await;
    assert!(ev["score"].as_f64().unwrap().abs() <= 10.0);
    let (s, bt) = call(&r, "POST", &format!("/api/combats/{id}/analysis/best-turn"), Some(json!({"node_budget": 3000}))).await;
    assert_eq!(s, StatusCode::OK, "{bt}");
    assert!(bt["summary"].is_string());
    assert!(bt["report"]["text"].as_str().unwrap().contains("RECOMMENDED TURN"), "{bt}");
    assert!(!bt["report"]["primary"].as_array().unwrap().is_empty(), "the acting creature has legal actions: {bt}");

    let (_, rp) = call(&r, "GET", &format!("/api/combats/{id}/replay"), None).await;
    assert_eq!(rp["identical"], true);
    assert_eq!(rp["commands"], 8);

    let (_, before) = call(&r, "GET", &format!("/api/combats/{id}"), None).await;
    let (s, after) = call(&r, "POST", &format!("/api/combats/{id}/undo"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(after["command_count"], before["command_count"].as_u64().unwrap() - 1);
    assert_eq!(after["pending"]["type"], "roll");

    let (s, tpk) = call(&r, "POST", &format!("/api/combats/{id}/analysis/tpk"), Some(json!({"runs": 10, "seed": 3}))).await;
    assert_eq!(s, StatusCode::OK, "{tpk}");
    let p = tpk["tpk_probability"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&p));
}

#[tokio::test]
async fn manual_combat_refuses_auto_roll() {
    let r = app();
    let (_, view) = call(&r, "POST", "/api/combats", Some(json!({"encounter_id": "sample-crypt-ambush"}))).await;
    let id = view["id"].as_str().unwrap();
    call(&r, "POST", &format!("/api/combats/{id}/commands"), Some(json!({"type": "start_combat"}))).await;
    let (s, _) = call(&r, "POST", &format!("/api/combats/{id}/auto-roll"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn systems_are_listed() {
    let r = app();
    let (s, v) = call(&r, "GET", "/api/systems", None).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<_> = v.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids, vec!["pf2e", "cpred"]);
    let (s, c) = call(&r, "GET", "/api/systems/cpred/content", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!c["weapons"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn cyberpunk_red_documents_are_validated() {
    let r = app();
    let (_, actors) = call(&r, "GET", "/api/systems/cpred/actors", None).await;
    assert!(actors.as_array().unwrap().len() >= 8, "{actors}");
    let (s, vex) = call(&r, "GET", "/api/systems/cpred/actors/vex", None).await;
    assert_eq!(s, StatusCode::OK);
    let mut copy = vex.clone();
    copy["id"] = json!("vex-2");
    copy["name"] = json!("Vex II");
    let (s, saved) = call(&r, "POST", "/api/systems/cpred/actors", Some(copy.clone())).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    let (s, _) = call(&r, "POST", "/api/systems/cpred/actors", Some(copy)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, bad) = call(&r, "POST", "/api/systems/cpred/actors", Some(json!({"name": "Nobody"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    // An encounter naming an unknown actor cannot be saved.
    let (_, enc) = call(&r, "GET", "/api/systems/cpred/encounters/warehouse-standoff", None).await;
    let mut broken = enc.clone();
    broken["id"] = json!("broken");
    broken["participants"][0]["actor_id"] = json!("ghost");
    let (s, _) = call(&r, "POST", "/api/systems/cpred/encounters", Some(broken)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&r, "GET", "/api/systems/pf2e/actors", None).await;
    assert_eq!(s, StatusCode::OK, "listing is harmless for any system");
    let (_, d) = call(&r, "DELETE", "/api/systems/cpred/actors/vex-2", None).await;
    assert_eq!(d["deleted"], true);
}

#[tokio::test]
async fn cyberpunk_red_combat_end_to_end() {
    let r = app();
    let (s, view) = call(
        &r,
        "POST",
        "/api/combats",
        Some(json!({"system": "cpred", "encounter_id": "warehouse-standoff", "dice": {"mode": "seeded", "seed": 11}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{view}");
    assert_eq!(view["system"], "cpred");
    assert_eq!(view["table"]["map"]["square"], "2 m");
    let id = view["id"].as_str().unwrap().to_string();

    let (s, v) = call(&r, "POST", &format!("/api/combats/{id}/commands"), Some(json!({"type": "start_combat"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["prompt"]["input"]["kind"], "die");
    assert_eq!(v["prompt"]["input"]["sides"], 10);
    // A PF2e command is rejected, not misread.
    let (s, _) = call(&r, "POST", &format!("/api/combats/{id}/commands"), Some(json!({"type": "provide_roll", "request_id": 999, "value": 25}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let mut view = v;
    for _ in 0..9 {
        let (s, v) = call(&r, "POST", &format!("/api/combats/{id}/auto-roll"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        view = v;
    }
    assert_eq!(view["table"]["phase"]["kind"], "active", "{}", view["table"]);
    assert!(view["log"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().starts_with("Initiative:")), "{}", view["log"]);

    let (_, actions) = call(&r, "GET", &format!("/api/combats/{id}/actions"), None).await;
    let actions = actions.as_array().unwrap();
    assert!(actions.iter().any(|a| a["group"] == "Movement" && a["move_to"].is_object()));

    let (_, ev) = call(&r, "GET", &format!("/api/combats/{id}/analysis/evaluation"), None).await;
    assert!(ev["score"].as_f64().unwrap().abs() <= 10.0);
    assert!(ev["factors"].as_array().unwrap().iter().any(|f| f["name"] == "Cover"));
    let (s, bt) = call(&r, "POST", &format!("/api/combats/{id}/analysis/best-turn"), Some(json!({"node_budget": 4000}))).await;
    assert_eq!(s, StatusCode::OK, "{bt}");
    assert!(bt["report"]["text"].as_str().unwrap().contains("RECOMMENDED TURN"));
    assert!(!bt["report"]["primary"].as_array().unwrap().is_empty(), "{bt}");

    // Play the recommended first action for real.
    let actor = view["current_actor"].clone();
    let first = bt["plan"]["actions"][0].clone();
    let (s, v) = call(&r, "POST", &format!("/api/combats/{id}/commands"), Some(json!({"type": "act", "actor": actor, "action": first}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (_, st) = call(&r, "GET", &format!("/api/combats/{id}/analysis/strategy?side=enemy&node_budget=2000"), None).await;
    assert_eq!(st["creatures"].as_array().unwrap().len(), 5, "{st}");
    let (_, sp) = call(&r, "GET", &format!("/api/combats/{id}/analysis/spells"), None).await;
    assert_eq!(sp, json!([]));
    let (s, tpk) = call(&r, "POST", &format!("/api/combats/{id}/analysis/tpk"), Some(json!({"runs": 5, "seed": 2}))).await;
    assert_eq!(s, StatusCode::OK, "{tpk}");

    let (_, rp) = call(&r, "GET", &format!("/api/combats/{id}/replay"), None).await;
    assert_eq!(rp["identical"], true, "{rp}");
    let (_, before) = call(&r, "GET", &format!("/api/combats/{id}"), None).await;
    let (s, after) = call(&r, "POST", &format!("/api/combats/{id}/undo"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(after["command_count"], before["command_count"].as_u64().unwrap() - 1);

    let (_, list) = call(&r, "GET", "/api/combats", None).await;
    assert!(list.as_array().unwrap().iter().any(|c| c["id"] == id.as_str() && c["system"] == "cpred"));
}
