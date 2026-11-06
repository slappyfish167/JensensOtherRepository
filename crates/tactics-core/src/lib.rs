//! # tactics-core
//!
//! System-neutral building blocks shared by every rules engine in the
//! workspace (Pathfinder 2e, Cyberpunk RED, ...):
//!
//! * [`ids`], [`side`], [`geom`], [`dice`], [`rng`] — primitives.
//! * [`system::GameSystem`] — the contract a rules engine fulfils so the
//!   server, store and analysis layer can drive it without knowing its rules.
//! * [`view`] — neutral read models (table, prompt, actions) for any UI.

pub mod dice;
pub mod geom;
pub mod ids;
pub mod rng;
pub mod side;
pub mod system;
pub mod view;

pub use dice::DiceExpr;
pub use geom::Pos;
pub use ids::CombatantId;
pub use rng::SeededDice;
pub use side::Side;
pub use system::GameSystem;
