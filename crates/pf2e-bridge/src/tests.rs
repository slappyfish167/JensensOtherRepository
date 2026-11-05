//! End-to-end bridge tests: snapshot → state → reports.

use crate::api::{Bridge, ErrorCode, ForecastOptions, TurnOptions};
use crate::convert::snapshot_to_state;
use crate::fixtures::{crypt_ambush_active, snapshot_from_state};
use crate::snapshot::*;
use pf2e_ai::evaluation::{Evaluator, HeuristicEvaluator};
use pf2e_ai::search::Unlimited;
use pf2e_core::combat::CombatantKind;
use pf2e_core::stats::Side;

/// Minimal combatant for geometry tests.
pub fn combatant(name: &str, x: i32, y: i32) -> CombatantSnapshot {
    CombatantSnapshot {
        token_id: format!("{name}-{x}-{y}"),
        actor_id: String::new(),
        name: name.into(),
        kind: CombatantKind::Monster,
        side: Side::Enemy,
        x,
        y,
        level: 1,
        size: Default::default(),
        traits: vec![],
        hp: 20,
        max_hp: 20,
        temp_hp: 0,
        ac: 16,
        fortitude: 6,
        reflex: 6,
        will: 6,
        perception: 6,
        speed: 25,
        attributes: Default::default(),
        skills: Default::default(),
        strikes: vec![StrikeSnapshot {
            name: "Claw".into(),
            attack_bonus: 8,
            damage_formula: "(1d6 + 3)[slashing]".into(),
            damage_type: None,
            traits: vec!["agile".into()],
            melee: true,
            reach_ft: Some(5),
            range_increment_ft: None,
            attribute: pf2e_core::stats::Attribute::Strength,
            damage_includes_conditions: false,
        }],
        spellcasting: vec![],
        iwr: Default::default(),
        reactions: Default::default(),
        shield: None,
        conditions: vec![],
        persistent: vec![],
        class_dc: None,
        sneak_attack: None,
        initiative: None,
        hazard: None,
        reaction_used: false,
        uses_dying_rules: false,
    }
}

fn crypt_snapshot() -> CombatSnapshot {
    snapshot_from_state(&crypt_ambush_active(7))
}

#[test]
fn round_trip_preserves_the_position() {
    let original = crypt_ambush_active(7);
    let snap = snapshot_from_state(&original);
    let conv = snapshot_to_state(&snap).expect("converts");
    assert!(conv.turn_active);
    assert_eq!(conv.state.combatants.len(), original.combatants.len());
    assert_eq!(conv.state.current_actor(), original.current_actor());
    for (a, b) in original.combatants.iter().zip(&conv.state.combatants) {
        assert_eq!((a.name.as_str(), a.position, a.hp, a.actions_remaining), (b.name.as_str(), b.position, b.hp, b.actions_remaining));
        assert_eq!(a.statblock.strikes.len(), b.statblock.strikes.len(), "{}", a.name);
        assert_eq!(a.spell_slots, b.spell_slots, "{}", a.name);
    }
    let ev = HeuristicEvaluator::default();
    let (x, y) = (ev.evaluate(&original), ev.evaluate(&conv.state));
    assert!((x - y).abs() < 0.05, "evaluation drifted in conversion: {x} vs {y}");
}

#[test]
fn bridge_answers_every_analysis() {
    let mut b = Bridge::new();
    let snap = crypt_snapshot();
    let summary: serde_json::Value = serde_json::from_str(&b.load(&serde_json::to_string(&snap).unwrap()).unwrap()).unwrap();
    assert_eq!(summary["turn_active"], true);

    let ev = b.evaluation().unwrap();
    assert!((-10.0..=10.0).contains(&ev.score));
    assert!(!ev.band.is_empty());

    let cur = snap.turn.current.clone().unwrap();
    let turn = b.best_turn_report(&cur, TurnOptions::default(), &Unlimited).unwrap();
    assert_eq!(turn.token_id, cur);
    assert!(!turn.hypothetical);
    assert!(!turn.steps.is_empty(), "{}", turn.summary);
    for s in &turn.steps {
        if let Some(to) = &s.to {
            assert_eq!(to.px, [to.square[0] as f64 * 100.0, to.square[1] as f64 * 100.0]);
        }
    }
    assert!(!turn.reasoning.is_empty());

    // A creature whose turn it is not: hypothetical turn.
    let other = snap.combatants.iter().find(|c| c.token_id != cur && c.kind != CombatantKind::Hazard).unwrap();
    let hypo = b.best_turn_report(&other.token_id, TurnOptions::default(), &Unlimited).unwrap();
    assert!(hypo.hypothetical);

    let plan = b.side_plan_report(Side::Enemy, TurnOptions { node_budget: Some(4000), beam: None }, &crate::api::unlimited).unwrap();
    assert_eq!(plan.creatures.len(), 3);
    assert!(plan.focus_token.is_some());

    let slices: Vec<_> = [(0, 20), (20, 20)]
        .iter()
        .map(|&(start, count)| {
            let o = ForecastOptions { runs: 40, seed: 5, max_rounds: 10, start, count: Some(count) };
            serde_json::from_str(&b.forecast_slice(&serde_json::to_string(&o).unwrap()).unwrap()).unwrap()
        })
        .collect();
    let f = b.merge_forecast_report(&slices).unwrap();
    assert_eq!(f.runs, 40);
    assert!((f.victory + f.defeat + f.unresolved - 1.0).abs() < 1e-9);

    let ex = b.explain_report(5).unwrap();
    assert!(ex.previous_score.is_none(), "first snapshot explains the position itself");

    // Second snapshot: an enemy took damage → explain the change.
    let mut snap2 = snap.clone();
    let gob = snap2.combatants.iter_mut().find(|c| c.name.starts_with("Goblin Cutter 1")).unwrap();
    gob.hp = 1;
    b.load(&serde_json::to_string(&snap2).unwrap()).unwrap();
    let ex = b.explain_report(5).unwrap();
    assert!(ex.delta > 0.0);
    assert!(ex.reasons[0].text.contains("Goblin Cutter 1"), "{:?}", ex.reasons);
    assert_eq!(ex.reasons[0].token_id.as_deref(), Some(gob_token(&snap2).as_str()));
}

fn gob_token(s: &CombatSnapshot) -> String {
    s.combatants.iter().find(|c| c.name.starts_with("Goblin Cutter 1")).unwrap().token_id.clone()
}

#[test]
fn errors_carry_stable_codes() {
    let mut b = Bridge::new();
    assert_eq!(b.evaluate().unwrap_err().code, ErrorCode::NoSnapshot);
    assert_eq!(b.load("{not json").unwrap_err().code, ErrorCode::InvalidJson);
    let mut snap = crypt_snapshot();
    snap.version = 99;
    assert_eq!(b.load(&serde_json::to_string(&snap).unwrap()).unwrap_err().code, ErrorCode::UnsupportedVersion);
    let mut snap = crypt_snapshot();
    snap.scene.grid = GridKind::Hex;
    assert_eq!(b.load(&serde_json::to_string(&snap).unwrap()).unwrap_err().code, ErrorCode::UnsupportedScene);
    let mut snap = crypt_snapshot();
    snap.combatants.push(snap.combatants[0].clone());
    assert_eq!(b.load(&serde_json::to_string(&snap).unwrap()).unwrap_err().code, ErrorCode::InvalidSnapshot);
    b.load(&serde_json::to_string(&crypt_snapshot()).unwrap()).unwrap();
    assert_eq!(b.best_turn("nope", "", &Unlimited).unwrap_err().code, ErrorCode::UnknownToken);
    let json = b.best_turn("nope", "", &Unlimited).unwrap_err().to_json();
    assert!(json.contains("\"code\":\"unknown_token\""), "{json}");
}

#[test]
fn live_state_is_applied() {
    let mut snap = crypt_snapshot();
    let i = snap.combatants.iter().position(|c| c.name.starts_with("Bone Sentinel")).unwrap();
    snap.combatants[i].conditions = vec![
        ConditionSnapshot { slug: "frightened".into(), value: Some(2) },
        ConditionSnapshot { slug: "off-guard".into(), value: None },
        ConditionSnapshot { slug: "confused".into(), value: None },
    ];
    snap.combatants[i].persistent = vec![PersistentSnapshot { formula: "1d6".into(), damage_type: "fire".into() }];
    snap.combatants[i].reaction_used = true;
    let conv = snapshot_to_state(&snap).unwrap();
    let c = &conv.state.combatants[i];
    use pf2e_core::conditions::ConditionKind as CK;
    assert_eq!(c.conditions.value(CK::Frightened), 2);
    assert!(c.has(CK::OffGuard));
    assert_eq!(c.persistent.len(), 1);
    assert!(!c.reaction_available);
    assert!(conv.warnings.iter().any(|w| w.contains("confused")), "{:?}", conv.warnings);
}

#[test]
fn dead_npcs_are_dropped_and_stacked_tokens_nudged() {
    let mut snap = crypt_snapshot();
    let n = snap.combatants.len();
    let mut corpse = snap.combatants.iter().find(|c| c.kind == CombatantKind::Monster).unwrap().clone();
    corpse.token_id = "corpse".into();
    corpse.hp = 0;
    snap.combatants.push(corpse);
    let mut stacked = snap.combatants[0].clone();
    stacked.token_id = "stacked".into();
    stacked.name = "Stacked".into();
    snap.combatants.push(stacked);
    let conv = snapshot_to_state(&snap).unwrap();
    assert_eq!(conv.state.combatants.len(), n + 1);
    assert!(conv.ids.id("corpse").is_none());
    let s = conv.state.combatants.last().unwrap();
    assert_ne!(s.position, conv.state.combatants[0].position);
    assert!(conv.warnings.iter().any(|w| w.contains("Stacked")));
}

#[test]
fn foundry_walls_constrain_the_engine() {
    // Two creatures adjacent across a grid-aligned wall: no melee Strike.
    let mut a = combatant("Hero", 4, 4);
    a.side = Side::Party;
    a.kind = CombatantKind::Character;
    a.token_id = "hero".into();
    let mut b = combatant("Orc", 5, 4);
    b.token_id = "orc".into();
    let snap = CombatSnapshot {
        version: SNAPSHOT_VERSION,
        scene: SceneSnapshot {
            id: "s".into(),
            name: "walls".into(),
            grid: GridKind::Square,
            grid_size_px: 100.0,
            grid_distance_ft: 5.0,
            origin_px: [0.0, 0.0],
            width: 10,
            height: 10,
            walls: vec![WallSnapshot { a: [5.0, 0.0], b: [5.0, 10.0], blocks_movement: true, blocks_sight: true, open_door: false }],
            terrain: vec![],
        },
        combatants: vec![a, b],
        turn: TurnSnapshot { started: true, round: 1, current: Some("hero".into()), order: vec!["hero".into(), "orc".into()], actions_spent: 0, attacks_made: 0 },
        options: SnapshotOptions::default(),
    };
    let conv = snapshot_to_state(&snap).unwrap();
    assert!(!conv.state.battlefield.edges.is_empty());
    let legal = conv.state.legal_actions();
    assert!(!legal.iter().any(|x| matches!(x, pf2e_core::combat::Action::Strike { .. })), "struck through a wall: {legal:?}");
}

/// Regenerates the cross-language fixture used by the TypeScript tests:
/// `PF2E_WRITE_FIXTURE=1 cargo test -p pf2e-bridge write_fixture`.
#[test]
fn write_fixture() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../foundry/test/fixtures/crypt-ambush.snapshot.json");
    let json = serde_json::to_string_pretty(&crypt_snapshot()).unwrap() + "\n";
    if std::env::var_os("PF2E_WRITE_FIXTURE").is_some() {
        std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
        std::fs::write(path, &json).unwrap();
    }
    if let Ok(existing) = std::fs::read_to_string(path) {
        // The committed fixture must still parse and convert.
        let snap: CombatSnapshot = serde_json::from_str(&existing).expect("fixture matches the snapshot schema");
        snapshot_to_state(&snap).expect("fixture converts");
    }
}
