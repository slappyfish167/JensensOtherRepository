//! What the analysis layer needs from a rules engine. A system implements
//! [`TacticalSystem`] on top of [`GameSystem`]; evaluation, search, the
//! tactical report, strategy and Monte Carlo then work unchanged.
//!
//! Every method is a *read* of rules-derived quantities (expected damage,
//! who is down, which actions are worth considering). None of them may
//! invent outcomes: simulation always goes through `GameSystem::apply`.

pub use crate::evaluation::EvalOptions;
use serde::{Deserialize, Serialize};
use tactics_core::{CombatantId, GameSystem, Side};

/// A combatant as the analysis layer sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    pub id: CombatantId,
    pub name: String,
    pub side: Side,
    /// A player character (dropping one removes a player's turn).
    pub is_pc: bool,
    /// Not a fighter (a hazard, an object): ignored by evaluation and targeting.
    pub ignored: bool,
    pub down: bool,
    pub dead: bool,
    /// HP + temporary HP.
    pub hp: i32,
    pub max_hp: i32,
}

/// What the search is allowed to consider at a decision node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateConfig {
    /// Maximum distinct movement destinations kept.
    pub max_moves: usize,
    /// Only actions involving this target (plus movement and defence).
    pub focus: Option<CombatantId>,
    /// Committed play: never return an empty list while a legal action exists.
    pub committed: bool,
}

/// What the engine is waiting for, from the search's point of view.
pub enum Node<C> {
    /// No input pending: the acting creature (or nobody) chooses.
    Ready,
    /// A pending roll, answered by weighted representative commands (weights sum to 1).
    Chance(Vec<(C, f64)>),
    /// A yes/no decision by `decider`.
    Decision { decider: CombatantId, accept: C, decline: C },
    /// Input is pending but cannot be answered by analysis.
    Stuck,
}

/// A "what if this aspect were neutral?" probe for the factor breakdown.
pub struct Counterfactual<S: GameSystem> {
    pub name: &'static str,
    pub detail: &'static str,
    pub apply: fn(&mut S::State, &mut EvalOptions),
}

pub trait TacticalSystem: GameSystem {
    /// Per-search scratch space (memoised expected damage, move generation).
    /// Use `()` when the system needs none. Caches must be exact: a cached
    /// answer equals the uncached one.
    type Cache: Default;

    // ----- roster ------------------------------------------------------------

    /// Every combatant, in a stable order.
    fn units(state: &Self::State) -> Vec<Unit>;
    /// HP + temporary HP (cheap; called at every search leaf).
    fn hp(state: &Self::State, id: CombatantId) -> i32;
    fn is_down(state: &Self::State, id: CombatantId) -> bool;
    /// Current conditions/statuses as labels (`frightened 1`, `prone`,
    /// `seriously wounded`). Leave out the death track: being dropped is
    /// reported as the chance to down a target instead.
    fn status_labels(state: &Self::State, id: CombatantId) -> Vec<String>;

    // ----- search plumbing -------------------------------------------------

    fn node(state: &Self::State) -> Node<Self::Command>;
    /// The actor still has something it may do this turn.
    fn turn_open(state: &Self::State, actor: CombatantId) -> bool;
    /// What remains of the turn is optional (e.g. an unused Move Action
    /// after the Action is spent), so stopping is a legitimate choice.
    fn may_stop(_state: &Self::State, _actor: CombatantId) -> bool {
        false
    }
    /// Tactically distinct legal actions, roughly best-first. Must only
    /// return actions from `legal_actions`.
    fn candidates(state: &Self::State, cfg: &CandidateConfig, cache: &mut Self::Cache) -> Vec<Self::Action>;
    /// How many actions a full turn can take from here (iterative deepening
    /// searches depth 1 … this).
    fn turn_depth(_state: &Self::State, _actor: CombatantId) -> u8 {
        3
    }
    /// Transposition key: equal keys mean the same future (same creatures,
    /// HP, conditions, resources, turn and actions left). `None` disables
    /// the transposition table for this system.
    fn state_key(_state: &Self::State) -> Option<u64> {
        None
    }
    /// The hostile creature an action is aimed at.
    fn action_target(action: &Self::Action) -> Option<CombatantId>;
    /// Kind of action, for telling near-duplicate plans apart.
    fn action_kind(action: &Self::Action) -> &'static str;
    /// Extra reasons a plan is good that only the system can see (flanking, cover).
    fn plan_notes(_state: &Self::State, _actor: CombatantId, _actions: &[Self::Action]) -> Vec<String> {
        vec![]
    }

    // ----- evaluation inputs -------------------------------------------------

    /// Hit points net of damage already locked in (e.g. persistent damage).
    fn effective_hp(state: &Self::State, id: CombatantId) -> f64;
    /// Best expected damage `attacker` deals `target` over a full round.
    /// `include_resources` allows limited resources (spell slots, ammo).
    fn round_damage(state: &Self::State, attacker: CombatantId, target: CombatantId, include_resources: bool) -> f64;
    /// Fraction of a round's damage kept after closing the distance (0..=1).
    fn engagement(state: &Self::State, attacker: CombatantId, target: CombatantId) -> f64;
    /// Called once per cached evaluation, before any `pair_damage`, so the
    /// system can precompute cache keys for this position.
    fn prepare_cache(_state: &Self::State, _cache: &mut Self::Cache) {}
    /// `round_damage` (× `engagement` when `position`), possibly memoised.
    /// Must equal the uncached value exactly.
    fn pair_damage(state: &Self::State, attacker: CombatantId, target: CombatantId, include_resources: bool, position: bool, _cache: &mut Self::Cache) -> f64 {
        let d = Self::round_damage(state, attacker, target, include_resources);
        if position {
            d * Self::engagement(state, attacker, target)
        } else {
            d
        }
    }
    /// Cache hits so far, for search statistics.
    fn cache_hits(_cache: &Self::Cache) -> u64 {
        0
    }
    /// Next turn's action budget relative to normal (slowed, stunned, ...).
    fn action_factor(state: &Self::State, id: CombatantId) -> f64;
    /// Share of a round's damage the current actor can still deal this turn.
    fn tempo_fraction(state: &Self::State, actor: CombatantId) -> f64;
    /// `Some(winner)` once the combat has ended.
    fn winner(state: &Self::State) -> Option<Option<Side>>;
    /// System-specific factors for the evaluation breakdown.
    fn counterfactuals() -> Vec<Counterfactual<Self>> {
        vec![]
    }

    // ----- queries -----------------------------------------------------------

    /// P(`attacker` drops `target` with what it has left this turn).
    fn kill_probability(state: &Self::State, attacker: CombatantId, target: CombatantId) -> f64;
    /// A copy in which `id` is starting a fresh turn (None if that needs input).
    fn hypothetical_turn(state: &Self::State, id: CombatantId) -> Option<Self::State>;
    /// Fast policy for Monte Carlo: a good legal action, or None to end the turn.
    fn scripted_action(state: &Self::State) -> Option<Self::Action>;
    /// What the actor has left this turn, for reports (`2 actions`, `Move + Action`).
    fn turn_budget(_state: &Self::State, _actor: CombatantId) -> Option<String> {
        None
    }
}
