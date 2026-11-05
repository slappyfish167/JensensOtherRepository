//! The resolution queue. Every multi-part rule (a Strike: attack roll →
//! damage roll → Shield Block? → apply) is a sequence of `Step`s. Steps that
//! need a human (a die roll or a reaction decision) stop the queue until the
//! matching command arrives; all other steps run automatically.

use crate::checks::DegreeOfSuccess;
use crate::damage::{DamagePart, DamageType};
use crate::dice::DiceExpr;
use crate::grid::{CoverLevel, Pos};
use crate::ids::CombatantId;
use crate::modifiers::Modifier;
use crate::models::statblock::ReactionKind;
use crate::stats::Save;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AttackSource {
    Strike { index: usize },
    Spell { spell_id: String, rank: u8, entry: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttackCtx {
    pub attacker: CombatantId,
    pub target: CombatantId,
    pub source: AttackSource,
    pub map_penalty: i32,
    /// Index of this attack in the attacker's turn (0 = first) for forceful.
    pub attack_number: u8,
    pub cover: CoverLevel,
    /// Reactive Strike: a crit disrupts the action carrying this tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disrupts: Option<u32>,
    /// Bonus from the sweep trait.
    #[serde(default)]
    pub sweep: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageRollSpec {
    pub label: String,
    pub dice: DiceExpr,
    pub damage_type: DamageType,
    #[serde(default)]
    pub precision: bool,
    /// Doubled by the engine after the roll (critical hit).
    #[serde(default)]
    pub doubled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SaveSource {
    Spell { caster: CombatantId, spell_id: String, rank: u8, dc: i32 },
    Hazard { hazard: CombatantId, dc: i32 },
}

impl SaveSource {
    pub fn dc(&self) -> i32 {
        match self {
            SaveSource::Spell { dc, .. } | SaveSource::Hazard { dc, .. } => *dc,
        }
    }
    pub fn origin(&self) -> CombatantId {
        match self {
            SaveSource::Spell { caster, .. } => *caster,
            SaveSource::Hazard { hazard, .. } => *hazard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillAction {
    Demoralize,
    Trip,
    Disable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveKind {
    Stride,
    Step,
}

/// One target's share of a multi-target damage roll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageShare {
    pub target: CombatantId,
    pub numerator: u8,
    pub denominator: u8,
    pub degree: Option<DegreeOfSuccess>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "step")]
pub enum Step {
    // ----- turn structure (automatic unless noted) -----
    /// Needs a d20.
    RollInitiative { combatant: CombatantId },
    FinalizeInitiative,
    BeginTurn,
    /// Needs a flat check.
    RecoveryCheck { combatant: CombatantId },
    EndTurn { combatant: CombatantId },
    /// Needs a damage roll.
    PersistentDamage { combatant: CombatantId, damage_type: DamageType },
    /// Needs a DC 15 flat check.
    PersistentRecovery { combatant: CombatantId, damage_type: DamageType },
    EndTurnTail { combatant: CombatantId },
    AdvanceTurn,

    // ----- action resolution -----
    /// Needs a flat check (DC 5 concealed, DC 11 hidden).
    ConcealmentCheck { tag: u32, ctx: AttackCtx, dc: i32 },
    /// Needs a d20.
    AttackRoll { tag: u32, ctx: AttackCtx },
    /// Needs one damage roll per spec, in order.
    DamageRolls {
        tag: u32,
        source: CombatantId,
        shares: Vec<DamageShare>,
        specs: Vec<DamageRollSpec>,
        rolled: Vec<DamagePart>,
        critical: bool,
        from_strike: bool,
        purpose: String,
    },
    DeliverDamage {
        tag: u32,
        source: CombatantId,
        target: CombatantId,
        parts: Vec<DamagePart>,
        critical: bool,
        from_strike: bool,
        #[serde(default)]
        shield_offered: bool,
        #[serde(default)]
        shield_blocked: bool,
    },
    /// Needs a d20 per pending target, one at a time.
    SaveSequence {
        tag: u32,
        source: SaveSource,
        save: Save,
        pending: Vec<CombatantId>,
        results: Vec<(CombatantId, DegreeOfSuccess)>,
    },
    ResolveSpell { tag: u32, caster: CombatantId, entry: usize, spell_id: String, rank: u8, targets: Vec<CombatantId> },
    SpellOutcomes { tag: u32, caster: CombatantId, spell_id: String, rank: u8, target: CombatantId, degree: DegreeOfSuccess },
    /// Needs a healing roll.
    HealRoll { tag: u32, caster: CombatantId, spell_id: String, dice: DiceExpr, targets: Vec<CombatantId> },
    /// Needs a d20.
    SkillCheck { tag: u32, actor: CombatantId, target: CombatantId, action: SkillAction, map_penalty: i32 },
    ContinueMove { tag: u32, mover: CombatantId, kind: MoveKind, remaining: Vec<Pos>, offered: Vec<CombatantId> },
    HazardTrigger { hazard: CombatantId, victim: CombatantId },
    /// Needs a yes/no decision from `decider`.
    Decision {
        tag: u32,
        decider: CombatantId,
        reaction: ReactionKind,
        prompt: String,
        on_accept: Vec<Step>,
        #[serde(default)]
        on_decline: Vec<Step>,
    },
}

impl Step {
    pub fn tag(&self) -> Option<u32> {
        match self {
            Step::ConcealmentCheck { tag, .. }
            | Step::AttackRoll { tag, .. }
            | Step::DamageRolls { tag, .. }
            | Step::DeliverDamage { tag, .. }
            | Step::SaveSequence { tag, .. }
            | Step::ResolveSpell { tag, .. }
            | Step::SpellOutcomes { tag, .. }
            | Step::HealRoll { tag, .. }
            | Step::SkillCheck { tag, .. }
            | Step::ContinueMove { tag, .. }
            | Step::Decision { tag, .. } => Some(*tag),
            _ => None,
        }
    }

    pub fn needs_input(&self) -> bool {
        match self {
            Step::RollInitiative { .. }
            | Step::RecoveryCheck { .. }
            | Step::PersistentDamage { .. }
            | Step::PersistentRecovery { .. }
            | Step::ConcealmentCheck { .. }
            | Step::AttackRoll { .. }
            | Step::DamageRolls { .. }
            | Step::HealRoll { .. }
            | Step::SkillCheck { .. }
            | Step::Decision { .. } => true,
            Step::SaveSequence { pending, .. } => !pending.is_empty(),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Public view of what the engine is waiting for.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RollKind {
    /// d20 + modifier vs DC: enter the natural die.
    Check { modifier: i32, dc: i32 },
    /// Plain d20 vs DC: enter the natural die.
    Flat { dc: i32 },
    /// Enter the rolled total of `dice`.
    Total { dice: DiceExpr },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollRequest {
    pub request_id: u32,
    pub roller: Option<CombatantId>,
    pub kind: RollKind,
    /// The dice to roll, e.g. "1d20+14".
    pub expression: String,
    pub purpose: String,
    /// Human-readable prompt: "Roll 1d20+14 for attack (Ogre → Valeros, AC 18)".
    pub prompt: String,
    /// What goes into the modifier / DC, for transparency.
    #[serde(default)]
    pub breakdown: Vec<Modifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub request_id: u32,
    pub decider: CombatantId,
    pub reaction: ReactionKind,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PendingInput {
    Roll(RollRequest),
    Decision(DecisionRequest),
}

impl PendingInput {
    pub fn request_id(&self) -> u32 {
        match self {
            PendingInput::Roll(r) => r.request_id,
            PendingInput::Decision(d) => d.request_id,
        }
    }
}
