//! Engine behaviour tests. Each test drives the engine purely through
//! commands, exactly as a UI would.

use super::command::{Action, Command, GmOp};
use super::event::Event;
use super::pending::{PendingInput, RollKind};
use super::replay::replay;
use super::setup::{CombatSetup, SetupParticipant};
use super::state::{CombatState, CombatantKind, LifeState, Phase};
use crate::builder::content::Content;
use crate::conditions::{Condition, ConditionKind as CK};
use crate::damage::DamageType;
use crate::grid::{CoverLevel, Pos};
use crate::ids::CombatantId;
use crate::models::battlefield::{Battlefield, Terrain};
use crate::models::statblock::{DamageComponent, ReactionKind, ShieldStats, Statblock, StrikeProfile, StrikeRange};
use crate::models::weapon::WeaponTrait;
use crate::stats::{Attribute, Side};

pub(crate) fn statblock(name: &str, hp: i32, ac: i32, attack: i32, damage: &str) -> Statblock {
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
            name: "Sword".into(),
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

fn participant(sb: Statblock, side: Side, x: i32, y: i32) -> SetupParticipant {
    SetupParticipant {
        name: sb.name.clone(),
        side,
        kind: if side == Side::Party { CombatantKind::Character } else { CombatantKind::Monster },
        statblock: sb,
        position: Pos::new(x, y),
        current_hp: None,
    }
}

fn build(parts: Vec<SetupParticipant>) -> CombatState {
    let content = Content::bundled().unwrap();
    CombatSetup::default().build(Battlefield::open(12, 12), parts, &content).unwrap()
}

/// Answers the pending roll and returns the events.
fn roll(s: &mut CombatState, value: i32) -> Vec<Event> {
    let id = s.pending().expect("a roll should be pending").request_id();
    s.apply(&Command::ProvideRoll { request_id: id, value, extra_modifier: 0 }).unwrap()
}

fn decide(s: &mut CombatState, accept: bool) -> Vec<Event> {
    let id = s.pending().expect("a decision should be pending").request_id();
    s.apply(&Command::Decide { request_id: id, accept }).unwrap()
}

/// Starts combat with natural initiative rolls given in combatant-id order.
fn start(s: &mut CombatState, inits: &[i32]) {
    s.apply(&Command::StartCombat).unwrap();
    for &v in inits {
        roll(s, v);
    }
    assert_eq!(s.phase, Phase::Active);
}

const A: CombatantId = CombatantId(1);
const B: CombatantId = CombatantId(2);
const C: CombatantId = CombatantId(3);

fn strike(actor: CombatantId, target: CombatantId) -> Command {
    Command::Act { actor, action: Action::Strike { actor_strike: 0, target, cover_override: None } }
}

fn pending_check(s: &CombatState) -> (i32, i32) {
    match s.pending() {
        Some(PendingInput::Roll(r)) => match r.kind {
            RollKind::Check { modifier, dc } => (modifier, dc),
            k => panic!("expected check, got {k:?}"),
        },
        p => panic!("expected roll, got {p:?}"),
    }
}

#[test]
fn initiative_ties_favour_enemies() {
    let mut s = build(vec![
        participant(statblock("Hero", 20, 18, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Orc", 20, 16, 9, "1d8+4"), Side::Enemy, 5, 5),
    ]);
    start(&mut s, &[12, 12]);
    assert_eq!(s.current_actor(), Some(B));
}

#[test]
fn strike_hits_applies_damage_and_map() {
    let mut s = build(vec![
        participant(statblock("Hero", 20, 18, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 1, 0),
    ]);
    start(&mut s, &[20, 1]);
    s.apply(&strike(A, B)).unwrap();
    assert_eq!(pending_check(&s), (9, 16));
    roll(&mut s, 10); // 19 vs 16: hit
    let ev = roll(&mut s, 7); // damage total 7
    assert!(ev.iter().any(|e| matches!(e, Event::DamageTaken { target, amount: 7, hp: 23, .. } if *target == B)));
    s.apply(&strike(A, B)).unwrap();
    assert_eq!(pending_check(&s).0, 4, "second strike takes -5 MAP");
    roll(&mut s, 2); // miss
    s.apply(&strike(A, B)).unwrap();
    assert_eq!(pending_check(&s).0, -1, "third strike takes -10 MAP");
}

#[test]
fn agile_map_and_crit_with_deadly() {
    let mut hero = statblock("Hero", 20, 18, 9, "1d6+3");
    hero.strikes[0].traits = vec![WeaponTrait::Agile, WeaponTrait::Deadly(10)];
    let mut s = build(vec![
        participant(hero, Side::Party, 0, 0),
        participant(statblock("Orc", 60, 16, 7, "1d6+2"), Side::Enemy, 1, 0),
    ]);
    start(&mut s, &[20, 1]);
    s.apply(&strike(A, B)).unwrap();
    roll(&mut s, 20); // nat 20, 29 vs 16 → crit
    let ev = roll(&mut s, 6); // base 6 → doubled 12
    assert!(ev.iter().any(|e| matches!(e, Event::DamageRolled { counted: 12, .. })));
    match s.pending() {
        Some(PendingInput::Roll(r)) => assert_eq!(r.expression, "1d10"),
        p => panic!("expected deadly die, got {p:?}"),
    }
    roll(&mut s, 4);
    assert_eq!(s.combatant(B).unwrap().hp, 60 - 16);
    s.apply(&strike(A, B)).unwrap();
    assert_eq!(pending_check(&s).0, 5, "agile MAP is -4");
}

#[test]
fn rejects_impossible_rolls() {
    let mut s = build(vec![
        participant(statblock("Hero", 20, 18, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 1, 0),
    ]);
    start(&mut s, &[20, 1]);
    s.apply(&strike(A, B)).unwrap();
    let id = s.pending().unwrap().request_id();
    assert!(s.apply(&Command::ProvideRoll { request_id: id, value: 21, extra_modifier: 0 }).is_err());
    assert!(s.apply(&Command::ProvideRoll { request_id: id + 1, value: 10, extra_modifier: 0 }).is_err());
    roll(&mut s, 15);
    let id = s.pending().unwrap().request_id();
    assert!(s.apply(&Command::ProvideRoll { request_id: id, value: 13, extra_modifier: 0 }).is_err(), "1d8+4 max is 12");
}

#[test]
fn illegal_actions_leave_state_untouched() {
    let mut s = build(vec![
        participant(statblock("Hero", 20, 18, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 6, 6),
    ]);
    start(&mut s, &[20, 1]);
    let before = s.clone();
    assert!(s.apply(&strike(A, B)).is_err(), "out of reach");
    assert!(s.apply(&strike(B, A)).is_err(), "not B's turn");
    assert!(s.apply(&Command::Act { actor: A, action: Action::Stride { to: Pos::new(11, 11) } }).is_err());
    assert_eq!(s, before);
}

#[test]
fn monster_dies_and_combat_ends() {
    let mut s = build(vec![
        participant(statblock("Hero", 20, 18, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Rat", 5, 12, 3, "1d4"), Side::Enemy, 1, 0),
    ]);
    start(&mut s, &[20, 1]);
    s.apply(&strike(A, B)).unwrap();
    roll(&mut s, 10);
    let ev = roll(&mut s, 8);
    assert!(ev.iter().any(|e| matches!(e, Event::Died { combatant } if *combatant == B)));
    assert_eq!(s.phase, Phase::Ended { winner: Some(Side::Party) });
}

#[test]
fn dying_and_recovery_checks() {
    let mut hero = statblock("Hero", 15, 10, 9, "1d8+4");
    hero.uses_dying_rules = true;
    let mut ally = statblock("Ally", 10, 18, 9, "1d8+4");
    ally.uses_dying_rules = true;
    let mut s = build(vec![
        participant(hero, Side::Party, 0, 0),
        participant(statblock("Ogre", 50, 16, 12, "2d6+6"), Side::Enemy, 1, 0),
        participant(ally, Side::Party, 8, 8),
    ]);
    start(&mut s, &[5, 20, 1]); // Ogre, Hero, Ally
    assert_eq!(s.current_actor(), Some(B));
    s.apply(&strike(B, A)).unwrap();
    roll(&mut s, 10); // 22 vs 10: crit
    let ev = roll(&mut s, 10); // 20 damage
    assert!(ev.iter().any(|e| matches!(e, Event::KnockedOut { dying: 2, .. })));
    let hero = s.combatant(A).unwrap();
    assert!(hero.has(CK::Unconscious));
    assert_eq!(hero.conditions.value(CK::Dying), 2);
    s.apply(&Command::EndTurn { actor: B }).unwrap();
    // Hero's turn: recovery check DC 12.
    match s.pending() {
        Some(PendingInput::Roll(r)) => assert_eq!(r.kind, RollKind::Flat { dc: 12 }),
        p => panic!("expected recovery check, got {p:?}"),
    }
    roll(&mut s, 5); // failure: dying 3
    assert_eq!(s.combatant(A).unwrap().conditions.value(CK::Dying), 3);
    // Turn auto-ended; ally acts.
    assert_eq!(s.current_actor(), Some(C));
    s.apply(&Command::EndTurn { actor: C }).unwrap();
    s.apply(&Command::EndTurn { actor: B }).unwrap();
    roll(&mut s, 1); // critical failure: dying 5 → dead
    assert!(s.combatant(A).unwrap().is_dead());
}

#[test]
fn healing_wakes_and_wounds() {
    let mut hero = statblock("Hero", 10, 10, 9, "1d8+4");
    hero.uses_dying_rules = true;
    let mut s = build(vec![
        participant(hero, Side::Party, 0, 0),
        participant(statblock("Ogre", 50, 16, 12, "2d6+6"), Side::Enemy, 1, 0),
        participant(statblock("Friend", 10, 18, 9, "1d8+4"), Side::Party, 8, 8),
    ]);
    start(&mut s, &[5, 20, 1]);
    s.apply(&strike(B, A)).unwrap();
    roll(&mut s, 5);
    roll(&mut s, 12);
    assert_eq!(s.combatant(A).unwrap().conditions.value(CK::Dying), 1);
    s.apply(&Command::Gm { op: GmOp::AdjustHp { target: A, delta: 5 } }).unwrap();
    let hero = s.combatant(A).unwrap();
    assert_eq!(hero.hp, 5);
    assert!(!hero.has(CK::Unconscious));
    assert_eq!(hero.conditions.value(CK::Wounded), 1);
}

#[test]
fn reactive_strike_on_movement() {
    let mut brute = statblock("Brute", 40, 16, 10, "1d10+5");
    brute.reactions = vec![ReactionKind::ReactiveStrike];
    let mut s = build(vec![
        participant(statblock("Hero", 30, 15, 9, "1d8+4"), Side::Party, 1, 0),
        participant(brute, Side::Enemy, 2, 0),
    ]);
    start(&mut s, &[20, 1]);
    s.apply(&Command::Act { actor: A, action: Action::Stride { to: Pos::new(1, 4) } }).unwrap();
    match s.pending() {
        Some(PendingInput::Decision(d)) => {
            assert_eq!(d.decider, B);
            assert_eq!(d.reaction, ReactionKind::ReactiveStrike);
        }
        p => panic!("expected reaction decision, got {p:?}"),
    }
    decide(&mut s, true);
    assert_eq!(pending_check(&s), (10, 15), "reactive strike ignores MAP");
    roll(&mut s, 10);
    roll(&mut s, 8);
    assert_eq!(s.combatant(A).unwrap().hp, 22);
    assert_eq!(s.combatant(A).unwrap().position, Pos::new(1, 4), "movement continues after the reaction");
    assert!(!s.combatant(B).unwrap().reaction_available);
}

#[test]
fn step_does_not_provoke() {
    let mut brute = statblock("Brute", 40, 16, 10, "1d10+5");
    brute.reactions = vec![ReactionKind::ReactiveStrike];
    let mut s = build(vec![
        participant(statblock("Hero", 30, 15, 9, "1d8+4"), Side::Party, 1, 0),
        participant(brute, Side::Enemy, 2, 0),
    ]);
    start(&mut s, &[20, 1]);
    s.apply(&Command::Act { actor: A, action: Action::Step { to: Pos::new(0, 0) } }).unwrap();
    assert!(s.pending().is_none());
    assert_eq!(s.combatant(A).unwrap().position, Pos::new(0, 0));
}

#[test]
fn shield_block_reduces_damage() {
    let mut hero = statblock("Hero", 30, 16, 9, "1d8+4");
    hero.reactions = vec![ReactionKind::ShieldBlock];
    hero.shield = Some(ShieldStats { name: "Steel".into(), ac_bonus: 2, hardness: 5, max_hp: 20, broken_threshold: 10 });
    let mut s = build(vec![participant(hero, Side::Party, 0, 0), participant(statblock("Orc", 30, 16, 12, "1d12+4"), Side::Enemy, 1, 0)]);
    start(&mut s, &[20, 1]);
    s.apply(&Command::Act { actor: A, action: Action::RaiseShield }).unwrap();
    s.apply(&Command::EndTurn { actor: A }).unwrap();
    s.apply(&strike(B, A)).unwrap();
    assert_eq!(pending_check(&s).1, 18, "raised shield adds +2 circumstance AC");
    roll(&mut s, 10);
    roll(&mut s, 12);
    decide(&mut s, true);
    let hero = s.combatant(A).unwrap();
    assert_eq!(hero.hp, 30 - 7);
    assert_eq!(hero.shield_hp, 20 - 7);
}

#[test]
fn persistent_damage_ticks_and_ends() {
    let mut s = build(vec![
        participant(statblock("Hero", 30, 16, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 5, 5),
    ]);
    start(&mut s, &[20, 1]);
    s.combatants[0].persistent.push(super::state::PersistentDamage { dice: "1d6".parse().unwrap(), damage_type: DamageType::Fire, source: None });
    s.apply(&Command::EndTurn { actor: A }).unwrap();
    roll(&mut s, 4); // damage
    assert_eq!(s.combatant(A).unwrap().hp, 26);
    roll(&mut s, 14); // DC 15 flat: fails
    assert_eq!(s.combatant(A).unwrap().persistent.len(), 1);
    assert_eq!(s.current_actor(), Some(B));
    s.apply(&Command::EndTurn { actor: B }).unwrap();
    s.apply(&Command::EndTurn { actor: A }).unwrap();
    roll(&mut s, 3);
    roll(&mut s, 15); // success
    assert!(s.combatant(A).unwrap().persistent.is_empty());
}

#[test]
fn concealment_flat_check() {
    let mut s = build(vec![
        participant(statblock("Hero", 30, 16, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 1, 0),
    ]);
    start(&mut s, &[20, 1]);
    s.apply(&Command::Gm { op: GmOp::AddCondition { target: B, condition: Condition::new(CK::Concealed, 0) } }).unwrap();
    s.apply(&strike(A, B)).unwrap();
    match s.pending() {
        Some(PendingInput::Roll(r)) => assert_eq!(r.kind, RollKind::Flat { dc: 5 }),
        p => panic!("{p:?}"),
    }
    roll(&mut s, 4); // fails: attack lost
    assert!(s.pending().is_none());
    assert_eq!(s.combatant(A).unwrap().attacks_this_turn, 1, "the attack still counts for MAP");
}

#[test]
fn cover_and_flanking_modify_ac() {
    let mut bf = Battlefield::open(12, 12);
    bf.set_terrain(Pos::new(3, 5), Terrain::Wall);
    // The wall blocks the centre line from (1,4) to (5,5) but not every line: standard cover.
    let content = Content::bundled().unwrap();
    let mut archer = statblock("Archer", 30, 16, 9, "1d8");
    archer.strikes[0].range = StrikeRange::Ranged { increment_ft: 60 };
    let mut s = CombatSetup::default()
        .build(
            bf,
            vec![
                participant(archer, Side::Party, 1, 4),
                participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 5, 5),
                participant(statblock("Fighter", 30, 16, 9, "1d8+4"), Side::Party, 4, 4),
                participant(statblock("Rogue", 30, 16, 9, "1d6+3"), Side::Party, 6, 6),
            ],
            &content,
        )
        .unwrap();
    start(&mut s, &[20, 1, 10, 9]);
    s.apply(&strike(A, B)).unwrap();
    assert_eq!(pending_check(&s).1, 18, "wall gives standard cover");
    roll(&mut s, 1);
    s.apply(&Command::Act { actor: A, action: Action::Strike { actor_strike: 0, target: B, cover_override: Some(CoverLevel::None) } }).unwrap();
    assert_eq!(pending_check(&s).1, 16, "GM override removes cover");
    roll(&mut s, 1);
    s.apply(&Command::EndTurn { actor: A }).unwrap();
    // Fighter (4,4) and rogue (6,6) flank the orc at (5,5).
    assert_eq!(s.current_actor(), Some(C));
    s.apply(&strike(C, B)).unwrap();
    assert_eq!(pending_check(&s).1, 14, "flanked target is off-guard");
}

#[test]
fn basic_save_spell_hits_two_targets() {
    let content = Content::bundled().unwrap();
    let mut wiz = statblock("Wizard", 20, 15, 5, "1d4");
    wiz.spellcasting = vec![crate::models::statblock::SpellcastingEntry {
        name: "Arcane".into(),
        tradition: crate::models::spell::Tradition::Arcane,
        kind: crate::models::statblock::SpellcastingKind::Prepared,
        attribute: Attribute::Intelligence,
        attack_bonus: 7,
        dc: 17,
        slots: Default::default(),
        spells: vec![],
        cantrips: vec!["electric-arc".into()],
    }];
    let mut s = CombatSetup::default()
        .build(
            Battlefield::open(12, 12),
            vec![
                participant(wiz, Side::Party, 0, 0),
                participant(statblock("Orc A", 30, 16, 7, "1d6+2"), Side::Enemy, 3, 0),
                participant(statblock("Orc B", 30, 16, 7, "1d6+2"), Side::Enemy, 3, 2),
            ],
            &content,
        )
        .unwrap();
    start(&mut s, &[20, 1, 1]);
    s.apply(&Command::Act {
        actor: A,
        action: Action::CastSpell { entry: 0, spell_id: "electric-arc".into(), rank: 1, targets: vec![B, C], aim: None },
    })
    .unwrap();
    assert_eq!(s.combatant(A).unwrap().actions_remaining, 1);
    assert_eq!(pending_check(&s), (5, 17));
    roll(&mut s, 12); // 17: success → half
    roll(&mut s, 1); // 6 → crit failure (nat 1) → double
    match s.pending() {
        Some(PendingInput::Roll(r)) => assert_eq!(r.expression, "2d4"),
        p => panic!("{p:?}"),
    }
    roll(&mut s, 6);
    assert_eq!(s.combatant(B).unwrap().hp, 27);
    assert_eq!(s.combatant(C).unwrap().hp, 18);
}

#[test]
fn fear_frightens_and_decays() {
    let content = Content::bundled().unwrap();
    let mut cleric = statblock("Cleric", 20, 15, 5, "1d6");
    cleric.spellcasting = vec![crate::models::statblock::SpellcastingEntry {
        name: "Divine".into(),
        tradition: crate::models::spell::Tradition::Divine,
        kind: crate::models::statblock::SpellcastingKind::Prepared,
        attribute: Attribute::Wisdom,
        attack_bonus: 7,
        dc: 17,
        slots: [(1u8, 2u8)].into_iter().collect(),
        spells: vec![crate::models::statblock::KnownSpell { spell_id: "fear".into(), rank: 1 }],
        cantrips: vec![],
    }];
    let mut s = CombatSetup::default()
        .build(
            Battlefield::open(12, 12),
            vec![participant(cleric, Side::Party, 0, 0), participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 4, 0)],
            &content,
        )
        .unwrap();
    start(&mut s, &[20, 1]);
    s.apply(&Command::Act { actor: A, action: Action::CastSpell { entry: 0, spell_id: "fear".into(), rank: 1, targets: vec![B], aim: None } })
        .unwrap();
    roll(&mut s, 5); // 10 vs 17: failure → frightened 2
    assert_eq!(s.combatant(B).unwrap().conditions.value(CK::Frightened), 2);
    assert_eq!(s.combatant(A).unwrap().spell_slots[0][&1], 1);
    s.apply(&Command::EndTurn { actor: A }).unwrap();
    // Orc's attack suffers frightened.
    s.apply(&Command::Gm { op: GmOp::Move { target: B, to: Pos::new(1, 0) } }).unwrap();
    s.apply(&strike(B, A)).unwrap();
    assert_eq!(pending_check(&s).0, 5);
    roll(&mut s, 1);
    s.apply(&Command::EndTurn { actor: B }).unwrap();
    assert_eq!(s.combatant(B).unwrap().conditions.value(CK::Frightened), 1);
}

#[test]
fn hazard_triggers_on_entry() {
    let content = Content::bundled().unwrap();
    let pit = content.monsters.iter().find(|m| m.id == "spiked-pit").unwrap().statblock.clone();
    let mut s = CombatSetup::default()
        .build(
            Battlefield::open(12, 12),
            vec![
                participant(statblock("Hero", 30, 16, 9, "1d8+4"), Side::Party, 0, 0),
                participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 10, 10),
                SetupParticipant {
                    name: "Pit".into(),
                    side: Side::Enemy,
                    kind: CombatantKind::Hazard,
                    statblock: pit,
                    position: Pos::new(2, 0),
                    current_hp: None,
                },
            ],
            &content,
        )
        .unwrap();
    start(&mut s, &[20, 1]); // the simple hazard does not roll initiative
    s.apply(&Command::Act { actor: A, action: Action::Stride { to: Pos::new(4, 0) } }).unwrap();
    assert_eq!(s.combatant(A).unwrap().position, Pos::new(2, 0), "movement stops at the trap");
    assert_eq!(pending_check(&s), (5, 17));
    roll(&mut s, 3); // failure: full damage
    roll(&mut s, 10);
    assert_eq!(s.combatant(A).unwrap().hp, 20);
    assert_eq!(s.combatant(C).unwrap().life, LifeState::Inactive);
}

#[test]
fn replay_reproduces_state_exactly() {
    let mut s = build(vec![
        participant(statblock("Hero", 20, 18, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 3, 0),
    ]);
    let initial = s.clone();
    let mut log = Vec::new();
    let mut run = |s: &mut CombatState, c: Command| {
        s.apply(&c).unwrap();
        log.push(c);
    };
    run(&mut s, Command::StartCombat);
    run(&mut s, Command::ProvideRoll { request_id: 1, value: 15, extra_modifier: 0 });
    run(&mut s, Command::ProvideRoll { request_id: 2, value: 3, extra_modifier: 0 });
    run(&mut s, Command::Act { actor: A, action: Action::Stride { to: Pos::new(2, 0) } });
    run(&mut s, strike(A, B));
    run(&mut s, Command::ProvideRoll { request_id: 3, value: 14, extra_modifier: 0 });
    run(&mut s, Command::ProvideRoll { request_id: 4, value: 9, extra_modifier: 2 });
    run(&mut s, Command::EndTurn { actor: A });
    let (replayed, _) = replay(&initial, &log).unwrap();
    assert_eq!(replayed, s);
    assert_eq!(replayed.fingerprint(), s.fingerprint());
    assert_eq!(s.combatant(B).unwrap().hp, 30 - 11);
    // State survives a JSON round trip.
    let json = serde_json::to_string(&s).unwrap();
    let back: CombatState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
}

#[test]
fn slowed_and_stunned_reduce_actions() {
    let mut s = build(vec![
        participant(statblock("Hero", 20, 18, 9, "1d8+4"), Side::Party, 0, 0),
        participant(statblock("Orc", 30, 16, 7, "1d6+2"), Side::Enemy, 5, 5),
    ]);
    start(&mut s, &[20, 1]);
    s.apply(&Command::Gm { op: GmOp::AddCondition { target: B, condition: Condition::new(CK::Slowed, 1) } }).unwrap();
    s.apply(&Command::EndTurn { actor: A }).unwrap();
    assert_eq!(s.combatant(B).unwrap().actions_remaining, 2);
    s.apply(&Command::Gm { op: GmOp::AddCondition { target: A, condition: Condition::new(CK::Stunned, 2) } }).unwrap();
    s.apply(&Command::EndTurn { actor: B }).unwrap();
    let hero = s.combatant(A).unwrap();
    assert_eq!(hero.actions_remaining, 1);
    assert!(!hero.has(CK::Stunned));
}

#[test]
fn legal_actions_are_all_accepted() {
    let content = Content::bundled().unwrap();
    let adept = content.monsters.iter().find(|m| m.id == "cult-adept").unwrap().statblock.clone();
    let mut s = CombatSetup::default()
        .build(
            Battlefield::open(12, 12),
            vec![participant(statblock("Hero", 20, 18, 9, "1d8+4"), Side::Party, 0, 0), participant(adept, Side::Enemy, 4, 0)],
            &content,
        )
        .unwrap();
    start(&mut s, &[1, 20]);
    assert_eq!(s.current_actor(), Some(B));
    let actions = s.legal_actions();
    assert!(actions.iter().any(|a| matches!(a, Action::CastSpell { spell_id, .. } if spell_id == "fear")));
    for a in actions {
        let mut probe = s.clone();
        probe.apply(&Command::Act { actor: B, action: a.clone() }).unwrap_or_else(|e| panic!("{a:?}: {e}"));
    }
}
