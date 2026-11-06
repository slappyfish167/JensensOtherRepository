//! Reproducibility: initial state + command log ⇒ identical final state.

use super::command::Command;
use super::engine::CombatError;
use super::event::Event;
use super::state::CombatState;
use serde::{Deserialize, Serialize};

/// Everything needed to reproduce a combat. This is what gets persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatLog {
    pub initial: CombatState,
    pub commands: Vec<Command>,
}

/// A command of the log failed while replaying.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[error("command #{index} failed during replay: {error}")]
pub struct ReplayError {
    pub index: usize,
    pub error: CombatError,
}

/// Replays `commands` from `initial`, returning the final state and every event.
pub fn replay(initial: &CombatState, commands: &[Command]) -> Result<(CombatState, Vec<Event>), ReplayError> {
    let mut state = initial.clone();
    let mut events = Vec::new();
    for (index, cmd) in commands.iter().enumerate() {
        let ev = state.apply_in_place(cmd).map_err(|error| ReplayError { index, error })?;
        events.extend(ev);
    }
    Ok((state, events))
}

impl CombatLog {
    pub fn new(initial: CombatState) -> Self {
        CombatLog { initial, commands: Vec::new() }
    }

    /// The state after every logged command.
    pub fn current(&self) -> Result<CombatState, ReplayError> {
        replay(&self.initial, &self.commands).map(|(s, _)| s)
    }

    /// Undo = drop the last command and replay.
    pub fn undo(&mut self) -> Result<CombatState, ReplayError> {
        self.commands.pop();
        self.current()
    }
}
