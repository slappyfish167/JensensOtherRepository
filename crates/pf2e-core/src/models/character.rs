//! Player character *build* (choices only). Derived numbers live in the
//! stat block produced by `builder::derive_statblock`.

use crate::models::feat::FeatType;
use crate::stats::{Attribute, Proficiency, Skill};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeBoosts {
    /// Boosts from the ancestry's free slots (fixed boosts are implicit).
    #[serde(default)]
    pub ancestry_free: Vec<Attribute>,
    /// Remaster option: take two free boosts and ignore the ancestry's fixed boosts and flaws.
    #[serde(default)]
    pub ancestry_alternate: bool,
    /// Background: first must be one of the background's options, second is free.
    #[serde(default)]
    pub background: Vec<Attribute>,
    pub class_key: Attribute,
    /// Four free boosts at character creation.
    #[serde(default)]
    pub level1_free: Vec<Attribute>,
    /// Four boosts at levels 5, 10, 15, 20 (key = level).
    #[serde(default)]
    pub level_ups: BTreeMap<u8, Vec<Attribute>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatChoice {
    pub feat_id: String,
    pub slot_type: FeatType,
    pub slot_level: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponItem {
    pub weapon_id: String,
    /// Weapon potency rune (+1..+3).
    #[serde(default)]
    pub potency: u8,
    /// 0 none, 1 striking, 2 greater striking, 3 major striking.
    #[serde(default)]
    pub striking: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmorItem {
    pub armor_id: String,
    #[serde(default)]
    pub potency: u8,
    /// 0 none, 1 resilient, 2 greater, 3 major.
    #[serde(default)]
    pub resilient: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Equipment {
    #[serde(default)]
    pub weapons: Vec<WeaponItem>,
    #[serde(default)]
    pub armor: Option<ArmorItem>,
    #[serde(default)]
    pub shield_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpellChoices {
    #[serde(default)]
    pub cantrips: Vec<String>,
    /// Spells prepared / in repertoire with their rank.
    #[serde(default)]
    pub spells: Vec<crate::models::statblock::KnownSpell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub level: u8,
    pub ancestry_id: String,
    pub background_id: String,
    pub class_id: String,
    pub boosts: AttributeBoosts,
    /// Skills chosen as trained at level 1 (beyond class/background fixed ones).
    #[serde(default)]
    pub trained_skills: Vec<Skill>,
    /// Skill increases by level (key = level).
    #[serde(default)]
    pub skill_increases: BTreeMap<u8, Skill>,
    #[serde(default)]
    pub feats: Vec<FeatChoice>,
    #[serde(default)]
    pub equipment: Equipment,
    #[serde(default)]
    pub spells: SpellChoices,
    #[serde(default)]
    pub notes: String,
}

/// Fully computed proficiency table for a character at its level.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProficiencyTable {
    pub perception: Proficiency,
    pub fortitude: Proficiency,
    pub reflex: Proficiency,
    pub will: Proficiency,
    pub unarmed: Proficiency,
    pub simple: Proficiency,
    pub martial: Proficiency,
    pub advanced: Proficiency,
    pub unarmored: Proficiency,
    pub light: Proficiency,
    pub medium: Proficiency,
    pub heavy: Proficiency,
    pub class_dc: Proficiency,
    pub spellcasting: Proficiency,
    pub skills: BTreeMap<Skill, Proficiency>,
}
