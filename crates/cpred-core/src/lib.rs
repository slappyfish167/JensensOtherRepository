//! # cpred-core
//!
//! A deterministic **Cyberpunk RED** combat rules engine.
//!
//! * **Pure**: no I/O, no clocks, no randomness. Every die is entered by a
//!   human (or by an external, opt-in seeded roller) as a command.
//! * **Serializable**: every model and the entire combat state are serde types.
//! * **Replayable**: `initial state + command log` reproduces a combat exactly
//!   (see [`combat::replay()`]).
//!
//! Layout:
//! * [`model`] — statblocks, weapons, armor, battlefields, encounters.
//! * [`rules`] — rule constants, tables and the small pure rule functions
//!   (the module documentation lists every rules assumption).
//! * [`content`] — the bundled sample content (`content/cpred/*.json`).
//! * [`combat`] — the combat state machine (`CombatState::apply(Command)`).
//! * [`analysis`] — rules-derived probability math for an AI layer.

pub mod analysis;
pub mod combat;
pub mod content;
pub mod model;
pub mod rules;

pub use tactics_core::{CombatantId, DiceExpr, Pos, Side};
