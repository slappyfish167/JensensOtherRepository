//! Feats with machine-checkable prerequisites and passive effects.

use crate::models::statblock::ReactionKind;
use crate::stats::{Attribute, Proficiency, Save, Skill};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatType {
    Ancestry,
    Class,
    Skill,
    General,
    Archetype,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Prerequisite {
    Feat { id: String },
    SkillRank { skill: Skill, rank: Proficiency },
    Attribute { attribute: Attribute, min: i32 },
    Class { id: String },
    Ancestry { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FeatEffect {
    /// Extra max HP equal to `per_level` × level.
    HpPerLevel { per_level: i32 },
    SpeedBonus { feet: u32 },
    GrantReaction { reaction: ReactionKind },
    SaveRank { save: Save, rank: Proficiency },
    PerceptionRank { rank: Proficiency },
    InitiativeBonus { value: i32 },
    /// Marks this feat as the dedication for `archetype`.
    Dedication { archetype: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feat {
    pub id: String,
    pub name: String,
    pub level: u8,
    pub feat_type: FeatType,
    /// For class feats: the class ids that may take it. For ancestry feats: ancestry ids.
    #[serde(default)]
    pub traits: Vec<String>,
    /// For archetype feats: which archetype they belong to.
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub prerequisites: Vec<Prerequisite>,
    #[serde(default)]
    pub effects: Vec<FeatEffect>,
    #[serde(default)]
    pub repeatable: bool,
    #[serde(default)]
    pub description: String,
}

impl Feat {
    pub fn is_dedication(&self) -> bool {
        self.effects.iter().any(|e| matches!(e, FeatEffect::Dedication { .. }))
    }
}
