use crate::evaluation::{Evaluator, HeuristicEvaluator};
use crate::queries::{rank_spells, rank_targets, recommend_turn, side_strategy};
use crate::search::{best_turn, SearchConfig};
use crate::simulate::{simulate, SimulationConfig};
use pf2e_core::builder::content::Content;
use pf2e_core::combat::{Action, CombatSetup, CombatState, CombatantKind, Command, SetupParticipant};
use pf2e_core::damage::DamageType;
use pf2e_core::grid::Pos;
use pf2e_core::models::battlefield::Battlefield;
use pf2e_core::models::statblock::{DamageComponent, Statblock, StrikeProfile, StrikeRange};
use pf2e_core::stats::{Attribute, Side};
use pf2e_core::CombatantId;

fn sb(name: &str, hp: i32, ac: i32, attack: i32, damage: &str) -> Statblock {
    Statblock {
        name: name.into(),
        level: 1,
        traits: vec![],
        size: Default::default(),
        attributes: Default::default(),
        max_hp: hp,
        ac,
        fortitude: 5,
        reflex: 5,
        will: 5,
        perception: 0,
        speed: 25,
        skills: Default::default(),
        strikes: vec![StrikeProfile {
            name: "Blade".into(),
            attack_bonus: attack,
            damage: vec![DamageComponent { dice: damage.parse().unwrap(), damage_type: DamageType::Slashing, precision: false }],
            traits: vec![],
            range: StrikeRange::Melee { reach_ft: 5 },
            attribute: Attribute::Strength,
            strength_damage: true,
        }],
        spellcasting: vec![],
        iwr: Default::default(),
        reactions: vec![],
        shield: None,
        class_dc: None,
        damage_riders: vec![],
        initiative_bonus: 0,
        uses_dying_rules: false,
        hazard: None,
    }
}

fn p(sb: Statblock, side: Side, x: i32, y: i32) -> SetupParticipant {
    SetupParticipant {
        name: sb.name.clone(),
        side,
        kind: if side == Side::Party { CombatantKind::Character } else { CombatantKind::Monster },
        statblock: sb,
        position: Pos::new(x, y),
        current_hp: None,
    }
}

fn state(parts: Vec<SetupParticipant>) -> CombatState {
    CombatSetup::default().build(Battlefield::open(14, 14), parts, &Content::bundled().unwrap()).unwrap()
}

fn start(s: &mut CombatState, inits: &[i32]) {
    s.apply(&Command::StartCombat).unwrap();
    for &v in inits {
        let id = s.pending().unwrap().request_id();
        s.apply(&Command::ProvideRoll { request_id: id, value: v, extra_modifier: 0 }).unwrap();
    }
}

#[test]
fn mirrored_sides_evaluate_to_zero() {
    let s = state(vec![
        p(sb("A", 30, 18, 9, "1d8+4"), Side::Party, 2, 2),
        p(sb("B", 30, 18, 9, "1d8+4"), Side::Enemy, 8, 8),
    ]);
    let e = HeuristicEvaluator::default().evaluate_detailed(&s);
    assert!(e.score.abs() < 1e-6, "{}", e.score);
    assert!((e.win_probability - 0.5).abs() < 1e-6);
}

#[test]
fn stronger_party_scores_positive_and_bounded() {
    let s = state(vec![
        p(sb("A", 60, 20, 12, "2d8+6"), Side::Party, 2, 2),
        p(sb("B", 15, 14, 5, "1d6"), Side::Enemy, 8, 8),
    ]);
    let ev = HeuristicEvaluator::default();
    let score = ev.evaluate(&s);
    assert!(score > 5.0 && score <= 10.0, "{score}");
    let flipped = state(vec![
        p(sb("A", 15, 14, 5, "1d6"), Side::Party, 2, 2),
        p(sb("B", 60, 20, 12, "2d8+6"), Side::Enemy, 8, 8),
    ]);
    assert!((ev.evaluate(&flipped) + score).abs() < 1e-6, "evaluation is antisymmetric");
}

#[test]
fn search_finishes_the_wounded_enemy() {
    let mut s = state(vec![
        p(sb("Hero", 40, 18, 10, "1d8+4"), Side::Party, 5, 5),
        p(sb("Healthy", 40, 16, 8, "1d8+3"), Side::Enemy, 6, 5),
        p(sb("Wounded", 40, 16, 8, "1d8+3"), Side::Enemy, 4, 5),
    ]);
    start(&mut s, &[20, 1, 1]);
    s.combatants[2].hp = 4;
    let before = s.clone();
    let plan = best_turn(&s, &HeuristicEvaluator::default(), SearchConfig::default()).unwrap();
    assert_eq!(s, before, "search must not mutate the real state");
    match &plan.actions[0] {
        Action::Strike { target, .. } => assert_eq!(*target, CombatantId(3)),
        other => panic!("expected a Strike on the wounded enemy, got {other:?}"),
    }
    assert!(plan.gain > 0.0);
}

#[test]
fn search_closes_distance_when_out_of_reach() {
    let mut s = state(vec![
        p(sb("Hero", 40, 18, 10, "1d8+4"), Side::Party, 1, 1),
        p(sb("Orc", 30, 16, 8, "1d8+3"), Side::Enemy, 5, 1),
    ]);
    start(&mut s, &[20, 1]);
    let rec = recommend_turn(&s, SearchConfig::default()).unwrap();
    assert!(matches!(rec.plan.actions[0], Action::Stride { .. }), "{:?}", rec.steps);
    assert!(rec.plan.actions.iter().any(|a| matches!(a, Action::Strike { .. })), "{:?}", rec.steps);
}

#[test]
fn targets_ranked_by_threat_and_fragility() {
    let mut s = state(vec![
        p(sb("Hero", 40, 18, 10, "1d8+4"), Side::Party, 5, 5),
        p(sb("Brute", 60, 16, 12, "2d10+6"), Side::Enemy, 6, 5),
        p(sb("Weakling", 8, 14, 4, "1d4"), Side::Enemy, 4, 5),
        p(sb("Glass Cannon", 10, 14, 12, "2d10+6"), Side::Enemy, 5, 6),
    ]);
    start(&mut s, &[20, 1, 1, 1]);
    let ranks = rank_targets(&s, CombatantId(1));
    assert_eq!(ranks[0].name, "Glass Cannon", "{:?}", ranks.iter().map(|r| (&r.name, r.priority)).collect::<Vec<_>>());
    assert!(ranks.iter().all(|r| (0.0..=1.0).contains(&r.kill_probability)));
}

#[test]
fn monte_carlo_is_reproducible_and_sensible() {
    let s = state(vec![
        p(sb("Hero", 60, 20, 14, "2d8+6"), Side::Party, 2, 2),
        p(sb("Rat", 8, 12, 3, "1d4"), Side::Enemy, 6, 2),
    ]);
    let cfg = SimulationConfig { runs: 30, seed: 9, max_rounds: 10 };
    let a = simulate(&s, cfg);
    let b = simulate(&s, cfg);
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    assert!(a.party_win_rate > 0.9, "{a:?}");
    assert!(a.tpk_probability < 0.1);
}

#[test]
fn spell_ranking_for_a_caster() {
    let content = Content::bundled().unwrap();
    let adept = content.monsters.iter().find(|m| m.id == "cult-adept").unwrap().statblock.clone();
    let mut s = CombatSetup::default()
        .build(
            Battlefield::open(14, 14),
            vec![
                p(sb("Hero", 30, 16, 9, "1d8+4"), Side::Party, 2, 2),
                SetupParticipant { name: "Adept".into(), side: Side::Enemy, kind: CombatantKind::Monster, statblock: adept, position: Pos::new(6, 2), current_hp: None },
            ],
            &content,
        )
        .unwrap();
    start(&mut s, &[1, 20]);
    let spells = rank_spells(&s);
    assert!(!spells.is_empty());
    assert!(spells.windows(2).all(|w| w[0].eval_gain >= w[1].eval_gain));
    let lance = spells.iter().find(|x| x.description.contains("Divine Lance")).unwrap();
    assert!(lance.expected_enemy_damage > 0.0);
    let strat = side_strategy(&s, Side::Enemy, SearchConfig { node_budget: 5_000, ..Default::default() });
    assert_eq!(strat.focus_target, Some(CombatantId(1)));
    assert!(strat.creatures[0].recommendation.is_some());
}

const FORBIDDEN: [&str; 4] = ["no action improves", "no move improves", "position is equal", "no recommendation"];

fn assert_no_passive_wording(rec: &crate::queries::TurnRecommendation) {
    for text in [&rec.summary, &rec.report.text, &rec.report.reason, &rec.report.position] {
        let t = text.to_lowercase();
        for f in FORBIDDEN {
            assert!(!t.contains(f), "forbidden wording {f:?} in {text:?}");
        }
    }
}

#[test]
fn monster_turn_reports_plan_alternate_damage_status_target_and_position() {
    // A brute between a sturdy fighter and a fragile caster.
    let mut s = state(vec![
        p(sb("Kira", 14, 16, 8, "1d8+3"), Side::Party, 4, 5),
        p(sb("Valeros", 40, 19, 9, "1d10+4"), Side::Party, 6, 5),
        p(sb("Cave Brute", 50, 17, 11, "1d12+5"), Side::Enemy, 5, 5),
    ]);
    start(&mut s, &[1, 1, 20]);
    assert_eq!(s.current_actor(), Some(CombatantId(3)));
    let rec = recommend_turn(&s, SearchConfig::default()).unwrap();
    assert_no_passive_wording(&rec);
    let r = &rec.report;
    // Concentrating on the fragile PC: two Strikes drop her about half the time.
    assert_eq!(rec.plan.actions.iter().filter(|a| matches!(a, Action::Strike { target: CombatantId(1), .. })).count(), 2, "{}", r.text);
    assert!(r.down_chance > 0.4, "{}", r.text);
    assert_eq!(rec.plan.actions.len(), 3, "a committed monster spends all three actions: {:?}", rec.steps);
    assert_eq!(r.priority_target, Some(CombatantId(1)), "the low-HP PC is the priority: {}", r.text);
    assert!(r.expected_damage > 5.0, "{}", r.text);
    assert!(r.down_chance > 0.0 && r.down_chance <= 1.0, "{}", r.text);
    assert!(!r.alternate.is_empty(), "an alternate plan is always offered when one exists: {}", r.text);
    assert_ne!(r.alternate, r.primary);
    assert!(r.position.ends_with("Advantage"), "{}", r.position);
    assert!(r.reason.contains("Kira has the lowest effective HP"), "{}", r.reason);
    assert!(r.reason.contains("action economy"), "{}", r.reason);
    for section in ["RECOMMENDED TURN", "Action 1:", "ALTERNATE TURN", "Expected Damage:", "Expected Status Effects:", "Priority Target:", "Chance to Down Target:", "Position Evaluation:", "Reason:"] {
        assert!(r.text.contains(section), "missing {section}:\n{}", r.text);
    }
}

#[test]
fn monster_never_passes_while_legal_actions_remain() {
    // Out of reach of everything and unable to move far: a passive engine
    // would end the turn; a committed one still acts (Demoralize, then the
    // best of whatever is left).
    let mut stuck = sb("Statue Guard", 30, 16, 8, "1d8+3");
    stuck.speed = 0;
    let mut s = state(vec![
        p(sb("Hero", 30, 18, 9, "1d8+4"), Side::Party, 2, 2),
        p(stuck, Side::Enemy, 7, 2),
    ]);
    start(&mut s, &[1, 20]);
    assert!(!s.legal_actions().is_empty());
    let rec = recommend_turn(&s, SearchConfig::default()).unwrap();
    assert_no_passive_wording(&rec);
    assert!(!rec.plan.actions.is_empty(), "{}", rec.report.text);
    assert!(rec.plan.actions.iter().any(|a| matches!(a, Action::Demoralize { .. })), "{}", rec.report.text);
    assert!(
        rec.report.expected_status_effects.iter().any(|e| e.effect.starts_with("frightened")),
        "Demoralize's frightened is reported: {}",
        rec.report.text
    );
}

#[test]
fn committed_search_completes_the_turn_even_when_out_of_budget() {
    let mut s = state(vec![
        p(sb("Hero", 40, 18, 10, "1d8+4"), Side::Party, 5, 5),
        p(sb("Orc", 40, 16, 8, "1d8+3"), Side::Enemy, 6, 5),
    ]);
    start(&mut s, &[1, 20]);
    let plan = best_turn(&s, &HeuristicEvaluator::default(), SearchConfig { node_budget: 10, ..Default::default() }).unwrap();
    assert!(plan.truncated);
    assert_eq!(plan.actions.len(), 3, "{:?}", plan.actions);
}

#[test]
fn a_creature_with_no_actions_left_gets_an_empty_but_explained_plan() {
    let mut s = state(vec![
        p(sb("Hero", 40, 18, 10, "1d8+4"), Side::Party, 5, 5),
        p(sb("Orc", 40, 16, 8, "1d8+3"), Side::Enemy, 9, 9),
    ]);
    start(&mut s, &[1, 20]);
    let orc = s.combatants.iter_mut().find(|c| c.id == CombatantId(2)).unwrap();
    orc.actions_remaining = 0;
    let rec = recommend_turn(&s, SearchConfig::default()).unwrap();
    assert!(rec.plan.actions.is_empty());
    assert_no_passive_wording(&rec);
    assert!(rec.report.reason.contains("no actions remaining"), "{}", rec.report.reason);
}

#[test]
fn simulated_monsters_spend_their_actions() {
    let mut stuck = sb("Statue Guard", 30, 16, 8, "1d8+3");
    stuck.speed = 0;
    let mut s = state(vec![
        p(sb("Hero", 30, 18, 9, "1d8+4"), Side::Party, 2, 2),
        p(stuck, Side::Enemy, 7, 2),
    ]);
    start(&mut s, &[1, 20]);
    assert!(crate::simulate::scripted_action(&s).is_some());
}

// ---------------------------------------------------------------------------

fn skirmish() -> CombatState {
    let mut s = state(vec![
        p(sb("Hero", 40, 18, 10, "1d8+4"), Side::Party, 3, 5),
        p(sb("Ally", 30, 17, 8, "1d6+3"), Side::Party, 2, 7),
        p(sb("Orc", 30, 16, 8, "1d8+3"), Side::Enemy, 7, 5),
        p(sb("Goblin", 18, 15, 7, "1d6+2"), Side::Enemy, 8, 7),
    ]);
    start(&mut s, &[20, 15, 5, 1]);
    s
}

#[test]
fn cached_evaluation_is_exact() {
    use crate::evaluation::EvalCache;
    use crate::simulate::{play_out, SimulationConfig};
    // Sample positions along a played-out combat; the cache is shared
    // across all of them, as it is within one search.
    let ev = HeuristicEvaluator::default();
    let mut cache = EvalCache::default();
    let s0 = skirmish();
    for run in 0..6 {
        let cfg = SimulationConfig { runs: 1, seed: run, max_rounds: (run as u32 % 3) + 1 };
        for s in [s0.clone(), play_out(&s0, &cfg, run as u32)] {
            for _ in 0..2 {
                let (plain, cached) = (ev.evaluate(&s), ev.evaluate_cached(&s, &mut cache));
                assert!((plain - cached).abs() < 1e-12, "cache changed the score: {plain} vs {cached}");
            }
        }
    }
    assert!(cache.hits > 0);
}

#[test]
fn search_accelerations_agree_when_the_beam_does_not_bind() {
    // With a beam wide enough that nothing is cut, ordering cannot change
    // the result: transpositions and iterative deepening must agree with a
    // plain full-depth search.
    let s = skirmish();
    let ev = HeuristicEvaluator::default();
    let wide = SearchConfig { beam: 64, node_budget: 1_000_000, ..Default::default() };
    let plain = best_turn(&s, &ev, SearchConfig { iterative: false, transpositions: false, ..wide }).unwrap();
    for (iterative, transpositions) in [(false, true), (true, false), (true, true)] {
        let fast = best_turn(&s, &ev, SearchConfig { iterative, transpositions, ..wide }).unwrap();
        assert!((fast.expected_score - plain.expected_score).abs() < 1e-9, "iterative={iterative} tt={transpositions}: {} vs {}", fast.expected_score, plain.expected_score);
        assert!(!fast.truncated);
    }
    let tt = best_turn(&s, &ev, SearchConfig { iterative: false, transpositions: true, ..wide }).unwrap();
    assert!(tt.nodes <= plain.nodes, "transpositions never add nodes ({} vs {})", tt.nodes, plain.nodes);
}

#[test]
fn budget_stops_with_a_complete_shallower_answer() {
    use crate::search::best_turn_with_budget;
    let s = skirmish();
    let ev = HeuristicEvaluator::default();
    let full = best_turn(&s, &ev, SearchConfig::default()).unwrap();
    assert_eq!(full.depth, 3);
    // Allow roughly the first iteration only.
    let stop_early = |nodes: usize| nodes > 40;
    let quick = best_turn_with_budget(&s, &ev, SearchConfig::default(), &stop_early).unwrap();
    assert!(quick.truncated);
    assert!(quick.depth < full.depth, "depth {}", quick.depth);
    assert!(!quick.actions.is_empty(), "an anytime answer still recommends something");
    // An exhausted budget never panics or recommends illegal actions.
    let none = best_turn_with_budget(&s, &ev, SearchConfig::default(), &|_: usize| true).unwrap();
    let mut probe = s.clone();
    for a in &none.actions {
        probe.apply(&Command::Act { actor: none.actor, action: a.clone() }).expect("legal");
        while let Some(pi) = probe.pending() {
            match pi {
                pf2e_core::combat::PendingInput::Roll(r) => probe.apply(&Command::ProvideRoll { request_id: r.request_id, value: 10, extra_modifier: 0 }).unwrap(),
                pf2e_core::combat::PendingInput::Decision(d) => probe.apply(&Command::Decide { request_id: d.request_id, accept: false }).unwrap(),
            };
        }
    }
}

#[test]
fn split_simulation_merges_exactly() {
    use crate::simulate::{simulate_range, SimulationTally};
    let s = skirmish();
    let cfg = SimulationConfig { runs: 40, seed: 3, max_rounds: 10 };
    let whole = simulate(&s, cfg);
    let mut merged = SimulationTally::default();
    for (start, count) in [(0, 13), (13, 13), (26, 14)] {
        merged.merge(&simulate_range(&s, cfg, start, count));
    }
    assert_eq!(serde_json::to_string(&merged.report()).unwrap(), serde_json::to_string(&whole).unwrap());
}

#[test]
fn explain_change_names_the_wounded_enemy() {
    use crate::explain::{explain_change, ReasonKind};
    let before = skirmish();
    let mut after = before.clone();
    let orc = after.combatants.iter_mut().find(|c| c.name == "Orc").unwrap();
    orc.hp = 6;
    let ex = explain_change(&before, &after, &HeuristicEvaluator::default(), 5);
    assert!(ex.delta > 0.0, "wounding an enemy helps the party ({})", ex.delta);
    let top = &ex.reasons[0];
    assert_eq!(top.kind, ReasonKind::HitPoints);
    assert!(top.text.contains("Orc") && top.impact > 0.0, "{top:?}");
}

#[test]
fn explain_position_reports_flanking() {
    use crate::explain::{explain_position, ReasonKind};
    let mut s = state(vec![
        p(sb("Hero", 40, 18, 10, "1d8+4"), Side::Party, 4, 5),
        p(sb("Ally", 40, 18, 10, "1d8+4"), Side::Party, 6, 5),
        p(sb("Orc", 30, 16, 8, "1d8+3"), Side::Enemy, 5, 5),
    ]);
    start(&mut s, &[20, 15, 1]);
    let ex = explain_position(&s, &HeuristicEvaluator::default(), 8);
    let flank = ex.reasons.iter().find(|r| r.kind == ReasonKind::Flanked).expect("flanking reason");
    assert!(flank.text.contains("Orc"));
    assert!(flank.impact > 0.0, "the flanked enemy favours the party ({})", flank.impact);
}
