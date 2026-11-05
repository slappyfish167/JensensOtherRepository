//! Events are *outputs* describing what happened. They are derived data: the
//! log of record is the command list, so events never need to be stored.

use crate::checks::DegreeOfSuccess;
use crate::conditions::ConditionKind;
use crate::damage::DamageLine;
use crate::grid::Pos;
use crate::ids::CombatantId;
use crate::stats::Side;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Event {
    CombatStarted,
    InitiativeOrder { order: Vec<(CombatantId, i32)> },
    RoundStarted { round: u32 },
    TurnStarted { combatant: CombatantId, actions: u8 },
    TurnEnded { combatant: CombatantId },
    ActionDeclared { actor: CombatantId, action: String, cost: u8 },
    Moved { combatant: CombatantId, from: Pos, to: Pos },
    CheckResolved {
        roller: Option<CombatantId>,
        purpose: String,
        natural: u8,
        total: i32,
        dc: i32,
        degree: DegreeOfSuccess,
    },
    FlatCheck { combatant: CombatantId, purpose: String, natural: u8, dc: i32, success: bool },
    DamageRolled { purpose: String, expression: String, rolled: i32, counted: i32 },
    DamageTaken { target: CombatantId, amount: i32, lines: Vec<DamageLine>, hp: i32, temp_hp: i32 },
    ShieldBlocked { target: CombatantId, prevented: i32, shield_hp: i32, broken: bool },
    Healed { target: CombatantId, amount: i32, hp: i32 },
    ConditionGained { target: CombatantId, condition: ConditionKind, value: u8 },
    ConditionLost { target: CombatantId, condition: ConditionKind },
    EffectGained { target: CombatantId, name: String },
    EffectEnded { target: CombatantId, name: String },
    ReactionUsed { reactor: CombatantId, reaction: String },
    ActionDisrupted { actor: CombatantId },
    KnockedOut { combatant: CombatantId, dying: u8 },
    Died { combatant: CombatantId },
    HazardTriggered { hazard: CombatantId, victim: CombatantId },
    HazardDisabled { hazard: CombatantId },
    CombatEnded { winner: Option<Side> },
    Note { text: String },
}
