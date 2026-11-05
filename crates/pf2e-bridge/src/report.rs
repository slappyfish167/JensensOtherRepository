//! Analysis results addressed to Foundry: token ids instead of engine ids,
//! scene squares and canvas pixels instead of crop coordinates, and short
//! human-readable reasoning. Built only from `pf2e-ai` / `pf2e-core` output.

use crate::convert::{Converted, ScenePoint};
use pf2e_ai::evaluation::{Evaluation, HeuristicEvaluator};
use pf2e_ai::explain::{explain_change, PositionExplanation};
use pf2e_ai::queries::{describe_action, rank_targets, side_strategy, TargetAssessment};
use pf2e_ai::search::{best_turn_with_budget, roll_outcomes, Budget, PlannedTurn, SearchConfig};
use pf2e_ai::simulate::SimulationReport;
use pf2e_core::combat::{Action, CombatState, Command, PendingInput};
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};

pub fn band(score: f64) -> &'static str {
    match score {
        s if s >= 7.0 => "decisive party advantage",
        s if s >= 3.5 => "clear party advantage",
        s if s >= 1.0 => "slight party edge",
        s if s > -1.0 => "balanced",
        s if s > -3.5 => "slight monster edge",
        s if s > -7.0 => "clear monster advantage",
        _ => "decisive monster advantage",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactorReport {
    pub name: String,
    pub value: f64,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationReport {
    /// −10 … +10, positive favours the party.
    pub score: f64,
    pub band: String,
    pub win_probability: f64,
    pub terminal: bool,
    pub factors: Vec<FactorReport>,
    pub party_hp: f64,
    pub enemy_hp: f64,
    pub party_damage_per_round: f64,
    pub enemy_damage_per_round: f64,
    /// Token whose turn it is (if a combat is running).
    pub current_token: Option<String>,
}

pub fn evaluation(conv: &Converted, ev: &Evaluation) -> EvaluationReport {
    EvaluationReport {
        score: ev.score,
        band: band(ev.score).into(),
        win_probability: ev.win_probability,
        terminal: ev.terminal,
        factors: ev.factors.iter().map(|f| FactorReport { name: f.name.clone(), value: f.value, detail: f.detail.clone() }).collect(),
        party_hp: ev.party.hit_points,
        enemy_hp: ev.enemies.hit_points,
        party_damage_per_round: ev.party.damage_per_round,
        enemy_damage_per_round: ev.enemies.damage_per_round,
        current_token: if conv.turn_active { conv.state.current_actor().map(|id| conv.token(id)) } else { None },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Strike,
    Stride,
    Step,
    Stand,
    RaiseShield,
    TakeCover,
    Demoralize,
    Trip,
    CastSpell,
    DisableHazard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    pub kind: StepKind,
    pub label: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub target_tokens: Vec<String>,
    /// Destination (moves) or aim point (area spells).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub to: Option<ScenePoint>,
    /// Squares crossed by a Stride, for drawing the path.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub path: Vec<ScenePoint>,
    /// The engine action, for hosts that replay the plan.
    pub action: Action,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnReport {
    pub token_id: String,
    pub name: String,
    pub steps: Vec<StepReport>,
    /// Party-positive evaluation now and expected after the turn.
    pub baseline_score: f64,
    pub expected_score: f64,
    /// Improvement from the acting creature's side (positive is good for it).
    pub gain: f64,
    pub reasoning: Vec<String>,
    pub depth: u8,
    pub nodes: usize,
    pub truncated: bool,
    /// True when the turn was analysed as if it started now (not its turn).
    pub hypothetical: bool,
    pub summary: String,
}

fn step_report(conv: &Converted, s: &CombatState, actor: CombatantId, a: &Action) -> StepReport {
    let tok = |id: &CombatantId| conv.token(*id);
    let (kind, targets, to, path) = match a {
        Action::Strike { target, .. } => (StepKind::Strike, vec![tok(target)], None, vec![]),
        Action::Stride { to } => {
            let path = s.movement(actor).path_to(*to).unwrap_or_default().into_iter().map(|p| conv.to_scene(p)).collect();
            (StepKind::Stride, vec![], Some(conv.to_scene(*to)), path)
        }
        Action::Step { to } => (StepKind::Step, vec![], Some(conv.to_scene(*to)), vec![]),
        Action::Stand => (StepKind::Stand, vec![], None, vec![]),
        Action::RaiseShield => (StepKind::RaiseShield, vec![], None, vec![]),
        Action::TakeCover => (StepKind::TakeCover, vec![], None, vec![]),
        Action::Demoralize { target } => (StepKind::Demoralize, vec![tok(target)], None, vec![]),
        Action::Trip { target } => (StepKind::Trip, vec![tok(target)], None, vec![]),
        Action::CastSpell { targets, aim, .. } => (StepKind::CastSpell, targets.iter().map(tok).collect(), aim.map(|p| conv.to_scene(p)), vec![]),
        Action::DisableHazard { hazard } => (StepKind::DisableHazard, vec![tok(hazard)], None, vec![]),
    };
    // Engine labels use crop coordinates; describe moves in scene squares.
    let label = match (a, &to) {
        (Action::Stride { .. }, Some(p)) => format!("Stride to square ({}, {})", p.square[0], p.square[1]),
        (Action::Step { .. }, Some(p)) => format!("Step to square ({}, {})", p.square[0], p.square[1]),
        _ => describe_action(s, actor, a),
    };
    StepReport { kind, label, target_tokens: targets, to, path, action: a.clone() }
}

/// Plays `actions` on a copy with each roll resolved to its most likely
/// outcome and reactions taken: the "expected line" used for reasoning.
/// Returns the states before each action plus the final state.
fn expected_line(s: &CombatState, actor: CombatantId, actions: &[Action]) -> Vec<CombatState> {
    let mut states = vec![s.clone()];
    let mut cur = s.clone();
    for a in actions {
        if cur.apply(&Command::Act { actor, action: a.clone() }).is_err() {
            break;
        }
        let mut guard = 0;
        while let Some(p) = cur.pending() {
            guard += 1;
            if guard > 64 {
                break;
            }
            let cmd = match p {
                PendingInput::Roll(r) => {
                    let best = roll_outcomes(&r).into_iter().max_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal)).map(|x| x.0).unwrap_or(10);
                    Command::ProvideRoll { request_id: r.request_id, value: best, extra_modifier: 0 }
                }
                PendingInput::Decision(d) => Command::Decide { request_id: d.request_id, accept: true },
            };
            if cur.apply(&cmd).is_err() {
                break;
            }
        }
        states.push(cur.clone());
    }
    states
}

fn reasoning(conv: &Converted, s: &CombatState, plan: &PlannedTurn, ev: &HeuristicEvaluator) -> Vec<String> {
    let mut out = Vec::new();
    let actor = plan.actor;
    // Why these targets: the target ranking for each struck creature.
    let ranks: Vec<TargetAssessment> = rank_targets(s, actor);
    let mut mentioned = Vec::new();
    for a in &plan.actions {
        if let Action::Strike { target, .. } | Action::Trip { target } | Action::Demoralize { target } = a {
            if mentioned.contains(target) {
                continue;
            }
            mentioned.push(*target);
            if let Some(r) = ranks.iter().find(|r| r.target == *target) {
                let best = ranks.first().map(|b| b.target == *target).unwrap_or(false);
                out.push(format!("{}{}: {}", r.name, if best { " (top priority target)" } else { "" }, r.rationale));
            }
        }
    }
    // What the expected line changes, measured by the evaluator.
    let line = expected_line(s, actor, &plan.actions);
    if let (Some(first), Some(last)) = (line.first(), line.last()) {
        let ex = explain_change(first, last, ev, 4);
        for r in ex.reasons.iter().filter(|r| r.subject != Some(actor) || r.kind != pf2e_ai::explain::ReasonKind::ActionEconomy) {
            out.push(format!("{} ({:+.1})", r.text, r.impact));
        }
    }
    if plan.truncated {
        out.push(format!("Search stopped early at depth {} ({} positions); the plan is the best fully analysed one.", plan.depth, plan.nodes));
    }
    let _ = conv;
    out.truncate(6);
    out
}

/// Best turn for `actor`. If it is not that creature's turn, analyses a
/// hypothetical turn starting now.
pub fn best_turn(conv: &Converted, actor: CombatantId, cfg: SearchConfig, budget: &dyn Budget) -> Option<TurnReport> {
    let ev = HeuristicEvaluator::default();
    let (state, hypothetical) = if conv.turn_active && conv.state.current_actor() == Some(actor) {
        (conv.state.clone(), false)
    } else {
        (conv.state.hypothetical_turn(actor)?, true)
    };
    let plan = best_turn_with_budget(&state, &ev, cfg, budget)?;
    let line = expected_line(&state, actor, &plan.actions);
    let steps: Vec<StepReport> = plan.actions.iter().enumerate().map(|(i, a)| step_report(conv, line.get(i).unwrap_or(&state), actor, a)).collect();
    let name = state.combatant(actor).map(|c| c.name.clone()).unwrap_or_default();
    let summary = if steps.is_empty() {
        format!("{name}: no action improves the position — end the turn")
    } else {
        format!("{name}: {} — expected {:+.1} for its side", steps.iter().map(|s| s.label.as_str()).collect::<Vec<_>>().join(", then "), plan.gain)
    };
    Some(TurnReport {
        token_id: conv.token(actor),
        name,
        reasoning: reasoning(conv, &state, &plan, &ev),
        steps,
        baseline_score: plan.baseline_score,
        expected_score: plan.expected_score,
        gain: plan.gain,
        depth: plan.depth,
        nodes: plan.nodes,
        truncated: plan.truncated,
        hypothetical,
        summary,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetReport {
    pub token_id: String,
    pub name: String,
    pub expected_damage: f64,
    pub kill_probability: f64,
    pub threat: f64,
    pub priority: f64,
    pub rationale: String,
}

fn target_report(conv: &Converted, t: &TargetAssessment) -> TargetReport {
    TargetReport {
        token_id: conv.token(t.target),
        name: t.name.clone(),
        expected_damage: t.expected_damage,
        kill_probability: t.kill_probability,
        threat: t.threat,
        priority: t.priority,
        rationale: t.rationale.clone(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreaturePlanReport {
    pub token_id: String,
    pub name: String,
    pub turn: Option<TurnReport>,
    pub targets: Vec<TargetReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidePlanReport {
    pub side: Side,
    pub focus_token: Option<String>,
    pub notes: Vec<String>,
    pub creatures: Vec<CreaturePlanReport>,
}

/// Creates one search budget per creature (e.g. a fresh deadline each).
pub type BudgetFactory<'a> = &'a dyn Fn() -> Box<dyn Budget>;

/// GM assistant: every creature of `side`, its best turn and targets.
pub fn side_plan(conv: &Converted, side: Side, cfg: SearchConfig, budgets: BudgetFactory) -> SidePlanReport {
    // Target votes and notes come from the existing strategy query; turns are
    // searched here so they carry scene coordinates and reasoning.
    let light = SearchConfig { node_budget: 1, ..cfg };
    let strat = side_strategy(&conv.state, side, light);
    let creatures = strat
        .creatures
        .iter()
        .map(|c| CreaturePlanReport {
            token_id: conv.token(c.combatant),
            name: c.name.clone(),
            turn: best_turn(conv, c.combatant, cfg, budgets().as_ref()),
            targets: c.preferred_targets.iter().map(|t| target_report(conv, t)).collect(),
        })
        .collect();
    SidePlanReport { side, focus_token: strat.focus_target.map(|id| conv.token(id)), notes: strat.notes, creatures }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatantOddsReport {
    pub token_id: String,
    pub name: String,
    pub side: Side,
    pub death_rate: f64,
    pub down_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastReport {
    pub runs: u32,
    pub victory: f64,
    pub defeat: f64,
    pub unresolved: f64,
    pub tpk: f64,
    pub any_pc_death: f64,
    pub average_rounds: f64,
    pub combatants: Vec<CombatantOddsReport>,
    /// 95% half-width of the victory estimate (normal approximation).
    pub margin_of_error: f64,
}

pub fn forecast(conv: &Converted, r: &SimulationReport) -> ForecastReport {
    let n = r.runs.max(1) as f64;
    let p = r.party_win_rate;
    ForecastReport {
        runs: r.runs,
        victory: r.party_win_rate,
        defeat: r.enemy_win_rate,
        unresolved: r.timeout_rate,
        tpk: r.tpk_probability,
        any_pc_death: r.any_pc_death_probability,
        average_rounds: r.average_rounds,
        combatants: r
            .combatants
            .iter()
            .map(|c| CombatantOddsReport { token_id: conv.token(c.id), name: c.name.clone(), side: c.side, death_rate: c.death_rate, down_rate: c.down_rate })
            .collect(),
        margin_of_error: 1.96 * (p * (1.0 - p) / n).sqrt(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonReport {
    pub kind: pf2e_ai::explain::ReasonKind,
    pub token_id: Option<String>,
    pub text: String,
    pub impact: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainReport {
    pub score: f64,
    pub previous_score: Option<f64>,
    pub delta: f64,
    pub reasons: Vec<ReasonReport>,
    pub factors: Vec<pf2e_ai::explain::FactorDelta>,
}

pub fn explanation(conv: &Converted, ex: PositionExplanation) -> ExplainReport {
    ExplainReport {
        score: ex.score,
        previous_score: ex.previous_score,
        delta: ex.delta,
        reasons: ex
            .reasons
            .into_iter()
            .map(|r| ReasonReport { kind: r.kind, token_id: r.subject.map(|id| conv.token(id)), text: r.text, impact: r.impact })
            .collect(),
        factors: ex.factors,
    }
}
