//! Character builder: validates a [`Character`] build against the rules and
//! derives its combat [`Statblock`].
//!
//! Validation never panics and never stops at the first problem: it returns a
//! full report so a UI can highlight every issue at once.

pub mod content;

use crate::dice::DiceExpr;
use crate::models::character::{Character, ProficiencyTable};
use crate::models::feat::{FeatEffect, FeatType, Prerequisite};
use crate::models::statblock::{
    DamageComponent, DamageRider, KnownSpell, ReactionKind, ShieldStats, SpellcastingEntry, Statblock, StrikeProfile,
    StrikeRange,
};
use crate::models::weapon::{ArmorCategory, WeaponCategory, WeaponTrait};
use crate::damage::DamageType;
use crate::stats::{Attribute, AttributeSet, Proficiency, Skill};
use content::{rank_at, BoostSlot, ClassFeatureKind, Content};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub code: String,
    /// JSON-pointer-ish path into the character ("feats/2").
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn is_legal(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Error)
    }
    fn error(&mut self, code: &str, path: impl Into<String>, msg: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: Severity::Error,
            code: code.into(),
            path: path.into(),
            message: msg.into(),
        });
    }
    fn warn(&mut self, code: &str, path: impl Into<String>, msg: impl Into<String>) {
        self.issues.push(ValidationIssue {
            severity: Severity::Warning,
            code: code.into(),
            path: path.into(),
            message: msg.into(),
        });
    }
}

/// Everything the builder computes, returned together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildResult {
    pub report: ValidationReport,
    pub attributes: AttributeSet,
    pub proficiencies: ProficiencyTable,
    /// Present when the build had no errors.
    pub statblock: Option<Statblock>,
}

const LEVEL_UP_BOOST_LEVELS: [u8; 4] = [5, 10, 15, 20];

/// Validates and derives in one pass.
pub fn build(c: &Character, content: &Content) -> BuildResult {
    let mut report = ValidationReport::default();
    let attributes = compute_attributes(c, content, &mut report);
    let proficiencies = compute_proficiencies(c, content, &attributes, &mut report);
    validate_feats(c, content, &attributes, &proficiencies, &mut report);
    validate_equipment(c, content, &mut report);
    validate_spells(c, content, &mut report);
    let statblock = if report.is_legal() {
        Some(derive_statblock(c, content, &attributes, &proficiencies))
    } else {
        None
    };
    BuildResult { report, attributes, proficiencies, statblock }
}

pub fn validate(c: &Character, content: &Content) -> ValidationReport {
    build(c, content).report
}

// ---------------------------------------------------------------------------
// Attributes
// ---------------------------------------------------------------------------

fn distinct(v: &[Attribute]) -> bool {
    let set: BTreeSet<_> = v.iter().collect();
    set.len() == v.len()
}

fn compute_attributes(c: &Character, content: &Content, r: &mut ValidationReport) -> AttributeSet {
    let mut attrs = AttributeSet::default();
    if !(1..=20).contains(&c.level) {
        r.error("level_range", "level", "level must be between 1 and 20");
    }
    if c.name.trim().is_empty() {
        r.error("name_required", "name", "character needs a name");
    }

    // Ancestry
    match content.ancestry(&c.ancestry_id) {
        None => r.error("unknown_ancestry", "ancestry_id", format!("unknown ancestry '{}'", c.ancestry_id)),
        Some(a) => {
            let b = &c.boosts;
            if b.ancestry_alternate {
                if b.ancestry_free.len() != 2 || !distinct(&b.ancestry_free) {
                    r.error("ancestry_boosts", "boosts/ancestry_free", "alternate ancestry boosts need two different attributes");
                }
                for &x in &b.ancestry_free {
                    *attrs.get_mut(x) += 1;
                }
            } else {
                let fixed: Vec<Attribute> = a
                    .boosts
                    .iter()
                    .filter_map(|s| if let BoostSlot::Fixed(x) = s { Some(*x) } else { None })
                    .collect();
                let free_slots = a.boosts.iter().filter(|s| matches!(s, BoostSlot::Free)).count();
                if b.ancestry_free.len() != free_slots {
                    r.error(
                        "ancestry_boosts",
                        "boosts/ancestry_free",
                        format!("{} has {} free boost(s); {} chosen", a.name, free_slots, b.ancestry_free.len()),
                    );
                }
                let mut all = fixed.clone();
                all.extend(&b.ancestry_free);
                if !distinct(&all) {
                    r.error("ancestry_boosts", "boosts/ancestry_free", "ancestry boosts must target different attributes");
                }
                for x in all {
                    *attrs.get_mut(x) += 1;
                }
                for &f in &a.flaws {
                    *attrs.get_mut(f) -= 1;
                }
            }
        }
    }

    // Background
    match content.background(&c.background_id) {
        None => r.error("unknown_background", "background_id", format!("unknown background '{}'", c.background_id)),
        Some(bg) => {
            let b = &c.boosts.background;
            if b.len() != 2 || !distinct(b) {
                r.error("background_boosts", "boosts/background", "background grants two boosts to different attributes");
            } else if !bg.boost_options.contains(&b[0]) {
                r.error(
                    "background_boosts",
                    "boosts/background/0",
                    format!("first {} boost must be one of {:?}", bg.name, bg.boost_options),
                );
            }
            for &x in b.iter().take(2) {
                *attrs.get_mut(x) += 1;
            }
        }
    }

    // Class key attribute
    match content.class(&c.class_id) {
        None => r.error("unknown_class", "class_id", format!("unknown class '{}'", c.class_id)),
        Some(cl) => {
            if !cl.key_attributes.contains(&c.boosts.class_key) {
                r.error(
                    "class_key",
                    "boosts/class_key",
                    format!("{} key attribute must be one of {:?}", cl.name, cl.key_attributes),
                );
            }
            *attrs.get_mut(c.boosts.class_key) += 1;
        }
    }

    // Four free level-1 boosts
    let l1 = &c.boosts.level1_free;
    if l1.len() != 4 || !distinct(l1) {
        r.error("level1_boosts", "boosts/level1_free", "choose four different attributes for the free level 1 boosts");
    }
    for &x in l1.iter().take(4) {
        *attrs.get_mut(x) += 1;
    }
    for a in Attribute::ALL {
        if attrs.get(a) > 4 {
            r.error("attribute_cap", "boosts", format!("{} cannot exceed +4 at level 1", a.short()));
        }
    }

    // Level-up boosts with partial boosts above +4.
    let mut partial: BTreeMap<Attribute, bool> = BTreeMap::new();
    for lvl in LEVEL_UP_BOOST_LEVELS {
        let chosen = c.boosts.level_ups.get(&lvl);
        if lvl <= c.level {
            match chosen {
                None => r.error("missing_boosts", format!("boosts/level_ups/{lvl}"), format!("level {lvl} boosts not chosen")),
                Some(v) if v.len() != 4 || !distinct(v) => {
                    r.error("level_boosts", format!("boosts/level_ups/{lvl}"), "four different attributes required")
                }
                _ => {}
            }
            for &x in chosen.into_iter().flatten().take(4) {
                if attrs.get(x) >= 4 {
                    let p = partial.entry(x).or_insert(false);
                    if *p {
                        *attrs.get_mut(x) += 1;
                    }
                    *p = !*p;
                } else {
                    *attrs.get_mut(x) += 1;
                }
            }
        } else if chosen.is_some() {
            r.error("future_boosts", format!("boosts/level_ups/{lvl}"), format!("boosts for level {lvl} exceed character level"));
        }
    }
    for k in c.boosts.level_ups.keys() {
        if !LEVEL_UP_BOOST_LEVELS.contains(k) {
            r.error("bad_boost_level", format!("boosts/level_ups/{k}"), "attribute boosts only occur at 5, 10, 15, 20");
        }
    }
    attrs
}

// ---------------------------------------------------------------------------
// Proficiencies & skills
// ---------------------------------------------------------------------------

fn compute_proficiencies(c: &Character, content: &Content, attrs: &AttributeSet, r: &mut ValidationReport) -> ProficiencyTable {
    let mut t = ProficiencyTable::default();
    let Some(cl) = content.class(&c.class_id) else { return t };
    let lvl = c.level;
    t.perception = rank_at(&cl.perception, lvl);
    t.fortitude = rank_at(&cl.fortitude, lvl);
    t.reflex = rank_at(&cl.reflex, lvl);
    t.will = rank_at(&cl.will, lvl);
    t.unarmed = rank_at(&cl.unarmed, lvl);
    t.simple = rank_at(&cl.simple, lvl);
    t.martial = rank_at(&cl.martial, lvl);
    t.advanced = rank_at(&cl.advanced, lvl);
    t.unarmored = rank_at(&cl.unarmored, lvl);
    t.light = rank_at(&cl.light, lvl);
    t.medium = rank_at(&cl.medium, lvl);
    t.heavy = rank_at(&cl.heavy, lvl);
    t.class_dc = rank_at(&cl.class_dc, lvl);
    if let Some(sc) = &cl.spellcasting {
        t.spellcasting = rank_at(&sc.proficiency, lvl);
    }

    // Feat-granted proficiencies.
    for f in c.feats.iter().filter_map(|f| content.feat(&f.feat_id)) {
        for e in &f.effects {
            match e {
                FeatEffect::SaveRank { save, rank } => {
                    let slot = match save {
                        crate::stats::Save::Fortitude => &mut t.fortitude,
                        crate::stats::Save::Reflex => &mut t.reflex,
                        crate::stats::Save::Will => &mut t.will,
                    };
                    *slot = (*slot).max(*rank);
                }
                FeatEffect::PerceptionRank { rank } => t.perception = t.perception.max(*rank),
                _ => {}
            }
        }
    }

    // Trained skills
    let mut fixed: BTreeSet<Skill> = cl.fixed_skills.iter().copied().collect();
    if let Some(bg) = content.background(&c.background_id) {
        fixed.extend(bg.trained_skills.iter().copied());
    }
    let allowed = (cl.additional_skills + attrs.intelligence).max(0) as usize;
    let chosen: BTreeSet<Skill> = c.trained_skills.iter().copied().collect();
    if chosen.len() != c.trained_skills.len() {
        r.error("duplicate_skill", "trained_skills", "a skill is listed twice");
    }
    if chosen.len() > allowed {
        r.error(
            "too_many_skills",
            "trained_skills",
            format!("{} may choose {} additional trained skills; {} chosen", cl.name, allowed, chosen.len()),
        );
    } else if chosen.len() < allowed {
        r.warn("unspent_skills", "trained_skills", format!("{} additional trained skill(s) unspent", allowed - chosen.len()));
    }
    for s in chosen.intersection(&fixed) {
        r.warn("redundant_skill", "trained_skills", format!("{s:?} is already trained by class or background"));
    }
    for s in fixed.iter().chain(chosen.iter()) {
        t.skills.insert(*s, Proficiency::Trained);
    }

    // Skill increases
    for (&lvl_inc, &skill) in &c.skill_increases {
        let path = format!("skill_increases/{lvl_inc}");
        if lvl_inc > c.level {
            r.error("future_increase", &path, "skill increase above character level");
            continue;
        }
        if !cl.skill_increase_levels.contains(&lvl_inc) {
            r.error("bad_increase_level", &path, format!("{} gets no skill increase at level {lvl_inc}", cl.name));
            continue;
        }
        let cur = t.skills.get(&skill).copied().unwrap_or_default();
        let next = cur.next().unwrap_or(Proficiency::Legendary);
        let min_level = match next {
            Proficiency::Master => 7,
            Proficiency::Legendary => 15,
            _ => 1,
        };
        if cur == Proficiency::Legendary {
            r.error("max_rank", &path, format!("{skill:?} is already legendary"));
        } else if lvl_inc < min_level {
            r.error("rank_level", &path, format!("{skill:?} to {next:?} requires level {min_level}"));
        }
        t.skills.insert(skill, next);
    }
    for lvl_inc in cl.skill_increase_levels.iter().filter(|&&l| l <= c.level) {
        if !c.skill_increases.contains_key(lvl_inc) {
            r.warn("unspent_increase", format!("skill_increases/{lvl_inc}"), format!("level {lvl_inc} skill increase unspent"));
        }
    }
    t
}

// ---------------------------------------------------------------------------
// Feats
// ---------------------------------------------------------------------------

fn validate_feats(c: &Character, content: &Content, attrs: &AttributeSet, profs: &ProficiencyTable, r: &mut ValidationReport) {
    let Some(cl) = content.class(&c.class_id) else { return };
    let mut used_slots: BTreeSet<(String, u8)> = BTreeSet::new();
    let mut taken: BTreeMap<&str, u8> = BTreeMap::new();
    // Order by slot level so "feat taken earlier" checks are meaningful.
    let mut order: Vec<usize> = (0..c.feats.len()).collect();
    order.sort_by_key(|&i| c.feats[i].slot_level);
    // Archetype tracking: archetype -> (dedication level, feats taken from it)
    let mut archetypes: Vec<(String, u8, usize)> = Vec::new();

    for i in order {
        let choice = &c.feats[i];
        let path = format!("feats/{i}");
        let Some(feat) = content.feat(&choice.feat_id) else {
            r.error("unknown_feat", &path, format!("unknown feat '{}'", choice.feat_id));
            continue;
        };
        if choice.slot_level > c.level {
            r.error("future_feat", &path, format!("{} uses a level {} slot", feat.name, choice.slot_level));
        }
        let slot_levels = match choice.slot_type {
            FeatType::Ancestry => &cl.ancestry_feat_levels,
            FeatType::Class | FeatType::Archetype => &cl.class_feat_levels,
            FeatType::Skill => &cl.skill_feat_levels,
            FeatType::General => &cl.general_feat_levels,
        };
        if !slot_levels.contains(&choice.slot_level) {
            r.error(
                "no_such_slot",
                &path,
                format!("{} has no {:?} feat slot at level {}", cl.name, choice.slot_type, choice.slot_level),
            );
        }
        let slot_key = match choice.slot_type {
            FeatType::Archetype => FeatType::Class,
            other => other,
        };
        if !used_slots.insert((format!("{slot_key:?}"), choice.slot_level)) {
            r.error("slot_reused", &path, format!("{:?} slot at level {} used twice", slot_key, choice.slot_level));
        }
        if feat.level > choice.slot_level {
            r.error("feat_level", &path, format!("{} is level {} (slot is level {})", feat.name, feat.level, choice.slot_level));
        }
        let fits = match slot_key {
            FeatType::Ancestry => feat.feat_type == FeatType::Ancestry && feat.traits.contains(&c.ancestry_id),
            FeatType::Class => {
                (feat.feat_type == FeatType::Class && feat.traits.contains(&c.class_id))
                    || feat.feat_type == FeatType::Archetype
            }
            FeatType::Skill => feat.feat_type == FeatType::Skill,
            FeatType::General => matches!(feat.feat_type, FeatType::General | FeatType::Skill),
            FeatType::Archetype => unreachable!(),
        };
        if !fits {
            r.error("slot_mismatch", &path, format!("{} cannot go in a {:?} slot", feat.name, choice.slot_type));
        }
        if let Some(prev) = taken.get(feat.id.as_str()) {
            if !feat.repeatable {
                r.error("duplicate_feat", &path, format!("{} already taken at level {prev}", feat.name));
            }
        }
        for pre in &feat.prerequisites {
            let ok = match pre {
                Prerequisite::Feat { id } => taken.contains_key(id.as_str())
                    || content
                        .background(&c.background_id)
                        .and_then(|b| b.skill_feat.as_ref())
                        .map(|f| f == id)
                        .unwrap_or(false),
                Prerequisite::SkillRank { skill, rank } => profs.skills.get(skill).copied().unwrap_or_default() >= *rank,
                Prerequisite::Attribute { attribute, min } => attrs.get(*attribute) >= *min,
                Prerequisite::Class { id } => c.class_id == *id,
                Prerequisite::Ancestry { id } => c.ancestry_id == *id,
            };
            if !ok {
                r.error("prerequisite", &path, format!("{} prerequisite not met: {pre:?}", feat.name));
            }
        }
        // Dedication rules.
        if feat.feat_type == FeatType::Archetype {
            let arch = feat.archetype.clone().unwrap_or_default();
            if feat.is_dedication() {
                if let Some((name, _, n)) = archetypes.last() {
                    if *n < 2 {
                        r.error(
                            "dedication_lock",
                            &path,
                            format!("take two more {name} archetype feats before another dedication"),
                        );
                    }
                }
                if cl.id == arch {
                    r.error("own_class_dedication", &path, "cannot take your own class's dedication");
                }
                archetypes.push((arch, choice.slot_level, 0));
            } else {
                match archetypes.iter_mut().find(|(a, _, _)| *a == arch) {
                    Some(entry) => entry.2 += 1,
                    None => r.error("missing_dedication", &path, format!("{} requires the {arch} dedication", feat.name)),
                }
            }
        }
        taken.insert(feat.id.as_str(), choice.slot_level);
    }

    for (kind, levels) in [
        (FeatType::Ancestry, &cl.ancestry_feat_levels),
        (FeatType::Class, &cl.class_feat_levels),
        (FeatType::Skill, &cl.skill_feat_levels),
        (FeatType::General, &cl.general_feat_levels),
    ] {
        for l in levels.iter().filter(|&&l| l <= c.level) {
            if !used_slots.contains(&(format!("{kind:?}"), *l)) {
                r.warn("empty_slot", "feats", format!("{kind:?} feat slot at level {l} is empty"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Equipment & spells
// ---------------------------------------------------------------------------

fn rune_item_level(potency: u8, striking: u8) -> u8 {
    let p = match potency {
        0 => 0,
        1 => 2,
        2 => 10,
        _ => 16,
    };
    let s = match striking {
        0 => 0,
        1 => 4,
        2 => 12,
        _ => 19,
    };
    p.max(s)
}

fn validate_equipment(c: &Character, content: &Content, r: &mut ValidationReport) {
    for (i, w) in c.equipment.weapons.iter().enumerate() {
        let path = format!("equipment/weapons/{i}");
        if content.weapon(&w.weapon_id).is_none() {
            r.error("unknown_weapon", &path, format!("unknown weapon '{}'", w.weapon_id));
        }
        if w.potency > 3 || w.striking > 3 {
            r.error("rune_range", &path, "potency and striking runes go up to 3");
        }
        if w.striking > 0 && w.potency == 0 {
            r.warn("rune_order", &path, "a striking rune normally requires a potency rune");
        }
        let item_level = rune_item_level(w.potency, w.striking);
        if item_level > c.level {
            r.warn("item_level", &path, format!("runes are item level {item_level}, above character level"));
        }
    }
    if let Some(a) = &c.equipment.armor {
        if content.armor_item(&a.armor_id).is_none() {
            r.error("unknown_armor", "equipment/armor", format!("unknown armor '{}'", a.armor_id));
        }
        if a.potency > 3 || a.resilient > 3 {
            r.error("rune_range", "equipment/armor", "armor potency and resilient runes go up to 3");
        }
        let lvl = [0u8, 5, 11, 18][a.potency.min(3) as usize].max([0u8, 8, 14, 20][a.resilient.min(3) as usize]);
        if lvl > c.level {
            r.warn("item_level", "equipment/armor", format!("armor runes are item level {lvl}, above character level"));
        }
    }
    if let Some(s) = &c.equipment.shield_id {
        if content.shield(s).is_none() {
            r.error("unknown_shield", "equipment/shield_id", format!("unknown shield '{s}'"));
        }
    }
}

fn validate_spells(c: &Character, content: &Content, r: &mut ValidationReport) {
    let Some(cl) = content.class(&c.class_id) else { return };
    let has_spells = !c.spells.cantrips.is_empty() || !c.spells.spells.is_empty();
    let Some(sc) = &cl.spellcasting else {
        if has_spells {
            r.error("not_a_caster", "spells", format!("{} has no spellcasting", cl.name));
        }
        return;
    };
    if c.spells.cantrips.len() > sc.cantrips as usize {
        r.error("too_many_cantrips", "spells/cantrips", format!("at most {} cantrips", sc.cantrips));
    }
    for (i, id) in c.spells.cantrips.iter().enumerate() {
        match content.spell(id) {
            None => r.error("unknown_spell", format!("spells/cantrips/{i}"), format!("unknown spell '{id}'")),
            Some(s) => {
                if !s.cantrip {
                    r.error("not_cantrip", format!("spells/cantrips/{i}"), format!("{} is not a cantrip", s.name));
                }
                if !s.traditions.contains(&sc.tradition) {
                    r.error("tradition", format!("spells/cantrips/{i}"), format!("{} is not on the {:?} list", s.name, sc.tradition));
                }
            }
        }
    }
    let slots = sc.slots.slots(c.level);
    let mut per_rank: BTreeMap<u8, u8> = BTreeMap::new();
    for (i, ks) in c.spells.spells.iter().enumerate() {
        let path = format!("spells/spells/{i}");
        *per_rank.entry(ks.rank).or_default() += 1;
        match content.spell(&ks.spell_id) {
            None => r.error("unknown_spell", &path, format!("unknown spell '{}'", ks.spell_id)),
            Some(s) => {
                if s.cantrip {
                    r.error("cantrip_in_slot", &path, format!("{} is a cantrip", s.name));
                }
                if s.rank > ks.rank {
                    r.error("rank_too_low", &path, format!("{} is rank {}", s.name, s.rank));
                }
                if !s.traditions.contains(&sc.tradition) {
                    r.error("tradition", &path, format!("{} is not on the {:?} list", s.name, sc.tradition));
                }
            }
        }
    }
    for (rank, n) in per_rank {
        let max = slots.get(&rank).copied().unwrap_or(0);
        if n > max {
            r.error("too_many_spells", "spells/spells", format!("{n} rank-{rank} spells but only {max} slots"));
        }
    }
}

// ---------------------------------------------------------------------------
// Derivation
// ---------------------------------------------------------------------------

fn weapon_rank(p: &ProficiencyTable, cat: WeaponCategory) -> Proficiency {
    match cat {
        WeaponCategory::Unarmed => p.unarmed,
        WeaponCategory::Simple => p.simple,
        WeaponCategory::Martial => p.martial,
        WeaponCategory::Advanced => p.advanced,
    }
}

fn spec_bonus(rank: Proficiency, greater: bool) -> i32 {
    let base = match rank {
        Proficiency::Expert => 2,
        Proficiency::Master => 3,
        Proficiency::Legendary => 4,
        _ => 0,
    };
    if greater {
        base * 2
    } else {
        base
    }
}

/// Builds the combat stat block. Call only on a build without errors (missing
/// content is tolerated by skipping it).
pub fn derive_statblock(c: &Character, content: &Content, attrs: &AttributeSet, p: &ProficiencyTable) -> Statblock {
    let level = c.level as i32;
    let cl = content.class(&c.class_id);
    let anc = content.ancestry(&c.ancestry_id);
    let feats: Vec<_> = c.feats.iter().filter_map(|f| content.feat(&f.feat_id)).collect();

    let features: Vec<&ClassFeatureKind> = cl
        .map(|cl| cl.features.iter().filter(|f| f.level <= c.level).map(|f| &f.feature).collect())
        .unwrap_or_default();
    let has_spec = features.iter().any(|f| matches!(f, ClassFeatureKind::WeaponSpecialization));
    let has_greater_spec = features.iter().any(|f| matches!(f, ClassFeatureKind::GreaterWeaponSpecialization));

    // HP
    let mut max_hp = anc.map(|a| a.hp).unwrap_or(8) + (cl.map(|c| c.hp_per_level).unwrap_or(8) + attrs.constitution) * level;
    for f in &feats {
        for e in &f.effects {
            if let FeatEffect::HpPerLevel { per_level } = e {
                max_hp += per_level * level;
            }
        }
    }

    // Armor & AC
    let armor = c.equipment.armor.as_ref().and_then(|a| content.armor_item(&a.armor_id).map(|d| (a, d)));
    let (armor_bonus, dex_cap, armor_rank, potency, resilient, speed_penalty, check_penalty) = match armor {
        Some((item, def)) => {
            let rank = match def.category {
                ArmorCategory::Unarmored => p.unarmored,
                ArmorCategory::Light => p.light,
                ArmorCategory::Medium => p.medium,
                ArmorCategory::Heavy => p.heavy,
            };
            let meets_str = attrs.strength >= def.strength;
            let sp = if meets_str { def.speed_penalty.saturating_sub(5) } else { def.speed_penalty };
            let cp = if meets_str { 0 } else { def.check_penalty };
            (def.ac_bonus, def.dex_cap, rank, item.potency as i32, item.resilient as i32, sp, cp)
        }
        None => (0, None, p.unarmored, 0, 0, 0, 0),
    };
    let dex_to_ac = dex_cap.map(|cap| attrs.dexterity.min(cap)).unwrap_or(attrs.dexterity);
    let ac = 10 + dex_to_ac + armor_rank.bonus(level) + armor_bonus + potency;

    // Speed
    let mut speed = anc.map(|a| a.speed).unwrap_or(25).saturating_sub(speed_penalty);
    let mut initiative_bonus = 0;
    let mut reactions: Vec<ReactionKind> = Vec::new();
    for f in &feats {
        for e in &f.effects {
            match e {
                FeatEffect::SpeedBonus { feet } => speed += feet,
                FeatEffect::GrantReaction { reaction } => reactions.push(*reaction),
                FeatEffect::InitiativeBonus { value } => initiative_bonus += value,
                _ => {}
            }
        }
    }
    for f in &features {
        if let ClassFeatureKind::Reaction { reaction } = f {
            reactions.push(*reaction);
        }
    }
    reactions.sort_by_key(|r| *r as u8);
    reactions.dedup();

    // Skills
    let mut skills = BTreeMap::new();
    for s in Skill::ALL {
        let rank = p.skills.get(&s).copied().unwrap_or_default();
        let mut v = attrs.get(s.attribute()) + rank.bonus(level);
        if matches!(s.attribute(), Attribute::Strength | Attribute::Dexterity) {
            v -= check_penalty;
        }
        skills.insert(s, v);
    }

    // Strikes
    let mut strikes = Vec::new();
    let mut add_strike = |name: String, cat: WeaponCategory, die: u32, dtype: DamageType, traits: &[WeaponTrait], ranged_inc: Option<u32>, potency: i32, striking: u32| {
        let rank = weapon_rank(p, cat);
        let finesse = traits.contains(&WeaponTrait::Finesse);
        let thrown = traits.iter().find_map(|t| if let WeaponTrait::Thrown(r) = t { Some(*r) } else { None });
        let reach = crate::models::weapon::reach_of(traits).unwrap_or(5);
        let spec = if has_spec { spec_bonus(rank, has_greater_spec) } else { 0 };
        let dice_count = 1 + striking;
        let mut make = |range: StrikeRange, label: String| {
            let ranged = matches!(range, StrikeRange::Ranged { .. });
            let attr = if ranged || (finesse && attrs.dexterity > attrs.strength) { Attribute::Dexterity } else { Attribute::Strength };
            let (dmg_mod, str_dmg) = if !ranged || thrown.is_some() {
                (attrs.strength, true)
            } else if traits.contains(&WeaponTrait::Propulsive) {
                (if attrs.strength > 0 { attrs.strength / 2 } else { attrs.strength }, true)
            } else {
                (0, false)
            };
            strikes.push(StrikeProfile {
                name: label,
                attack_bonus: attrs.get(attr) + rank.bonus(level) + potency,
                damage: vec![DamageComponent {
                    dice: DiceExpr::new(dice_count, die, dmg_mod + spec),
                    damage_type: dtype,
                    precision: false,
                }],
                traits: traits.to_vec(),
                range,
                attribute: attr,
                strength_damage: str_dmg,
            });
        };
        match ranged_inc {
            Some(inc) => make(StrikeRange::Ranged { increment_ft: inc }, name),
            None => {
                make(StrikeRange::Melee { reach_ft: reach }, name.clone());
                if let Some(inc) = thrown {
                    make(StrikeRange::Ranged { increment_ft: inc }, format!("{name} (thrown)"));
                }
            }
        }
    };
    for w in &c.equipment.weapons {
        if let Some(def) = content.weapon(&w.weapon_id) {
            add_strike(
                def.name.clone(),
                def.category,
                def.damage_die,
                def.damage_type,
                &def.traits,
                def.range_increment,
                w.potency as i32,
                w.striking as u32,
            );
        }
    }
    add_strike(
        "Fist".into(),
        WeaponCategory::Unarmed,
        4,
        DamageType::Bludgeoning,
        &[WeaponTrait::Agile, WeaponTrait::Finesse, WeaponTrait::Nonlethal],
        None,
        0,
        0,
    );

    // Damage riders
    let mut damage_riders = Vec::new();
    if let Some(dice) = features
        .iter()
        .filter_map(|f| if let ClassFeatureKind::SneakAttack { dice } = f { Some(dice.clone()) } else { None })
        .max_by_key(|d| d.dice_count())
    {
        damage_riders.push(DamageRider::SneakAttack { dice });
    }

    // Spellcasting
    let mut spellcasting = Vec::new();
    if let Some(sc) = cl.and_then(|c| c.spellcasting.as_ref()) {
        let attack = attrs.get(sc.attribute) + p.spellcasting.bonus(level);
        let mut spells: Vec<KnownSpell> = c.spells.spells.clone();
        spells.sort_by(|a, b| (a.rank, &a.spell_id).cmp(&(b.rank, &b.spell_id)));
        spellcasting.push(SpellcastingEntry {
            name: format!("{:?} {:?} spells", sc.tradition, sc.kind),
            tradition: sc.tradition,
            kind: sc.kind,
            attribute: sc.attribute,
            attack_bonus: attack,
            dc: 10 + attack,
            slots: sc.slots.slots(c.level),
            spells,
            cantrips: c.spells.cantrips.clone(),
        });
    }

    let shield = c.equipment.shield_id.as_ref().and_then(|id| content.shield(id)).map(|s| ShieldStats {
        name: s.name.clone(),
        ac_bonus: s.ac_bonus,
        hardness: s.hardness,
        max_hp: s.hp,
        broken_threshold: s.broken_threshold,
    });

    let key = c.boosts.class_key;
    Statblock {
        name: c.name.clone(),
        level,
        traits: anc.map(|a| a.traits.clone()).unwrap_or_default(),
        size: anc.map(|a| a.size).unwrap_or_default(),
        attributes: *attrs,
        max_hp,
        ac,
        fortitude: attrs.constitution + p.fortitude.bonus(level) + resilient,
        reflex: attrs.dexterity + p.reflex.bonus(level) + resilient,
        will: attrs.wisdom + p.will.bonus(level) + resilient,
        perception: attrs.wisdom + p.perception.bonus(level),
        speed,
        skills,
        strikes,
        spellcasting,
        iwr: Default::default(),
        reactions,
        shield,
        class_dc: Some(10 + attrs.get(key) + p.class_dc.bonus(level)),
        damage_riders,
        initiative_bonus,
        uses_dying_rules: true,
        hazard: None,
    }
}

#[cfg(test)]
mod tests;
