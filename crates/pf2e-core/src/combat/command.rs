//! Commands are the *only* inputs to a combat. A combat is fully determined
//! by its initial state and its ordered command log.

use crate::conditions::Condition;
use crate::grid::{CoverLevel, Pos};
use crate::ids::CombatantId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Command {
    /// Begin rolling initiative.
    StartCombat,
    /// Answer the pending roll request. For d20 checks and flat checks
    /// `value` is the natural die face (1–20); for damage/healing it is the
    /// rolled total. `extra_modifier` lets the GM add an unmodelled
    /// situational bonus; it is part of the log, so replays stay exact.
    ProvideRoll {
        request_id: u32,
        value: i32,
        #[serde(default)]
        extra_modifier: i32,
    },
    /// Answer a pending reaction decision.
    Decide { request_id: u32, accept: bool },
    /// The current creature takes an action.
    Act { actor: CombatantId, action: Action },
    /// The current creature ends its turn.
    EndTurn { actor: CombatantId },
    /// GM override (logged and replayable like everything else).
    Gm { op: GmOp },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Action {
    Strike {
        actor_strike: usize,
        target: CombatantId,
        /// GM ruling that replaces the computed cover.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cover_override: Option<CoverLevel>,
    },
    Stride { to: Pos },
    Step { to: Pos },
    Stand,
    RaiseShield,
    TakeCover,
    Demoralize { target: CombatantId },
    Trip { target: CombatantId },
    CastSpell {
        entry: usize,
        spell_id: String,
        rank: u8,
        #[serde(default)]
        targets: Vec<CombatantId>,
        /// Burst centre / cone and line aim point.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        aim: Option<Pos>,
    },
    DisableHazard { hazard: CombatantId },
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::Strike { .. } => "Strike",
            Action::Stride { .. } => "Stride",
            Action::Step { .. } => "Step",
            Action::Stand => "Stand",
            Action::RaiseShield => "Raise a Shield",
            Action::TakeCover => "Take Cover",
            Action::Demoralize { .. } => "Demoralize",
            Action::Trip { .. } => "Trip",
            Action::CastSpell { .. } => "Cast a Spell",
            Action::DisableHazard { .. } => "Disable a Device",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GmOp {
    AdjustHp { target: CombatantId, delta: i32 },
    SetTempHp { target: CombatantId, value: i32 },
    AddCondition { target: CombatantId, condition: Condition },
    RemoveCondition { target: CombatantId, condition: crate::conditions::ConditionKind },
    Move { target: CombatantId, to: Pos },
    /// Abandon whatever is being resolved (e.g. a mis-declared action).
    ClearPending,
}
