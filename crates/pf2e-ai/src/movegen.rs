//! Candidate generation: the engine lists *every* legal action (including
//! one Stride per reachable square); search only needs the tactically
//! distinct ones. This module prunes, never invents: every candidate it
//! returns came from `CombatState::legal_actions`.

use pf2e_core::combat::{Action, CombatState};
use pf2e_core::conditions::ConditionKind as CK;
use pf2e_core::grid::{creature_distance, within_reach, Pos};
use pf2e_core::models::statblock::StrikeRange;
use pf2e_core::CombatantId;

#[derive(Debug, Clone, Copy)]
pub struct MoveGenConfig {
    /// Maximum Stride destinations kept.
    pub max_moves: usize,
    /// Only keep actions involving this target (plus movement).
    pub focus: Option<CombatantId>,
}

impl Default for MoveGenConfig {
    fn default() -> Self {
        MoveGenConfig { max_moves: 5, focus: None }
    }
}

pub fn candidates(state: &CombatState, cfg: &MoveGenConfig) -> Vec<Action> {
    let Some(actor) = state.current_actor() else { return vec![] };
    let a = state.combatant(actor).expect("actor");
    let all = state.legal_actions();
    let foes: Vec<_> = state
        .combatants
        .iter()
        .filter(|c| a.side.is_hostile_to(c.side) && !c.is_down() && !c.is_hazard())
        .filter(|c| cfg.focus.map(|f| f == c.id).unwrap_or(true))
        .collect();

    let mut out: Vec<Action> = Vec::new();
    let mut strides: Vec<Pos> = Vec::new();
    for act in all {
        match &act {
            Action::Stride { to } => strides.push(*to),
            Action::Step { to } => {
                // Steps matter mostly for leaving reach without provoking.
                let adjacent_to_foe = foes.iter().any(|f| creature_distance(a.position, a.statblock.size, f.position, f.statblock.size) <= 5);
                let leaves = foes.iter().all(|f| creature_distance(*to, a.statblock.size, f.position, f.statblock.size) > 5);
                if adjacent_to_foe && leaves && !out.iter().any(|x| matches!(x, Action::Step { .. })) {
                    out.push(act);
                }
            }
            Action::TakeCover => {}
            Action::Strike { target, .. } | Action::Demoralize { target } | Action::Trip { target } => {
                if cfg.focus.map(|f| f == *target).unwrap_or(true) {
                    out.push(act);
                }
            }
            Action::CastSpell { targets, .. } => {
                let ok = match cfg.focus {
                    Some(f) => targets.is_empty() || targets.contains(&f),
                    None => true,
                };
                if ok {
                    out.push(act);
                }
            }
            _ => out.push(act),
        }
    }
    // Keep only the best Strike per (target, MAP-equivalent weapon) — the
    // legal list has one entry per weapon; search sorts them out cheaply,
    // so we keep them all.

    // Movement: squares from which we can attack each foe (closest first,
    // preferring flanking), plus a retreat square for ranged creatures.
    if !a.has(CK::Immobilized) {
        let mm = state.movement(actor);
        let melee_reach = a.statblock.strikes.iter().filter_map(|s| match s.range {
            StrikeRange::Melee { reach_ft } => Some(reach_ft),
            _ => None,
        }).max();
        let ranged = a.statblock.strikes.iter().any(|s| !s.is_melee()) || !a.statblock.spellcasting.is_empty();
        let mut picks: Vec<(f64, Pos)> = Vec::new();
        for f in &foes {
            if let Some(reach) = melee_reach {
                let mut best: Option<(f64, Pos)> = None;
                for &to in &strides {
                    if within_reach(to, a.statblock.size, f.position, f.statblock.size, reach) {
                        let cost = mm.cost_to(to).unwrap_or(999) as f64;
                        // Prefer squares where an ally flanks (opposite side of the foe).
                        let flank_bonus = state
                            .combatants
                            .iter()
                            .filter(|al| al.side == a.side && al.id != actor && al.can_act())
                            .any(|al| {
                                pf2e_core::grid::flanks(to, a.statblock.size, al.position, al.statblock.size, f.position, f.statblock.size)
                            });
                        let score = cost - if flank_bonus { 15.0 } else { 0.0 } + f.hp as f64 * 0.1;
                        if best.map(|b| score < b.0).unwrap_or(true) {
                            best = Some((score, to));
                        }
                    }
                }
                if let Some(b) = best {
                    picks.push(b);
                }
            }
        }
        // Approach the nearest foe if nothing is reachable this Stride.
        if picks.is_empty() && !foes.is_empty() && !strides.is_empty() {
            let mut best = (u32::MAX, strides[0]);
            for &to in &strides {
                let d = foes.iter().map(|f| creature_distance(to, a.statblock.size, f.position, f.statblock.size)).min().unwrap_or(u32::MAX);
                if d < best.0 {
                    best = (d, to);
                }
            }
            picks.push((0.0, best.1));
        }
        if ranged && !strides.is_empty() && !foes.is_empty() {
            // Retreat: maximise distance to the closest foe.
            let mut best = (0u32, strides[0]);
            for &to in &strides {
                let d = foes.iter().map(|f| creature_distance(to, a.statblock.size, f.position, f.statblock.size)).min().unwrap_or(0);
                if d > best.0 {
                    best = (d, to);
                }
            }
            picks.push((1000.0, best.1));
        }
        picks.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal).then(x.1.cmp(&y.1)));
        let mut seen = Vec::new();
        for (_, to) in picks {
            if !seen.contains(&to) && seen.len() < cfg.max_moves {
                seen.push(to);
                out.push(Action::Stride { to });
            }
        }
    }
    out
}
