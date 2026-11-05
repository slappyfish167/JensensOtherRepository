//! Effects: named bundles of modifiers with a duration (spells like *bless*,
//! Raise a Shield, Take Cover, immunity markers, ...).

use crate::conditions::Expiry;
use crate::ids::CombatantId;
use crate::modifiers::Modifier;
use crate::stats::Save;
use serde::{Deserialize, Serialize};

/// Which rolls or DCs a modifier grant applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Selector {
    AttackRolls,
    Ac,
    AllSaves,
    Save(Save),
    Perception,
    Skills,
    AllChecks,
    SpellDc,
    Damage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModifierGrant {
    pub selector: Selector,
    pub modifier: Modifier,
}

/// Marker semantics the rules engine understands beyond plain modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum EffectTag {
    /// The bearer is temporarily immune to the named action/spell from `source`.
    ImmuneTo(String),
    ShieldRaised,
    TakingCover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub id: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CombatantId>,
    #[serde(default)]
    pub grants: Vec<ModifierGrant>,
    #[serde(default)]
    pub tags: Vec<EffectTag>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Expiry>,
}
