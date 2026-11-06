//! Rules-derived probability math for an AI layer (no search here).
//!
//! * [`exploding_d10`], [`p_beat`], [`p_opposed`] — the check die.
//! * [`attack_profile`], [`expected_damage`] — the numbers of one attack.
//! * [`representative_answers`] — a small weighted set of answers to the
//!   pending roll, so expectiminimax search need not enumerate every face.

use crate::combat::{CheckRole, CheckRoll, Command, CombatState, Step};
use crate::model::HitLocation;
use crate::rules;
use std::collections::BTreeMap;
use tactics_core::{CombatantId, DiceExpr};

pub use crate::rules::AttackMode;

// ---------------------------------------------------------------------------
// The check die
// ---------------------------------------------------------------------------

/// Exact distribution of the d10 of a check including critical follow-ups:
/// faces 2..9 at 0.1 each, `10 + k` and `1 − k` (k = 1..10) at 0.01 each.
/// Sorted by value.
pub fn exploding_d10() -> Vec<(i32, f64)> {
    let mut out: Vec<(i32, f64)> = Vec::with_capacity(28);
    out.extend((1..=10).rev().map(|k| (1 - k, 0.01)));
    out.extend((2..=9).map(|f| (f, 0.1)));
    out.extend((1..=10).map(|k| (10 + k, 0.01)));
    out
}

/// P(modifier + X > beat) for the exploding check die X.
pub fn p_beat(modifier: i32, beat: i32) -> f64 {
    exploding_d10().into_iter().filter(|(x, _)| modifier + x > beat).map(|(_, p)| p).sum()
}

/// P(attacker total > defender total), both rolling exploding d10s.
pub fn p_opposed(att_mod: i32, def_mod: i32) -> f64 {
    margin_opposed(att_mod, def_mod).values().sum()
}

/// Distribution of the positive margins `att_mod + X − beat` (misses dropped).
fn margin_vs(att_mod: i32, beat: i32) -> BTreeMap<i32, f64> {
    let mut out = BTreeMap::new();
    for (x, p) in exploding_d10() {
        let m = att_mod + x - beat;
        if m > 0 {
            *out.entry(m).or_insert(0.0) += p;
        }
    }
    out
}

/// Distribution of positive margins of an opposed roll.
fn margin_opposed(att_mod: i32, def_mod: i32) -> BTreeMap<i32, f64> {
    let mut out = BTreeMap::new();
    for (d, pd) in exploding_d10() {
        for (m, pm) in margin_vs(att_mod, def_mod + d) {
            *out.entry(m).or_insert(0.0) += pd * pm;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Attack profiles
// ---------------------------------------------------------------------------

/// The numbers of one possible attack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackProfile {
    /// Attacker check modifier (STAT + skill + penalties).
    pub modifier: i32,
    /// Range DV (ranged attacks).
    pub dv: Option<i32>,
    /// Defender's DEX + Evasion: melee defence, or dodge if `can_dodge`.
    pub defender_modifier: Option<i32>,
    pub can_dodge: bool,
    /// A hit would only damage cover.
    pub blocked_by_cover: bool,
    pub location: HitLocation,
    /// Damage dice (for autofire: the dice before the multiplier).
    pub damage: DiceExpr,
    /// Effective SP at the location (after halving).
    pub sp: i32,
    pub head: bool,
    pub autofire_max: Option<u8>,
    /// Enough ammunition is loaded for this attack.
    pub has_ammo: bool,
}

/// Profile of an attack. `None` if the attack is impossible regardless of the
/// turn economy and ammunition (unknown combatant or weapon, range, line of
/// sight, melee reach, wrong mode). Ammunition is reported in `has_ammo`.
pub fn attack_profile(state: &CombatState, attacker: CombatantId, weapon: usize, target: CombatantId, mode: AttackMode) -> Option<AttackProfile> {
    let plan = state.plan_attack(attacker, weapon, target, mode).ok()?;
    let t = state.combatant(target)?;
    let (modifier, _) = state.attack_modifier(attacker, weapon, mode);
    let melee = plan.weapon.melee;
    let can_dodge = !melee && t.can_dodge(&state.rules);
    let defender_modifier = (melee || can_dodge).then(|| state.evasion_modifier(target).0);
    let damage = if mode == AttackMode::Autofire { state.rules.autofire_damage.clone() } else { plan.weapon.damage.clone() };
    Some(AttackProfile {
        modifier,
        dv: plan.dv,
        defender_modifier,
        can_dodge,
        blocked_by_cover: plan.cover.is_some(),
        location: plan.location,
        damage,
        sp: rules::effective_sp(t.armor_sp.get(plan.location), plan.weapon.halves_armor()),
        head: plan.location == HitLocation::Head,
        autofire_max: if mode == AttackMode::Autofire { plan.weapon.autofire } else { None },
        has_ammo: plan.has_ammo(),
    })
}

/// Distribution of the hit margin (attacker total − number to beat, positive
/// only). For ranged attacks against a dodger, the defender is assumed to
/// dodge iff that lowers the hit chance.
fn hit_margins(p: &AttackProfile) -> BTreeMap<i32, f64> {
    match (p.dv, p.defender_modifier) {
        (None, Some(def)) => margin_opposed(p.modifier, def),
        (Some(dv), Some(def)) if p.can_dodge => {
            let vs_dv = margin_vs(p.modifier, dv);
            let vs_dodge = margin_opposed(p.modifier, def);
            if vs_dodge.values().sum::<f64>() < vs_dv.values().sum::<f64>() {
                vs_dodge
            } else {
                vs_dv
            }
        }
        (Some(dv), _) => margin_vs(p.modifier, dv),
        (None, None) => BTreeMap::new(),
    }
}

/// Exact distribution of the face sum of `dice` (flat modifier included).
pub fn dice_sum_distribution(dice: &DiceExpr) -> BTreeMap<i32, f64> {
    let mut dist = BTreeMap::from([(dice.modifier, 1.0)]);
    for t in &dice.terms {
        for _ in 0..t.count {
            let mut next = BTreeMap::new();
            for (s, p) in &dist {
                for f in 1..=t.sides as i32 {
                    *next.entry(s + f).or_insert(0.0) += p / t.sides as f64;
                }
            }
            dist = next;
        }
    }
    dist
}

/// P(two or more 6s) on the d6 terms of `dice`.
pub fn p_critical_injury(dice: &DiceExpr) -> f64 {
    let n: u32 = dice.terms.iter().filter(|t| t.sides == 6).map(|t| t.count).sum();
    if n < 2 {
        return 0.0;
    }
    let q: f64 = 5.0 / 6.0;
    1.0 - q.powi(n as i32) - n as f64 * (1.0 / 6.0) * q.powi(n as i32 - 1)
}

/// Expected HP loss of one attack: hit chance × (armor-reduced damage,
/// doubled on head hits) + critical-injury bonus damage; autofire uses the
/// exact multiplier distribution. 0 if impossible or blocked by cover.
/// Not capped at the target's remaining HP.
pub fn expected_damage(state: &CombatState, attacker: CombatantId, weapon: usize, target: CombatantId, mode: AttackMode) -> f64 {
    let Some(p) = attack_profile(state, attacker, weapon, target, mode) else { return 0.0 };
    if p.blocked_by_cover {
        return 0.0;
    }
    let margins = hit_margins(&p);
    let p_hit: f64 = margins.values().sum();
    let dist = dice_sum_distribution(&p.damage);
    let head = if p.head { HitLocation::Head } else { HitLocation::Body };
    let through = |dmg: i32| rules::armor_result(dmg, p.sp, head).0 as f64;
    let crit = p_critical_injury(&p.damage) * state.rules.critical_injury_damage as f64;
    let armor_part = match p.autofire_max {
        Some(max) => margins
            .iter()
            .map(|(m, pm)| {
                let mult = rules::autofire_multiplier(*m, 0, max);
                pm * dist.iter().map(|(s, ps)| ps * through(s * mult)).sum::<f64>()
            })
            .sum(),
        None => p_hit * dist.iter().map(|(s, ps)| ps * through(*s)).sum::<f64>(),
    };
    armor_part + p_hit * crit
}

// ---------------------------------------------------------------------------
// Representative answers for search
// ---------------------------------------------------------------------------

/// Weighted answers to the pending roll (weights sum to 1). Empty if nothing
/// or a decision is pending. Every command is accepted by `apply`.
pub fn representative_answers(state: &CombatState) -> Vec<(Command, f64)> {
    let Some(step) = state.queue.front() else { return vec![] };
    let id = state.input_seq;
    let face = |f: i32, p: f64| (Command::roll(id, f), p);
    match step {
        Step::RollInitiative { .. } => (1..=10).map(|f| face(f, 0.1)).collect(),
        Step::DeathSave { combatant } => {
            let threshold = state.death_save_threshold(*combatant).clamp(0, 9);
            let p_ok = threshold as f64 / 10.0;
            let mut out = vec![];
            if threshold >= 1 {
                out.push(face(1, p_ok));
            }
            out.push(face(10, 1.0 - p_ok));
            out
        }
        Step::Check { roll } => match roll.beat {
            Some(beat) => {
                let mut out = grouped(state, roll, (2..=9).map(|f| (f, roll.modifier + f)), beat, 0.1, id);
                out.push(face(10, 0.1));
                out.push(face(1, 0.1));
                out
            }
            None => buckets(&[(&[2, 3, 4], 3), (&[5, 6, 7], 6), (&[8, 9], 8), (&[1], 1), (&[10], 10)], id),
        },
        Step::CritFollowUp { roll, subtotal, sign, .. } => match (roll.beat, &roll.role) {
            (Some(beat), CheckRole::Attack { .. }) => grouped(state, roll, (1..=10).map(|f| (f, subtotal + sign * f)), beat, 0.1, id),
            _ => buckets(&[(&[1, 2, 3], 2), (&[4, 5, 6, 7], 5), (&[8, 9, 10], 9)], id),
        },
        Step::DamageRoll { dice, .. } => damage_answers(dice, id),
        Step::DodgeDecision { .. } | Step::FinalizeInitiative | Step::BeginTurn | Step::EndTurn { .. } | Step::AdvanceTurn | Step::BeginAttack { .. } => {
            vec![]
        }
    }
}

/// Groups `(face, total)` pairs by outcome against `beat` (success/failure,
/// or the autofire multiplier) with one representative (smallest) face each.
fn grouped(state: &CombatState, roll: &CheckRoll, totals: impl Iterator<Item = (i32, i32)>, beat: i32, p_each: f64, id: u32) -> Vec<(Command, f64)> {
    let max = roll.autofire_max(state);
    let mut groups: BTreeMap<i32, (i32, f64)> = BTreeMap::new();
    for (f, total) in totals {
        let key = match max {
            Some(m) => rules::autofire_multiplier(total, beat, m),
            None => rules::beats(total, beat) as i32,
        };
        let g = groups.entry(key).or_insert((f, 0.0));
        g.0 = g.0.min(f);
        g.1 += p_each;
    }
    groups.into_values().map(|(f, p)| (Command::roll(id, f), p)).collect()
}

fn buckets(spec: &[(&[i32], i32)], id: u32) -> Vec<(Command, f64)> {
    spec.iter().map(|(faces, rep)| (Command::roll(id, *rep), faces.len() as f64 / 10.0)).collect()
}

/// "No critical" and "critical" damage branches with conditional-mean faces.
fn damage_answers(dice: &DiceExpr, id: u32) -> Vec<(Command, f64)> {
    let sides: Vec<i32> = dice.terms.iter().flat_map(|t| (0..t.count).map(move |_| t.sides as i32)).collect();
    // Joint distribution of (number of sixes on d6 dice, capped at 2; face sum).
    let mut dist: BTreeMap<(u8, i32), f64> = BTreeMap::from([((0, 0), 1.0)]);
    for &s in &sides {
        let mut next = BTreeMap::new();
        for (&(six, sum), &p) in &dist {
            for f in 1..=s {
                let six = if s == 6 && f == 6 { (six + 1).min(2) } else { six };
                *next.entry((six, sum + f)).or_insert(0.0) += p / s as f64;
            }
        }
        dist = next;
    }
    let (mut p_crit, mut sum_crit, mut sum_plain) = (0.0, 0.0, 0.0);
    for (&(six, sum), &p) in &dist {
        if six >= 2 {
            p_crit += p;
            sum_crit += p * sum as f64;
        } else {
            sum_plain += p * sum as f64;
        }
    }
    let d6_count = sides.iter().filter(|&&s| s == 6).count();
    let p_plain = 1.0 - p_crit;
    let mut out = vec![];
    if p_plain > 0.0 {
        let target = (sum_plain / p_plain).round() as i32;
        out.push((Command::faces(id, plain_faces(&sides, target)), if d6_count < 2 { 1.0 } else { p_plain }));
    }
    if d6_count >= 2 && p_crit > 0.0 {
        let target = (sum_crit / p_crit).round() as i32;
        out.push((Command::faces(id, crit_faces(&sides, target)), p_crit));
    }
    out
}

/// Raises faces round-robin (each up to its cap) until they sum to `target`.
fn spread(faces: &mut [i32], caps: &[i32], target: i32) {
    let mut remaining = target - faces.iter().sum::<i32>();
    while remaining > 0 {
        let mut progressed = false;
        for (f, cap) in faces.iter_mut().zip(caps) {
            if remaining > 0 && *f < *cap {
                *f += 1;
                remaining -= 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
}

/// Faces summing to `target` with at most one 6 on the d6 dice.
fn plain_faces(sides: &[i32], target: i32) -> Vec<i32> {
    let mut faces = vec![1; sides.len()];
    let caps: Vec<i32> = sides.iter().map(|&s| if s == 6 { 5 } else { s }).collect();
    spread(&mut faces, &caps, target);
    let short = target - faces.iter().sum::<i32>();
    if short > 0 {
        if let Some(i) = sides.iter().position(|&s| s == 6) {
            faces[i] = (faces[i] + short).min(6);
        }
    }
    faces
}

/// Faces summing to `target` with the first two d6 dice showing 6.
fn crit_faces(sides: &[i32], target: i32) -> Vec<i32> {
    let mut faces = vec![1; sides.len()];
    let mut caps: Vec<i32> = sides.to_vec();
    for i in sides.iter().enumerate().filter(|(_, &s)| s == 6).map(|(i, _)| i).take(2) {
        faces[i] = 6;
        caps[i] = 6;
    }
    spread(&mut faces, &caps, target);
    faces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exploding_distribution_sums_to_one() {
        let d = exploding_d10();
        assert_eq!(d.len(), 28);
        let total: f64 = d.iter().map(|(_, p)| p).sum();
        assert!((total - 1.0).abs() < 1e-12);
        assert!(d.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(d.first().map(|x| x.0), Some(-9));
        assert_eq!(d.last().map(|x| x.0), Some(20));
    }

    #[test]
    fn p_beat_hand_computed() {
        // Need X > 5: faces 6..9 (0.4) + every exploding 10 (0.1) = 0.5.
        assert!((p_beat(10, 15) - 0.5).abs() < 1e-12);
        // Need X > 9: only exploding 10s.
        assert!((p_beat(10, 19) - 0.1).abs() < 1e-12);
        // Need X > 15: 10 + k with k ≥ 6 → 5 × 0.01.
        assert!((p_beat(10, 25) - 0.05).abs() < 1e-12);
        // Need X > 0: faces 2..9, 10s and 1−k > 0 never → 0.8 + 0.1 = 0.9.
        assert!((p_beat(10, 10) - 0.9).abs() < 1e-12);
        assert_eq!(p_beat(0, 100), 0.0);
    }

    #[test]
    fn opposed_symmetry() {
        let p = p_opposed(10, 10);
        let q = p_opposed(10, 10);
        assert!((p - q).abs() < 1e-12);
        // Ties go to the defender, so equal modifiers give the attacker < 1/2.
        assert!(p < 0.5 && p > 0.4);
        assert!(p_opposed(15, 10) > p_opposed(10, 10));
    }

    #[test]
    fn damage_branches() {
        let dice: DiceExpr = "5d6".parse().unwrap();
        let ans = damage_answers(&dice, 1);
        assert_eq!(ans.len(), 2);
        let total: f64 = ans.iter().map(|(_, p)| p).sum();
        assert!((total - 1.0).abs() < 1e-9);
        assert!((ans[1].1 - p_critical_injury(&dice)).abs() < 1e-9);
        let one: DiceExpr = "1d6".parse().unwrap();
        assert_eq!(damage_answers(&one, 1).len(), 1);
    }
}
