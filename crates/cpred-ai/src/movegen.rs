//! Candidate generation for Cyberpunk RED. The engine lists *every* legal
//! action (one Move and one Run per reachable square); search only needs
//! the tactically distinct ones. This module prunes, never invents: every
//! candidate it returns came from `CombatState::legal_actions`.
//!
//! Movement is where Cyberpunk RED fights are won — cover blocks ranged
//! attacks outright — so destinations are chosen for what they do:
//! the best **cover** square (fewest enemies with a clear shot), the best
//! **firing** square (most enemies we can shoot), the **closest** square to
//! the nearest enemy (melee, or no shot anywhere) and a **retreat** square.

use crate::ev::action_damage;
use cpred_core::combat::{Action, CombatState, Combatant};
use tactics_ai::CandidateConfig;
use tactics_core::{CombatantId, Pos};

/// Why a destination was picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveReason {
    Cover,
    Firing,
    Close,
    Retreat,
}

/// Standing hostile fighters (mortally wounded ones are out of the fight).
pub fn foes<'a>(s: &'a CombatState, a: &'a Combatant, focus: Option<CombatantId>) -> Vec<&'a Combatant> {
    s.combatants
        .iter()
        .filter(|c| c.alive && !c.is_down() && a.side.is_hostile_to(c.side))
        .filter(|c| focus.map(|f| f == c.id).unwrap_or(true))
        .collect()
}

/// Enemies with a clear ranged shot at `sq` (line of sight, no cover next to `sq`).
pub fn exposure(s: &CombatState, foes: &[&Combatant], sq: Pos) -> usize {
    foes.iter().filter(|f| f.statblock.weapons.iter().any(|w| !w.melee) && s.line_of_sight(f.position, sq) && s.blocking_cover(f.position, sq).is_none()).count()
}

/// Enemies the actor could attack from `sq`.
pub fn shots_from(s: &CombatState, a: &Combatant, foes: &[&Combatant], sq: Pos) -> usize {
    let ranged = a.statblock.weapons.iter().any(|w| !w.melee);
    foes.iter()
        .filter(|f| sq.chebyshev(f.position) == 1 || (ranged && s.line_of_sight(sq, f.position) && s.blocking_cover(sq, f.position).is_none()))
        .count()
}

fn nearest(foes: &[&Combatant], sq: Pos) -> u32 {
    foes.iter().map(|f| sq.chebyshev(f.position)).min().unwrap_or(0)
}

/// Prefers melee: its best melee weapon out-damages its best ranged one.
fn prefers_melee(s: &CombatState, a: &Combatant, foes: &[&Combatant]) -> bool {
    let best = |melee: bool| {
        a.statblock
            .weapons
            .iter()
            .enumerate()
            .filter(|(_, w)| w.melee == melee)
            .map(|(i, _)| foes.iter().map(|f| crate::ev::potential_attack(s, a.id, i, f.id, cpred_core::rules::AttackMode::Single) * a.statblock.weapons[i].rof as f64).fold(0.0, f64::max))
            .fold(0.0, f64::max)
    };
    best(true) > best(false)
}

/// Distinct destinations among `squares`, each tagged with why it was picked.
pub fn pick_squares(s: &CombatState, a: &Combatant, foes: &[&Combatant], squares: &[(Pos, u32)], max: usize) -> Vec<(Pos, MoveReason)> {
    if squares.is_empty() || foes.is_empty() || max == 0 {
        return vec![];
    }
    let scored: Vec<(Pos, u32, usize, usize, u32)> = squares.iter().map(|&(p, cost)| (p, cost, exposure(s, foes, p), shots_from(s, a, foes, p), nearest(foes, p))).collect();
    let melee = prefers_melee(s, a, foes);
    let mut picks: Vec<(Pos, MoveReason)> = Vec::new();
    let push = |p: Option<Pos>, r: MoveReason, picks: &mut Vec<(Pos, MoveReason)>| {
        if let Some(p) = p {
            if !picks.iter().any(|(q, _)| *q == p) {
                picks.push((p, r));
            }
        }
    };
    // Close in: adjacency for melee, otherwise just nearer.
    let close = scored.iter().min_by_key(|x| (x.4, x.2, x.1, x.0)).map(|x| x.0);
    // Most targets, then least exposed, then cheapest.
    let firing = scored.iter().filter(|x| x.3 > 0).min_by_key(|x| (std::cmp::Reverse(x.3), x.2, x.1, x.0)).map(|x| x.0);
    // Least exposed, keeping a shot if possible, then cheapest.
    let cover = scored.iter().min_by_key(|x| (x.2, std::cmp::Reverse(x.3.min(1)), x.1, x.0)).map(|x| x.0);
    let retreat = scored.iter().max_by_key(|x| (x.4, std::cmp::Reverse(x.2), std::cmp::Reverse(x.1), std::cmp::Reverse(x.0))).map(|x| x.0);
    if melee {
        push(close, MoveReason::Close, &mut picks);
        push(cover, MoveReason::Cover, &mut picks);
    } else {
        push(cover, MoveReason::Cover, &mut picks);
        push(firing, MoveReason::Firing, &mut picks);
        push(close, MoveReason::Close, &mut picks);
    }
    push(retreat, MoveReason::Retreat, &mut picks);
    picks.truncate(max);
    picks
}

/// Tactically distinct legal actions for the current actor, best-first:
/// attacks by expected damage, reloads, then movement.
pub fn candidates(s: &CombatState, cfg: &CandidateConfig) -> Vec<Action> {
    let Some(actor) = s.current_actor() else { return vec![] };
    let Some(a) = s.combatant(actor) else { return vec![] };
    let all = s.legal_actions();
    let targets = foes(s, a, cfg.focus);
    let mut attacks: Vec<(f64, Action)> = Vec::new();
    let mut other: Vec<Action> = Vec::new();
    let mut moves: Vec<(Pos, u32)> = vec![];
    let mut runs: Vec<(Pos, u32)> = vec![];
    let mm = if a.economy.move_available || a.economy.action_available { Some(s.movement(actor)) } else { None };
    let cost = |p: Pos| mm.as_ref().and_then(|m| m.cost_to(p)).unwrap_or(99);
    for act in &all {
        match act {
            Action::Move { to } => moves.push((*to, cost(*to))),
            Action::Run { to } => runs.push((*to, cost(*to))),
            Action::Reload { .. } => other.push(act.clone()),
            _ => {
                if let Some((weapon, target, mode)) = act.attack() {
                    if targets.iter().any(|t| t.id == target) {
                        let ev = cpred_core::analysis::expected_damage(s, actor, weapon, target, mode);
                        attacks.push((ev, act.clone()));
                    }
                }
            }
        }
    }
    attacks.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    // Pointless attacks (blocked by cover, no chance) only stay if nothing else hits.
    if attacks.iter().any(|(ev, _)| *ev > 0.0) {
        attacks.retain(|(ev, _)| *ev > 0.0);
    }
    let mut out: Vec<Action> = attacks.into_iter().map(|(_, a)| a).collect();
    out.extend(other);
    let all_foes = foes(s, a, None);
    for (p, _) in pick_squares(s, a, &all_foes, &moves, cfg.max_moves) {
        out.push(Action::Move { to: p });
    }
    // Running gives up the attack: only worth a look toward a firing / closing square.
    for (p, r) in pick_squares(s, a, &all_foes, &runs, cfg.max_moves.min(2)) {
        if matches!(r, MoveReason::Close | MoveReason::Firing | MoveReason::Cover) {
            out.push(Action::Run { to: p });
        }
    }
    if out.is_empty() && cfg.committed {
        out.extend(all.iter().take(cfg.max_moves.max(1)).cloned());
    }
    out
}

/// Fast policy for Monte Carlo: shoot the best target, reload when empty,
/// otherwise move to shoot (or into cover after shooting), run to close.
pub fn scripted_action(s: &CombatState) -> Option<Action> {
    let actor = s.current_actor()?;
    let a = s.combatant(actor)?;
    let e = a.economy;
    let legal = s.legal_actions();
    let targets = foes(s, a, None);
    if targets.is_empty() {
        return None;
    }
    let best_attack = |st: &CombatState| {
        st.legal_actions()
            .into_iter()
            .filter_map(|act| act.attack().map(|(w, t, m)| (cpred_core::analysis::expected_damage(st, actor, w, t, m), act)))
            .filter(|(ev, _)| *ev > 0.3)
            .max_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal))
    };
    if e.action_available || e.shots_left > 0 {
        let now = best_attack(s);
        // Fresh Action and Move: a step to a better firing square first?
        if e.action_available && e.move_available {
            let squares: Vec<(Pos, u32)> = legal.iter().filter_map(|x| match x { Action::Move { to } => Some((*to, 0)), _ => None }).collect();
            let here = now.as_ref().map(|x| x.0).unwrap_or(0.0);
            let better = pick_squares(s, a, &targets, &squares, 4)
                .into_iter()
                .filter(|(_, r)| matches!(r, MoveReason::Firing | MoveReason::Close | MoveReason::Cover))
                .filter_map(|(p, _)| {
                    let mut moved = s.clone();
                    moved.combatant_mut(actor)?.position = p;
                    moved.combatant_mut(actor)?.economy.move_available = false;
                    best_attack(&moved).map(|(ev, _)| (ev, p))
                })
                .max_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((ev, p)) = better {
                if ev > here * 1.25 + 0.5 {
                    return Some(Action::Move { to: p });
                }
            }
        }
        if let Some((_, act)) = now {
            return Some(act);
        }
    }
    if e.action_available {
        let empty = legal.iter().find(|x| matches!(x, Action::Reload { weapon } if a.ammo.get(*weapon) == Some(&0)));
        if let Some(r) = empty {
            return Some(r.clone());
        }
    }
    let can_hit_now = (0..a.statblock.weapons.len()).any(|w| targets.iter().any(|t| action_damage(s, actor, w, t.id, false) > 0.0 && cpred_core::analysis::attack_profile(s, actor, w, t.id, cpred_core::rules::AttackMode::Single).map(|p| !p.blocked_by_cover).unwrap_or(false)));
    if e.move_available {
        let squares: Vec<(Pos, u32)> = legal.iter().filter_map(|x| match x { Action::Move { to } => Some((*to, 0)), _ => None }).collect();
        let picks = pick_squares(s, a, &targets, &squares, 4);
        let want = if e.action_available && !can_hit_now { [MoveReason::Firing, MoveReason::Close] } else { [MoveReason::Cover, MoveReason::Cover] };
        if let Some((p, _)) = picks.iter().find(|(_, r)| want.contains(r)) {
            if *p != a.position {
                return Some(Action::Move { to: *p });
            }
        }
    }
    if e.action_available && !can_hit_now {
        let squares: Vec<(Pos, u32)> = legal.iter().filter_map(|x| match x { Action::Run { to } => Some((*to, 0)), _ => None }).collect();
        if let Some((p, _)) = pick_squares(s, a, &targets, &squares, 4).into_iter().find(|(_, r)| matches!(r, MoveReason::Firing | MoveReason::Close)) {
            return Some(Action::Run { to: p });
        }
    }
    None
}
