//! High-level tactical questions, each answered with data plus a short
//! human-readable rationale.

use crate::analysis::{engagement_factor, expected_strike, round_damage};
use crate::evaluation::{Evaluator, HeuristicEvaluator};
use crate::search::{best_turn, expectation_after, PlannedTurn, SearchConfig, Searcher, StatusExpectation};
use pf2e_core::checks::{degree_distribution, DegreeOfSuccess};
use pf2e_core::combat::pending::{AttackCtx, AttackSource};
use pf2e_core::combat::rules;
use pf2e_core::combat::{Action, CombatState, CombatantKind};
use pf2e_core::grid::{flanks, within_reach, CoverLevel, Pos};
use pf2e_core::models::statblock::StrikeRange;
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};

/// Foes the actor would flank from `to` (an ally able to act already
/// threatens the far side, and `to` is within the actor's melee reach).
fn flanked_from(s: &CombatState, actor: CombatantId, to: Pos) -> Vec<String> {
    let Some(a) = s.combatant(actor) else { return vec![] };
    let reach = |c: &pf2e_core::combat::Combatant| {
        c.statblock.strikes.iter().filter_map(|w| match w.range {
            StrikeRange::Melee { reach_ft } => Some(reach_ft),
            _ => None,
        }).max()
    };
    let Some(my_reach) = reach(a) else { return vec![] };
    s.combatants
        .iter()
        .filter(|f| a.side.is_hostile_to(f.side) && !f.is_down() && !f.is_hazard())
        .filter(|f| within_reach(to, a.statblock.size, f.position, f.statblock.size, my_reach))
        .filter(|f| {
            s.combatants.iter().filter(|al| al.side == a.side && al.id != actor && al.can_act()).any(|al| {
                reach(al).map(|r| within_reach(al.position, al.statblock.size, f.position, f.statblock.size, r)).unwrap_or(false)
                    && flanks(to, a.statblock.size, al.position, al.statblock.size, f.position, f.statblock.size)
            })
        })
        .map(|f| f.name.clone())
        .collect()
}

pub fn describe_action(s: &CombatState, actor: CombatantId, a: &Action) -> String {
    let name = |id: &CombatantId| s.combatant(*id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_string());
    let flank = |to: &Pos| {
        let f = flanked_from(s, actor, *to);
        if f.is_empty() { String::new() } else { format!(", flanking {}", f.join(" and ")) }
    };
    match a {
        Action::Strike { actor_strike, target, .. } => {
            let w = s.combatant(actor).and_then(|c| c.statblock.strikes.get(*actor_strike)).map(|w| w.name.clone()).unwrap_or_default();
            format!("Strike {} with {w}", name(target))
        }
        Action::Stride { to } => format!("Stride to ({}, {}){}", to.x, to.y, flank(to)),
        Action::Step { to } => format!("Step to ({}, {}){}", to.x, to.y, flank(to)),
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
pub struct TurnRecommendation {
    pub plan: PlannedTurn,
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

fn opposing(side: Side) -> Side {
    match side {
        Side::Enemy => Side::Party,
        _ => Side::Enemy,
    }
}

/// Signed position from the actor's side, never phrased as "equal".
fn position_text(side: Side, v: f64) -> String {
    if v >= 0.0 {
        format!("{:+.1} {} Advantage", v, side_label(side))
    } else {
        format!("{:+.1} {} Advantage", -v, side_label(opposing(side)))
    }
}

/// Plan steps described against the state each step is taken in.
fn describe_plan(s: &CombatState, actor: CombatantId, actions: &[Action]) -> Vec<String> {
    actions.iter().map(|a| describe_action(s, actor, a)).collect()
}

fn why_cannot_act(s: &CombatState, actor: CombatantId) -> String {
    let Some(c) = s.combatant(actor) else { return "is not in the combat".into() };
    if c.is_down() {
        "is down".into()
    } else if !c.can_act() {
        "is paralyzed".into()
    } else if c.actions_remaining == 0 {
        "has no actions remaining this turn".into()
    } else {
        "has no legal action".into()
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn reason_text(s: &CombatState, plan: &PlannedTurn, steps: &[String], target: Option<CombatantId>, ranks: &[TargetAssessment]) -> String {
    let mut parts: Vec<String> = Vec::new();
    match target.and_then(|t| s.combatant(t)) {
        Some(t) => {
            let ehp = |c: &pf2e_core::combat::Combatant| c.hp + c.temp_hp;
            let standing: Vec<_> = ranks.iter().filter_map(|r| s.combatant(r.target)).collect();
            let lowest = standing.iter().map(|c| ehp(c)).min();
            let me = ranks.iter().find(|r| r.target == t.id);
            let top_threat = ranks.iter().map(|r| r.threat).fold(f64::MIN, f64::max);
            if lowest == Some(ehp(t)) && standing.len() > 1 {
                parts.push(format!("{} has the lowest effective HP ({})", t.name, ehp(t)));
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
                let economy = if t.kind == CombatantKind::Character && down >= 0.15 { " — removing a player grants an action economy advantage" } else { "" };
                parts.push(format!("{:.0}% chance to drop {} this turn{economy}", 100.0 * down, t.name));
            }
        }
        None => parts.push("no hostile creature can be attacked this turn, so the plan sets up the next one".into()),
    }
    if steps.iter().any(|x| x.contains(", flanking ")) {
        parts.push("the move sets up a flank (target off-guard, −2 AC)".into());
    }
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
pub fn recommend_turn(s: &CombatState, cfg: SearchConfig) -> Option<TurnRecommendation> {
    let eval = HeuristicEvaluator::default();
    let plan = best_turn(s, &eval, cfg)?;
    let actor = plan.actor;
    let me = s.combatant(actor)?;
    let side = me.side;
    let who = me.name.clone();
    let steps = describe_plan(s, actor, &plan.actions);
    let ranks = rank_targets(s, actor);
    let target = plan.metrics.primary_target(&plan.actions).or_else(|| ranks.first().map(|r| r.target));
    let down_chance = target.and_then(|t| plan.metrics.target(t)).map(|t| t.down_chance).unwrap_or(0.0);
    let sign = if side == Side::Enemy { -1.0 } else { 1.0 };
    let position_score = sign * plan.expected_score;

    let (alternate, alternate_note) = match &plan.alternate {
        Some(alt) => {
            let at = alt.metrics.primary_target(&alt.actions);
            let down = at.and_then(|t| alt.metrics.target(t)).map(|t| t.down_chance).unwrap_or(0.0);
            let mut note = format!("expected damage {:.1}", alt.metrics.expected_damage);
            if let Some(n) = at.and_then(|t| s.combatant(t)) {
                note.push_str(&format!(", {:.0}% to down {}", 100.0 * down, n.name));
            }
            note.push_str(&format!(", position {}", position_text(side, sign * alt.expected_score)));
            (describe_plan(s, actor, &alt.actions), note)
        }
        None => (vec![], String::new()),
    };

    let reason = if plan.actions.is_empty() {
        format!("{who} {}.", why_cannot_act(s, actor))
    } else {
        reason_text(s, &plan, &steps, target, &ranks)
    };
    let summary = if steps.is_empty() {
        format!("{who} {}; the turn passes", why_cannot_act(s, actor))
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
        priority_target_name: target.and_then(|t| s.combatant(t)).map(|c| c.name.clone()),
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
