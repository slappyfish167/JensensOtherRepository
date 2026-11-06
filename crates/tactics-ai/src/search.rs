//! Expectiminimax search over one creature's turn, for any game system.
//!
//! * **Decision nodes** (the acting creature's choice): max over the
//!   system's candidate actions. The search is **committed** by default:
//!   ending the turn is *not* a candidate while any legal action remains
//!   (unless the system says the rest of the turn is optional, e.g. an
//!   unused Move Action), so the result is always the best legal combat
//!   turn, never "do nothing". `allow_end_turn` restores advisory mode.
//! * **Chance nodes** (a pending die roll): the system answers with weighted
//!   representative rolls; the *real engine* resolves every branch, so the
//!   search can never disagree with the rules.
//! * **Reaction / decision nodes**: the deciding creature's side picks what
//!   is best for it (max for the actor's allies, min for its enemies).
//!
//! Speed-ups (exact unless noted):
//! * **Transposition table** — decision nodes are memoised by the system's
//!   [`TacticalSystem::state_key`] and remaining depth, so move orders that
//!   reach the same position (and chance branches that resolve identically)
//!   are searched once.
//! * **Evaluation cache** — the system's [`TacticalSystem::Cache`] reuses
//!   per-pair expected damage across leaves.
//! * **Iterative deepening** — depth 1, 2, … actions. Each completed
//!   iteration is a valid answer, so a [`Budget`] (e.g. a wall-clock deadline
//!   supplied by the host) can stop the search and the deepest *completed*
//!   result is returned. A history table of action gains from shallower
//!   iterations orders candidates before the beam cut (heuristic: it affects
//!   which actions survive the beam, not the value of those that do).
//!
//! Alongside the score every node carries the turn's outcome metrics
//! (damage dealt and taken, who drops, which statuses land), aggregated with
//! the same probabilities and choices as the score, so the reported
//! expectations describe exactly the adaptive policy the search found.

use crate::evaluation::Evaluator;
use crate::system::{CandidateConfig, Node as Input, TacticalSystem};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use tactics_core::{CombatantId, Side};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Soft cap on nodes visited (see the budget windows below).
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
        SearchConfig { node_budget: 40_000, beam: 8, max_moves: 4, focus: None, allow_end_turn: false, iterative: true, transpositions: true }
    }
}

/// External stop signal, polled every few nodes. The search stays pure:
/// hosts that want a wall-clock limit implement this with their own clock.
pub trait Budget {
    fn exhausted(&self, nodes: usize) -> bool;
}

/// Never stops the search (the node budget still applies).
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

/// How often the external budget is polled (power of two: masked, not divided).
const BUDGET_POLL: usize = 32;

/// Bound on memoised decision nodes (bounded memory in WebAssembly hosts).
const MAX_TT_ENTRIES: usize = 200_000;

fn combine(parts: &[u64]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Expected outcome against one hostile creature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetOutcome {
    pub target: CombatantId,
    pub name: String,
    pub expected_damage: f64,
    /// Probability the creature is down at the end of the turn.
    pub down_chance: f64,
}

/// A condition or status the turn may inflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusExpectation {
    pub target: CombatantId,
    pub name: String,
    /// e.g. `frightened 1`, `prone`, `critical injury (head)`.
    pub effect: String,
    pub probability: f64,
}

/// Expectations over the chance tree of the planned turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnMetrics {
    /// HP removed from hostile creatures.
    pub expected_damage: f64,
    /// HP the actor loses (reactions, hazards).
    pub expected_damage_taken: f64,
    /// Hostile creatures, most damaged first.
    pub targets: Vec<TargetOutcome>,
    /// Most likely first.
    pub status_effects: Vec<StatusExpectation>,
}

impl TurnMetrics {
    /// The creature a plan is aimed at: the one most of its actions target
    /// (ties: most expected damage and chance to drop it), otherwise the one
    /// taking the most expected damage.
    pub fn primary_target(&self, action_targets: impl IntoIterator<Item = CombatantId>) -> Option<CombatantId> {
        let weight = |id: CombatantId| self.target(id).map(|t| t.expected_damage + 100.0 * t.down_chance).unwrap_or(0.0);
        let mut counts: Vec<(CombatantId, usize)> = Vec::new();
        for t in action_targets {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternatePlan<A> {
    pub actions: Vec<A>,
    pub expected_score: f64,
    pub gain: f64,
    pub metrics: TurnMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTurn<A> {
    pub actor: CombatantId,
    pub actions: Vec<A>,
    /// Expected evaluation after the turn (party-positive scale).
    pub expected_score: f64,
    /// Evaluation now (party-positive scale).
    pub baseline_score: f64,
    /// Change in evaluation from the actor's point of view (may be negative:
    /// a committed turn is the best available, not necessarily a gain).
    pub gain: f64,
    pub metrics: TurnMetrics,
    /// The best root choice that differs meaningfully from `actions`.
    pub alternate: Option<AlternatePlan<A>>,
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

impl<A> PlannedTurn<A> {
    pub fn primary_target<S: TacticalSystem<Action = A>>(&self) -> Option<CombatantId> {
        self.metrics.primary_target(self.actions.iter().filter_map(S::action_target))
    }
}

/// What a hostile creature looked like before the turn.
struct FoeSnap {
    id: CombatantId,
    name: String,
    hp: i32,
    down: bool,
    labels: Vec<String>,
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

struct Node<A> {
    value: f64,
    line: Vec<A>,
    out: Outcome,
}

impl<A: Clone> Clone for Node<A> {
    fn clone(&self) -> Self {
        Node { value: self.value, line: self.line.clone(), out: self.out.clone() }
    }
}

pub struct Searcher<'a, S: TacticalSystem> {
    eval: &'a dyn Evaluator<S>,
    budget: &'a dyn Budget,
    cfg: SearchConfig,
    actor: CombatantId,
    side: Side,
    sign: f64,
    actor_hp: i32,
    foes: Vec<FoeSnap>,
    root_nodes: Vec<Node<S::Action>>,
    /// Budget window of the root candidate being searched: each opening may
    /// use what is left but must leave a reserve for the openings after it,
    /// so late candidates are never judged on a fully collapsed
    /// (optimistic, most-likely-outcome) tree or skipped outright.
    window_start: usize,
    window: usize,
    /// Node count when the current iteration started.
    iteration_start: usize,
    pub nodes: usize,
    pub truncated: bool,
    /// The external budget ran out; the current iteration is incomplete.
    pub aborted: bool,
    pub cache: S::Cache,
    tt: HashMap<u64, Node<S::Action>>,
    pub tt_hits: u64,
    /// Best gain over stopping seen for each action (by its debug form).
    history: HashMap<String, f64>,
}

impl<'a, S: TacticalSystem> Searcher<'a, S> {
    pub fn new(eval: &'a dyn Evaluator<S>, cfg: SearchConfig, state: &S::State, actor: CombatantId) -> Self {
        Self::with_budget(eval, cfg, state, actor, &Unlimited)
    }

    pub fn with_budget(eval: &'a dyn Evaluator<S>, cfg: SearchConfig, state: &S::State, actor: CombatantId, budget: &'a dyn Budget) -> Self {
        let units = S::units(state);
        let side = units.iter().find(|u| u.id == actor).map(|u| u.side).unwrap_or(Side::Party);
        let foes = units
            .iter()
            .filter(|u| side.is_hostile_to(u.side) && !u.ignored)
            .map(|u| FoeSnap { id: u.id, name: u.name.clone(), hp: u.hp, down: u.down, labels: S::status_labels(state, u.id) })
            .collect();
        Searcher {
            eval,
            budget,
            cfg,
            actor,
            side,
            sign: if side == Side::Enemy { -1.0 } else { 1.0 },
            actor_hp: S::hp(state, actor),
            foes,
            root_nodes: Vec::new(),
            window_start: 0,
            window: cfg.node_budget,
            iteration_start: 0,
            nodes: 0,
            truncated: false,
            aborted: false,
            cache: S::Cache::default(),
            tt: HashMap::new(),
            tt_hits: 0,
            history: HashMap::new(),
        }
    }

    /// +1 for a party actor, −1 for an enemy actor (scores are party-positive).
    pub fn sign(&self) -> f64 {
        self.sign
    }

    fn leaf(&mut self, s: &S::State) -> f64 {
        self.sign * self.eval.evaluate_cached(s, &mut self.cache)
    }

    fn outcome(&self, s: &S::State) -> Outcome {
        let n = self.foes.len();
        let mut o = Outcome { dealt: vec![0.0; n], downs: vec![0.0; n], ..Default::default() };
        o.taken = (self.actor_hp - S::hp(s, self.actor)).max(0) as f64;
        for (i, f) in self.foes.iter().enumerate() {
            o.dealt[i] = (f.hp - S::hp(s, f.id)).max(0) as f64;
            if !f.down && S::is_down(s, f.id) {
                o.downs[i] = 1.0;
            }
            for label in S::status_labels(s, f.id) {
                if !f.labels.contains(&label) {
                    o.status.insert((i, label), 1.0);
                }
            }
        }
        o
    }

    fn leaf_node(&mut self, s: &S::State) -> Node<S::Action> {
        Node { value: self.leaf(s), line: vec![], out: self.outcome(s) }
    }

    fn used(&self) -> usize {
        self.nodes - self.window_start
    }

    fn collapse(&self) -> bool {
        self.used() > self.window / 2
    }

    /// Resolves pending input (chance/decision nodes) and then continues
    /// with `then` once the engine waits for the actor again.
    fn resolve<F>(&mut self, s: S::State, then: &mut F) -> Node<S::Action>
    where
        F: FnMut(&mut Self, S::State) -> Node<S::Action>,
    {
        self.nodes += 1;
        if S::is_over(&s) {
            return self.leaf_node(&s);
        }
        match S::node(&s) {
            Input::Ready => then(self, s),
            Input::Stuck => self.leaf_node(&s),
            Input::Chance(mut outs) => {
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
                for (cmd, p) in outs {
                    let mut child = s.clone();
                    if S::apply_in_place(&mut child, &cmd).is_err() {
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
            Input::Decision { decider, accept, decline } => {
                let ours = S::units(&s).iter().find(|u| u.id == decider).map(|u| u.side == self.side).unwrap_or(false);
                let mut best: Option<Node<S::Action>> = None;
                for cmd in [accept, decline] {
                    let mut child = s.clone();
                    if S::apply_in_place(&mut child, &cmd).is_err() {
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

    fn candidates(&mut self, s: &S::State) -> Vec<S::Action> {
        let committed = !self.cfg.allow_end_turn;
        let mut c = S::candidates(s, &CandidateConfig { max_moves: self.cfg.max_moves, focus: self.cfg.focus, committed }, &mut self.cache);
        if c.is_empty() && self.cfg.focus.is_some() && committed {
            // The focus target is unreachable or gone: still play the turn.
            c = S::candidates(s, &CandidateConfig { max_moves: self.cfg.max_moves, focus: None, committed: true }, &mut self.cache);
        }
        if !self.history.is_empty() {
            // Stable: equal history keeps the system's own order.
            let h = |a: &S::Action| self.history.get(&format!("{a:?}")).copied().unwrap_or(f64::NEG_INFINITY);
            c.sort_by(|x, y| h(y).partial_cmp(&h(x)).unwrap_or(std::cmp::Ordering::Equal));
        }
        c
    }

    fn act(&self, s: &S::State, action: &S::Action) -> Option<S::State> {
        let mut child = s.clone();
        S::apply_in_place(&mut child, &S::act(self.actor, action.clone())).ok()?;
        Some(child)
    }

    /// Value of the best continuation of the actor's turn from `s`, using at
    /// most `depth` more actions.
    fn decide(&mut self, s: S::State, root: bool, depth: u8) -> Node<S::Action> {
        if depth == 0 || self.aborted || S::current_actor(&s) != Some(self.actor) || !S::turn_open(&s, self.actor) {
            return self.leaf_node(&s);
        }
        if self.nodes & (BUDGET_POLL - 1) == 0 && self.budget.exhausted(self.nodes) {
            self.aborted = true;
            return self.leaf_node(&s);
        }
        let key = if self.cfg.transpositions { S::state_key(&s).map(|k| combine(&[k, depth as u64])) } else { None };
        if let (false, Some(k)) = (root, key) {
            if let Some(hit) = self.tt.get(&k) {
                self.tt_hits += 1;
                return hit.clone();
            }
        }
        let may_stop = self.cfg.allow_end_turn || S::may_stop(&s, self.actor);
        let over = self.used() >= self.window;
        if over && may_stop {
            self.truncated = true;
            return self.leaf_node(&s);
        }
        let mut cands = self.candidates(&s);
        if !root && cands.len() > self.cfg.beam {
            cands.truncate(self.cfg.beam);
        }
        if over {
            self.truncated = true;
            return self.greedy(s, cands, depth);
        }
        // Committed play: ending the turn is only the answer when no
        // candidate can legally be taken (or the system says what is left
        // of the turn is optional).
        let stop = self.leaf_node(&s);
        let stop_value = stop.value;
        let mut best: Option<Node<S::Action>> = if may_stop { Some(stop) } else { None };
        let n_cands = cands.len();
        for (i, act) in cands.into_iter().enumerate() {
            let Some(child) = self.act(&s, &act) else { continue };
            if root {
                // Use what is left of this iteration's budget, minus a
                // guaranteed reserve for every opening still to be searched.
                let reserve = self.cfg.node_budget / (4 * n_cands);
                let left = self.cfg.node_budget.saturating_sub(self.nodes - self.iteration_start);
                self.window_start = self.nodes;
                self.window = left.saturating_sub(reserve * (n_cands - i - 1)).max(reserve).max(1);
            }
            let mut n = self.resolve(child, &mut |me: &mut Self, st| me.decide(st, false, depth - 1));
            if self.aborted {
                return best.unwrap_or(n);
            }
            let e = self.history.entry(format!("{act:?}")).or_insert(f64::NEG_INFINITY);
            *e = e.max(n.value - stop_value);
            n.line.insert(0, act);
            if root {
                self.root_nodes.push(n.clone());
            }
            if best.as_ref().map(|b| n.value > b.value + 1e-9).unwrap_or(true) {
                best = Some(n);
            }
        }
        let best = best.unwrap_or_else(|| self.leaf_node(&s));
        if let (Some(k), false) = (key, self.aborted) {
            if self.tt.len() >= MAX_TT_ENTRIES {
                self.tt.clear();
            }
            self.tt.insert(k, best.clone());
        }
        best
    }

    /// Out of budget: pick each remaining action by one-ply lookahead so the
    /// turn is still completed rather than abandoned.
    fn greedy(&mut self, s: S::State, cands: Vec<S::Action>, depth: u8) -> Node<S::Action> {
        let mut best: Option<(f64, S::Action, S::State)> = None;
        for act in cands {
            let Some(child) = self.act(&s, &act) else { continue };
            let v = self.resolve(child.clone(), &mut |me: &mut Self, st| me.leaf_node(&st)).value;
            if best.as_ref().map(|b| v > b.0 + 1e-9).unwrap_or(true) {
                best = Some((v, act, child));
            }
        }
        match best {
            Some((_, act, child)) => {
                let mut n = self.resolve(child, &mut |me: &mut Self, st| me.decide(st, false, depth.saturating_sub(1)));
                n.line.insert(0, act);
                n
            }
            None => self.leaf_node(&s),
        }
    }

    /// Expected leaf value (actor's perspective) right after `action` resolves.
    pub fn one_ply(&mut self, s: &S::State, action: &S::Action) -> Option<f64> {
        let child = self.act(s, action)?;
        Some(self.resolve(child, &mut |me: &mut Self, st| me.leaf_node(&st)).value)
    }

    fn metrics(&self, o: &Outcome) -> TurnMetrics {
        let mut targets: Vec<TargetOutcome> = self
            .foes
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.down)
            .map(|(i, f)| TargetOutcome {
                target: f.id,
                name: f.name.clone(),
                expected_damage: o.dealt.get(i).copied().unwrap_or(0.0),
                down_chance: o.downs.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0),
            })
            .collect();
        targets.sort_by(|a, b| b.expected_damage.partial_cmp(&a.expected_damage).unwrap_or(std::cmp::Ordering::Equal));
        let mut status_effects: Vec<StatusExpectation> = o
            .status
            .iter()
            .filter(|(_, p)| **p >= 0.005)
            .map(|((i, effect), p)| StatusExpectation {
                target: self.foes[*i].id,
                name: self.foes[*i].name.clone(),
                effect: effect.clone(),
                probability: p.clamp(0.0, 1.0),
            })
            .collect();
        status_effects.sort_by(|a, b| b.probability.partial_cmp(&a.probability).unwrap_or(std::cmp::Ordering::Equal));
        TurnMetrics { expected_damage: o.dealt.iter().sum(), expected_damage_taken: o.taken, targets, status_effects }
    }
}

/// Two root choices are near-duplicates when they open with the same kind
/// of action against the same creature (e.g. the same Strike with another
/// weapon, or a move to a neighbouring square toward the same foe).
fn same_idea<S: TacticalSystem>(a: &[S::Action], ta: Option<CombatantId>, b: &[S::Action], tb: Option<CombatantId>) -> bool {
    let kind = |l: &[S::Action]| l.first().map(S::action_kind);
    kind(a) == kind(b) && ta == tb
}

/// One completed search iteration: best line, root alternatives, depth.
type Iteration<A> = (Node<A>, Vec<Node<A>>, u8);

/// The best sequence of actions for whoever's turn it is.
///
/// Returns `None` only when nobody can act (combat not running, or input is
/// pending). With the default (committed) configuration `actions` is empty
/// only when the creature literally has no legal action.
pub fn best_turn<S: TacticalSystem>(state: &S::State, eval: &dyn Evaluator<S>, cfg: SearchConfig) -> Option<PlannedTurn<S::Action>> {
    best_turn_with_budget::<S>(state, eval, cfg, &Unlimited)
}

/// [`best_turn`] that stops when `budget` is exhausted, returning the
/// deepest completed iteration. Even a budget exhausted at once still
/// yields at least one legal action (chosen by one-ply lookahead).
pub fn best_turn_with_budget<S: TacticalSystem>(state: &S::State, eval: &dyn Evaluator<S>, cfg: SearchConfig, budget: &dyn Budget) -> Option<PlannedTurn<S::Action>> {
    let actor = S::current_actor(state)?;
    if S::pending(state).is_some() {
        return None;
    }
    let mut searcher = Searcher::with_budget(eval, cfg, state, actor, budget);
    let baseline = eval.evaluate(state);
    let max_depth = S::turn_depth(state, actor).max(1);
    let depths: Vec<u8> = if cfg.iterative { (1..=max_depth).collect() } else { vec![max_depth] };
    // (best line, root alternatives, depth) of the deepest completed iteration.
    let mut result: Option<Iteration<S::Action>> = None;
    for depth in depths {
        searcher.root_nodes.clear();
        searcher.iteration_start = searcher.nodes;
        searcher.window_start = searcher.nodes;
        searcher.window = cfg.node_budget;
        let node = searcher.decide(state.clone(), true, depth);
        let roots = std::mem::take(&mut searcher.root_nodes);
        if searcher.aborted {
            // Keep a partial first iteration only if nothing better exists.
            if result.is_none() {
                result = Some((node, roots, 0));
            }
            break;
        }
        result = Some((node, roots, depth));
    }
    let (mut best, mut roots, depth) = result?;
    if best.line.is_empty() && !S::legal_actions(state).is_empty() && !cfg.allow_end_turn {
        // The budget ran out before any action was searched: never answer
        // "do nothing" while a legal action exists.
        let cands = searcher.candidates(state);
        best = searcher.greedy(state.clone(), cands, 1);
        roots.clear();
    }
    let metrics = searcher.metrics(&best.out);
    let targets_of = |line: &[S::Action]| line.iter().filter_map(S::action_target).collect::<Vec<_>>();
    let target = metrics.primary_target(targets_of(&best.line));

    roots.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
    let others: Vec<(Node<S::Action>, TurnMetrics, Option<CombatantId>)> = roots
        .into_iter()
        .filter(|n| n.line.first() != best.line.first())
        .map(|n| {
            let m = searcher.metrics(&n.out);
            let t = m.primary_target(targets_of(&n.line));
            (n, m, t)
        })
        .collect();
    let pick = others
        .iter()
        .position(|(n, _, t)| !same_idea::<S>(&n.line, *t, &best.line, target))
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
        truncated: searcher.truncated || searcher.aborted,
        depth,
        transposition_hits: searcher.tt_hits,
        eval_cache_hits: S::cache_hits(&searcher.cache),
    })
}

/// Expectation of an arbitrary metric over the chance outcomes of `action`,
/// assuming every offered decision is accepted.
pub fn expectation_after<S: TacticalSystem, F: Fn(&S::State) -> f64>(state: &S::State, actor: CombatantId, action: &S::Action, metric: &F) -> Option<f64> {
    let mut child = state.clone();
    S::apply_in_place(&mut child, &S::act(actor, action.clone())).ok()?;
    Some(walk::<S, F>(child, metric, 0))
}

fn walk<S: TacticalSystem, F: Fn(&S::State) -> f64>(s: S::State, metric: &F, depth: usize) -> f64 {
    if depth > 64 {
        return metric(&s);
    }
    match S::node(&s) {
        Input::Ready | Input::Stuck => metric(&s),
        Input::Chance(outs) => {
            let mut total = 0.0;
            for (cmd, p) in outs {
                let mut child = s.clone();
                if S::apply_in_place(&mut child, &cmd).is_ok() {
                    total += p * walk::<S, F>(child, metric, depth + 1);
                }
            }
            total
        }
        Input::Decision { accept, .. } => {
            let mut child = s.clone();
            match S::apply_in_place(&mut child, &accept) {
                Ok(_) => walk::<S, F>(child, metric, depth + 1),
                Err(_) => metric(&s),
            }
        }
    }
}
