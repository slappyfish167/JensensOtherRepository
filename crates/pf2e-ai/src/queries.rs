//! High-level tactical questions, each answered with data plus a short
//! human-readable rationale.

use crate::analysis::{engagement_factor, expected_strike, round_damage};
use crate::evaluation::{Evaluator, HeuristicEvaluator};
use crate::search::{best_turn, expectation_after, PlannedTurn, SearchConfig, Searcher};
use pf2e_core::checks::{degree_distribution, DegreeOfSuccess};
use pf2e_core::combat::pending::{AttackCtx, AttackSource};
use pf2e_core::combat::rules;
use pf2e_core::combat::{Action, CombatState};
use pf2e_core::grid::CoverLevel;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};

pub fn describe_action(s: &CombatState, actor: CombatantId, a: &Action) -> String {
    let name = |id: &CombatantId| s.combatant(*id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_string());
    match a {
        Action::Strike { actor_strike, target, .. } => {
            let w = s.combatant(actor).and_then(|c| c.statblock.strikes.get(*actor_strike)).map(|w| w.name.clone()).unwrap_or_default();
            format!("Strike {} with {w}", name(target))
        }
        Action::Stride { to } => format!("Stride to ({}, {})", to.x, to.y),
        Action::Step { to } => format!("Step to ({}, {})", to.x, to.y),
        Action::Stand => "Stand".into(),
        Action::RaiseShield => "Raise a Shield".into(),
        Action::TakeCover => "Take Cover".into(),
        Action::Demoralize { target } => format!("Demoralize {}", name(target)),
        Action::Trip { target } => format!("Trip {}", name(target)),
        Action::CastSpell { spell_id, rank, targets, aim, .. } => {
            let sname = s.spells.get(spell_id).map(|x| x.name.clone()).unwrap_or_else(|| spell_id.clone());
            let on = if !targets.is_empty() {
                format!(" on {}", targets.iter().map(name).collect::<Vec<_>>().join(", "))
            } else if let Some(p) = aim {
                format!(" at ({}, {})", p.x, p.y)
            } else {
                String::new()
            };
            format!("Cast {sname} (rank {rank}){on}")
        }
        Action::DisableHazard { hazard } => format!("Disable {}", name(hazard)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecommendation {
    pub plan: PlannedTurn,
    pub steps: Vec<String>,
    pub summary: String,
}

pub fn recommend_turn(s: &CombatState, cfg: SearchConfig) -> Option<TurnRecommendation> {
    let eval = HeuristicEvaluator::default();
    let plan = best_turn(s, &eval, cfg)?;
    let steps: Vec<String> = plan.actions.iter().map(|a| describe_action(s, plan.actor, a)).collect();
    let who = s.combatant(plan.actor).map(|c| c.name.clone()).unwrap_or_default();
    let summary = if steps.is_empty() {
        format!("{who}: no action improves the position; end the turn")
    } else {
        format!(
            "{who}: {} (evaluation {:+.1} → {:+.1})",
            steps.join(", then "),
            plan.baseline_score,
            plan.expected_score
        )
    };
    Some(TurnRecommendation { plan, steps, summary })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetAssessment {
    pub target: CombatantId,
    pub name: String,
    pub hp: i32,
    /// Expected damage the attacker deals this turn if it focuses this target.
    pub expected_damage: f64,
    /// Probability of dropping the target this turn (strike-based estimate).
    pub kill_probability: f64,
    /// The target's own expected damage per round against the attacker's side.
    pub threat: f64,
    pub priority: f64,
    pub rationale: String,
}

/// "What is the strongest target?" for `attacker` (any combatant).
pub fn rank_targets(s: &CombatState, attacker: CombatantId) -> Vec<TargetAssessment> {
    let Some(a) = s.combatant(attacker) else { return vec![] };
    let actions = if s.current_actor() == Some(attacker) { a.actions_remaining } else { 3 };
    let mut out = Vec::new();
    for t in s.combatants.iter().filter(|t| a.side.is_hostile_to(t.side) && !t.is_down() && !t.is_hazard()) {
        let engage = engagement_factor(s, attacker, t.id);
        let expected = round_damage(s, attacker, t.id, true) * engage * actions as f64 / 3.0;
        let kill = kill_probability(s, attacker, t.id, if engage < 1.0 { actions.saturating_sub(1) } else { actions });
        let allies: Vec<_> = s.combatants.iter().filter(|c| c.side == a.side && !c.is_down() && !c.is_hazard()).collect();
        let threat = if allies.is_empty() {
            0.0
        } else {
            allies.iter().map(|al| round_damage(s, t.id, al.id, true)).sum::<f64>() / allies.len() as f64
        };
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

/// P(total strike damage ≥ target HP) using `actions` Strikes with the best
/// weapon at each MAP step, enumerating every degree combination.
fn kill_probability(s: &CombatState, attacker: CombatantId, target: CombatantId, actions: u8) -> f64 {
    let a = s.combatant(attacker).expect("attacker");
    let t = s.combatant(target).expect("target");
    let mut dists: Vec<Vec<(f64, f64)>> = Vec::new(); // per strike: (prob, damage)
    for n in 0..actions.min(3) {
        let Some((idx, _)) = a
            .statblock
            .strikes
            .iter()
            .enumerate()
            .map(|(i, _)| (i, expected_strike(s, attacker, target, i, n, CoverLevel::None)))
            .max_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal))
        else {
            break;
        };
        let st = &a.statblock.strikes[idx];
        let ctx = AttackCtx {
            attacker,
            target,
            source: AttackSource::Strike { index: idx },
            map_penalty: rules::map_penalty(n, st.is_agile()),
            attack_number: n,
            cover: CoverLevel::None,
            disrupts: None,
            sweep: false,
        };
        let nums = rules::attack_numbers(s, &ctx);
        let d = degree_distribution(nums.modifier, nums.ac);
        let hit = crate::analysis::expected_damage_after_iwr(&rules::strike_damage_specs(s, &ctx, false), t, 1.0);
        let crit = crate::analysis::expected_damage_after_iwr(&rules::strike_damage_specs(s, &ctx, true), t, 1.0);
        let miss = d[DegreeOfSuccess::Failure.index()] + d[DegreeOfSuccess::CriticalFailure.index()];
        dists.push(vec![(miss, 0.0), (d[DegreeOfSuccess::Success.index()], hit), (d[DegreeOfSuccess::CriticalSuccess.index()], crit)]);
    }
    let mut acc: Vec<(f64, f64)> = vec![(1.0, 0.0)];
    for d in dists {
        let mut next = Vec::new();
        for (p0, v0) in &acc {
            for (p1, v1) in &d {
                next.push((p0 * p1, v0 + v1));
            }
        }
        acc = next;
    }
    acc.iter().filter(|(_, v)| *v >= t.hp as f64).map(|(p, _)| p).sum()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpellAssessment {
    pub action: Action,
    pub description: String,
    /// Change in evaluation (acting side's perspective) after the spell resolves.
    pub eval_gain: f64,
    pub expected_enemy_damage: f64,
    pub expected_ally_healing: f64,
    pub kill_probability: f64,
}

/// "Which spell maximises expected value?" for the creature whose turn it is.
pub fn rank_spells(s: &CombatState) -> Vec<SpellAssessment> {
    let Some(actor) = s.current_actor() else { return vec![] };
    let Some(me) = s.combatant(actor) else { return vec![] };
    let side = me.side;
    let eval = HeuristicEvaluator::default();
    let sign = if side == pf2e_core::stats::Side::Enemy { -1.0 } else { 1.0 };
    let base = sign * eval.evaluate(s);
    let enemy_hp = |st: &CombatState| -> f64 {
        st.combatants.iter().filter(|c| side.is_hostile_to(c.side) && !c.is_hazard()).map(|c| c.hp as f64).sum()
    };
    let ally_hp = |st: &CombatState| -> f64 { st.combatants.iter().filter(|c| c.side == side).map(|c| c.hp as f64).sum() };
    let enemies_down = |st: &CombatState| -> f64 {
        st.combatants.iter().filter(|c| side.is_hostile_to(c.side) && !c.is_hazard() && c.is_down()).count() as f64
    };
    let (e0, a0, d0) = (enemy_hp(s), ally_hp(s), enemies_down(s));
    let mut out = Vec::new();
    for act in s.legal_actions().into_iter().filter(|a| matches!(a, Action::CastSpell { .. })) {
        let mut searcher = Searcher::new(&eval, SearchConfig::default(), s, actor);
        let Some(v) = searcher.one_ply(s, &act) else { continue };
        let e1 = expectation_after(s, actor, &act, &enemy_hp).unwrap_or(e0);
        let a1 = expectation_after(s, actor, &act, &ally_hp).unwrap_or(a0);
        let k = expectation_after(s, actor, &act, &|st: &CombatState| if enemies_down(st) > d0 { 1.0 } else { 0.0 }).unwrap_or(0.0);
        out.push(SpellAssessment {
            description: describe_action(s, actor, &act),
            action: act,
            eval_gain: v - base,
            expected_enemy_damage: e0 - e1,
            expected_ally_healing: (a1 - a0).max(0.0),
            kill_probability: k,
        });
    }
    out.sort_by(|x, y| y.eval_gain.partial_cmp(&x.eval_gain).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreaturePlan {
    pub combatant: CombatantId,
    pub name: String,
    pub recommendation: Option<TurnRecommendation>,
    pub preferred_targets: Vec<TargetAssessment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub side: pf2e_core::stats::Side,
    pub focus_target: Option<CombatantId>,
    pub creatures: Vec<CreaturePlan>,
    pub notes: Vec<String>,
}

/// "What is the optimal monster strategy?" — each creature's best turn if it
/// acted now, plus a shared focus-fire target.
pub fn side_strategy(s: &CombatState, side: pf2e_core::stats::Side, cfg: SearchConfig) -> Strategy {
    let mut creatures = Vec::new();
    let mut votes: std::collections::BTreeMap<CombatantId, f64> = Default::default();
    for c in s.combatants.iter().filter(|c| c.side == side && !c.is_down() && c.in_initiative) {
        let targets = rank_targets(s, c.id);
        for t in &targets {
            *votes.entry(t.target).or_default() += t.priority;
        }
        let recommendation = if s.current_actor() == Some(c.id) && s.pending().is_none() {
            recommend_turn(s, cfg)
        } else {
            s.hypothetical_turn(c.id).and_then(|h| recommend_turn(&h, cfg))
        };
        creatures.push(CreaturePlan { combatant: c.id, name: c.name.clone(), recommendation, preferred_targets: targets.into_iter().take(3).collect() });
    }
    let focus_target = votes.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal)).map(|(id, _)| *id);
    let mut notes = Vec::new();
    if let Some(f) = focus_target.and_then(|id| s.combatant(id)) {
        notes.push(format!("Focus fire on {} ({} HP): it offers the best threat removed per point of damage.", f.name, f.hp));
    }
    let eval = HeuristicEvaluator::default().evaluate_detailed(s);
    if let Some(top) = eval.factors.iter().max_by(|a, b| a.value.abs().partial_cmp(&b.value.abs()).unwrap_or(std::cmp::Ordering::Equal)) {
        notes.push(format!("Biggest swing factor right now: {} ({:+.1}) — {}", top.name, top.value, top.detail));
    }
    Strategy { side, focus_target, creatures, notes }
}
