//! PF2e position evaluation: the system-neutral Lanchester evaluator
//! (`tactics_ai::evaluation`) fed by PF2e rules through [`crate::system::Pf2e`].
//!
//! Conditions (frightened, off-guard, slowed, ...), cover and spell slots
//! enter through the core rules that compute attack totals, AC and the
//! available actions, so the evaluator never hard-codes a condition.

pub use tactics_ai::evaluation::{EvalOptions, Evaluation, Evaluator, Factor, SideSummary};

pub type HeuristicEvaluator = tactics_ai::HeuristicEvaluator<crate::system::Pf2e>;
