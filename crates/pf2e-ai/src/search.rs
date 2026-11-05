//! Expectiminimax search over one creature's turn.
//!
//! * **Decision nodes** (the acting creature's choice): max over candidate
//!   actions from `movegen`, or stopping (end turn).
//! * **Chance nodes** (a pending die roll): the engine's own roll request is
//!   answered with one representative die face per achievable degree of
//!   success, weighted by its exact probability; damage rolls use the
//!   average. The *real engine* resolves every branch, so the search can
//!   never disagree with the rules.
//! * **Reaction nodes**: the reacting creature's side picks what is best for
//!   it (max for the actor's allies, min for its enemies).
//!
//! Leaves are scored by an [`Evaluator`] from the acting side's perspective.
//!
//! Speed-ups (all exact unless noted):
//! * **Transposition table** — decision nodes are memoised by
//!   [`state_key`](crate::keys::state_key) and remaining depth, so move
//!   orders that reach the same position (and chance branches that resolve
//!   identically, e.g. a critical failure and a failure) are searched once.
//! * **Evaluation cache** — per-pair expected damage is reused across
//!   leaves ([`EvalCache`]).
//! * **Iterative deepening** — depth 1, 2, … actions. Each completed
//!   iteration is a valid answer, so a [`Budget`] (e.g. a wall-clock
//!   deadline supplied by the host) can stop the search at any time and the
//!   deepest *completed* result is returned. A history table of action
//!   gains from shallower iterations orders candidates before the beam cut,
//!   so the beam keeps actions that proved good (heuristic: affects which
//!   actions survive the beam, not the value of those that do).

use crate::evaluation::{EvalCache, Evaluator};
use crate::keys::{combine, state_key};
use crate::movegen::{MoveGenCache, MoveGenConfig};
use pf2e_core::checks::representative_faces;
use pf2e_core::combat::{Action, CombatState, Command, PendingInput, RollKind, RollRequest};
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Hard cap on nodes visited; beyond half of it chance nodes collapse
    /// to their most likely outcome.
    pub node_budget: usize,
    /// Candidate actions kept below the root.
    pub beam: usize,
    pub max_moves: usize,
    /// Restrict actions to those involving this target.
    #[serde(default)]
    pub focus: Option<CombatantId>,
    /// Search depth 1, 2, … so a [`Budget`] can stop early with a complete
    /// shallower answer. When false a single full-depth pass is made.
    #[serde(default = "yes")]
    pub iterative: bool,
    /// Memoise decision nodes (exact; costs memory proportional to nodes).
    #[serde(default = "yes")]
    pub transpositions: bool,
}

fn yes() -> bool {
    true
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig { node_budget: 40_000, beam: 8, max_moves: 4, focus: None, iterative: true, transpositions: true }
    }
}

/// External stop signal, polled every few nodes. The search stays pure:
/// hosts that want a wall-clock limit implement this with their own clock.
pub trait Budget {
    fn exhausted(&self, nodes: usize) -> bool;
}

/// Never stops the search (node budget still applies).
pub struct Unlimited;

impl Budget for Unlimited {
    fn exhausted(&self, _nodes: usize) -> bool {
        false
    }
}

impl<F: Fn(usize) -> bool> Budget for F {
    fn exhausted(&self, nodes: usize) -> bool {
        self(nodes)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTurn {
    pub actor: CombatantId,
    pub actions: Vec<Action>,
    /// Expected evaluation after the turn (party-positive scale).
    pub expected_score: f64,
    /// Evaluation now (party-positive scale).
    pub baseline_score: f64,
    /// Improvement from the actor's point of view.
    pub gain: f64,
    pub nodes: usize,
    pub truncated: bool,
    /// Deepest fully searched number of actions.
    #[serde(default)]
    pub depth: u8,
    #[serde(default)]
    pub transposition_hits: u64,
    #[serde(default)]
    pub eval_cache_hits: u64,
}

/// One weighted answer to a roll request.
pub fn roll_outcomes(r: &RollRequest) -> Vec<(i32, f64)> {
    match &r.kind {
        RollKind::Check { modifier, dc } => {
            representative_faces(*modifier, *dc).into_iter().map(|(f, _, p)| (f as i32, p)).collect()
        }
        RollKind::Flat { dc } => representative_faces(0, *dc).into_iter().map(|(f, _, p)| (f as i32, p)).collect(),
        RollKind::Total { dice } => vec![((dice.average().round() as i32).clamp(dice.min(), dice.max()), 1.0)],
    }
}

type Line = (f64, Vec<Action>);

/// How often the external budget is polled (power of two: masked, not divided).
const BUDGET_POLL: usize = 32;

pub struct Searcher<'a> {
    eval: &'a dyn Evaluator,
    budget: &'a dyn Budget,
    cfg: SearchConfig,
    actor: CombatantId,
    side: Side,
    sign: f64,
    pub nodes: usize,
    pub truncated: bool,
    /// The external budget ran out; the current iteration is incomplete.
    pub aborted: bool,
    pub cache: EvalCache,
    pub movegen: MoveGenCache,
    tt: HashMap<u64, Line>,
    pub tt_hits: u64,
    /// Best gain over stopping seen for each action (by its debug form).
    history: HashMap<String, f64>,
}

impl<'a> Searcher<'a> {
    pub fn new(eval: &'a dyn Evaluator, cfg: SearchConfig, state: &CombatState, actor: CombatantId) -> Self {
        Self::with_budget(eval, cfg, state, actor, &Unlimited)
    }

    pub fn with_budget(eval: &'a dyn Evaluator, cfg: SearchConfig, state: &CombatState, actor: CombatantId, budget: &'a dyn Budget) -> Self {
        let side = state.combatant(actor).map(|c| c.side).unwrap_or(Side::Party);
        Searcher {
            eval,
            budget,
            cfg,
            actor,
            side,
            sign: if side == Side::Enemy { -1.0 } else { 1.0 },
            nodes: 0,
            truncated: false,
            aborted: false,
            cache: EvalCache::default(),
            movegen: MoveGenCache::default(),
            tt: HashMap::new(),
            tt_hits: 0,
            history: HashMap::new(),
        }
    }

    fn leaf(&mut self, s: &CombatState) -> f64 {
        self.sign * self.eval.evaluate_cached(s, &mut self.cache)
    }

    fn collapse(&self) -> bool {
        self.nodes > self.cfg.node_budget / 2
    }

    /// Resolves pending input (chance/reaction nodes) and then continues
    /// with `then` once the engine waits for the actor again.
    fn resolve<F>(&mut self, s: CombatState, then: &mut F) -> Line
    where
        F: FnMut(&mut Self, CombatState) -> Line,
    {
        self.nodes += 1;
        if s.is_over() {
            return (self.leaf(&s), vec![]);
        }
        match s.pending() {
            None => then(self, s),
            Some(PendingInput::Roll(r)) => {
                let mut outs = roll_outcomes(&r);
                if self.collapse() && outs.len() > 1 {
                    self.truncated = true;
                    outs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    outs.truncate(1);
                    outs[0].1 = 1.0;
                }
                let mut total = 0.0;
                let mut weight = 0.0;
                // Principal variation follows the most likely outcome.
                let mut line = (0.0, vec![]);
                for (value, p) in outs {
                    let mut child = s.clone();
                    if child.apply_in_place(&Command::ProvideRoll { request_id: r.request_id, value, extra_modifier: 0 }).is_err() {
                        continue;
                    }
                    let (v, l) = self.resolve(child, then);
                    total += p * v;
                    weight += p;
                    if p > line.0 {
                        line = (p, l);
                    }
                }
                let line = line.1;
                (if weight > 0.0 { total / weight } else { self.leaf(&s) }, line)
            }
            Some(PendingInput::Decision(d)) => {
                let ours = s.combatant(d.decider).map(|c| c.side == self.side).unwrap_or(false);
                let mut best: Option<Line> = None;
                for accept in [true, false] {
                    let mut child = s.clone();
                    if child.apply_in_place(&Command::Decide { request_id: d.request_id, accept }).is_err() {
                        continue;
                    }
                    let (v, l) = self.resolve(child, then);
                    let better = match &best {
                        None => true,
                        Some((b, _)) => if ours { v > *b } else { v < *b },
                    };
                    if better {
                        best = Some((v, l));
                    }
                }
                best.unwrap_or_else(|| (self.leaf(&s), vec![]))
            }
        }
    }

    /// Value of the best continuation of the actor's turn from `s`, using at
    /// most `depth` more actions.
    fn decide(&mut self, s: CombatState, root: bool, depth: u8) -> Line {
        let stop = self.leaf(&s);
        if depth == 0 || self.aborted || s.current_actor() != Some(self.actor) {
            return (stop, vec![]);
        }
        let Some(a) = s.combatant(self.actor) else { return (stop, vec![]) };
        if a.actions_remaining == 0 || !a.can_act() {
            return (stop, vec![]);
        }
        if self.nodes >= self.cfg.node_budget {
            self.truncated = true;
            return (stop, vec![]);
        }
        if self.nodes & (BUDGET_POLL - 1) == 0 && self.budget.exhausted(self.nodes) {
            self.aborted = true;
            return (stop, vec![]);
        }
        let key = combine(&[state_key(&s), depth as u64]);
        if self.cfg.transpositions && !root {
            if let Some(hit) = self.tt.get(&key) {
                self.tt_hits += 1;
                return hit.clone();
            }
        }
        let mut cands = self.movegen.candidates(&s, &MoveGenConfig { max_moves: self.cfg.max_moves, focus: self.cfg.focus });
        order(&mut cands);
        if !self.history.is_empty() {
            // Stable: equal history keeps the static order.
            let h = |a: &Action| self.history.get(&format!("{a:?}")).copied().unwrap_or(f64::NEG_INFINITY);
            cands.sort_by(|x, y| h(y).partial_cmp(&h(x)).unwrap_or(std::cmp::Ordering::Equal));
        }
        if !root && cands.len() > self.cfg.beam {
            cands.truncate(self.cfg.beam);
        }
        let mut best = (stop, vec![]);
        for act in cands {
            let mut child = s.clone();
            if child.apply_in_place(&Command::Act { actor: self.actor, action: act.clone() }).is_err() {
                continue;
            }
            let (v, mut line) = self.resolve(child, &mut |me: &mut Self, st| me.decide(st, false, depth - 1));
            if self.aborted {
                return best;
            }
            let gain = v - stop;
            let k = format!("{act:?}");
            let e = self.history.entry(k).or_insert(f64::NEG_INFINITY);
            *e = e.max(gain);
            if v > best.0 + 1e-9 {
                line.insert(0, act);
                best = (v, line);
            }
        }
        if self.cfg.transpositions && !self.aborted {
            self.tt.insert(key, best.clone());
        }
        best
    }

    /// Expected leaf value right after `action` resolves (one ply).
    pub fn one_ply(&mut self, s: &CombatState, action: &Action) -> Option<f64> {
        let mut child = s.clone();
        child.apply_in_place(&Command::Act { actor: self.actor, action: action.clone() }).ok()?;
        Some(self.resolve(child, &mut |me: &mut Self, st| (me.leaf(&st), vec![])).0)
    }
}

/// Cheap move ordering so the beam keeps the likeliest-good actions.
fn order(c: &mut [Action]) {
    let rank = |a: &Action| match a {
        Action::Strike { .. } => 0,
        Action::CastSpell { .. } => 1,
        Action::Trip { .. } | Action::Demoralize { .. } => 2,
        Action::Stride { .. } => 3,
        Action::Step { .. } => 4,
        Action::RaiseShield | Action::Stand => 5,
        _ => 6,
    };
    c.sort_by_key(rank);
}

/// The best sequence of actions for whoever's turn it is.
pub fn best_turn(state: &CombatState, eval: &dyn Evaluator, cfg: SearchConfig) -> Option<PlannedTurn> {
    best_turn_with_budget(state, eval, cfg, &Unlimited)
}

/// [`best_turn`] that stops when `budget` is exhausted, returning the
/// deepest completed iteration (depth ≥ 1 whenever any action is legal).
pub fn best_turn_with_budget(state: &CombatState, eval: &dyn Evaluator, cfg: SearchConfig, budget: &dyn Budget) -> Option<PlannedTurn> {
    let actor = state.current_actor()?;
    if state.pending().is_some() {
        return None;
    }
    let mut searcher = Searcher::with_budget(eval, cfg, state, actor, budget);
    let baseline = eval.evaluate(state);
    let max_depth = state.combatant(actor).map(|c| c.actions_remaining).unwrap_or(0).max(1);
    let depths: Vec<u8> = if cfg.iterative { (1..=max_depth).collect() } else { vec![max_depth] };
    let mut result: Option<(Line, u8)> = None;
    for depth in depths {
        let line = searcher.decide(state.clone(), true, depth);
        if searcher.aborted {
            // The first iteration is tiny; keep its partial answer only if
            // nothing better exists.
            if result.is_none() {
                result = Some((line, 0));
            }
            break;
        }
        result = Some((line, depth));
        if searcher.nodes >= cfg.node_budget {
            break;
        }
    }
    let ((v, actions), depth) = result?;
    Some(PlannedTurn {
        actor,
        actions,
        expected_score: searcher.sign * v,
        baseline_score: baseline,
        gain: v - searcher.sign * baseline,
        nodes: searcher.nodes,
        truncated: searcher.truncated || searcher.aborted,
        depth,
        transposition_hits: searcher.tt_hits,
        eval_cache_hits: searcher.cache.hits,
    })
}

/// Expectation of an arbitrary metric over the chance outcomes of `action`,
/// assuming every offered reaction is taken.
pub fn expectation_after<F: Fn(&CombatState) -> f64>(state: &CombatState, actor: CombatantId, action: &Action, metric: &F) -> Option<f64> {
    let mut child = state.clone();
    child.apply_in_place(&Command::Act { actor, action: action.clone() }).ok()?;
    Some(walk(child, metric, 0))
}

fn walk<F: Fn(&CombatState) -> f64>(s: CombatState, metric: &F, depth: usize) -> f64 {
    if depth > 64 {
        return metric(&s);
    }
    match s.pending() {
        None => metric(&s),
        Some(PendingInput::Roll(r)) => {
            let mut total = 0.0;
            for (value, p) in roll_outcomes(&r) {
                let mut child = s.clone();
                if child.apply_in_place(&Command::ProvideRoll { request_id: r.request_id, value, extra_modifier: 0 }).is_ok() {
                    total += p * walk(child, metric, depth + 1);
                }
            }
            total
        }
        Some(PendingInput::Decision(d)) => {
            let mut child = s.clone();
            match child.apply_in_place(&Command::Decide { request_id: d.request_id, accept: true }) {
                Ok(_) => walk(child, metric, depth + 1),
                Err(_) => metric(&s),
            }
        }
    }
}
