//! The unified combat stat block. Characters are *derived* into a stat block
//! by the builder; monsters and hazards *are* stat blocks. The combat engine
//! only ever sees stat blocks, which keeps it independent of how a creature
//! was built.

use crate::damage::{DamageType, Iwr};
use crate::dice::DiceExpr;
use crate::grid::Pos;
use crate::models::spell::Tradition;
use crate::models::weapon::{has_trait, WeaponTrait};
use crate::stats::{Attribute, AttributeSet, Save, Size, Skill};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactionKind {
    /// Formerly Attack of Opportunity.
    ReactiveStrike,
    ShieldBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum StrikeRange {
    Melee { reach_ft: u32 },
    Ranged { increment_ft: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageComponent {
    pub dice: DiceExpr,
    pub damage_type: DamageType,
    #[serde(default)]
    pub precision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrikeProfile {
    pub name: String,
    /// Total attack modifier before MAP and situational modifiers.
    pub attack_bonus: i32,
    /// First component is the weapon's base damage (doubled on a crit and
    /// modified by deadly/fatal); later ones are riders such as runes.
    pub damage: Vec<DamageComponent>,
    #[serde(default)]
    pub traits: Vec<WeaponTrait>,
    pub range: StrikeRange,
    /// Attribute the attack roll is based on (for clumsy / enfeebled).
    #[serde(default = "default_attr")]
    pub attribute: Attribute,
    /// Whether the flat damage includes Strength (enfeebled reduces it).
    #[serde(default)]
    pub strength_damage: bool,
}

fn default_attr() -> Attribute {
    Attribute::Strength
}

impl StrikeProfile {
    pub fn is_agile(&self) -> bool {
        has_trait(&self.traits, &WeaponTrait::Agile)
    }
    pub fn is_melee(&self) -> bool {
        matches!(self.range, StrikeRange::Melee { .. })
    }
    pub fn average_damage(&self) -> f64 {
        self.damage.iter().map(|d| d.dice.average().max(1.0)).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpellcastingKind {
    Prepared,
    Spontaneous,
    Innate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownSpell {
    pub spell_id: String,
    /// Rank it is prepared/known at (cantrips: ignored, auto-heightened).
    pub rank: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpellcastingEntry {
    pub name: String,
    pub tradition: Tradition,
    pub kind: SpellcastingKind,
    pub attribute: Attribute,
    pub attack_bonus: i32,
    pub dc: i32,
    /// Slots per rank (rank → count).
    #[serde(default)]
    pub slots: BTreeMap<u8, u8>,
    #[serde(default)]
    pub spells: Vec<KnownSpell>,
    #[serde(default)]
    pub cantrips: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShieldStats {
    pub name: String,
    pub ac_bonus: i32,
    pub hardness: i32,
    pub max_hp: i32,
    pub broken_threshold: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DamageRider {
    /// Extra precision damage against off-guard targets (rogue).
    SneakAttack { dice: DiceExpr },
    /// Flat bonus to strike damage (weapon specialization etc).
    FlatDamage { value: i32, source: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisableOption {
    pub skill: Skill,
    pub dc: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum HazardEffect {
    Save { save: Save, dc: i32, damage: DamageComponent, basic: bool },
    Strike { strike_index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HazardProfile {
    /// Complex hazards take turns in initiative; simple hazards only trigger.
    #[serde(default)]
    pub complex: bool,
    pub stealth_dc: i32,
    #[serde(default)]
    pub disable: Vec<DisableOption>,
    /// Entering any of these squares triggers the hazard. Offsets are
    /// relative to the hazard's placed position; empty = its own square.
    #[serde(default)]
    pub trigger_offsets: Vec<Pos>,
    #[serde(default)]
    pub trigger_effect: Option<HazardEffect>,
    /// If false the hazard deactivates after triggering once.
    #[serde(default)]
    pub resets: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Statblock {
    pub name: String,
    pub level: i32,
    #[serde(default)]
    pub traits: Vec<String>,
    #[serde(default)]
    pub size: Size,
    #[serde(default)]
    pub attributes: AttributeSet,
    pub max_hp: i32,
    pub ac: i32,
    pub fortitude: i32,
    pub reflex: i32,
    pub will: i32,
    pub perception: i32,
    #[serde(default = "default_speed")]
    pub speed: u32,
    #[serde(default)]
    pub skills: BTreeMap<Skill, i32>,
    #[serde(default)]
    pub strikes: Vec<StrikeProfile>,
    #[serde(default)]
    pub spellcasting: Vec<SpellcastingEntry>,
    #[serde(default)]
    pub iwr: Iwr,
    #[serde(default)]
    pub reactions: Vec<ReactionKind>,
    #[serde(default)]
    pub shield: Option<ShieldStats>,
    #[serde(default)]
    pub class_dc: Option<i32>,
    #[serde(default)]
    pub damage_riders: Vec<DamageRider>,
    /// Extra initiative modifier (e.g. Incredible Initiative).
    #[serde(default)]
    pub initiative_bonus: i32,
    /// Player characters use the dying rules; others die at 0 HP.
    #[serde(default)]
    pub uses_dying_rules: bool,
    #[serde(default)]
    pub hazard: Option<HazardProfile>,
}

fn default_speed() -> u32 {
    25
}

impl Statblock {
    pub fn save(&self, s: Save) -> i32 {
        match s {
            Save::Fortitude => self.fortitude,
            Save::Reflex => self.reflex,
            Save::Will => self.will,
        }
    }

    /// Skill modifier; untrained skills fall back to the attribute modifier.
    pub fn skill(&self, s: Skill) -> i32 {
        self.skills.get(&s).copied().unwrap_or_else(|| self.attributes.get(s.attribute()))
    }

    pub fn has_reaction(&self, r: ReactionKind) -> bool {
        self.reactions.contains(&r)
    }

    /// Longest melee reach among strikes (default 5 ft).
    pub fn melee_reach(&self) -> Option<u32> {
        self.strikes
            .iter()
            .filter_map(|s| match s.range {
                StrikeRange::Melee { reach_ft } => Some(reach_ft),
                _ => None,
            })
            .max()
    }

    pub fn is_hazard(&self) -> bool {
        self.hazard.is_some()
    }
}

impl HazardProfile {
    /// Absolute trigger squares for a hazard placed at `at`.
    pub fn trigger_squares(&self, at: Pos) -> Vec<Pos> {
        if self.trigger_offsets.is_empty() {
            return vec![at];
        }
        self.trigger_offsets.iter().map(|o| Pos::new(at.x + o.x, at.y + o.y)).collect()
    }
}
