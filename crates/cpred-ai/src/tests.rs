use crate::CyberpunkRed;
use cpred_core::combat::{Action, CombatSetup, CombatState, Command, PendingInput, SetupParticipant};
use cpred_core::content::Content;
use cpred_core::model::{ActorKind, Battlefield};
use tactics_ai::{best_turn, recommend_turn, side_strategy, simulate, Evaluator, HeuristicEvaluator, SearchConfig, SimulationConfig, TacticalSystem};
use tactics_core::{CombatantId, GameSystem, Pos, SeededDice, Side};

fn content() -> Content {
    Content::bundled().unwrap()
}

fn part(c: &Content, actor: &str, name: &str, side: Side, x: i32, y: i32, hp: Option<i32>) -> SetupParticipant {
    let a = c.actor(actor).unwrap_or_else(|| panic!("no actor {actor}"));
    SetupParticipant { name: name.into(), side, kind: a.kind, statblock: a.statblock.clone(), position: Pos::new(x, y), current_hp: hp }
}

fn map(rows: &[&str]) -> Battlefield {
    Battlefield { width: rows[0].len() as u32, height: rows.len() as u32, map: rows.iter().map(|r| r.to_string()).collect(), notes: String::new() }
}

/// Starts the combat, answering initiative with the given natural d10s (in
/// prompt order), and returns the state at the first turn.
fn start(mut s: CombatState, faces: &[i32]) -> CombatState {
    s.apply(&Command::StartCombat).unwrap();
    for &f in faces {
        let Some(PendingInput::Roll(r)) = s.pending() else { panic!("expected an initiative roll") };
        s.apply(&Command::roll(r.request_id, f)).unwrap();
    }
    assert!(s.current_actor().is_some(), "combat started");
    s
}

const FORBIDDEN: [&str; 4] = ["no action improves", "no move improves", "position is equal", "no recommendation"];

#[test]
fn hostile_turn_is_committed_and_fully_reported() {
    let c = content();
    let s = CombatSetup::build(
        map(&["..............", "..............", "..............", "..............", ".............."]),
        vec![
            part(&c, "corp-security", "Corp Security", Side::Enemy, 10, 2, None),
            part(&c, "moth", "Moth", Side::Party, 3, 1, Some(9)),
            part(&c, "rook-haldane", "Rook", Side::Party, 3, 3, None),
        ],
        &c,
    )
    .unwrap();
    let s = start(s, &[10, 1, 1]);
    assert_eq!(s.current_actor(), Some(CombatantId(1)), "the corp goon acts first");
    let rec = recommend_turn::<CyberpunkRed>(&s, SearchConfig::default()).expect("a turn");
    let r = &rec.report;
    for t in [&rec.summary, &r.text, &r.reason, &r.position] {
        for f in FORBIDDEN {
            assert!(!t.to_lowercase().contains(f), "forbidden wording {f:?} in {t}");
        }
    }
    assert!(!rec.plan.actions.is_empty(), "{}", r.text);
    assert!(rec.plan.actions.iter().any(|a| a.attack().is_some()), "the goon shoots: {}", r.text);
    assert_eq!(r.priority_target, Some(CombatantId(2)), "the wounded, lightly armored PC is the priority: {}", r.text);
    assert!(r.expected_damage > 0.0 && r.down_chance > 0.0, "{}", r.text);
    assert!(!r.alternate.is_empty(), "{}", r.text);
    assert!(r.position.ends_with("Advantage"), "{}", r.position);
    // Every planned action is legal when replayed along the most likely line.
    let mut st = s.clone();
    for a in &rec.plan.actions {
        assert!(st.check_action(CombatantId(1), a).is_ok() || st.pending().is_some(), "{a:?} legal");
        if st.pending().is_some() {
            break;
        }
        st.apply(&Command::Act { actor: CombatantId(1), action: a.clone() }).unwrap();
        while let Some(PendingInput::Roll(r)) = st.pending() {
            let (cmd, _) = cpred_core::analysis::representative_answers(&st).into_iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
            let _ = r;
            st.apply(&cmd).unwrap();
        }
        if let Some(PendingInput::Decision(d)) = st.pending() {
            st.apply(&Command::Decide { request_id: d.request_id, accept: false }).unwrap();
        }
    }
}

#[test]
fn a_shooter_with_no_line_of_sight_still_moves_to_shoot() {
    let c = content();
    let s = CombatSetup::build(
        map(&["...........", "...........", ".....#.....", ".....#.....", ".....#.....", "..........."]),
        // An assault rifle against Kevlar: worth getting a clear shot.
        vec![part(&c, "corp-security", "Corp Security", Side::Enemy, 7, 3, None), part(&c, "moth", "Moth", Side::Party, 3, 3, None)],
        &c,
    )
    .unwrap();
    let s = start(s, &[10, 1]);
    assert!(!s.line_of_sight(Pos::new(7, 3), Pos::new(3, 3)));
    let plan = best_turn::<CyberpunkRed>(&s, &HeuristicEvaluator::<CyberpunkRed>::default(), SearchConfig::default()).unwrap();
    assert!(!plan.actions.is_empty());
    assert!(matches!(plan.actions[0], Action::Move { .. } | Action::Run { .. }), "{:?}", plan.actions);
    assert!(plan.actions.iter().any(|a| a.attack().is_some()), "moves around the wall and fires: {:?}", plan.actions);
}

#[test]
fn after_shooting_the_search_may_hold_or_take_cover() {
    let c = content();
    // Cover square at (5,2) between the ganger's alternatives and the PC.
    let s = CombatSetup::build(
        map(&["...........", "...........", ".....%.....", "...........", "..........."]),
        vec![part(&c, "street-ganger", "Ganger", Side::Enemy, 6, 1, None), part(&c, "vex", "Vex", Side::Party, 2, 2, None)],
        &c,
    )
    .unwrap();
    let s = start(s, &[10, 1]);
    let plan = best_turn::<CyberpunkRed>(&s, &HeuristicEvaluator::<CyberpunkRed>::default(), SearchConfig::default()).unwrap();
    assert!(plan.actions.iter().any(|a| a.attack().is_some()), "{:?}", plan.actions);
    // Whatever it does with the Move Action, it never ends exposed when cover was one step away.
    let mut st = s.clone();
    if let Some(to) = plan.actions.iter().rev().find_map(|a| a.move_to()) {
        st.combatant_mut(CombatantId(1)).unwrap().position = to;
        let vex = st.combatant(CombatantId(2)).unwrap().position;
        assert!(st.blocking_cover(vex, to).is_some() || !st.line_of_sight(vex, to), "moved into cover: {to:?} {:?}", plan.actions);
    }
}

#[test]
fn evaluation_is_antisymmetric_and_bounded() {
    let c = content();
    let build = |a: &str, b: &str| {
        CombatSetup::build(
            map(&["..........", "..........", ".........."]),
            vec![part(&c, a, "A", Side::Party, 1, 1, None), part(&c, b, "B", Side::Enemy, 8, 1, None)],
            &c,
        )
        .unwrap()
    };
    let ev = HeuristicEvaluator::<CyberpunkRed>::default();
    let mirrored = build("corp-security", "corp-security");
    assert!(ev.evaluate(&mirrored).abs() < 1e-6);
    let strong = ev.evaluate(&build("vex", "street-ganger"));
    let weak = ev.evaluate(&build("street-ganger", "vex"));
    assert!(strong > 0.5 && strong <= 10.0, "{strong}");
    assert!((strong + weak).abs() < 1e-6, "{strong} vs {weak}");
    let detail = ev.evaluate_detailed(&mirrored);
    assert!(detail.factors.iter().any(|f| f.name == "Cover"));
}

#[test]
fn representative_answers_drive_the_search_through_every_roll() {
    // A whole committed turn of every combatant in the sample encounter.
    let c = content();
    let mut s = c.build_encounter("warehouse-standoff").unwrap();
    let mut dice = SeededDice::new(5);
    s.apply(&Command::StartCombat).unwrap();
    while s.current_actor().is_none() {
        let cmd = CyberpunkRed::auto_answer(&s, &mut dice).unwrap();
        s.apply(&cmd).unwrap();
    }
    for _ in 0..4 {
        if s.is_over() {
            break;
        }
        let actor = s.current_actor().unwrap();
        let rec = recommend_turn::<CyberpunkRed>(&s, SearchConfig { node_budget: 8_000, ..Default::default() }).unwrap();
        assert!(!rec.plan.actions.is_empty(), "{} has a turn: {}", s.name_of(actor), rec.report.text);
        // Play the first action for real with seeded dice, then end the turn.
        s.apply(&Command::Act { actor, action: rec.plan.actions[0].clone() }).unwrap();
        while s.pending().is_some() {
            let cmd = match CyberpunkRed::node(&s) {
                tactics_ai::Node::Decision { accept, .. } => accept,
                _ => CyberpunkRed::auto_answer(&s, &mut dice).unwrap(),
            };
            s.apply(&cmd).unwrap();
        }
        if s.current_actor() == Some(actor) {
            s.apply(&Command::EndTurn { actor }).unwrap();
        }
        while s.pending().is_some() {
            let cmd = CyberpunkRed::auto_answer(&s, &mut dice).unwrap();
            s.apply(&cmd).unwrap();
        }
    }
}

#[test]
fn enemy_strategy_covers_every_standing_npc() {
    let c = content();
    let mut s = c.build_encounter("warehouse-standoff").unwrap();
    let mut dice = SeededDice::new(2);
    s.apply(&Command::StartCombat).unwrap();
    while s.current_actor().is_none() {
        let cmd = CyberpunkRed::auto_answer(&s, &mut dice).unwrap();
        s.apply(&cmd).unwrap();
    }
    let strat = side_strategy::<CyberpunkRed>(&s, Side::Enemy, SearchConfig { node_budget: 3_000, ..Default::default() });
    assert_eq!(strat.creatures.len(), 5);
    assert!(strat.focus_target.is_some());
    assert!(strat.creatures.iter().filter_map(|c| c.recommendation.as_ref()).all(|r| !r.plan.actions.is_empty()));
}

#[test]
fn monte_carlo_is_reproducible_and_ends_fights() {
    let c = content();
    let s = c.build_encounter("warehouse-standoff").unwrap();
    // Armor makes Cyberpunk RED fights long: allow 30 rounds.
    let cfg = SimulationConfig { runs: 12, seed: 3, max_rounds: 30 };
    let a = simulate::<CyberpunkRed>(&s, cfg);
    let b = simulate::<CyberpunkRed>(&s, cfg);
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    assert!(a.timeout_rate < 0.5, "simulated monsters actually fight: {a:?}");
    assert!((a.party_win_rate + a.enemy_win_rate + a.timeout_rate - 1.0).abs() < 1e-9);
}

#[test]
fn neutral_views_describe_the_table() {
    let c = content();
    let s = c.build_encounter("warehouse-standoff").unwrap();
    let t = CyberpunkRed::table(&s);
    assert_eq!(t.system, "cpred");
    assert_eq!(t.combatants.len(), 9);
    assert_eq!(t.map.square, "2 m");
    assert!(t.map.legend.iter().any(|l| l.glyph == "%" && l.class == "cover"));
    let mut s = s;
    s.apply(&Command::StartCombat).unwrap();
    let p = CyberpunkRed::prompt(&s).unwrap();
    assert!(matches!(p.input, Some(tactics_core::view::InputSpec::Die { sides: 10 })));
    let units = CyberpunkRed::units(&s);
    assert!(units.iter().filter(|u| u.is_pc).count() == 4 && units.iter().all(|u| u.kind_is_known()));
}

trait KnownKind {
    fn kind_is_known(&self) -> bool;
}

impl KnownKind for tactics_ai::Unit {
    fn kind_is_known(&self) -> bool {
        !self.ignored && self.max_hp > 0
    }
}

#[test]
fn kill_probability_reflects_wounds() {
    let c = content();
    let build = |hp| {
        start(
            CombatSetup::build(
                map(&["........", "........"]),
                vec![part(&c, "rook-haldane", "Rook", Side::Enemy, 2, 0, None), part(&c, "moth", "Moth", Side::Party, 3, 0, Some(hp))],
                &c,
            )
            .unwrap(),
            &[10, 1],
        )
    };
    let healthy = CyberpunkRed::kill_probability(&build(30), CombatantId(1), CombatantId(2));
    let wounded = CyberpunkRed::kill_probability(&build(4), CombatantId(1), CombatantId(2));
    assert!((0.0..=1.0).contains(&healthy) && wounded > healthy, "{healthy} {wounded}");
    let _ = ActorKind::Pc;
}

