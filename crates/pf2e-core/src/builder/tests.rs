use super::content::Content;
use super::*;
use crate::models::character::{Character, FeatChoice};
use crate::models::feat::FeatType;

fn content() -> Content {
    Content::bundled().unwrap()
}

fn load(name: &str) -> Character {
    let text = match name {
        "fighter" => include_str!("../../../../content/characters/fighter.json"),
        "rogue" => include_str!("../../../../content/characters/rogue.json"),
        "cleric" => include_str!("../../../../content/characters/cleric.json"),
        "wizard" => include_str!("../../../../content/characters/wizard.json"),
        _ => unreachable!(),
    };
    serde_json::from_str(text).unwrap()
}

fn errors(r: &ValidationReport) -> Vec<String> {
    r.issues.iter().filter(|i| i.severity == Severity::Error).map(|i| format!("{}: {}", i.code, i.message)).collect()
}

#[test]
fn sample_characters_are_legal() {
    let c = content();
    for name in ["fighter", "rogue", "cleric", "wizard"] {
        let result = build(&load(name), &c);
        assert!(result.report.is_legal(), "{name}: {:?}", errors(&result.report));
        assert!(result.statblock.is_some());
    }
}

#[test]
fn fighter_numbers() {
    let r = build(&load("fighter"), &content());
    let sb = r.statblock.unwrap();
    assert_eq!(r.attributes.strength, 4);
    assert_eq!(sb.max_hp, 8 + 10 + 2); // ancestry + class + Con
    assert_eq!(sb.ac, 10 + 2 + 3 + 3); // dex (cap 2) + trained + scale mail
    let longsword = &sb.strikes[0];
    assert_eq!(longsword.attack_bonus, 4 + 5); // Str + expert at level 1
    assert_eq!(longsword.damage[0].dice.to_string(), "1d8+4");
    let bow = sb.strikes.iter().find(|s| s.name == "Shortbow").unwrap();
    assert_eq!(bow.attack_bonus, 2 + 5);
    assert_eq!(bow.damage[0].dice.to_string(), "1d6");
    assert!(sb.reactions.contains(&ReactionKind::ReactiveStrike));
    assert!(sb.reactions.contains(&ReactionKind::ShieldBlock));
    assert_eq!(sb.fortitude, 2 + 5);
    assert_eq!(sb.will, 1 + 3);
    assert!(sb.uses_dying_rules);
}

#[test]
fn rogue_gets_sneak_attack_and_finesse() {
    let sb = build(&load("rogue"), &content()).statblock.unwrap();
    let rapier = &sb.strikes[0];
    assert_eq!(rapier.attribute, Attribute::Dexterity);
    assert_eq!(rapier.attack_bonus, 4 + 3);
    assert!(sb.damage_riders.iter().any(|r| matches!(r, DamageRider::SneakAttack { dice } if dice.to_string() == "1d6")));
}

#[test]
fn caster_entry_is_derived() {
    let sb = build(&load("wizard"), &content()).statblock.unwrap();
    let e = &sb.spellcasting[0];
    assert_eq!(e.attack_bonus, 4 + 3);
    assert_eq!(e.dc, 17);
    assert_eq!(e.slots.get(&1), Some(&2));
}

#[test]
fn illegal_boosts_are_reported() {
    let mut ch = load("fighter");
    ch.boosts.level1_free = vec![Attribute::Strength, Attribute::Strength, Attribute::Dexterity, Attribute::Wisdom];
    let r = validate(&ch, &content());
    assert!(!r.is_legal());
    assert!(r.issues.iter().any(|i| i.code == "level1_boosts"));
}

#[test]
fn attribute_cap_at_level_one() {
    let mut ch = load("fighter");
    // Str from ancestry, background, class and free = +4; add Str via alternate ancestry too.
    ch.boosts.ancestry_alternate = true;
    ch.boosts.ancestry_free = vec![Attribute::Strength, Attribute::Constitution];
    let r = validate(&ch, &content());
    assert!(r.is_legal(), "{:?}", errors(&r));
    ch.boosts.level1_free = vec![Attribute::Strength, Attribute::Dexterity, Attribute::Constitution, Attribute::Wisdom];
    ch.boosts.background = vec![Attribute::Strength, Attribute::Wisdom];
    assert!(validate(&ch, &content()).is_legal());
}

#[test]
fn feat_rules() {
    let c = content();
    let mut ch = load("fighter");
    // Wrong class feat
    ch.feats.push(FeatChoice { feat_id: "nimble-dodge".into(), slot_type: FeatType::Class, slot_level: 2 });
    ch.level = 2;
    let r = validate(&ch, &c);
    assert!(r.issues.iter().any(|i| i.code == "slot_mismatch"), "{:?}", errors(&r));

    // Archetype feat without dedication
    let mut ch = load("fighter");
    ch.level = 4;
    ch.boosts.level_ups.clear();
    ch.feats.push(FeatChoice { feat_id: "basic-maneuver".into(), slot_type: FeatType::Archetype, slot_level: 4 });
    let r = validate(&ch, &c);
    assert!(r.issues.iter().any(|i| i.code == "missing_dedication"));

    // Feat above slot level
    let mut ch = load("fighter");
    ch.level = 2;
    ch.feats.push(FeatChoice { feat_id: "intimidating-strike".into(), slot_type: FeatType::Class, slot_level: 1 });
    let r = validate(&ch, &c);
    assert!(r.issues.iter().any(|i| i.code == "feat_level" || i.code == "slot_reused"));
}

#[test]
fn level_up_boosts_required_and_partial_above_four() {
    let c = content();
    let mut ch = load("fighter");
    ch.level = 5;
    let r = validate(&ch, &c);
    assert!(r.issues.iter().any(|i| i.code == "missing_boosts"));
    ch.boosts.level_ups.insert(5, vec![Attribute::Strength, Attribute::Dexterity, Attribute::Constitution, Attribute::Wisdom]);
    ch.boosts.level_ups.insert(10, vec![Attribute::Strength, Attribute::Dexterity, Attribute::Constitution, Attribute::Wisdom]);
    ch.level = 10;
    let r = build(&ch, &c);
    // +4 → partial at 5 → +5 at 10.
    assert_eq!(r.attributes.strength, 5);
}

#[test]
fn spells_must_match_tradition_and_slots() {
    let c = content();
    let mut ch = load("wizard");
    ch.spells.spells.push(crate::models::statblock::KnownSpell { spell_id: "heal".into(), rank: 1 });
    let r = validate(&ch, &c);
    assert!(r.issues.iter().any(|i| i.code == "tradition"));
    assert!(r.issues.iter().any(|i| i.code == "too_many_spells"));
}

#[test]
fn json_roundtrip_is_lossless() {
    let ch = load("cleric");
    let json = serde_json::to_string_pretty(&ch).unwrap();
    let back: Character = serde_json::from_str(&json).unwrap();
    assert_eq!(ch, back);
}
