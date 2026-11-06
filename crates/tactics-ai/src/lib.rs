//! # tactics-ai
//!
//! System-neutral tactical analysis. It **contains no game rules**: it reads
//! rules-derived quantities through [`system::TacticalSystem`] and simulates
//! by applying commands to cloned states through `GameSystem::apply`.
//!
//! * [`evaluation`] — Stockfish-style −10..+10 score with factor breakdown.
//! * [`search`] — committed expectiminimax over a creature's turn.
//! * [`report`] — tactical report, strongest target, side strategy.
//! * [`simulate`] — Monte Carlo win/TPK probabilities.

pub mod evaluation;
pub mod report;
pub mod search;
pub mod simulate;
pub mod system;

pub use evaluation::{EvalOptions, Evaluation, Evaluator, Factor, HeuristicEvaluator, SideSummary};
pub use report::{rank_targets, recommend_turn, side_strategy, CreaturePlan, Strategy, TacticalReport, TargetAssessment, TurnRecommendation};
pub use search::{best_turn, AlternatePlan, PlannedTurn, SearchConfig, StatusExpectation, TargetOutcome, TurnMetrics};
pub use simulate::{simulate, SimulationConfig, SimulationReport};
pub use system::{CandidateConfig, Counterfactual, Node, TacticalSystem, Unit};
