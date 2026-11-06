//! PF2e entry points to the system-neutral turn search (`tactics_ai::search`).

use crate::system::Pf2e;
use pf2e_core::combat::{Action, CombatState};
use pf2e_core::CombatantId;
pub use crate::system::roll_outcomes;
pub use tactics_ai::search::{Budget, SearchConfig, StatusExpectation, TargetOutcome, TurnMetrics, Unlimited};
use tactics_ai::Evaluator;

pub type PlannedTurn = tactics_ai::PlannedTurn<Action>;
pub type AlternatePlan = tactics_ai::AlternatePlan<Action>;
pub type Searcher<'a> = tactics_ai::search::Searcher<'a, Pf2e>;

/// The best sequence of actions for whoever's turn it is (see `tactics_ai::search::best_turn`).
pub fn best_turn(state: &CombatState, eval: &dyn Evaluator<Pf2e>, cfg: SearchConfig) -> Option<PlannedTurn> {
    tactics_ai::search::best_turn::<Pf2e>(state, eval, cfg)
}

/// [`best_turn`] that stops when `budget` is exhausted, returning the
/// deepest completed iteration (see `tactics_ai::search::best_turn_with_budget`).
pub fn best_turn_with_budget(state: &CombatState, eval: &dyn Evaluator<Pf2e>, cfg: SearchConfig, budget: &dyn Budget) -> Option<PlannedTurn> {
    tactics_ai::search::best_turn_with_budget::<Pf2e>(state, eval, cfg, budget)
}

/// Expectation of a metric over the chance outcomes of `action` (reactions accepted).
pub fn expectation_after<F: Fn(&CombatState) -> f64>(state: &CombatState, actor: CombatantId, action: &Action, metric: &F) -> Option<f64> {
    tactics_ai::search::expectation_after::<Pf2e, F>(state, actor, action, metric)
}
