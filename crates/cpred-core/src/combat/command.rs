//! Commands are the *only* inputs to a combat. A combat is fully determined
//! by its initial state and its ordered command log.

use crate::model::HitLocation;
use crate::rules::AttackMode;
use serde::{Deserialize, Serialize};
use tactics_core::{CombatantId, Pos};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Command {
    /// Begin rolling initiative.
    StartCombat,
    /// Answer the pending roll request.
    ///
    /// * Checks, critical follow-ups, initiative and death saves: `value` is
    ///   the natural d10 face (1–10).
    /// * Damage: `value` is ignored and `faces` must list every die face in
    ///   the order of the dice expression.
    ///
    /// `extra_modifier` is an unmodelled situational modifier the GM adds to
    /// a check total (or to the damage total). It is ignored for death saves.
    /// It is part of the log, so replays stay exact.
    ProvideRoll {
        request_id: u32,
        value: i32,
        #[serde(default)]
        faces: Vec<i32>,
        #[serde(default)]
        extra_modifier: i32,
    },
    /// Answer a pending yes/no decision.
    Decide { request_id: u32, accept: bool },
    /// The current combatant takes an action.
    Act { actor: CombatantId, action: Action },
    /// The current combatant ends its turn.
    EndTurn { actor: CombatantId },
    /// GM override (logged and replayable like everything else).
    Gm { op: GmOp },
}

impl Command {
    /// Convenience constructor for a natural-d10 answer.
    pub fn roll(request_id: u32, value: i32) -> Command {
        Command::ProvideRoll { request_id, value, faces: Vec::new(), extra_modifier: 0 }
    }

    /// Convenience constructor for a damage answer.
    pub fn faces(request_id: u32, faces: Vec<i32>) -> Command {
        let value = faces.iter().sum();
        Command::ProvideRoll { request_id, value, faces, extra_modifier: 0 }
    }
}

/// Something the acting combatant does on its turn.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Action {
    /// Uses the Move Action.
    Move { to: Pos },
    /// Uses the Action to move again.
    Run { to: Pos },
    /// One attack with a weapon (ROF 2 weapons get two per Action).
    Attack { weapon: usize, target: CombatantId },
    /// Ranged attack at the head, −8 to hit, uses the whole Action.
    AimedShot { weapon: usize, target: CombatantId },
    /// Autofire, uses the whole Action and 10 rounds.
    Autofire { weapon: usize, target: CombatantId },
    /// Refill a weapon's magazine.
    Reload { weapon: usize },
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::Move { .. } => "Move",
            Action::Run { .. } => "Run",
            Action::Attack { .. } => "Attack",
            Action::AimedShot { .. } => "Aimed Shot",
            Action::Autofire { .. } => "Autofire",
            Action::Reload { .. } => "Reload",
        }
    }

    /// `(weapon, target, mode)` for attack actions.
    pub fn attack(&self) -> Option<(usize, CombatantId, AttackMode)> {
        match *self {
            Action::Attack { weapon, target } => Some((weapon, target, AttackMode::Single)),
            Action::AimedShot { weapon, target } => Some((weapon, target, AttackMode::Aimed)),
            Action::Autofire { weapon, target } => Some((weapon, target, AttackMode::Autofire)),
            _ => None,
        }
    }

    /// Destination of movement actions.
    pub fn move_to(&self) -> Option<Pos> {
        match *self {
            Action::Move { to } | Action::Run { to } => Some(to),
            _ => None,
        }
    }
}

/// GM overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GmOp {
    /// Change HP by `delta` (clamped to 0..=max). Does not trigger death saves.
    AdjustHp { target: CombatantId, delta: i32 },
    /// Set the current SP of one location.
    SetSp { target: CombatantId, location: HitLocation, value: i32 },
    /// Set the rounds loaded in a weapon (clamped to the magazine size).
    SetAmmo { target: CombatantId, weapon: usize, value: u32 },
    /// Teleport a combatant to a free, standable square.
    Move { target: CombatantId, to: Pos },
    /// Record a Critical Injury (no bonus damage; raises the death save penalty).
    AddCriticalInjury { target: CombatantId, location: HitLocation },
    /// Abandon whatever action is being resolved (turn structure is kept).
    ClearPending,
}
