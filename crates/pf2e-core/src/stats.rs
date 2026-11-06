//! Attributes, saves, skills, proficiency and size.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribute {
    Strength,
    Dexterity,
    Constitution,
    Intelligence,
    Wisdom,
    Charisma,
}

impl Attribute {
    pub const ALL: [Attribute; 6] = [
        Attribute::Strength,
        Attribute::Dexterity,
        Attribute::Constitution,
        Attribute::Intelligence,
        Attribute::Wisdom,
        Attribute::Charisma,
    ];

    pub fn short(self) -> &'static str {
        match self {
            Attribute::Strength => "Str",
            Attribute::Dexterity => "Dex",
            Attribute::Constitution => "Con",
            Attribute::Intelligence => "Int",
            Attribute::Wisdom => "Wis",
            Attribute::Charisma => "Cha",
        }
    }
}

/// Attribute *modifiers* (Remaster style: no separate scores).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeSet {
    #[serde(default)]
    pub strength: i32,
    #[serde(default)]
    pub dexterity: i32,
    #[serde(default)]
    pub constitution: i32,
    #[serde(default)]
    pub intelligence: i32,
    #[serde(default)]
    pub wisdom: i32,
    #[serde(default)]
    pub charisma: i32,
}

impl AttributeSet {
    pub fn get(&self, a: Attribute) -> i32 {
        match a {
            Attribute::Strength => self.strength,
            Attribute::Dexterity => self.dexterity,
            Attribute::Constitution => self.constitution,
            Attribute::Intelligence => self.intelligence,
            Attribute::Wisdom => self.wisdom,
            Attribute::Charisma => self.charisma,
        }
    }

    pub fn get_mut(&mut self, a: Attribute) -> &mut i32 {
        match a {
            Attribute::Strength => &mut self.strength,
            Attribute::Dexterity => &mut self.dexterity,
            Attribute::Constitution => &mut self.constitution,
            Attribute::Intelligence => &mut self.intelligence,
            Attribute::Wisdom => &mut self.wisdom,
            Attribute::Charisma => &mut self.charisma,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Save {
    Fortitude,
    Reflex,
    Will,
}

impl Save {
    pub fn attribute(self) -> Attribute {
        match self {
            Save::Fortitude => Attribute::Constitution,
            Save::Reflex => Attribute::Dexterity,
            Save::Will => Attribute::Wisdom,
        }
    }
}

impl fmt::Display for Save {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Save::Fortitude => "Fortitude",
            Save::Reflex => "Reflex",
            Save::Will => "Will",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skill {
    Acrobatics,
    Arcana,
    Athletics,
    Crafting,
    Deception,
    Diplomacy,
    Intimidation,
    Medicine,
    Nature,
    Occultism,
    Performance,
    Religion,
    Society,
    Stealth,
    Survival,
    Thievery,
}

impl Skill {
    pub const ALL: [Skill; 16] = [
        Skill::Acrobatics,
        Skill::Arcana,
        Skill::Athletics,
        Skill::Crafting,
        Skill::Deception,
        Skill::Diplomacy,
        Skill::Intimidation,
        Skill::Medicine,
        Skill::Nature,
        Skill::Occultism,
        Skill::Performance,
        Skill::Religion,
        Skill::Society,
        Skill::Stealth,
        Skill::Survival,
        Skill::Thievery,
    ];

    pub fn attribute(self) -> Attribute {
        use Skill::*;
        match self {
            Acrobatics | Stealth | Thievery => Attribute::Dexterity,
            Athletics => Attribute::Strength,
            Arcana | Crafting | Occultism | Society => Attribute::Intelligence,
            Medicine | Nature | Religion | Survival => Attribute::Wisdom,
            Deception | Diplomacy | Intimidation | Performance => Attribute::Charisma,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Proficiency {
    #[default]
    Untrained,
    Trained,
    Expert,
    Master,
    Legendary,
}

impl Proficiency {
    /// Untrained adds nothing; otherwise level + 2/4/6/8.
    pub fn bonus(self, level: i32) -> i32 {
        match self {
            Proficiency::Untrained => 0,
            Proficiency::Trained => level + 2,
            Proficiency::Expert => level + 4,
            Proficiency::Master => level + 6,
            Proficiency::Legendary => level + 8,
        }
    }

    pub fn next(self) -> Option<Proficiency> {
        match self {
            Proficiency::Untrained => Some(Proficiency::Trained),
            Proficiency::Trained => Some(Proficiency::Expert),
            Proficiency::Expert => Some(Proficiency::Master),
            Proficiency::Master => Some(Proficiency::Legendary),
            Proficiency::Legendary => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    Tiny,
    Small,
    #[default]
    Medium,
    Large,
    Huge,
    Gargantuan,
}

impl Size {
    /// Side length of the creature's space in 5-foot squares.
    pub fn squares(self) -> i32 {
        match self {
            Size::Tiny | Size::Small | Size::Medium => 1,
            Size::Large => 2,
            Size::Huge => 3,
            Size::Gargantuan => 4,
        }
    }
}

pub use tactics_core::Side;
