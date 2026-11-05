//! The combat engine: a deterministic state machine driven by commands.
//!
//! ```text
//!   CombatState ──apply(Command)──▶ CombatState' + Vec<Event>
//!        │
//!        └── pending() ─▶ Option<PendingInput>   (roll request / reaction decision)
//! ```

pub mod actions;
pub mod command;
pub mod engine;
pub mod event;
pub mod pending;
pub mod replay;
pub mod rules;
pub mod setup;
pub mod state;

pub use command::{Action, Command, GmOp};
pub use engine::{CombatError, CombatResult};
pub use event::Event;
pub use pending::{PendingInput, RollKind, RollRequest};
pub use replay::{replay, CombatLog, ReplayError};
pub use setup::{CombatSetup, SetupError, SetupParticipant};
pub use state::{CombatState, Combatant, CombatantKind, InitiativeEntry, LifeState, Phase};

#[cfg(test)]
mod tests;
