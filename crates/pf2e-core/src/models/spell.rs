//! Spells: defense, targeting, damage with heightening, and per-degree outcomes.

use crate::conditions::{ConditionKind, Duration};
use crate::damage::DamageType;
use crate::dice::DiceExpr;
use crate::effects::ModifierGrant;
use crate::grid::AreaShape;
use crate::stats::Save;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tradition {
    Arcane,
    Divine,
    Occult,
    Primal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SpellDefense {
    /// Automatic effect (no roll), e.g. *force barrage*, *heal*, *bless*.
    None,
    /// Spell attack roll against AC.
    Attack,
    /// Each target attempts a save. `basic` = standard damage halving/doubling.
    Save { save: Save, basic: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SpellTargeting {
    /// Up to `max` creatures within range.
    Creatures { max: u8, #[serde(default)] allies: bool },
    Area { area: AreaShape, #[serde(default)] allies_only: bool },
    SelfOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpellDamage {
    pub dice: DiceExpr,
    pub damage_type: DamageType,
    /// Every `heighten_every` ranks above the base rank add `heighten_add`.
    #[serde(default)]
    pub heighten_every: u8,
    #[serde(default)]
    pub heighten_add: DiceExpr,
}

impl SpellDamage {
    pub fn at_rank(&self, base_rank: u8, cast_rank: u8) -> DiceExpr {
        if self.heighten_every == 0 || cast_rank <= base_rank {
            return self.dice.clone();
        }
        let steps = ((cast_rank - base_rank) / self.heighten_every) as u32;
        self.dice.plus(&self.heighten_add.times(steps))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SpellOutcome {
    /// Multiply rolled damage by numerator/denominator (½ = 1/2, double = 2/1).
    DamageMultiplier { numerator: u8, denominator: u8 },
    ApplyCondition { condition: ConditionKind, #[serde(default)] value: u8, duration: Duration },
    Persistent { dice: DiceExpr, damage_type: DamageType },
    GrantEffect { name: String, grants: Vec<ModifierGrant>, duration: Duration },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DegreeOutcomes {
    #[serde(default)]
    pub critical_success: Vec<SpellOutcome>,
    #[serde(default)]
    pub success: Vec<SpellOutcome>,
    #[serde(default)]
    pub failure: Vec<SpellOutcome>,
    #[serde(default)]
    pub critical_failure: Vec<SpellOutcome>,
}

impl DegreeOutcomes {
    pub fn for_degree(&self, d: crate::checks::DegreeOfSuccess) -> &[SpellOutcome] {
        use crate::checks::DegreeOfSuccess::*;
        match d {
            CriticalSuccess => &self.critical_success,
            Success => &self.success,
            Failure => &self.failure,
            CriticalFailure => &self.critical_failure,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spell {
    pub id: String,
    pub name: String,
    pub rank: u8,
    #[serde(default)]
    pub cantrip: bool,
    pub traditions: Vec<Tradition>,
    #[serde(default)]
    pub traits: Vec<String>,
    pub actions: u8,
    /// `None` = touch / self.
    #[serde(default)]
    pub range_ft: Option<u32>,
    pub targeting: SpellTargeting,
    pub defense: SpellDefense,
    #[serde(default)]
    pub damage: Option<SpellDamage>,
    #[serde(default)]
    pub healing: Option<SpellDamage>,
    /// For save spells: the *target's* degree. For attack spells: the
    /// *caster's* degree. For no-defense spells only `success` is used.
    #[serde(default)]
    pub outcomes: DegreeOutcomes,
    #[serde(default)]
    pub description: String,
}

impl Spell {
    pub fn has_trait(&self, t: &str) -> bool {
        self.traits.iter().any(|x| x == t)
    }

    pub fn is_harmful(&self) -> bool {
        !matches!(self.targeting, SpellTargeting::Creatures { allies: true, .. })
            && !matches!(self.targeting, SpellTargeting::Area { allies_only: true, .. })
            && self.healing.is_none()
    }
}
