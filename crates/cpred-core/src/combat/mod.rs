//! The combat engine: a deterministic state machine driven by commands.
//!
//! ```text
//!   CombatState ──apply(Command)──▶ CombatState' + Vec<Event>
//!        │
//!        └── pending() ─▶ Option<PendingInput>   (roll request / dodge decision)
//! ```

pub mod actions;
pub mod attack;
pub mod command;
pub mod engine;
pub mod event;
pub mod pending;
pub mod replay;
pub mod setup;
pub mod state;

pub use crate::rules::{AttackMode, RulesOptions, RulesTables};
pub use actions::MovementMap;
pub use attack::AttackPlan;
pub use command::{Action, Command, GmOp};
pub use engine::{CombatError, CombatResult};
pub use event::Event;
pub use pending::{AttackCtx, CheckRole, CheckRoll, DecisionRequest, PendingInput, RollKind, RollRequest, Step};
pub use replay::{replay, CombatLog, ReplayError};
pub use setup::{CombatSetup, SetupError, SetupParticipant};
pub use state::{ArmorSp, CombatState, Combatant, CoverObject, CriticalInjury, InitiativeEntry, Phase, TurnEconomy};

#[cfg(test)]
mod tests;
