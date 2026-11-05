//! # pf2e-ai
//!
//! Optional tactical layer on top of `pf2e-core`. It **contains no game
//! rules**: it reads state, asks the engine for legal actions, and simulates
//! by applying commands to cloned states. Removing this crate leaves a fully
//! functional rules engine.
//!
//! * [`evaluation`] — Stockfish-style −10..+10 score with factor breakdown.
//! * [`search`] — expectiminimax over a creature's turn ("best 3-action turn").
//! * [`queries`] — strongest target, best spell, side strategy.
//! * [`simulate`] — Monte Carlo win/TPK probabilities.

pub mod analysis;
pub mod evaluation;
pub mod explain;
pub mod keys;
pub mod movegen;
pub mod queries;
pub mod search;
pub mod simulate;

pub use evaluation::{Evaluation, Evaluator, HeuristicEvaluator};
pub use search::{best_turn, best_turn_with_budget, Budget, PlannedTurn, SearchConfig, Unlimited};
pub use simulate::{simulate, simulate_range, SimulationConfig, SimulationReport, SimulationTally};

#[cfg(test)]
mod tests;
