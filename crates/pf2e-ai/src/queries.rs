//! High-level tactical questions, each answered with data plus a short
//! human-readable rationale.

use crate::analysis::expected_strike;
use crate::evaluation::{Evaluator, HeuristicEvaluator};
use crate::search::{expectation_after, SearchConfig, Searcher};
use crate::system::Pf2e;
use pf2e_core::checks::{degree_distribution, DegreeOfSuccess};
use pf2e_core::combat::pending::{AttackCtx, AttackSource};
use pf2e_core::combat::rules;
use pf2e_core::combat::{Action, CombatState};
use pf2e_core::grid::{flanks, within_reach, CoverLevel, Pos};
use pf2e_core::models::statblock::StrikeRange;
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};

/// Foes the actor would flank from `to` (an ally able to act already
/// threatens the far side, and `to` is within the actor's melee reach).
pub fn flanked_from(s: &CombatState, actor: CombatantId, to: Pos) -> Vec<String> {
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

/// P(total strike damage ≥ target HP) using `actions` Strikes with the best
/// weapon at each MAP step, enumerating every degree combination.
pub fn kill_probability(s: &CombatState, attacker: CombatantId, target: CombatantId, actions: u8) -> f64 {
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

pub type TurnRecommendation = tactics_ai::TurnRecommendation<Action>;
pub type CreaturePlan = tactics_ai::CreaturePlan<Action>;
pub type Strategy = tactics_ai::Strategy<Action>;
pub use tactics_ai::{TacticalReport, TargetAssessment};

/// The best legal turn for whoever is acting, with the full tactical report.
pub fn recommend_turn(s: &CombatState, cfg: SearchConfig) -> Option<TurnRecommendation> {
    tactics_ai::recommend_turn::<Pf2e>(s, cfg)
}

/// "What is the strongest target?" for `attacker` (any combatant).
pub fn rank_targets(s: &CombatState, attacker: CombatantId) -> Vec<TargetAssessment> {
    tactics_ai::rank_targets::<Pf2e>(s, attacker)
}

/// "What is the optimal monster strategy?" for one side.
pub fn side_strategy(s: &CombatState, side: Side, cfg: SearchConfig) -> Strategy {
    tactics_ai::side_strategy::<Pf2e>(s, side, cfg)
}
