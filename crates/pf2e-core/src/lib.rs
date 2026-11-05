//! # pf2e-core
//!
//! A deterministic Pathfinder Second Edition rules engine.
//!
//! * **Pure**: no I/O, no clocks, no randomness. Dice results are inputs.
//! * **Serializable**: every model and the entire combat state are serde types.
//! * **Replayable**: `initial state + command log` reproduces a combat exactly
//!   (see [`combat::replay`]).
//!
//! Layout:
//! * [`dice`], [`checks`], [`modifiers`], [`stats`], [`damage`],
//!   [`conditions`], [`effects`], [`grid`] — rule primitives.
//! * [`models`] — content and asset models (characters, monsters, spells, ...).
//! * [`builder`] — character validation and stat-block derivation.
//! * [`combat`] — the combat state machine.
//! * [`rng`] — opt-in seeded dice for drivers (never used by the rules).

pub mod builder;
pub mod checks;
pub mod combat;
pub mod conditions;
pub mod damage;
pub mod dice;
pub mod effects;
pub mod grid;
pub mod ids;
pub mod models;
pub mod modifiers;
pub mod rng;
pub mod stats;

pub use ids::CombatantId;
