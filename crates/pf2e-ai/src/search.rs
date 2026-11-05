//! Expectiminimax search over one creature's turn.
//!
//! * **Decision nodes** (the acting creature's choice): max over candidate
//!   actions from `movegen`. The search is **committed** by default: ending
//!   the turn early is *not* a candidate while any legal action remains, so
//!   the result is always the best legal combat turn, never "do nothing".
//!   Passing `allow_end_turn` restores the advisory behaviour in which
//!   stopping competes with acting.
//! * **Chance nodes** (a pending die roll): the engine's own roll request is
//!   answered with one representative die face per achievable degree of
//!   success, weighted by its exact probability; damage rolls use the
//!   average. The *real engine* resolves every branch, so the search can
//!   never disagree with the rules.
//! * **Reaction nodes**: the reacting creature's side picks what is best for
//!   it (max for the actor's allies, min for its enemies).
//!
//! Leaves are scored by an [`Evaluator`] from the acting side's perspective.
//! Alongside the score every node carries the turn's outcome metrics
//! (damage dealt and taken, who drops, which conditions land), aggregated
//! with the same probabilities and choices as the score, so the reported
//! expectations describe exactly the adaptive policy the search found.

use crate::evaluation::Evaluator;
use crate::movegen::{candidates, MoveGenConfig};
use pf2e_core::checks::representative_faces;
use pf2e_core::combat::{Action, CombatState, Combatant, Command, PendingInput, RollKind, RollRequest};
use pf2e_core::conditions::ConditionKind;
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Soft cap on nodes visited; beyond half of it chance nodes collapse
    /// to their most likely outcome, beyond all of it the rest of the turn
    /// is filled greedily (one-ply lookahead per action).
    pub node_budget: usize,
    /// Candidate actions kept below the root.
    pub beam: usize,
    pub max_moves: usize,
    /// Restrict actions to those involving this target.
    #[serde(default)]
    pub focus: Option<CombatantId>,
    /// Advisory mode: let "end the turn now" compete with acting. Off by
    /// default — the engine plays to win and always spends its actions.
    #[serde(default)]
    pub allow_end_turn: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig { node_budget: 40_000, beam: 8, max_moves: 4, focus: None, allow_end_turn: false }
    }
}

/// Expected outcome against one hostile creature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetOutcome {
    pub target: CombatantId,
    pub name: String,
    pub expected_damage: f64,
    /// Probability the creature is down (unconscious or dead) at the end of the turn.
    pub down_chance: f64,
}

/// A condition, persistent damage or effect the turn may inflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusExpectation {
    pub target: CombatantId,
    pub name: String,
    /// e.g. `frightened 1`, `prone`, `persistent bleed 1d6`.
    pub effect: String,
    pub probability: f64,
}

/// Expectations over the chance tree of the planned turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnMetrics {
    /// HP + temp HP removed from hostile creatures.
    pub expected_damage: f64,
    /// HP + temp HP the actor loses (reactions, hazards).
    pub expected_damage_taken: f64,
    /// Hostile creatures, most damaged first.
    pub targets: Vec<TargetOutcome>,
    /// Most likely first.
    pub status_effects: Vec<StatusExpectation>,
}

impl TurnMetrics {
    /// The creature this turn is aimed at: the one most actions of the
    /// planned line are directed at (ties: most expected damage and chance
    /// to drop it), otherwise the one taking the most expected damage.
    pub fn primary_target(&self, actions: &[Action]) -> Option<CombatantId> {
        let weight = |id: CombatantId| self.target(id).map(|t| t.expected_damage + 100.0 * t.down_chance).unwrap_or(0.0);
        let mut counts: Vec<(CombatantId, usize)> = Vec::new();
        for t in actions.iter().filter_map(action_target) {
            match counts.iter_mut().find(|(id, _)| *id == t) {
                Some(c) => c.1 += 1,
                None => counts.push((t, 1)),
            }
        }
        let by = |a: &(CombatantId, f64, usize), b: &(CombatantId, f64, usize)| a.2.cmp(&b.2).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        counts
            .iter()
            .map(|(id, n)| (*id, weight(*id), *n))
            .max_by(by)
            .or_else(|| self.targets.iter().map(|t| (t.target, weight(t.target), 0)).filter(|t| t.1 > 1e-6).max_by(by))
            .map(|t| t.0)
    }

    pub fn target(&self, id: CombatantId) -> Option<&TargetOutcome> {
        self.targets.iter().find(|t| t.target == id)
    }
}

/// The hostile creature an action is aimed at, if any.
pub fn action_target(a: &Action) -> Option<CombatantId> {
    match a {
        Action::Strike { target, .. } | Action::Demoralize { target } | Action::Trip { target } => Some(*target),
        Action::CastSpell { targets, .. } => targets.first().copied(),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternatePlan {
    pub actions: Vec<Action>,
    pub expected_score: f64,
    pub gain: f64,
    pub metrics: TurnMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTurn {
    pub actor: CombatantId,
    pub actions: Vec<Action>,
    /// Expected evaluation after the turn (party-positive scale).
    pub expected_score: f64,
    /// Evaluation now (party-positive scale).
    pub baseline_score: f64,
    /// Change in evaluation from the actor's point of view (may be negative:
    /// a committed turn is the best available, not necessarily a gain).
    pub gain: f64,
    pub metrics: TurnMetrics,
    /// The best root choice that differs meaningfully from `actions`
    /// (another target or another kind of opening action).
    pub alternate: Option<AlternatePlan>,
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

/// What a hostile creature looked like before the turn.
struct FoeSnap {
    id: CombatantId,
    hp: i32,
    down: bool,
    conditions: Vec<(ConditionKind, u8)>,
    persistent: Vec<String>,
    effects: Vec<String>,
}

fn effective_hp(c: &Combatant) -> i32 {
    c.hp.max(0) + c.temp_hp.max(0)
}

fn persistent_labels(c: &Combatant) -> Vec<String> {
    c.persistent.iter().map(|p| format!("persistent {} {}", format!("{:?}", p.damage_type).to_lowercase(), p.dice)).collect()
}

/// Metrics of one leaf (or, aggregated, of a subtree).
#[derive(Clone, Default)]
struct Outcome {
    taken: f64,
    dealt: Vec<f64>,
    downs: Vec<f64>,
    status: BTreeMap<(usize, String), f64>,
}

impl Outcome {
    fn add_scaled(&mut self, o: &Outcome, p: f64) {
        self.taken += p * o.taken;
        if self.dealt.len() < o.dealt.len() {
            self.dealt.resize(o.dealt.len(), 0.0);
            self.downs.resize(o.downs.len(), 0.0);
        }
        for (i, v) in o.dealt.iter().enumerate() {
            self.dealt[i] += p * v;
        }
        for (i, v) in o.downs.iter().enumerate() {
            self.downs[i] += p * v;
        }
        for (k, v) in &o.status {
            *self.status.entry(k.clone()).or_default() += p * v;
        }
    }

    fn scale(&mut self, k: f64) {
        self.taken *= k;
        self.dealt.iter_mut().for_each(|v| *v *= k);
        self.downs.iter_mut().for_each(|v| *v *= k);
        self.status.values_mut().for_each(|v| *v *= k);
    }
}

#[derive(Clone)]
struct Node {
    value: f64,
    line: Vec<Action>,
    out: Outcome,
}

pub struct Searcher<'a> {
    eval: &'a dyn Evaluator,
    cfg: SearchConfig,
    actor: CombatantId,
    side: Side,
    sign: f64,
    actor_hp: i32,
    foes: Vec<FoeSnap>,
    root_nodes: Vec<Node>,
    /// Budget window of the root candidate being searched: each opening may
    /// use what is left but must leave a reserve for the openings after it,
    /// so late candidates are never judged on a fully collapsed
    /// (optimistic, most-likely-outcome) tree or skipped outright.
    window_start: usize,
    window: usize,
    pub nodes: usize,
    pub truncated: bool,
}

impl<'a> Searcher<'a> {
    pub fn new(eval: &'a dyn Evaluator, cfg: SearchConfig, state: &CombatState, actor: CombatantId) -> Self {
        let me = state.combatant(actor);
        let side = me.map(|c| c.side).unwrap_or(Side::Party);
        let foes = state
            .combatants
            .iter()
            .filter(|c| side.is_hostile_to(c.side) && !c.is_hazard())
            .map(|c| FoeSnap {
                id: c.id,
                hp: effective_hp(c),
                down: c.is_down(),
                conditions: c.conditions.0.iter().map(|k| (k.kind, k.value)).collect(),
                persistent: persistent_labels(c),
                effects: c.effects.iter().map(|e| e.name.clone()).collect(),
            })
            .collect();
        Searcher {
            eval,
            cfg,
            actor,
            side,
            sign: if side == Side::Enemy { -1.0 } else { 1.0 },
            actor_hp: me.map(effective_hp).unwrap_or(0),
            foes,
            root_nodes: Vec::new(),
            window_start: 0,
            window: cfg.node_budget,
            nodes: 0,
            truncated: false,
        }
    }

    fn leaf(&self, s: &CombatState) -> f64 {
        self.sign * self.eval.evaluate(s)
    }

    fn outcome(&self, s: &CombatState) -> Outcome {
        let mut o = Outcome { dealt: vec![0.0; self.foes.len()], downs: vec![0.0; self.foes.len()], ..Default::default() };
        o.taken = s.combatant(self.actor).map(|c| (self.actor_hp - effective_hp(c)).max(0) as f64).unwrap_or(0.0);
        for (i, f) in self.foes.iter().enumerate() {
            let Some(c) = s.combatant(f.id) else { continue };
            o.dealt[i] = (f.hp - effective_hp(c)).max(0) as f64;
            let down = c.is_down();
            if down && !f.down {
                o.downs[i] = 1.0;
            }
            for k in &c.conditions.0 {
                use ConditionKind::*;
                // The death track is reported as the chance to down the target.
                if matches!(k.kind, Dying | Unconscious | Wounded | Doomed) || (down && matches!(k.kind, Prone | OffGuard)) {
                    continue;
                }
                let before = f.conditions.iter().find(|(kind, _)| *kind == k.kind).map(|(_, v)| *v);
                let new = match before {
                    None => true,
                    Some(v) => k.kind.is_valued() && k.value > v,
                };
                if new {
                    let label = if k.kind.is_valued() { format!("{} {}", k.kind.label(), k.value) } else { k.kind.label() };
                    o.status.insert((i, label), 1.0);
                }
            }
            for p in persistent_labels(c) {
                if !f.persistent.contains(&p) {
                    o.status.insert((i, p), 1.0);
                }
            }
            for e in &c.effects {
                if !f.effects.contains(&e.name) {
                    o.status.insert((i, e.name.clone()), 1.0);
                }
            }
        }
        o
    }

    fn leaf_node(&self, s: &CombatState) -> Node {
        Node { value: self.leaf(s), line: vec![], out: self.outcome(s) }
    }

    fn used(&self) -> usize {
        self.nodes - self.window_start
    }

    fn collapse(&self) -> bool {
        self.used() > self.window / 2
    }

    /// Resolves pending input (chance/reaction nodes) and then continues
    /// with `then` once the engine waits for the actor again.
    fn resolve<F>(&mut self, s: CombatState, then: &mut F) -> Node
    where
        F: FnMut(&mut Self, CombatState) -> Node,
    {
        self.nodes += 1;
        if s.is_over() {
            return self.leaf_node(&s);
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
                let mut agg = Outcome::default();
                // Principal variation follows the most likely outcome.
                let mut line = (0.0, vec![]);
                for (value, p) in outs {
                    let mut child = s.clone();
                    if child.apply_in_place(&Command::ProvideRoll { request_id: r.request_id, value, extra_modifier: 0 }).is_err() {
                        continue;
                    }
                    let n = self.resolve(child, then);
                    total += p * n.value;
                    weight += p;
                    agg.add_scaled(&n.out, p);
                    if p > line.0 {
                        line = (p, n.line);
                    }
                }
                if weight > 0.0 {
                    agg.scale(1.0 / weight);
                    Node { value: total / weight, line: line.1, out: agg }
                } else {
                    self.leaf_node(&s)
                }
            }
            Some(PendingInput::Decision(d)) => {
                let ours = s.combatant(d.decider).map(|c| c.side == self.side).unwrap_or(false);
                let mut best: Option<Node> = None;
                for accept in [true, false] {
                    let mut child = s.clone();
                    if child.apply_in_place(&Command::Decide { request_id: d.request_id, accept }).is_err() {
                        continue;
                    }
                    let n = self.resolve(child, then);
                    let better = match &best {
                        None => true,
                        Some(b) => if ours { n.value > b.value } else { n.value < b.value },
                    };
                    if better {
                        best = Some(n);
                    }
                }
                best.unwrap_or_else(|| self.leaf_node(&s))
            }
        }
    }

    fn candidates(&self, s: &CombatState) -> Vec<Action> {
        let mut c = candidates(s, &MoveGenConfig { max_moves: self.cfg.max_moves, focus: self.cfg.focus, committed: !self.cfg.allow_end_turn });
        if c.is_empty() && self.cfg.focus.is_some() && !self.cfg.allow_end_turn {
            // The focus target is unreachable or gone: still play the turn.
            c = candidates(s, &MoveGenConfig { max_moves: self.cfg.max_moves, focus: None, committed: true });
        }
        order(&mut c);
        c
    }

    /// Value of the best continuation of the actor's turn from `s`.
    fn decide(&mut self, s: CombatState, root: bool) -> Node {
        if s.current_actor() != Some(self.actor) {
            return self.leaf_node(&s);
        }
        let Some(a) = s.combatant(self.actor) else { return self.leaf_node(&s) };
        if a.actions_remaining == 0 || !a.can_act() {
            return self.leaf_node(&s);
        }
        let over = self.used() >= self.window;
        if over && self.cfg.allow_end_turn {
            self.truncated = true;
            return self.leaf_node(&s);
        }
        let mut cands = self.candidates(&s);
        if !root && cands.len() > self.cfg.beam {
            cands.truncate(self.cfg.beam);
        }
        if over {
            self.truncated = true;
            return self.greedy(s, cands);
        }
        // Committed play: ending the turn is only the answer when no
        // candidate can legally be taken.
        let mut best: Option<Node> = if self.cfg.allow_end_turn { Some(self.leaf_node(&s)) } else { None };
        let n_cands = cands.len();
        for (i, act) in cands.into_iter().enumerate() {
            let mut child = s.clone();
            if child.apply_in_place(&Command::Act { actor: self.actor, action: act.clone() }).is_err() {
                continue;
            }
            if root {
                self.window_start = self.nodes;
                // Use what is left, minus a guaranteed reserve for every
                // opening still to be searched.
                let reserve = self.cfg.node_budget / (4 * n_cands);
                let left = self.cfg.node_budget.saturating_sub(self.nodes);
                self.window = left.saturating_sub(reserve * (n_cands - i - 1)).max(reserve).max(1);
            }
            let mut n = self.resolve(child, &mut |me: &mut Self, st| me.decide(st, false));
            n.line.insert(0, act);
            if root {
                self.root_nodes.push(n.clone());
            }
            if best.as_ref().map(|b| n.value > b.value + 1e-9).unwrap_or(true) {
                best = Some(n);
            }
        }
        best.unwrap_or_else(|| self.leaf_node(&s))
    }

    /// Out of budget: pick each remaining action by one-ply lookahead so the
    /// turn is still completed rather than abandoned.
    fn greedy(&mut self, s: CombatState, cands: Vec<Action>) -> Node {
        let mut best: Option<(f64, Action, CombatState)> = None;
        for act in cands {
            let mut child = s.clone();
            if child.apply_in_place(&Command::Act { actor: self.actor, action: act.clone() }).is_err() {
                continue;
            }
            let v = self.resolve(child.clone(), &mut |me: &mut Self, st| me.leaf_node(&st)).value;
            if best.as_ref().map(|b| v > b.0 + 1e-9).unwrap_or(true) {
                best = Some((v, act, child));
            }
        }
        match best {
            Some((_, act, child)) => {
                let mut n = self.resolve(child, &mut |me: &mut Self, st| me.decide(st, false));
                n.line.insert(0, act);
                n
            }
            None => self.leaf_node(&s),
        }
    }

    /// Expected leaf value right after `action` resolves (one ply).
    pub fn one_ply(&mut self, s: &CombatState, action: &Action) -> Option<f64> {
        let mut child = s.clone();
        child.apply_in_place(&Command::Act { actor: self.actor, action: action.clone() }).ok()?;
        Some(self.resolve(child, &mut |me: &mut Self, st| me.leaf_node(&st)).value)
    }

    fn metrics(&self, s: &CombatState, o: &Outcome) -> TurnMetrics {
        let name = |id: CombatantId| s.combatant(id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_string());
        let mut targets: Vec<TargetOutcome> = self
            .foes
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.down)
            .map(|(i, f)| TargetOutcome {
                target: f.id,
                name: name(f.id),
                expected_damage: o.dealt.get(i).copied().unwrap_or(0.0),
                down_chance: o.downs.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0),
            })
            .collect();
        targets.sort_by(|a, b| b.expected_damage.partial_cmp(&a.expected_damage).unwrap_or(std::cmp::Ordering::Equal));
        let mut status_effects: Vec<StatusExpectation> = o
            .status
            .iter()
            .filter(|(_, p)| **p >= 0.005)
            .map(|((i, effect), p)| StatusExpectation { target: self.foes[*i].id, name: name(self.foes[*i].id), effect: effect.clone(), probability: p.clamp(0.0, 1.0) })
            .collect();
        status_effects.sort_by(|a, b| b.probability.partial_cmp(&a.probability).unwrap_or(std::cmp::Ordering::Equal));
        TurnMetrics { expected_damage: o.dealt.iter().sum(), expected_damage_taken: o.taken, targets, status_effects }
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
        Action::TakeCover => 6,
        _ => 7,
    };
    c.sort_by_key(rank);
}

/// Two root choices are near-duplicates when they open with the same kind
/// of action against the same creature (e.g. the same Strike with another
/// weapon, or a Stride to a neighbouring square toward the same foe).
fn same_idea(a: &[Action], ta: Option<CombatantId>, b: &[Action], tb: Option<CombatantId>) -> bool {
    let kind = |l: &[Action]| l.first().map(|x| x.name());
    kind(a) == kind(b) && ta == tb
}

/// The best sequence of actions for whoever's turn it is.
///
/// Returns `None` only when nobody can act (combat not running, or input is
/// pending). With the default (committed) configuration `actions` is empty
/// only when the creature literally has no legal action.
pub fn best_turn(state: &CombatState, eval: &dyn Evaluator, cfg: SearchConfig) -> Option<PlannedTurn> {
    let actor = state.current_actor()?;
    if state.pending().is_some() {
        return None;
    }
    let mut searcher = Searcher::new(eval, cfg, state, actor);
    let baseline = eval.evaluate(state);
    let best = searcher.decide(state.clone(), true);
    let metrics = searcher.metrics(state, &best.out);
    let target = metrics.primary_target(&best.line);

    let mut roots = std::mem::take(&mut searcher.root_nodes);
    roots.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
    let others: Vec<(Node, TurnMetrics, Option<CombatantId>)> = roots
        .into_iter()
        .filter(|n| n.line.first() != best.line.first())
        .map(|n| {
            let m = searcher.metrics(state, &n.out);
            let t = m.primary_target(&n.line);
            (n, m, t)
        })
        .collect();
    let pick = others
        .iter()
        .position(|(n, _, t)| !same_idea(&n.line, *t, &best.line, target))
        .or(if others.is_empty() { None } else { Some(0) });
    let alternate = pick.map(|i| {
        let (n, m, _) = &others[i];
        AlternatePlan { actions: n.line.clone(), expected_score: searcher.sign * n.value, gain: n.value - searcher.sign * baseline, metrics: m.clone() }
    });

    Some(PlannedTurn {
        actor,
        actions: best.line,
        expected_score: searcher.sign * best.value,
        baseline_score: baseline,
        gain: best.value - searcher.sign * baseline,
        metrics,
        alternate,
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
