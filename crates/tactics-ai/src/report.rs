//! High-level tactical questions for any game system, each answered with
//! data plus a short human-readable rationale.

use crate::evaluation::{Evaluator, HeuristicEvaluator};
use crate::search::{best_turn, PlannedTurn, SearchConfig, StatusExpectation};
use crate::system::{TacticalSystem, Unit};
use serde::{Deserialize, Serialize};
use tactics_core::{CombatantId, Side};

/// The full tactical read-out for one creature's turn: what to do, what
/// to do instead, and what it is expected to achieve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TacticalReport {
    pub actor_name: String,
    /// Primary action plan, one line per action.
    pub primary: Vec<String>,
    /// Alternate action plan (empty only when no other legal opening exists).
    pub alternate: Vec<String>,
    pub alternate_note: String,
    /// Expected HP removed from hostile creatures this turn.
    pub expected_damage: f64,
    /// Expected HP the actor loses this turn (reactions etc.).
    pub expected_damage_taken: f64,
    pub expected_status_effects: Vec<StatusExpectation>,
    pub priority_target: Option<CombatantId>,
    pub priority_target_name: Option<String>,
    /// Chance the priority target is down at the end of the turn.
    pub down_chance: f64,
    /// Expected evaluation after the turn from the actor's side (positive = actor's side ahead).
    pub position_score: f64,
    /// e.g. `+0.3 Monster Advantage`.
    pub position: String,
    pub reason: String,
    /// Everything above as a printable block.
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecommendation<A> {
    pub plan: PlannedTurn<A>,
    pub steps: Vec<String>,
    pub summary: String,
    pub report: TacticalReport,
}

fn side_label(side: Side) -> &'static str {
    match side {
        Side::Enemy => "Monster",
        Side::Party => "Party",
        Side::Neutral => "Neutral",
    }
}

/// Signed position from the actor's side, never phrased as "equal".
pub fn position_text(side: Side, v: f64) -> String {
    if v >= 0.0 {
        format!("{:+.1} {} Advantage", v, side_label(side))
    } else {
        format!("{:+.1} {} Advantage", -v, side_label(side.opposing()))
    }
}

fn describe_plan<S: TacticalSystem>(s: &S::State, actor: CombatantId, actions: &[S::Action]) -> Vec<String> {
    actions.iter().map(|a| S::describe_action(s, actor, a)).collect()
}

fn why_cannot_act<S: TacticalSystem>(s: &S::State, actor: CombatantId) -> String {
    match S::units(s).into_iter().find(|u| u.id == actor) {
        None => "is not in the combat".into(),
        Some(u) if u.dead => "is dead".into(),
        Some(u) if u.down => "is down".into(),
        Some(_) if !S::turn_open(s, actor) => "has no actions remaining this turn".into(),
        Some(_) => "has no legal action".into(),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn reason_text<S: TacticalSystem>(s: &S::State, actor: CombatantId, plan: &PlannedTurn<S::Action>, target: Option<CombatantId>, ranks: &[TargetAssessment]) -> String {
    let units = S::units(s);
    let unit = |id: CombatantId| units.iter().find(|u| u.id == id);
    let mut parts: Vec<String> = Vec::new();
    match target.and_then(unit) {
        Some(t) => {
            let standing: Vec<&Unit> = ranks.iter().filter_map(|r| unit(r.target)).collect();
            let lowest = standing.iter().map(|c| c.hp).min();
            let me = ranks.iter().find(|r| r.target == t.id);
            let top_threat = ranks.iter().map(|r| r.threat).fold(f64::MIN, f64::max);
            if lowest == Some(t.hp) && standing.len() > 1 {
                parts.push(format!("{} has the lowest effective HP ({})", t.name, t.hp));
            }
            if let Some(m) = me {
                if ranks.len() > 1 && m.threat >= top_threat - 1e-9 && m.threat > 0.0 {
                    parts.push(format!("{} is the biggest threat on the field ({:.1} damage/round)", t.name, m.threat));
                }
            }
            if parts.is_empty() {
                if ranks.first().map(|r| r.target == t.id).unwrap_or(false) {
                    parts.push(format!("{} offers the most threat removed per point of damage", t.name));
                } else {
                    parts.push(format!("{} is the best target this turn can reach", t.name));
                }
            }
            let down = plan.metrics.target(t.id).map(|x| x.down_chance).unwrap_or(0.0);
            if down >= 0.005 {
                let economy = if t.is_pc && down >= 0.15 { " — removing a player grants an action economy advantage" } else { "" };
                parts.push(format!("{:.0}% chance to drop {} this turn{economy}", 100.0 * down, t.name));
            }
        }
        None => parts.push("no hostile creature can be attacked this turn, so the plan sets up the next one".into()),
    }
    parts.extend(S::plan_notes(s, actor, &plan.actions));
    if let Some(st) = plan.metrics.status_effects.first().filter(|x| x.probability >= 0.2) {
        parts.push(format!("{:.0}% to leave {} {}", 100.0 * st.probability, st.name, st.effect));
    }
    if plan.metrics.expected_damage_taken >= 0.5 {
        parts.push(format!("accepts {:.1} expected damage from reactions", plan.metrics.expected_damage_taken));
    }
    if plan.gain < -1e-6 {
        parts.push("every line loses some ground here; this one loses the least".into());
    }
    format!("{}.", capitalise(&parts.join("; ")))
}

fn report_text(r: &TacticalReport) -> String {
    let mut t = format!("RECOMMENDED TURN — {}\n\n", r.actor_name);
    if r.primary.is_empty() {
        t.push_str("No legal actions.\n");
    }
    for (i, a) in r.primary.iter().enumerate() {
        t.push_str(&format!("Action {}: {}.\n", i + 1, a));
    }
    t.push_str("\nALTERNATE TURN\n\n");
    if r.alternate.is_empty() {
        t.push_str("None — no other legal opening.\n");
    }
    for (i, a) in r.alternate.iter().enumerate() {
        t.push_str(&format!("Action {}: {}.\n", i + 1, a));
    }
    if !r.alternate_note.is_empty() {
        t.push_str(&format!("({})\n", r.alternate_note));
    }
    t.push_str(&format!("\nExpected Damage:\n{:.1}\n", r.expected_damage));
    t.push_str("\nExpected Status Effects:\n");
    if r.expected_status_effects.is_empty() {
        t.push_str("None\n");
    }
    for st in r.expected_status_effects.iter().take(6) {
        t.push_str(&format!("{}: {} ({:.0}%)\n", st.name, st.effect, 100.0 * st.probability));
    }
    t.push_str(&format!("\nPriority Target:\n{}\n", r.priority_target_name.clone().unwrap_or_else(|| "None in reach".into())));
    t.push_str(&format!("\nChance to Down Target:\n{:.0}%\n", 100.0 * r.down_chance));
    t.push_str(&format!("\nPosition Evaluation:\n{}\n", r.position));
    t.push_str(&format!("\nReason:\n{}\n", r.reason));
    t
}

/// The best legal turn for whoever is acting, with an alternate plan and
/// the expected damage, status effects, priority target, chance to down it
/// and resulting position. `None` only when no creature is ready to act.
pub fn recommend_turn<S: TacticalSystem>(s: &S::State, cfg: SearchConfig) -> Option<TurnRecommendation<S::Action>> {
    let eval = HeuristicEvaluator::<S>::default();
    let plan = best_turn::<S>(s, &eval, cfg)?;
    let actor = plan.actor;
    let me = S::units(s).into_iter().find(|u| u.id == actor)?;
    let side = me.side;
    let who = me.name.clone();
    let steps = describe_plan::<S>(s, actor, &plan.actions);
    let ranks = rank_targets::<S>(s, actor);
    let target = plan.primary_target::<S>().or_else(|| ranks.first().map(|r| r.target));
    let down_chance = target.and_then(|t| plan.metrics.target(t)).map(|t| t.down_chance).unwrap_or(0.0);
    let sign = if side == Side::Enemy { -1.0 } else { 1.0 };
    let position_score = sign * plan.expected_score;
    let name_of = |id: CombatantId| S::units(s).into_iter().find(|u| u.id == id).map(|u| u.name);

    let (alternate, alternate_note) = match &plan.alternate {
        Some(alt) => {
            let at = alt.metrics.primary_target(alt.actions.iter().filter_map(S::action_target));
            let down = at.and_then(|t| alt.metrics.target(t)).map(|t| t.down_chance).unwrap_or(0.0);
            let mut note = format!("expected damage {:.1}", alt.metrics.expected_damage);
            if let Some(n) = at.and_then(name_of) {
                note.push_str(&format!(", {:.0}% to down {}", 100.0 * down, n));
            }
            note.push_str(&format!(", position {}", position_text(side, sign * alt.expected_score)));
            (describe_plan::<S>(s, actor, &alt.actions), note)
        }
        None => (vec![], String::new()),
    };

    let reason = if plan.actions.is_empty() {
        format!("{who} {}.", why_cannot_act::<S>(s, actor))
    } else {
        reason_text::<S>(s, actor, &plan, target, &ranks)
    };
    let summary = if steps.is_empty() {
        format!("{who} {}; the turn passes", why_cannot_act::<S>(s, actor))
    } else {
        format!("{who}: {} (evaluation {:+.1} → {:+.1})", steps.join(", then "), plan.baseline_score, plan.expected_score)
    };
    let mut report = TacticalReport {
        actor_name: who,
        primary: steps.clone(),
        alternate,
        alternate_note,
        expected_damage: plan.metrics.expected_damage,
        expected_damage_taken: plan.metrics.expected_damage_taken,
        expected_status_effects: plan.metrics.status_effects.clone(),
        priority_target: target,
        priority_target_name: target.and_then(name_of),
        down_chance,
        position_score,
        position: position_text(side, position_score),
        reason,
        text: String::new(),
    };
    report.text = report_text(&report);
    Some(TurnRecommendation { plan, steps, summary, report })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetAssessment {
    pub target: CombatantId,
    pub name: String,
    pub hp: i32,
    /// Expected damage the attacker deals this turn if it focuses this target.
    pub expected_damage: f64,
    /// Probability of dropping the target this turn.
    pub kill_probability: f64,
    /// The target's own expected damage per round against the attacker's side.
    pub threat: f64,
    pub priority: f64,
    pub rationale: String,
}

/// "What is the strongest target?" for `attacker` (any combatant).
pub fn rank_targets<S: TacticalSystem>(s: &S::State, attacker: CombatantId) -> Vec<TargetAssessment> {
    let units = S::units(s);
    let Some(a) = units.iter().find(|u| u.id == attacker) else { return vec![] };
    let share = if S::current_actor(s) == Some(attacker) { S::tempo_fraction(s, attacker) } else { 1.0 };
    let allies: Vec<&Unit> = units.iter().filter(|c| c.side == a.side && !c.down && !c.ignored).collect();
    let mut out = Vec::new();
    for t in units.iter().filter(|t| a.side.is_hostile_to(t.side) && !t.down && !t.ignored) {
        let engage = S::engagement(s, attacker, t.id);
        let expected = S::round_damage(s, attacker, t.id, true) * engage * share;
        let kill = S::kill_probability(s, attacker, t.id).clamp(0.0, 1.0);
        let threat = if allies.is_empty() { 0.0 } else { allies.iter().map(|al| S::round_damage(s, t.id, al.id, true)).sum::<f64>() / allies.len() as f64 };
        // Damage that removes threat sooner is worth more: threat removed per HP.
        let priority = threat * (kill + expected / t.hp.max(1) as f64);
        let rationale = format!(
            "{:.1} expected damage ({:.0}% of its {} HP), {:.0}% to drop it now, it threatens {:.1} damage/round",
            expected,
            100.0 * expected / t.hp.max(1) as f64,
            t.hp,
            100.0 * kill,
            threat
        );
        out.push(TargetAssessment { target: t.id, name: t.name.clone(), hp: t.hp, expected_damage: expected, kill_probability: kill, threat, priority, rationale });
    }
    out.sort_by(|x, y| y.priority.partial_cmp(&x.priority).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreaturePlan<A> {
    pub combatant: CombatantId,
    pub name: String,
    pub recommendation: Option<TurnRecommendation<A>>,
    pub preferred_targets: Vec<TargetAssessment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy<A> {
    pub side: Side,
    pub focus_target: Option<CombatantId>,
    pub creatures: Vec<CreaturePlan<A>>,
    pub notes: Vec<String>,
}

/// "What is the optimal monster strategy?" — each creature's best turn if it
/// acted now, plus a shared focus-fire target.
pub fn side_strategy<S: TacticalSystem>(s: &S::State, side: Side, cfg: SearchConfig) -> Strategy<S::Action> {
    let mut creatures = Vec::new();
    let mut votes: std::collections::BTreeMap<CombatantId, f64> = Default::default();
    let units = S::units(s);
    for c in units.iter().filter(|c| c.side == side && !c.down && !c.ignored) {
        let targets = rank_targets::<S>(s, c.id);
        for t in &targets {
            *votes.entry(t.target).or_default() += t.priority;
        }
        let recommendation = if S::current_actor(s) == Some(c.id) && S::pending(s).is_none() {
            recommend_turn::<S>(s, cfg)
        } else {
            S::hypothetical_turn(s, c.id).and_then(|h| recommend_turn::<S>(&h, cfg))
        };
        creatures.push(CreaturePlan { combatant: c.id, name: c.name.clone(), recommendation, preferred_targets: targets.into_iter().take(3).collect() });
    }
    let focus_target = votes.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal)).map(|(id, _)| *id);
    let mut notes = Vec::new();
    if let Some(f) = focus_target.and_then(|id| units.iter().find(|u| u.id == id)) {
        notes.push(format!("Focus fire on {} ({} HP): it offers the best threat removed per point of damage.", f.name, f.hp));
    }
    let eval = HeuristicEvaluator::<S>::default().evaluate_detailed(s);
    if let Some(top) = eval.factors.iter().max_by(|a, b| a.value.abs().partial_cmp(&b.value.abs()).unwrap_or(std::cmp::Ordering::Equal)) {
        notes.push(format!("Biggest swing factor right now: {} ({:+.1}) — {}", top.name, top.value, top.detail));
    }
    Strategy { side, focus_target, creatures, notes }
}

/// Evaluation of the current position (convenience for drivers).
pub fn evaluate<S: TacticalSystem>(s: &S::State) -> f64 {
    HeuristicEvaluator::<S>::default().evaluate(s)
}
