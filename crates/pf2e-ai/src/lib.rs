//! # pf2e-ai
//!
//! The Pathfinder 2e adapter for the system-neutral analysis layer
//! (`tactics-ai`). It **contains no game rules**: [`system::Pf2e`] exposes
//! `pf2e-core` through the `GameSystem` / `TacticalSystem` contracts, and
//! the expected-value helpers in [`analysis`] are built on core rule
//! functions. Removing this crate leaves a fully functional rules engine.
//!
//! * [`system`] — the `Pf2e` adapter, including its exact search caches
//!   ([`keys`] for transposition and pair keys, [`evaluation::EvalCache`]).
//! * [`explain`] — natural-language explanations of positions and changes.
//! * [`evaluation`], [`search`], [`queries`], [`simulate`] — PF2e entry
//!   points to the generic evaluator, search, report and Monte Carlo, plus
//!   PF2e-only questions (best spell).

pub mod analysis;
pub mod evaluation;
pub mod explain;
pub mod keys;
pub mod movegen;
pub mod queries;
pub mod search;
pub mod simulate;
pub mod system;

pub use evaluation::{Evaluation, Evaluator, HeuristicEvaluator};
pub use search::{best_turn, best_turn_with_budget, Budget, PlannedTurn, SearchConfig, Unlimited};
pub use simulate::{simulate, simulate_range, SimulationConfig, SimulationReport, SimulationTally};
pub use system::Pf2e;

#[cfg(test)]
mod tests;
