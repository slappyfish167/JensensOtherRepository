//! Expected-value helpers for Cyberpunk RED, built strictly on `cpred-core`
//! rule functions: attack modifiers, DVs, evasion, SP halving, critical
//! injury odds and the exploding-d10 math all come from the core crate.

use cpred_core::analysis::{attack_profile, dice_sum_distribution, expected_damage, p_beat, p_critical_injury, p_opposed};
use cpred_core::combat::{CombatState, Combatant};
use cpred_core::model::{HitLocation, Weapon};
use cpred_core::rules::{self, AttackMode, METRES_PER_SQUARE};
use tactics_core::CombatantId;

/// Share of a round's damage kept against a target hiding behind cover:
/// the attacker has to reposition or shoot the cover down first.
const BEHIND_COVER: f64 = 0.4;

/// Expected HP loss from one hit of `dice` against `sp` (doubled past SP on
/// head hits), plus the critical-injury bonus.
pub fn hit_damage(state: &CombatState, dice: &tactics_core::DiceExpr, sp: i32, head: bool) -> f64 {
    let past: f64 = dice_sum_distribution(dice).iter().map(|(d, p)| p * (d - sp).max(0) as f64).sum();
    past * if head { 2.0 } else { 1.0 } + p_critical_injury(dice) * state.rules.critical_injury_damage as f64
}

/// What one attack would be worth if range, line of sight and cover allowed
/// it: the attacker's modifier against the target's defence at the weapon's
/// best range band (or the current one).
pub fn potential_attack(state: &CombatState, attacker: CombatantId, weapon: usize, target: CombatantId, mode: AttackMode) -> f64 {
    let (Some(a), Some(t)) = (state.combatant(attacker), state.combatant(target)) else { return 0.0 };
    let Some(w) = a.statblock.weapons.get(weapon) else { return 0.0 };
    if (w.melee && mode != AttackMode::Single) || (mode == AttackMode::Autofire && w.autofire.is_none()) {
        return 0.0;
    }
    let (modifier, _) = state.attack_modifier(attacker, weapon, mode);
    let location = if mode == AttackMode::Aimed { HitLocation::Head } else { HitLocation::Body };
    let sp = rules::effective_sp(t.armor_sp.get(location), w.halves_armor());
    let p_hit = if w.melee {
        p_opposed(modifier, state.evasion_modifier(target).0)
    } else {
        let distance_m = a.position.chebyshev(t.position) * METRES_PER_SQUARE;
        let autofire = mode == AttackMode::Autofire;
        let bands = state.rules.range_bands_m.len();
        let dv = state
            .rules
            .range_band(distance_m)
            .and_then(|b| state.rules.range_dv(w, b, autofire))
            .or_else(|| (0..bands).filter_map(|b| state.rules.range_dv(w, b, autofire)).min());
        let Some(dv) = dv else { return 0.0 };
        let vs_dv = p_beat(modifier, dv);
        if t.can_dodge(&state.rules) {
            vs_dv.min(p_opposed(modifier, state.evasion_modifier(target).0))
        } else {
            vs_dv
        }
    };
    if mode == AttackMode::Autofire {
        // A typical hit margin gives a ×2 multiplier (capped by the weapon);
        // approximated as twice the dice before armor.
        let m = w.autofire.unwrap_or(1).clamp(1, 2) as u32;
        return p_hit * hit_damage(state, &state.rules.autofire_damage.times(m), sp, false);
    }
    p_hit * hit_damage(state, &w.damage, sp, location == HitLocation::Head)
}

fn loaded(c: &Combatant, weapon: usize, rounds: u32) -> bool {
    let w = &c.statblock.weapons[weapon];
    w.magazine == 0 || c.ammo.get(weapon).copied().unwrap_or(0) >= rounds
}

/// Best expected damage of one Action with `weapon` against `target` from
/// the current position (ROF shots, aimed shot or autofire).
pub fn action_damage(state: &CombatState, attacker: CombatantId, weapon: usize, target: CombatantId, include_resources: bool) -> f64 {
    let Some(a) = state.combatant(attacker) else { return 0.0 };
    let Some(w): Option<&Weapon> = a.statblock.weapons.get(weapon) else { return 0.0 };
    let mut modes = vec![(AttackMode::Single, w.rof.max(1) as f64, 1u32)];
    if !w.melee {
        modes.push((AttackMode::Aimed, 1.0, 1));
    }
    if include_resources && w.autofire.is_some() {
        modes.push((AttackMode::Autofire, 1.0, state.rules.autofire_rounds));
    }
    let mut best = 0.0f64;
    for (mode, shots, rounds) in modes {
        let value = match attack_profile(state, attacker, weapon, target, mode) {
            Some(p) if p.blocked_by_cover => potential_attack(state, attacker, weapon, target, mode) * BEHIND_COVER,
            Some(_) => expected_damage(state, attacker, weapon, target, mode),
            // Out of range / no line of sight / not adjacent: what it would
            // be worth once in position (the engagement factor discounts it).
            None => potential_attack(state, attacker, weapon, target, mode),
        } * shots;
        // An empty magazine costs this Action to reload.
        let value = if loaded(a, weapon, rounds) { value } else if mode == AttackMode::Autofire { 0.0 } else { value * 0.5 };
        best = best.max(value);
    }
    best
}

/// Best expected damage `attacker` deals `target` in a full round.
pub fn round_damage(state: &CombatState, attacker: CombatantId, target: CombatantId, include_resources: bool) -> f64 {
    let Some(a) = state.combatant(attacker) else { return 0.0 };
    (0..a.statblock.weapons.len()).map(|w| action_damage(state, attacker, w, target, include_resources)).fold(0.0, f64::max)
}

/// Fraction of a round's damage kept after closing to `target`.
pub fn engagement(state: &CombatState, attacker: CombatantId, target: CombatantId) -> f64 {
    let (Some(a), Some(t)) = (state.combatant(attacker), state.combatant(target)) else { return 0.0 };
    let n = a.statblock.weapons.len();
    let usable = (0..n).any(|w| attack_profile(state, attacker, w, target, AttackMode::Single).is_some());
    if usable {
        return 1.0;
    }
    let has_ranged = a.statblock.weapons.iter().any(|w| !w.melee);
    if has_ranged && state.line_of_sight(a.position, t.position) {
        return 0.8; // out of the band: one Move closes it
    }
    let speed = (a.effective_stat(cpred_core::model::Stat::Move).max(1) * 2) as f64; // Move + Run
    let gap = a.position.chebyshev(t.position).saturating_sub(1) as f64;
    let rounds = (gap / speed).ceil().min(3.0);
    if has_ranged {
        0.8 - 0.1 * rounds.min(2.0)
    } else {
        0.6 + 0.4 * (3.0 - rounds) / 3.0
    }
}

/// Distribution of HP loss from one Action of single attacks (`shots`).
fn shots_distribution(state: &CombatState, attacker: CombatantId, weapon: usize, target: CombatantId, mode: AttackMode, shots: u8) -> Vec<(f64, f64)> {
    let Some(p) = attack_profile(state, attacker, weapon, target, mode) else { return vec![(1.0, 0.0)] };
    if p.blocked_by_cover || !p.has_ammo {
        return vec![(1.0, 0.0)];
    }
    let p_hit = (expected_damage(state, attacker, weapon, target, mode) / hit_damage(state, &p.damage, p.sp, p.head).max(1e-9)).clamp(0.0, 1.0);
    let crit = p_critical_injury(&p.damage);
    let bonus = state.rules.critical_injury_damage as f64;
    let mut one: Vec<(f64, f64)> = vec![(1.0 - p_hit, 0.0)];
    for (d, pd) in dice_sum_distribution(&p.damage) {
        let past = (d - p.sp).max(0) as f64 * if p.head { 2.0 } else { 1.0 };
        one.push((p_hit * pd * (1.0 - crit), past));
        one.push((p_hit * pd * crit, past + bonus));
    }
    let mut acc = vec![(1.0, 0.0)];
    for _ in 0..shots.max(1) {
        let mut next = Vec::with_capacity(acc.len() * one.len());
        for (p0, v0) in &acc {
            for (p1, v1) in &one {
                next.push((p0 * p1, v0 + v1));
            }
        }
        // Merge equal damage totals to keep the convolution small.
        next.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        acc.clear();
        for (p, v) in next {
            match acc.last_mut() {
                Some((pp, vv)) if (*vv - v).abs() < 1e-9 => *pp += p,
                _ => acc.push((p, v)),
            }
        }
    }
    acc
}

/// P(`attacker` drops `target` this turn with its best single weapon line).
pub fn kill_probability(state: &CombatState, attacker: CombatantId, target: CombatantId) -> f64 {
    let (Some(a), Some(t)) = (state.combatant(attacker), state.combatant(target)) else { return 0.0 };
    let acting = state.current_actor() == Some(attacker);
    let e = a.economy;
    let mut best = 0.0f64;
    for (w, weapon) in a.statblock.weapons.iter().enumerate() {
        let lines: Vec<(AttackMode, u8)> = if acting && !e.action_available {
            if e.rof_weapon == Some(w) && e.shots_left > 0 { vec![(AttackMode::Single, e.shots_left)] } else { vec![] }
        } else {
            let mut l = vec![(AttackMode::Single, weapon.rof.max(1))];
            if !weapon.melee {
                l.push((AttackMode::Aimed, 1));
            }
            l
        };
        for (mode, shots) in lines {
            let p: f64 = shots_distribution(state, attacker, w, target, mode, shots).iter().filter(|(_, v)| *v >= t.hp as f64).map(|(p, _)| p).sum();
            best = best.max(p);
        }
    }
    best.clamp(0.0, 1.0)
}
