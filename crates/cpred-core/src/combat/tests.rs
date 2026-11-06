//! Engine behaviour tests. Each test drives the engine purely through
//! commands, exactly as a UI would.

use super::*;
use crate::analysis;
use crate::content::Content;
use crate::model::{ActorKind, Armor, ArmorPiece, Battlefield, HitLocation, Skill, Stat, Statblock, Stats, Weapon};
use crate::rules::{AttackMode, RulesOptions};
use std::collections::BTreeSet;
use tactics_core::{CombatantId, Pos, SeededDice, Side};

const A: CombatantId = CombatantId(1);
const B: CombatantId = CombatantId(2);
const C: CombatantId = CombatantId(3);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn content() -> Content {
    Content::bundled().expect("bundled content loads")
}

fn weapon(id: &str) -> Weapon {
    content().weapon(id).cloned().expect("known weapon")
}

fn piece(name: &str, sp: i32, penalty: i32) -> ArmorPiece {
    ArmorPiece { id: String::new(), name: name.into(), sp, penalty }
}

/// REF 6, DEX 6, BODY 6, MOVE 6, every combat skill 4, max HP 40, a heavy
/// pistol, no armor.
fn sb(name: &str) -> Statblock {
    Statblock {
        name: name.into(),
        stats: Stats { int: 5, ref_: 6, dex: 6, tech: 5, cool: 5, will: 5, luck: 5, move_: 6, body: 6, emp: 5 },
        skills: Skill::ALL.iter().map(|s| (*s, 4)).collect(),
        weapons: vec![weapon("heavy_pistol")],
        armor: Armor::default(),
        max_hp: Some(40),
        notes: String::new(),
    }
}

fn part(name: &str, side: Side, statblock: Statblock, x: i32, y: i32) -> SetupParticipant {
    SetupParticipant { name: name.into(), side, kind: ActorKind::Npc, statblock, position: Pos::new(x, y), current_hp: None }
}

fn build(bf: Battlefield, parts: Vec<SetupParticipant>) -> CombatState {
    CombatSetup::build(bf, parts, &content()).expect("setup")
}

fn build_opts(bf: Battlefield, parts: Vec<SetupParticipant>, options: RulesOptions) -> CombatState {
    CombatSetup::build_with_options(bf, parts, &content(), options).expect("setup")
}

/// Checks `representative_answers` for the current pending roll: weights sum
/// to 1 and every answer is accepted.
fn check_representatives(s: &CombatState) {
    let answers = analysis::representative_answers(s);
    match s.pending() {
        Some(PendingInput::Roll(_)) => {
            assert!(!answers.is_empty(), "no representative answers for {:?}", s.queue.front());
            let total: f64 = answers.iter().map(|(_, p)| p).sum();
            assert!((total - 1.0).abs() < 1e-9, "weights sum to {total} for {:?}", s.queue.front());
            for (cmd, p) in &answers {
                assert!(*p > 0.0);
                let mut t = s.clone();
                t.apply(cmd).unwrap_or_else(|e| panic!("representative {cmd:?} rejected: {e}"));
            }
        }
        _ => assert!(answers.is_empty()),
    }
}

fn request(s: &CombatState) -> RollRequest {
    match s.pending() {
        Some(PendingInput::Roll(r)) => r,
        other => panic!("expected a roll request, got {other:?}"),
    }
}

fn roll(s: &mut CombatState, value: i32) -> Vec<Event> {
    check_representatives(s);
    let id = request(s).request_id;
    s.apply(&Command::roll(id, value)).expect("roll accepted")
}

fn faces(s: &mut CombatState, f: &[i32]) -> Vec<Event> {
    check_representatives(s);
    let id = request(s).request_id;
    s.apply(&Command::faces(id, f.to_vec())).expect("faces accepted")
}

fn decide(s: &mut CombatState, accept: bool) -> Vec<Event> {
    let Some(PendingInput::Decision(d)) = s.pending() else { panic!("expected a decision") };
    s.apply(&Command::Decide { request_id: d.request_id, accept }).expect("decision accepted")
}

fn act(s: &mut CombatState, actor: CombatantId, action: Action) -> Vec<Event> {
    s.apply(&Command::Act { actor, action: action.clone() }).unwrap_or_else(|e| panic!("{action:?} rejected: {e}"))
}

fn try_act(s: &mut CombatState, actor: CombatantId, action: Action) -> CombatResult<Vec<Event>> {
    s.apply(&Command::Act { actor, action })
}

fn end_turn(s: &mut CombatState, actor: CombatantId) {
    s.apply(&Command::EndTurn { actor }).expect("end turn");
}

/// Starts combat and enters the initiative naturals in combatant-id order.
fn start(s: &mut CombatState, naturals: &[i32]) {
    s.apply(&Command::StartCombat).expect("start");
    for &n in naturals {
        roll(s, n);
    }
    assert_eq!(s.phase, Phase::Active);
}

/// A (#1, party) vs B (#2, enemy) on an open map, two squares apart; A acts first.
fn duel(a: Statblock, b: Statblock) -> CombatState {
    let mut s = build(Battlefield::open(12, 12), vec![part("A", Side::Party, a, 0, 0), part("B", Side::Enemy, b, 2, 0)]);
    start(&mut s, &[10, 1]);
    assert_eq!(s.current_actor(), Some(A));
    s
}

fn melee_duel(a: Statblock, b: Statblock) -> CombatState {
    let mut s = build(Battlefield::open(12, 12), vec![part("A", Side::Party, a, 0, 0), part("B", Side::Enemy, b, 1, 0)]);
    start(&mut s, &[10, 1]);
    s
}

fn hp(s: &CombatState, id: CombatantId) -> i32 {
    s.combatant(id).map(|c| c.hp).unwrap_or(-1)
}

fn brawling_index(s: &CombatState, id: CombatantId) -> usize {
    s.combatant(id).map(|c| c.statblock.weapons.len() - 1).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Content and setup
// ---------------------------------------------------------------------------

#[test]
fn bundled_content_loads_and_sample_encounter_builds() {
    let c = content();
    assert_eq!(c.weapons.len(), 13);
    assert_eq!(c.armor.len(), 7);
    assert!(c.actors.iter().filter(|a| a.kind == ActorKind::Pc).count() >= 4);
    assert!(c.actors.iter().filter(|a| a.kind == ActorKind::Npc).count() >= 3);
    let mut s = c.build_encounter("warehouse-standoff").expect("encounter builds");
    assert_eq!(s.combatants.len(), 9);
    assert!(s.combatants.iter().all(|x| x.statblock.weapons.last().map(|w| w.id.as_str()) == Some(Weapon::BRAWLING_ID)));
    assert_eq!(s.combatants.iter().filter(|x| x.name.starts_with("Street Ganger")).count(), 2);
    assert!(s.combatants.iter().any(|x| x.name == "Street Ganger 2"));
    assert!(s.cover.len() >= 5);
    assert!(s.cover.iter().all(|o| o.hp == 20));
    let vex = &s.combatants[0];
    assert_eq!(vex.max_hp(), 45);
    assert_eq!(vex.armor_sp, ArmorSp { body: 11, head: 11 });
    assert_eq!(vex.ammo[0], 25);
    s.apply(&Command::StartCombat).expect("start");
    for _ in 0..9 {
        roll(&mut s, 5);
    }
    assert_eq!(s.phase, Phase::Active);
    assert_eq!(s.initiative.len(), 9);
}

#[test]
fn hp_formula_and_max_hp_override() {
    let c = content();
    let vex = c.actor("vex").expect("vex");
    assert_eq!(vex.statblock.derived_max_hp(), 45); // BODY 7 + WILL 6 → 10 + 5 × 7
    let rook = c.actor("rook-haldane").expect("rook");
    assert_eq!(rook.statblock.effective_max_hp(), 50); // 8 + 7 → 10 + 5 × 8
    let ganger = c.actor("street-ganger").expect("ganger");
    assert_eq!(ganger.statblock.derived_max_hp(), 30);
    assert_eq!(ganger.statblock.effective_max_hp(), 25);
}

#[test]
fn armor_penalty_reduces_ref_dex_move() {
    let mut a = sb("A");
    a.armor.body = Some(piece("Flak", 15, 4));
    a.armor.head = Some(piece("Kevlar", 7, 0));
    let s = build(Battlefield::open(5, 5), vec![part("A", Side::Party, a, 0, 0)]);
    let c = s.combatant(A).expect("A");
    assert_eq!(c.effective_stat(Stat::Ref), 2);
    assert_eq!(c.effective_stat(Stat::Dex), 2);
    assert_eq!(c.effective_stat(Stat::Move), 2);
    assert_eq!(c.effective_stat(Stat::Body), 6);
}

#[test]
fn setup_rejects_bad_positions() {
    let mut bf = Battlefield::open(4, 4);
    bf.map[0] = "#%..".into();
    let c = content();
    let err = |x, y| CombatSetup::build(bf.clone(), vec![part("A", Side::Party, sb("A"), x, y)], &c).is_err();
    assert!(err(0, 0));
    assert!(err(1, 0));
    assert!(err(9, 9));
    assert!(!err(2, 0));
    let overlap = CombatSetup::build(bf.clone(), vec![part("A", Side::Party, sb("A"), 2, 2), part("B", Side::Enemy, sb("B"), 2, 2)], &c);
    assert!(matches!(overlap, Err(SetupError::Overlap(..))));
}

#[test]
fn brawling_damage_scales_with_body() {
    let mut weak = sb("W");
    weak.stats.body = 4;
    let mut strong = sb("S");
    strong.stats.body = 11;
    let s = build(Battlefield::open(5, 5), vec![part("W", Side::Party, weak, 0, 0), part("S", Side::Enemy, strong, 2, 2)]);
    let dmg = |id| s.combatant(id).and_then(|c| c.statblock.weapons.last()).map(|w| w.damage.to_string()).unwrap_or_default();
    assert_eq!(dmg(A), "1d6");
    assert_eq!(dmg(B), "4d6");
    let b = s.combatant(B).and_then(|c| c.statblock.weapons.last()).cloned().expect("brawling");
    assert_eq!(b.rof, 2);
    assert!(b.melee && !b.halves_armor());
}

// ---------------------------------------------------------------------------
// Initiative and turns
// ---------------------------------------------------------------------------

#[test]
fn initiative_ordering_and_ties() {
    let mut fast = sb("Fast");
    fast.stats.ref_ = 7;
    let parts = vec![
        part("One", Side::Party, sb("One"), 0, 0),
        part("Fast", Side::Party, fast, 1, 0),
        part("Three", Side::Enemy, sb("Three"), 2, 0),
        part("Four", Side::Enemy, sb("Four"), 3, 0),
    ];
    let mut s = build(Battlefield::open(6, 6), parts);
    s.apply(&Command::StartCombat).expect("start");
    let r = request(&s);
    assert!(matches!(r.kind, RollKind::Initiative { modifier: 6 }));
    assert!(r.prompt.contains("1d10+6"));
    // Totals: One 11, Fast 11 (higher REF), Three 11, Four 15.
    for n in [5, 4, 5, 9] {
        roll(&mut s, n);
    }
    let order: Vec<_> = s.initiative.iter().map(|e| (e.combatant.0, e.value)).collect();
    assert_eq!(order, vec![(4, 15), (2, 11), (1, 11), (3, 11)]);
    assert_eq!(s.current_actor(), Some(CombatantId(4)));
    // Initiative 10 does not explode.
    let mut t = build(Battlefield::open(6, 6), vec![part("A", Side::Party, sb("A"), 0, 0), part("B", Side::Enemy, sb("B"), 3, 3)]);
    t.apply(&Command::StartCombat).expect("start");
    roll(&mut t, 10);
    roll(&mut t, 1);
    assert_eq!(t.initiative[0].value, 16);
    assert_eq!(t.phase, Phase::Active);
}

#[test]
fn turns_rotate_and_rounds_advance() {
    let mut s = duel(sb("A"), sb("B"));
    assert_eq!(s.round, 1);
    end_turn(&mut s, A);
    assert_eq!(s.current_actor(), Some(B));
    assert!(matches!(s.apply(&Command::EndTurn { actor: A }), Err(CombatError::NotYourTurn { .. })));
    end_turn(&mut s, B);
    assert_eq!(s.current_actor(), Some(A));
    assert_eq!(s.round, 2);
    assert_eq!(s.combatant(A).map(|c| c.economy), Some(TurnEconomy::fresh()));
}

// ---------------------------------------------------------------------------
// Movement
// ---------------------------------------------------------------------------

#[test]
fn move_respects_move_stat_and_terrain() {
    let mut a = sb("A");
    a.stats.move_ = 4;
    let mut bf = Battlefield::open(10, 10);
    bf.map[2] = "~.........".into();
    bf.map[3] = "%#........".into();
    let mut s = build(bf, vec![part("A", Side::Party, a, 0, 0), part("B", Side::Enemy, sb("B"), 9, 9)]);
    start(&mut s, &[10, 1]);
    let m = s.movement(A);
    assert_eq!(m.cost_to(Pos::new(4, 0)), Some(4));
    assert_eq!(m.cost_to(Pos::new(2, 4)), Some(4));
    assert_eq!(m.cost_to(Pos::new(0, 4)), None); // detour round cover + wall costs 6 (no corner cutting)
    assert_eq!(m.cost_to(Pos::new(5, 0)), None); // beyond MOVE
    assert_eq!(m.cost_to(Pos::new(3, 3)), Some(3)); // diagonal = 1 square each
    assert_eq!(m.cost_to(Pos::new(0, 2)), Some(3)); // difficult: 1 + 2
    assert_eq!(m.cost_to(Pos::new(0, 3)), None); // cover object
    assert_eq!(m.cost_to(Pos::new(1, 3)), None); // wall
    assert_eq!(m.cost_to(Pos::new(0, 0)), None); // start excluded
    assert!(try_act(&mut s, A, Action::Move { to: Pos::new(5, 0) }).is_err());
    assert!(try_act(&mut s, A, Action::Move { to: Pos::new(1, 3) }).is_err());
    act(&mut s, A, Action::Move { to: Pos::new(4, 0) });
    assert_eq!(s.combatant(A).map(|c| c.position), Some(Pos::new(4, 0)));
    assert!(try_act(&mut s, A, Action::Move { to: Pos::new(5, 0) }).is_err()); // Move Action used
    act(&mut s, A, Action::Run { to: Pos::new(8, 0) }); // Run from the new square
    assert!(try_act(&mut s, A, Action::Run { to: Pos::new(9, 0) }).is_err());
    assert!(s.legal_actions().is_empty());
}

#[test]
fn allies_can_be_passed_enemies_cannot() {
    let mut bf = Battlefield::open(6, 3);
    bf.map[0] = "######".into();
    bf.map[2] = "######".into();
    let parts = vec![
        part("A", Side::Party, sb("A"), 0, 1),
        part("Ally", Side::Party, sb("Ally"), 1, 1),
        part("Foe", Side::Enemy, sb("Foe"), 5, 1),
    ];
    let mut s = build(bf.clone(), parts);
    start(&mut s, &[10, 1, 1]);
    let m = s.movement(A);
    assert_eq!(m.cost_to(Pos::new(1, 1)), None); // occupied by the ally
    assert_eq!(m.cost_to(Pos::new(2, 1)), Some(2)); // through the ally
    let parts = vec![
        part("A", Side::Party, sb("A"), 0, 1),
        part("Foe", Side::Enemy, sb("Foe"), 1, 1),
        part("Ally", Side::Party, sb("Ally"), 5, 1),
    ];
    let mut s = build(bf, parts);
    start(&mut s, &[10, 1, 1]);
    assert!(s.movement(A).destinations.is_empty());
}

// ---------------------------------------------------------------------------
// Attacks
// ---------------------------------------------------------------------------

#[test]
fn rof_two_allows_exactly_two_attacks_per_action() {
    let mut a = sb("A");
    a.weapons.push(weapon("very_heavy_pistol"));
    let mut s = duel(a, sb("B"));
    let r = {
        act(&mut s, A, Action::Attack { weapon: 0, target: B });
        request(&s)
    };
    assert!(matches!(r.kind, RollKind::Check { modifier: 10, beat: Some(13) }));
    assert_eq!(r.prompt, "Roll 1d10+10 for Heavy Pistol attack (A → B, beat DV 13) — enter the natural d10");
    assert!(s.legal_actions().is_empty());
    assert!(try_act(&mut s, A, Action::Attack { weapon: 0, target: B }).is_err()); // input pending
    roll(&mut s, 2); // 12, miss
    assert!(s.pending().is_none());
    assert!(try_act(&mut s, A, Action::Attack { weapon: 1, target: B }).is_err()); // must be the same weapon
    assert!(s.legal_actions().contains(&Action::Attack { weapon: 0, target: B }));
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 2);
    assert!(try_act(&mut s, A, Action::Attack { weapon: 0, target: B }).is_err());
    assert_eq!(s.combatant(A).map(|c| c.ammo[0]), Some(6));
    // ROF 1: one attack per Action.
    end_turn(&mut s, A);
    end_turn(&mut s, B);
    act(&mut s, A, Action::Attack { weapon: 1, target: B });
    roll(&mut s, 2);
    assert!(try_act(&mut s, A, Action::Attack { weapon: 1, target: B }).is_err());
    assert!(try_act(&mut s, A, Action::Attack { weapon: 0, target: B }).is_err());
}

#[test]
fn aimed_shot_penalty_head_sp_and_doubling() {
    let mut b = sb("B");
    b.armor = Armor { body: Some(piece("Light Armorjack", 11, 0)), head: Some(piece("Kevlar", 7, 0)) };
    let mut s = duel(sb("A"), b);
    act(&mut s, A, Action::AimedShot { weapon: 0, target: B });
    let r = request(&s);
    assert!(matches!(r.kind, RollKind::Check { modifier: 2, beat: Some(13) }));
    assert!(r.breakdown.contains(&("aimed shot".to_string(), -8)));
    roll(&mut s, 10); // critical: follow-up
    assert!(matches!(request(&s).kind, RollKind::CritFollowUp { sign: 1 }));
    roll(&mut s, 9); // 10 + 2 + 9 = 21 > 13
    assert!(matches!(request(&s).kind, RollKind::Damage { .. }));
    let ev = faces(&mut s, &[5, 4, 3]); // 12 vs head SP 7 → 5 × 2
    assert!(ev.contains(&Event::HitLocation { target: B, location: HitLocation::Head }));
    assert_eq!(hp(&s, B), 30);
    assert_eq!(s.combatant(B).map(|c| c.armor_sp), Some(ArmorSp { body: 11, head: 6 }));
    // The aimed shot used the whole Action.
    assert!(try_act(&mut s, A, Action::Attack { weapon: 0, target: B }).is_err());
}

#[test]
fn armor_stops_without_ablation_and_penetration_ablates() {
    let mut b = sb("B");
    b.armor.body = Some(piece("Light Armorjack", 11, 0));
    let mut s = duel(sb("A"), b);
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 9);
    let ev = faces(&mut s, &[4, 4, 3]);
    assert!(ev.iter().any(|e| matches!(e, Event::ArmorStopped { damage: 11, sp: 11, .. })));
    assert_eq!(hp(&s, B), 40);
    assert_eq!(s.combatant(B).map(|c| c.armor_sp.body), Some(11));
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 9);
    let ev = faces(&mut s, &[5, 5, 4]);
    assert!(ev.contains(&Event::ArmorPenetrated { target: B, location: HitLocation::Body, damage: 14, sp_before: 11, sp_after: 10 }));
    assert_eq!(hp(&s, B), 37);
}

#[test]
fn melee_weapons_halve_sp_brawling_does_not() {
    let mut a = sb("A");
    a.weapons = vec![weapon("heavy_melee")];
    let mut b = sb("B");
    b.armor.body = Some(piece("Light Armorjack", 11, 0));
    let mut s = melee_duel(a.clone(), b.clone());
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    let r = request(&s);
    assert_eq!(r.roller, Some(B)); // the defender rolls first
    assert!(matches!(r.kind, RollKind::Check { modifier: 10, beat: None }));
    roll(&mut s, 2); // defence 12
    assert!(matches!(request(&s).kind, RollKind::Check { modifier: 10, beat: Some(12) }));
    roll(&mut s, 9); // 19 > 12
    faces(&mut s, &[3, 3, 3]); // 9 vs SP ceil(11/2) = 6
    assert_eq!(hp(&s, B), 37);
    assert_eq!(s.combatant(B).map(|c| c.armor_sp.body), Some(10));

    let mut s = melee_duel(a, b);
    let brawl = brawling_index(&s, A);
    act(&mut s, A, Action::Attack { weapon: brawl, target: B });
    roll(&mut s, 2);
    roll(&mut s, 9);
    let ev = faces(&mut s, &[5, 5]); // 10 vs full SP 11
    assert!(ev.iter().any(|e| matches!(e, Event::ArmorStopped { damage: 10, sp: 11, .. })));
    assert_eq!(hp(&s, B), 40);
}

#[test]
fn melee_ties_go_to_the_defender() {
    let mut a = sb("A");
    a.weapons = vec![weapon("medium_melee")];
    let mut s = melee_duel(a, sb("B"));
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 5); // defence 15
    let ev = roll(&mut s, 5); // attack 15: not greater
    assert!(ev.contains(&Event::Missed { attacker: A, target: B }));
    assert!(s.pending().is_none());
    // Melee needs adjacency.
    let mut s = duel(sb("A"), sb("B"));
    let brawl = brawling_index(&s, A);
    assert!(try_act(&mut s, A, Action::Attack { weapon: brawl, target: B }).is_err());
}

#[test]
fn critical_injury_on_two_sixes() {
    let mut s = duel(sb("A"), sb("B"));
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 9);
    let ev = faces(&mut s, &[6, 1, 6]); // 13 + 5 bonus
    assert!(ev.iter().any(|e| matches!(e, Event::CriticalInjury { target: B, location: HitLocation::Body, .. })));
    let b = s.combatant(B).expect("B");
    assert_eq!(b.hp, 40 - 13 - 5);
    assert_eq!(b.injuries, vec![CriticalInjury { location: HitLocation::Body, round: 1 }]);
    assert_eq!(b.death_save_penalty, 1);
    // Bonus damage ignores armor, even when SP stops the hit itself.
    let mut b2 = sb("B");
    b2.armor.body = Some(piece("Metalgear", 18, 0));
    let mut s = duel(sb("A"), b2);
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 9);
    faces(&mut s, &[6, 6, 1]);
    assert_eq!(hp(&s, B), 35);
    assert_eq!(s.combatant(B).map(|c| c.armor_sp.body), Some(18));
}

#[test]
fn dodge_offered_only_at_ref_eight_and_replaces_dv() {
    let mut quick = sb("B");
    quick.stats.ref_ = 8;
    let mut s = duel(sb("A"), quick.clone());
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    let Some(PendingInput::Decision(d)) = s.pending() else { panic!("expected dodge decision") };
    assert_eq!(d.decider, B);
    assert!(analysis::representative_answers(&s).is_empty());
    let ev = decide(&mut s, true);
    assert!(ev.contains(&Event::DodgeDeclared { combatant: B, attacker: A }));
    let r = request(&s);
    assert_eq!(r.roller, Some(B));
    assert!(matches!(r.kind, RollKind::Check { modifier: 10, beat: None }));
    roll(&mut s, 7); // dodge 17
    let r = request(&s);
    assert!(matches!(r.kind, RollKind::Check { modifier: 10, beat: Some(17) }));
    assert!(r.prompt.contains("beat dodge 17"));
    roll(&mut s, 7); // 17, not enough
    assert_eq!(hp(&s, B), 40);
    // Declining keeps the DV.
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    decide(&mut s, false);
    assert!(matches!(request(&s).kind, RollKind::Check { beat: Some(13), .. }));
    // REF 7 (or armor-reduced REF): no decision.
    let mut slow = quick;
    slow.armor.body = Some(piece("Medium Armorjack", 12, 2));
    let mut s = duel(sb("A"), slow);
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    assert!(matches!(request(&s).kind, RollKind::Check { beat: Some(13), .. }));
}

#[test]
fn autofire_multiplier_is_capped_and_uses_ten_rounds() {
    let mut a = sb("A");
    a.weapons = vec![weapon("smg")];
    let mut s = duel(a, sb("B"));
    act(&mut s, A, Action::Autofire { weapon: 0, target: B });
    assert_eq!(s.combatant(A).map(|c| c.ammo[0]), Some(20));
    let r = request(&s);
    assert!(matches!(r.kind, RollKind::Check { modifier: 10, beat: Some(20) })); // autofire DV, Autofire skill
    roll(&mut s, 10);
    roll(&mut s, 10); // 30: margin 10, capped at ×3
    let r = request(&s);
    assert!(matches!(&r.kind, RollKind::Damage { dice } if dice.to_string() == "2d6"));
    let ev = faces(&mut s, &[4, 3]);
    assert!(ev.iter().any(|e| matches!(e, Event::DamageRolled { total: 21, .. })));
    assert_eq!(hp(&s, B), 19);
    assert!(try_act(&mut s, A, Action::Attack { weapon: 0, target: B }).is_err()); // whole Action
    // Fumble path: a natural 1 asks for a d10 to subtract.
    end_turn(&mut s, A);
    end_turn(&mut s, B);
    act(&mut s, A, Action::Autofire { weapon: 0, target: B });
    roll(&mut s, 1);
    assert!(matches!(request(&s).kind, RollKind::CritFollowUp { sign: -1 }));
    let ev = roll(&mut s, 3);
    assert!(ev.contains(&Event::Missed { attacker: A, target: B }));
    assert!(s.pending().is_none());
    // Not enough rounds.
    end_turn(&mut s, A);
    end_turn(&mut s, B);
    s.apply(&Command::Gm { op: GmOp::SetAmmo { target: A, weapon: 0, value: 9 } }).expect("gm");
    assert!(try_act(&mut s, A, Action::Autofire { weapon: 0, target: B }).is_err());
    let brawl = brawling_index(&s, A);
    assert!(try_act(&mut s, A, Action::Autofire { weapon: brawl, target: B }).is_err());
}

#[test]
fn autofire_small_margin_multiplier() {
    let mut a = sb("A");
    a.weapons = vec![weapon("smg")];
    let mut s = duel(a, sb("B"));
    act(&mut s, A, Action::Autofire { weapon: 0, target: B });
    roll(&mut s, 10);
    roll(&mut s, 1); // 10 + 10 + 1 = 21 vs DV 20: margin 1 → ×1
    assert!(request(&s).purpose.contains("×1"));
    faces(&mut s, &[3, 4]);
    assert_eq!(hp(&s, B), 33);
}

#[test]
fn reload_and_out_of_ammo() {
    let mut s = duel(sb("A"), sb("B"));
    assert!(try_act(&mut s, A, Action::Reload { weapon: 0 }).is_err()); // full
    s.apply(&Command::Gm { op: GmOp::SetAmmo { target: A, weapon: 0, value: 0 } }).expect("gm");
    assert!(try_act(&mut s, A, Action::Attack { weapon: 0, target: B }).is_err());
    assert!(try_act(&mut s, A, Action::AimedShot { weapon: 0, target: B }).is_err());
    assert!(!s.legal_actions().iter().any(|a| matches!(a, Action::Attack { weapon: 0, .. })));
    assert!(s.legal_actions().contains(&Action::Reload { weapon: 0 }));
    let ev = act(&mut s, A, Action::Reload { weapon: 0 });
    assert!(ev.contains(&Event::Reloaded { combatant: A, weapon: 0, ammo: 8 }));
    assert!(try_act(&mut s, A, Action::Attack { weapon: 0, target: B }).is_err()); // Action used
    let brawl = brawling_index(&s, A);
    assert!(try_act(&mut s, A, Action::Reload { weapon: brawl }).is_err());
}

#[test]
fn cover_blocks_hits_and_is_destroyed() {
    let mut bf = Battlefield::open(8, 3);
    bf.map[0] = "...%....".into();
    let parts = vec![part("A", Side::Party, sb("A"), 0, 0), part("B", Side::Enemy, sb("B"), 4, 0)];
    let mut s = build_opts(bf, parts, RulesOptions { cover_hp: 20, ..RulesOptions::default() });
    start(&mut s, &[10, 1]);
    let profile = analysis::attack_profile(&s, A, 0, B, AttackMode::Single).expect("profile");
    assert!(profile.blocked_by_cover);
    assert_eq!(analysis::expected_damage(&s, A, 0, B, AttackMode::Single), 0.0);
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    assert!(matches!(request(&s).kind, RollKind::Check { beat: Some(15), .. })); // 8 m: band 7–12
    roll(&mut s, 9);
    let ev = faces(&mut s, &[6, 6, 1]);
    assert!(ev.contains(&Event::CoverHit { pos: Pos::new(3, 0), damage: 13, hp: 7 }));
    let b = s.combatant(B).expect("B");
    assert_eq!((b.hp, b.injuries.len()), (40, 0));
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 9);
    let ev = faces(&mut s, &[4, 4, 4]);
    assert!(ev.contains(&Event::CoverDestroyed { pos: Pos::new(3, 0) }));
    assert_eq!(s.battlefield.map[0], "........");
    assert!(s.cover.is_empty());
    assert_eq!(hp(&s, B), 40);
    let profile = analysis::attack_profile(&s, A, 0, B, AttackMode::Single).expect("profile");
    assert!(!profile.blocked_by_cover);
    // Cover not adjacent to the target does nothing.
    let mut bf = Battlefield::open(8, 3);
    bf.map[0] = ".%......".into();
    let s = build(bf, vec![part("A", Side::Party, sb("A"), 0, 0), part("B", Side::Enemy, sb("B"), 4, 0)]);
    assert!(!analysis::attack_profile(&s, A, 0, B, AttackMode::Single).expect("profile").blocked_by_cover);
}

#[test]
fn range_bands_select_the_dv() {
    let mut bf = Battlefield::open(30, 3);
    bf.map[2] = "..............................".into();
    let s = build(bf, vec![part("A", Side::Party, sb("A"), 0, 0), part("B", Side::Enemy, sb("B"), 4, 0), part("C", Side::Enemy, sb("C"), 13, 0)]);
    // 4 squares = 8 m → band 7–12; 13 squares = 26 m → band 26–50.
    assert_eq!(s.plan_attack(A, 0, B, AttackMode::Single).ok().and_then(|p| p.dv), Some(15));
    assert_eq!(s.plan_attack(A, 0, C, AttackMode::Single).ok().and_then(|p| p.dv), Some(25));
}

#[test]
fn line_of_sight_through_walls_is_illegal() {
    let mut bf = Battlefield::open(6, 3);
    bf.map[0] = "..#...".into();
    let mut s = build(bf, vec![part("A", Side::Party, sb("A"), 0, 0), part("B", Side::Enemy, sb("B"), 4, 0)]);
    start(&mut s, &[10, 1]);
    let err = try_act(&mut s, A, Action::Attack { weapon: 0, target: B }).expect_err("blocked");
    assert!(err.to_string().contains("line of sight"), "{err}");
    assert!(analysis::attack_profile(&s, A, 0, B, AttackMode::Single).is_none());
    assert!(!s.legal_actions().iter().any(|a| a.attack().is_some()));
}

#[test]
fn wound_penalties_appear_in_breakdowns() {
    let mut s = build(
        Battlefield::open(12, 12),
        vec![
            SetupParticipant { current_hp: Some(15), ..part("A", Side::Party, sb("A"), 0, 0) },
            part("B", Side::Enemy, sb("B"), 2, 0),
            SetupParticipant { current_hp: Some(0), ..part("C", Side::Party, sb("C"), 0, 2) },
        ],
    );
    let a = s.combatant(A).expect("A");
    assert!(a.is_seriously_wounded() && a.wound_penalty() == -2);
    start(&mut s, &[10, 1, 9]);
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    let r = request(&s);
    assert!(r.breakdown.contains(&("seriously wounded".to_string(), -2)));
    assert!(matches!(r.kind, RollKind::Check { modifier: 8, .. }));
    roll(&mut s, 2);
    end_turn(&mut s, A);
    // C (mortally wounded) starts with a death save, then acts at −4.
    assert_eq!(s.current_actor(), Some(C));
    assert!(matches!(request(&s).kind, RollKind::DeathSave { threshold: 5 }));
    roll(&mut s, 2);
    let c = s.combatant(C).expect("C");
    assert_eq!((c.death_save_penalty, c.effective_stat(Stat::Move)), (1, 1));
    act(&mut s, C, Action::Attack { weapon: 0, target: B });
    let r = request(&s);
    assert!(r.breakdown.contains(&("mortally wounded".to_string(), -4)));
    assert!(matches!(r.kind, RollKind::Check { modifier: 6, .. }));
}

#[test]
fn death_saves_success_failure_and_death() {
    let parts = vec![
        SetupParticipant { current_hp: Some(0), ..part("A", Side::Party, sb("A"), 0, 0) },
        part("B", Side::Enemy, sb("B"), 2, 0),
        part("C", Side::Party, sb("C"), 0, 2),
    ];
    let mut s = build(Battlefield::open(12, 12), parts);
    s.apply(&Command::StartCombat).expect("start");
    for n in [10, 5, 1] {
        roll(&mut s, n);
    }
    assert_eq!(s.current_actor(), Some(A));
    assert!(s.legal_actions().is_empty()); // death save pending
    assert!(s.hypothetical_turn(A).is_none());
    let ev = roll(&mut s, 5); // BODY 6, penalty 0: 5 < 6
    assert!(ev.contains(&Event::DeathSave { combatant: A, face: 5, threshold: 5, success: true }));
    assert!(!s.legal_actions().is_empty());
    end_turn(&mut s, A);
    end_turn(&mut s, B);
    end_turn(&mut s, C);
    assert_eq!(s.current_actor(), Some(A));
    assert!(matches!(request(&s).kind, RollKind::DeathSave { threshold: 4 }));
    let ev = roll(&mut s, 5); // 5 + 1 = 6, not < 6
    assert!(ev.contains(&Event::Died { combatant: A }));
    assert_eq!(s.current_actor(), Some(B)); // the dead skip their turn
    end_turn(&mut s, B);
    end_turn(&mut s, C);
    assert_eq!(s.current_actor(), Some(B)); // A is skipped next round too
    // A natural 10 always fails, even with a high BODY.
    let mut tough = sb("T");
    tough.stats.body = 12;
    let parts = vec![SetupParticipant { current_hp: Some(0), ..part("T", Side::Party, tough, 0, 0) }, part("B", Side::Enemy, sb("B"), 2, 0), part("C", Side::Party, sb("C"), 0, 2)];
    let mut s = build(Battlefield::open(12, 12), parts);
    start(&mut s, &[10, 5, 1]);
    assert!(roll(&mut s, 10).contains(&Event::Died { combatant: A }));
}

#[test]
fn damage_to_a_mortally_wounded_target_forces_a_death_save() {
    let parts = vec![
        part("A", Side::Party, sb("A"), 0, 0),
        SetupParticipant { current_hp: Some(0), ..part("B", Side::Enemy, sb("B"), 2, 0) },
        part("C", Side::Enemy, sb("C"), 6, 6),
    ];
    let mut s = build(Battlefield::open(12, 12), parts);
    start(&mut s, &[10, 1, 1]);
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 9);
    faces(&mut s, &[2, 2, 2]);
    assert_eq!(s.combatant(B).map(|b| b.hp), Some(0));
    assert_eq!(request(&s).roller, Some(B));
    assert!(matches!(request(&s).kind, RollKind::DeathSave { .. }));
    roll(&mut s, 9);
    assert!(s.combatant(B).is_some_and(|b| b.is_dead()));
    // Option off: no save.
    let parts = vec![
        part("A", Side::Party, sb("A"), 0, 0),
        SetupParticipant { current_hp: Some(0), ..part("B", Side::Enemy, sb("B"), 2, 0) },
        part("C", Side::Enemy, sb("C"), 6, 6),
    ];
    let mut s = build_opts(Battlefield::open(12, 12), parts, RulesOptions { death_save_on_damage: false, ..RulesOptions::default() });
    start(&mut s, &[10, 1, 1]);
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 9);
    faces(&mut s, &[2, 2, 2]);
    assert!(s.pending().is_none());
}

#[test]
fn combat_ends_when_a_side_is_down() {
    let parts = vec![part("A", Side::Party, sb("A"), 0, 0), SetupParticipant { current_hp: Some(5), ..part("B", Side::Enemy, sb("B"), 2, 0) }];
    let mut s = build(Battlefield::open(12, 12), parts);
    start(&mut s, &[10, 1]);
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    roll(&mut s, 9);
    let ev = faces(&mut s, &[3, 3, 3]);
    assert!(ev.contains(&Event::MortallyWounded { combatant: B }));
    assert!(ev.contains(&Event::CombatEnded { winner: Some(Side::Party) }));
    assert_eq!(s.phase, Phase::Ended { winner: Some(Side::Party) });
    assert!(s.is_over() && s.pending().is_none() && s.current_actor().is_none());
    assert!(matches!(s.apply(&Command::EndTurn { actor: A }), Err(CombatError::CombatOver)));
    assert!(s.legal_actions().is_empty());
}

// ---------------------------------------------------------------------------
// Robustness
// ---------------------------------------------------------------------------

#[test]
fn illegal_commands_leave_the_state_unchanged() {
    let mut s = duel(sb("A"), sb("B"));
    act(&mut s, A, Action::Move { to: Pos::new(1, 1) });
    let before = s.clone();
    let bad = [
        Command::Act { actor: A, action: Action::Move { to: Pos::new(5, 5) } },
        Command::Act { actor: B, action: Action::Attack { weapon: 0, target: A } },
        Command::Act { actor: A, action: Action::Attack { weapon: 7, target: B } },
        Command::Act { actor: A, action: Action::Attack { weapon: 0, target: A } },
        Command::Act { actor: A, action: Action::Run { to: Pos::new(11, 11) } },
        Command::roll(1, 5),
        Command::StartCombat,
        Command::Gm { op: GmOp::SetAmmo { target: A, weapon: 1, value: 3 } },
        Command::Gm { op: GmOp::Move { target: A, to: Pos::new(2, 0) } },
        Command::Gm { op: GmOp::AdjustHp { target: CombatantId(99), delta: 1 } },
    ];
    for cmd in &bad {
        assert!(s.apply(cmd).is_err(), "{cmd:?} should fail");
        assert_eq!(s, before, "{cmd:?} changed the state");
    }
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    let before = s.clone();
    let id = request(&s).request_id;
    for cmd in [
        Command::roll(id, 0),
        Command::roll(id, 11),
        Command::Decide { request_id: id, accept: true },
        Command::EndTurn { actor: A },
    ] {
        assert!(s.apply(&cmd).is_err(), "{cmd:?} should fail");
        assert_eq!(s, before);
    }
    roll(&mut s, 9);
    let before = s.clone();
    let id = request(&s).request_id;
    for f in [vec![3, 3], vec![3, 3, 7], vec![0, 3, 3], vec![3, 3, 3, 3]] {
        assert!(matches!(s.apply(&Command::faces(id, f)), Err(CombatError::InvalidRoll(_))));
        assert_eq!(s, before);
    }
}

#[test]
fn stale_request_ids_are_rejected() {
    let mut s = duel(sb("A"), sb("B"));
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    let id = request(&s).request_id;
    let stale = s.apply(&Command::roll(id - 1, 9));
    assert!(matches!(stale, Err(CombatError::WrongRequest { .. })));
    assert!(s.apply(&Command::roll(id + 1, 9)).is_err());
    roll(&mut s, 9);
    assert_eq!(request(&s).request_id, id + 1);
    assert!(matches!(s.apply(&Command::faces(id, vec![1, 1, 1])), Err(CombatError::WrongRequest { .. })));
}

#[test]
fn gm_operations() {
    let mut s = duel(sb("A"), sb("B"));
    let gm = |s: &mut CombatState, op| s.apply(&Command::Gm { op }).expect("gm op");
    gm(&mut s, GmOp::AdjustHp { target: B, delta: -100 });
    assert!(s.combatant(B).is_some_and(|b| b.is_mortally_wounded()));
    assert!(s.is_over());
    let mut s = duel(sb("A"), sb("B"));
    gm(&mut s, GmOp::SetSp { target: B, location: HitLocation::Head, value: 9 });
    gm(&mut s, GmOp::AddCriticalInjury { target: B, location: HitLocation::Head });
    gm(&mut s, GmOp::Move { target: B, to: Pos::new(5, 5) });
    let b = s.combatant(B).expect("B");
    assert_eq!((b.armor_sp.head, b.death_save_penalty, b.injuries.len(), b.position), (9, 1, 1, Pos::new(5, 5)));
    act(&mut s, A, Action::Attack { weapon: 0, target: B });
    assert!(s.pending().is_some());
    gm(&mut s, GmOp::ClearPending);
    assert!(s.pending().is_none() && s.queue.is_empty());
    assert_eq!(s.current_actor(), Some(A));
    end_turn(&mut s, A);
    assert_eq!(s.current_actor(), Some(B));
}

#[test]
fn legal_actions_cover_attacks_moves_and_reloads() {
    let mut a = sb("A");
    a.weapons = vec![weapon("smg"), weapon("heavy_pistol")];
    let parts = vec![
        part("A", Side::Party, a, 0, 0),
        part("B", Side::Enemy, sb("B"), 2, 0),
        part("Ally", Side::Party, sb("Ally"), 0, 1),
    ];
    let mut s = build(Battlefield::open(12, 12), parts);
    start(&mut s, &[10, 1, 1]);
    let legal = s.legal_actions();
    assert!(legal.contains(&Action::Attack { weapon: 0, target: B }));
    assert!(legal.contains(&Action::Autofire { weapon: 0, target: B }));
    assert!(legal.contains(&Action::AimedShot { weapon: 1, target: B }));
    assert!(!legal.contains(&Action::Autofire { weapon: 1, target: B }));
    assert!(!legal.iter().any(|x| x.attack().is_some_and(|(_, t, _)| t == C)));
    assert!(legal.contains(&Action::Move { to: Pos::new(6, 6) }));
    assert!(legal.contains(&Action::Run { to: Pos::new(6, 6) }));
    assert!(!legal.contains(&Action::Move { to: Pos::new(0, 1) }));
    for action in &legal {
        assert!(s.check_action(A, action).is_ok());
    }
    assert!(s.check_action(A, &Action::Attack { weapon: 1, target: C }).is_ok()); // friendly fire is legal
}

#[test]
fn hypothetical_turn_gives_a_fresh_turn() {
    let mut s = build(Battlefield::open(12, 12), vec![part("A", Side::Party, sb("A"), 0, 0), part("B", Side::Enemy, sb("B"), 2, 0)]);
    assert!(s.hypothetical_turn(B).is_none()); // not started
    start(&mut s, &[10, 1]);
    act(&mut s, A, Action::Move { to: Pos::new(0, 3) });
    let h = s.hypothetical_turn(B).expect("hypothetical");
    assert_eq!(h.current_actor(), Some(B));
    assert_eq!(h.combatant(B).map(|b| b.economy), Some(TurnEconomy::fresh()));
    assert!(!h.legal_actions().is_empty());
    let h = s.hypothetical_turn(A).expect("hypothetical");
    assert_eq!(h.combatant(A).map(|a| a.economy), Some(TurnEconomy::fresh()));
    assert_eq!(s.current_actor(), Some(A)); // original untouched
    assert!(s.hypothetical_turn(CombatantId(9)).is_none());
}

#[test]
fn analysis_profiles_and_expected_damage() {
    let mut b = sb("B");
    b.armor.body = Some(piece("Light Armorjack", 11, 0));
    let mut a = sb("A");
    a.weapons = vec![weapon("assault_rifle"), weapon("heavy_melee")];
    let s = duel(a, b);
    let p = analysis::attack_profile(&s, A, 0, B, AttackMode::Single).expect("profile");
    assert_eq!((p.dv, p.sp, p.can_dodge, p.head, p.has_ammo), (Some(17), 11, false, false, true));
    let single = analysis::expected_damage(&s, A, 0, B, AttackMode::Single);
    let aimed = analysis::expected_damage(&s, A, 0, B, AttackMode::Aimed);
    let auto = analysis::expected_damage(&s, A, 0, B, AttackMode::Autofire);
    assert!(single > 0.0 && aimed > 0.0 && auto > 0.0, "{single} {aimed} {auto}");
    // Hand check of the single shot: modifier 10 vs DV 17 needs X > 7.
    let p_hit = analysis::p_beat(10, 17);
    assert!((p_hit - 0.3).abs() < 1e-12);
    let dist = analysis::dice_sum_distribution(&"5d6".parse().expect("dice"));
    let through: f64 = dist.iter().map(|(d, p)| p * (d - 11).max(0) as f64).sum();
    let expected = p_hit * (through + 5.0 * analysis::p_critical_injury(&"5d6".parse().expect("dice")));
    assert!((single - expected).abs() < 1e-9);
    // Melee needs adjacency; autofire needs an autofire weapon.
    assert!(analysis::attack_profile(&s, A, 1, B, AttackMode::Single).is_none());
    assert!(analysis::attack_profile(&s, A, 1, B, AttackMode::Autofire).is_none());
    // Dodging is assumed whenever it lowers the hit chance.
    let mut quick = sb("Q");
    quick.stats.ref_ = 9;
    quick.stats.dex = 2;
    quick.skills.insert(Skill::Evasion, 0);
    let s = duel(sb("A"), quick);
    let p = analysis::attack_profile(&s, A, 0, B, AttackMode::Single).expect("profile");
    assert!(p.can_dodge && p.defender_modifier == Some(2));
    let e = analysis::expected_damage(&s, A, 0, B, AttackMode::Single);
    let vs_dv = analysis::p_beat(10, 13) * (10.5 + 5.0 * analysis::p_critical_injury(&"3d6".parse().expect("dice")));
    assert!((e - vs_dv).abs() < 1e-9, "weak dodger should not dodge: {e} vs {vs_dv}");
}

// ---------------------------------------------------------------------------
// Serialization and replay
// ---------------------------------------------------------------------------

#[test]
fn command_json_shape() {
    let c: Command = serde_json::from_str(r#"{"type":"provide_roll","request_id":3,"value":7}"#).expect("parse");
    assert_eq!(c, Command::roll(3, 7));
    let c: Command = serde_json::from_str(r#"{"type":"act","actor":1,"action":{"kind":"aimed_shot","weapon":0,"target":2}}"#).expect("parse");
    assert_eq!(c, Command::Act { actor: A, action: Action::AimedShot { weapon: 0, target: B } });
    let c: Command = serde_json::from_str(r#"{"type":"gm","op":{"kind":"set_sp","target":2,"location":"head","value":4}}"#).expect("parse");
    assert_eq!(c, Command::Gm { op: GmOp::SetSp { target: B, location: HitLocation::Head, value: 4 } });
    let json = serde_json::to_value(Phase::Ended { winner: Some(Side::Party) }).expect("json");
    assert_eq!(json, serde_json::json!({"kind": "ended", "winner": "party"}));
}

/// Picks a deterministic command for the scripted fight.
fn scripted_command(s: &CombatState, dice: &mut SeededDice) -> Option<Command> {
    if s.is_over() {
        return None;
    }
    match s.pending() {
        Some(PendingInput::Roll(r)) => Some(match r.kind {
            RollKind::Damage { dice: d } => Command::faces(r.request_id, dice.faces(&d)),
            _ => Command::roll(r.request_id, dice.die(10) as i32),
        }),
        Some(PendingInput::Decision(d)) => Some(Command::Decide { request_id: d.request_id, accept: d.request_id % 2 == 0 }),
        None => {
            let actor = s.current_actor()?;
            let legal = s.legal_actions();
            if let Some(a) = legal.iter().find(|a| matches!(a, Action::Autofire { .. })).or_else(|| legal.iter().find(|a| a.attack().is_some())) {
                return Some(Command::Act { actor, action: a.clone() });
            }
            if let Some(a) = legal.iter().find(|a| matches!(a, Action::Reload { .. })) {
                return Some(Command::Act { actor, action: a.clone() });
            }
            let me = s.combatant(actor)?;
            let nearest = |p: Pos| s.enemies_of(actor).filter(|e| !e.is_down()).map(|e| e.position.chebyshev(p)).min().unwrap_or(0);
            let here = nearest(me.position);
            let best = legal
                .iter()
                .filter_map(|a| match a {
                    Action::Move { to } => Some((nearest(*to), a)),
                    _ => None,
                })
                .min_by_key(|(d, _)| *d);
            match best {
                Some((d, a)) if d < here && d >= 1 => Some(Command::Act { actor, action: a.clone() }),
                _ => Some(Command::EndTurn { actor }),
            }
        }
    }
}

#[test]
fn scripted_fight_replays_exactly() {
    let initial = content().build_encounter("warehouse-standoff").expect("encounter");
    let mut s = initial.clone();
    s.apply(&Command::StartCombat).expect("start");
    let mut log = vec![Command::StartCombat];
    let mut dice = SeededDice::new(2077);
    let mut kinds = BTreeSet::new();
    for _ in 0..3000 {
        if let Some(PendingInput::Roll(r)) = s.pending() {
            check_representatives(&s);
            kinds.insert(match r.kind {
                RollKind::Check { beat: Some(_), .. } => "attack check",
                RollKind::Check { beat: None, .. } => "defence check",
                RollKind::CritFollowUp { .. } => "crit follow-up",
                RollKind::DeathSave { .. } => "death save",
                RollKind::Initiative { .. } => "initiative",
                RollKind::Damage { .. } => "damage",
            });
        }
        let Some(cmd) = scripted_command(&s, &mut dice) else { break };
        s.apply(&cmd).unwrap_or_else(|e| panic!("scripted {cmd:?} rejected: {e}"));
        log.push(cmd);
    }
    assert!(s.round >= 2, "fight should last several rounds (round {})", s.round);
    for k in ["initiative", "attack check", "damage", "crit follow-up"] {
        assert!(kinds.contains(k), "never saw a {k} roll: {kinds:?}");
    }
    let (replayed, _) = replay(&initial, &log).expect("replay");
    assert_eq!(replayed.fingerprint(), s.fingerprint());
    assert_eq!(replayed, s);
    // The state and the log survive JSON round trips.
    let json = serde_json::to_string(&s).expect("serialize state");
    let back: CombatState = serde_json::from_str(&json).expect("deserialize state");
    assert_eq!(back.fingerprint(), s.fingerprint());
    let log_json = serde_json::to_string(&log).expect("serialize log");
    let log_back: Vec<Command> = serde_json::from_str(&log_json).expect("deserialize log");
    let (again, _) = replay(&initial, &log_back).expect("replay");
    assert_eq!(again.fingerprint(), s.fingerprint());
    // Undo via the combat log.
    let mut clog = CombatLog { initial: initial.clone(), commands: log };
    let undone = clog.undo().expect("undo");
    assert_ne!(undone.fingerprint(), s.fingerprint());
}

#[test]
fn representative_answers_for_death_saves_and_defence_follow_ups() {
    // Death save branch weights.
    let parts = vec![
        SetupParticipant { current_hp: Some(0), ..part("A", Side::Party, sb("A"), 0, 0) },
        part("B", Side::Enemy, sb("B"), 1, 0),
        part("C", Side::Party, sb("C"), 0, 2),
    ];
    let mut s = build(Battlefield::open(12, 12), parts);
    start(&mut s, &[10, 5, 1]);
    let ans = analysis::representative_answers(&s);
    assert_eq!(ans.len(), 2);
    assert!((ans[0].1 - 0.5).abs() < 1e-12); // faces 1..5 succeed
    roll(&mut s, 1);
    // Melee defence critical follow-up uses buckets.
    let brawl = brawling_index(&s, A);
    act(&mut s, A, Action::Attack { weapon: brawl, target: B });
    roll(&mut s, 10); // defender crit
    assert_eq!(analysis::representative_answers(&s).len(), 3);
    roll(&mut s, 4);
    // Attacker check groups by outcome: beat = 10 + 10 + 4 = 24, modifier 6 (−4 mortally).
    let ans = analysis::representative_answers(&s);
    assert!(ans.len() <= 4);
    roll(&mut s, 10);
    roll(&mut s, 10); // 6 + 20 = 26 > 24
    assert!(matches!(request(&s).kind, RollKind::Damage { .. }));
    faces(&mut s, &[6, 6]);
}
