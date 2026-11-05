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

use crate::evaluation::Evaluator;
use crate::movegen::{candidates, MoveGenConfig};
use pf2e_core::checks::representative_faces;
use pf2e_core::combat::{Action, CombatState, Command, PendingInput, RollKind, RollRequest};
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};

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
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig { node_budget: 40_000, beam: 8, max_moves: 4, focus: None }
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

pub struct Searcher<'a> {
    eval: &'a dyn Evaluator,
    cfg: SearchConfig,
    actor: CombatantId,
    side: Side,
    sign: f64,
    pub nodes: usize,
    pub truncated: bool,
}

impl<'a> Searcher<'a> {
    pub fn new(eval: &'a dyn Evaluator, cfg: SearchConfig, state: &CombatState, actor: CombatantId) -> Self {
        let side = state.combatant(actor).map(|c| c.side).unwrap_or(Side::Party);
        Searcher { eval, cfg, actor, side, sign: if side == Side::Enemy { -1.0 } else { 1.0 }, nodes: 0, truncated: false }
    }

    fn leaf(&self, s: &CombatState) -> f64 {
        self.sign * self.eval.evaluate(s)
    }

    fn collapse(&self) -> bool {
        self.nodes > self.cfg.node_budget / 2
    }

    /// Resolves pending input (chance/reaction nodes) and then continues
    /// with `then` once the engine waits for the actor again.
    fn resolve<F>(&mut self, s: CombatState, then: &mut F) -> (f64, Vec<Action>)
    where
        F: FnMut(&mut Self, CombatState) -> (f64, Vec<Action>),
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
                let mut best: Option<(f64, Vec<Action>)> = None;
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

    /// Value of the best continuation of the actor's turn from `s`.
    fn decide(&mut self, s: CombatState, root: bool) -> (f64, Vec<Action>) {
        let stop = self.leaf(&s);
        if s.current_actor() != Some(self.actor) {
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
        let mut cands = candidates(&s, &MoveGenConfig { max_moves: self.cfg.max_moves, focus: self.cfg.focus });
        order(&mut cands);
        if !root && cands.len() > self.cfg.beam {
            cands.truncate(self.cfg.beam);
        }
        let mut best = (stop, vec![]);
        for act in cands {
            let mut child = s.clone();
            if child.apply_in_place(&Command::Act { actor: self.actor, action: act.clone() }).is_err() {
                continue;
            }
            let (v, mut line) = self.resolve(child, &mut |me: &mut Self, st| me.decide(st, false));
            if v > best.0 + 1e-9 {
                line.insert(0, act);
                best = (v, line);
            }
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
    let actor = state.current_actor()?;
    if state.pending().is_some() {
        return None;
    }
    let mut searcher = Searcher::new(eval, cfg, state, actor);
    let baseline = eval.evaluate(state);
    let (v, actions) = searcher.decide(state.clone(), true);
    Some(PlannedTurn {
        actor,
        actions,
        expected_score: searcher.sign * v,
        baseline_score: baseline,
        gain: v - searcher.sign * baseline,
        nodes: searcher.nodes,
        truncated: searcher.truncated,
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
