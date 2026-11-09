//! Expected-value helpers built strictly on `pf2e_core` rule functions.
//! Nothing here re-implements a rule: attack totals, AC, MAP, damage dice,
//! IWR and concealment all come from the core crate.

use pf2e_core::checks::{degree_distribution, flat_check_success_chance, DegreeOfSuccess};
use pf2e_core::combat::actions::cantrip_rank;
use pf2e_core::combat::pending::{AttackCtx, AttackSource, DamageRollSpec};
use pf2e_core::combat::rules;
use pf2e_core::combat::{CombatState, Combatant};
use pf2e_core::damage::{apply_iwr, DamagePart};
use pf2e_core::grid::{creature_distance, within_reach, CoverLevel};
use pf2e_core::models::spell::{SpellDefense, SpellTargeting};
use pf2e_core::models::statblock::StrikeRange;
use pf2e_core::CombatantId;

/// Average damage of a set of roll specs after the target's IWR.
pub fn expected_damage_after_iwr(specs: &[DamageRollSpec], target: &Combatant, multiplier: f64) -> f64 {
    let parts: Vec<DamagePart> = specs
        .iter()
        .map(|s| {
            let avg = s.dice.average().max(1.0) * if s.doubled { 2.0 } else { 1.0 } * multiplier;
            DamagePart { amount: avg.round() as i32, damage_type: s.damage_type, precision: s.precision }
        })
        .collect();
    apply_iwr(&parts, &target.statblock.iwr).total as f64
}

/// Expected damage of one Strike (attack number `n` in the turn).
pub fn expected_strike(state: &CombatState, attacker: CombatantId, target: CombatantId, index: usize, n: u8, cover: CoverLevel) -> f64 {
    let a = state.combatant(attacker).expect("attacker");
    let t = state.combatant(target).expect("target");
    let Some(s) = a.statblock.strikes.get(index) else { return 0.0 };
    let ctx = AttackCtx {
        attacker,
        target,
        source: AttackSource::Strike { index },
        map_penalty: rules::map_penalty(n, s.is_agile()),
        attack_number: n,
        cover,
        disrupts: None,
        sweep: false,
    };
    let nums = rules::attack_numbers(state, &ctx);
    let dist = degree_distribution(nums.modifier, nums.ac);
    let hit = expected_damage_after_iwr(&rules::strike_damage_specs(state, &ctx, false), t, 1.0);
    let crit = expected_damage_after_iwr(&rules::strike_damage_specs(state, &ctx, true), t, 1.0);
    let conceal = rules::concealment_dc(state, attacker, target).map(flat_check_success_chance).unwrap_or(1.0);
    conceal * (dist[DegreeOfSuccess::Success.index()] * hit + dist[DegreeOfSuccess::CriticalSuccess.index()] * crit)
}

/// Expected damage of casting a damaging spell at one target (ignores slots).
pub fn expected_spell(state: &CombatState, caster: CombatantId, target: CombatantId, entry: usize, spell_id: &str, rank: u8) -> f64 {
    let a = state.combatant(caster).expect("caster");
    let t = state.combatant(target).expect("target");
    let Some(spell) = state.spells.get(spell_id) else { return 0.0 };
    let Some(dmg) = &spell.damage else { return 0.0 };
    let Some(e) = a.statblock.spellcasting.get(entry) else { return 0.0 };
    let dice = dmg.at_rank(spell.rank, rank);
    let spec = |doubled| DamageRollSpec { label: String::new(), dice: dice.clone(), damage_type: dmg.damage_type, precision: false, doubled };
    match spell.defense {
        SpellDefense::None => expected_damage_after_iwr(&[spec(false)], t, 1.0),
        SpellDefense::Attack => {
            let ctx = AttackCtx {
                attacker: caster,
                target,
                source: AttackSource::Spell { spell_id: spell_id.to_string(), rank, entry },
                map_penalty: rules::map_penalty(a.attacks_this_turn, false),
                attack_number: a.attacks_this_turn,
                cover: CoverLevel::None,
                disrupts: None,
                sweep: false,
            };
            let nums = rules::attack_numbers(state, &ctx);
            let d = degree_distribution(nums.modifier, nums.ac);
            d[DegreeOfSuccess::Success.index()] * expected_damage_after_iwr(&[spec(false)], t, 1.0)
                + d[DegreeOfSuccess::CriticalSuccess.index()] * expected_damage_after_iwr(&[spec(true)], t, 1.0)
        }
        SpellDefense::Save { save, basic } => {
            let dc = rules::spell_dc(a, entry);
            let (m, _) = rules::save_modifier(t, save);
            let d = degree_distribution(m, dc);
            let full = expected_damage_after_iwr(&[spec(false)], t, 1.0);
            let _ = e;
            let mult = |deg: DegreeOfSuccess| -> f64 {
                use DegreeOfSuccess::*;
                match (basic, deg) {
                    (_, CriticalSuccess) => 0.0,
                    (true, Success) => 0.5,
                    (false, Success) => 0.0,
                    (_, Failure) => 1.0,
                    (true, CriticalFailure) => 2.0,
                    (false, CriticalFailure) => 1.0,
                }
            };
            DegreeOfSuccess::ALL.iter().map(|&deg| d[deg.index()] * mult(deg) * full).sum()
        }
    }
}

/// Number of creatures a spell typically affects (for EV estimates).
pub fn typical_targets(state: &CombatState, spell_id: &str) -> f64 {
    match state.spells.get(spell_id).map(|s| s.targeting) {
        Some(SpellTargeting::Creatures { max, .. }) => (max as f64).min(2.0),
        Some(SpellTargeting::Area { .. }) => 1.8,
        _ => 1.0,
    }
}

/// Best expected damage `attacker` can deal `target` in a full 3-action
/// round from its current position, ignoring movement.
pub fn round_damage(state: &CombatState, attacker: CombatantId, target: CombatantId, include_spells: bool) -> f64 {
    round_damage_with_actions(state, attacker, target, include_spells, 3)
}

/// [`round_damage`] for a round in which the attacker has only `actions`
/// (0..=3) actions to spend on attacking.
pub fn round_damage_with_actions(state: &CombatState, attacker: CombatantId, target: CombatantId, include_spells: bool, actions: usize) -> f64 {
    let actions = actions.min(3);
    let a = state.combatant(attacker).expect("attacker");
    let n = a.statblock.strikes.len();
    let mut per_slot = [0.0f64; 3];
    for (slot, best) in per_slot.iter_mut().enumerate().take(actions) {
        for i in 0..n {
            *best = best.max(expected_strike(state, attacker, target, i, slot as u8, CoverLevel::None));
        }
    }
    let strikes = per_slot.iter().sum::<f64>();
    if !include_spells {
        return strikes;
    }
    let mut best_spell = 0.0f64;
    for (ei, e) in a.statblock.spellcasting.iter().enumerate() {
        let cr = cantrip_rank(a.statblock.level);
        let mut opts: Vec<(&String, u8)> = e.cantrips.iter().map(|c| (c, cr)).collect();
        for k in &e.spells {
            if a.spell_slots.get(ei).and_then(|s| s.get(&k.rank)).copied().unwrap_or(0) > 0 {
                opts.push((&k.spell_id, k.rank));
            }
        }
        for (sid, rank) in opts {
            let cost = state.spells.get(sid).map(|s| s.actions).unwrap_or(2) as usize;
            if cost > actions {
                continue;
            }
            let ev = expected_spell(state, attacker, target, ei, sid, rank) * typical_targets(state, sid);
            let rest: f64 = per_slot.iter().take(actions - cost).sum();
            best_spell = best_spell.max(ev + rest);
        }
    }
    strikes.max(best_spell)
}

/// Share of the rest of the fight a prone creature's next turn represents:
/// it spends that turn's first action to Stand.
pub const STAND_TAX_WEIGHT: f64 = 1.0 / 3.0;
/// Share of the rest of the fight during which a prone creature is
/// off-guard: only attackers who act before its next turn benefit.
pub const PRONE_TARGET_WEIGHT: f64 = 1.0 / 6.0;

/// Share of the rest of the fight that frightened `value` is in force.
/// Frightened drops by 1 at the end of each of the creature's turns, so its
/// penalty summed over the next three rounds is `v + (v−1) + …`; spreading
/// that over three rounds at the full value `v` gives `(v + 1) / 6`.
pub fn frightened_weight(value: u8) -> f64 {
    ((value as f64 + 1.0) / 6.0).min(1.0)
}

/// Expected damage per round of `attacker` against `target` *for the rest
/// of the fight*, which is what position evaluation needs.
///
/// [`round_damage`] reads conditions as they are right now. Some of them
/// last only until the creature's next turn: a prone creature Stands as its
/// first action, and frightened decays each turn. Treating them as
/// permanent made Trip and Demoralize look worth more than a solid hit, so
/// the engine recommended them every turn. Here each such condition counts
/// only for the share of the fight it is actually in force:
///
/// * the baseline is the round with both creatures standing and unafraid;
/// * an attacker that is prone loses one action next turn (Stand), weighted
///   by [`STAND_TAX_WEIGHT`]; the −2 prone attack penalty is never paid,
///   since it stands before attacking;
/// * a prone target is off-guard until it stands ([`PRONE_TARGET_WEIGHT`]);
/// * frightened on either side counts for [`frightened_weight`].
///
/// Every term is still computed by [`round_damage`] from core rules; only
/// the weighting is a heuristic. With no such conditions on either creature
/// this is exactly [`round_damage`].
pub fn sustained_round_damage(state: &CombatState, attacker: CombatantId, target: CombatantId, include_spells: bool) -> f64 {
    use pf2e_core::conditions::ConditionKind as CK;
    let (Some(a), Some(t)) = (state.combatant(attacker), state.combatant(target)) else { return 0.0 };
    // Only conditions held directly count: an unconscious creature is
    // implicitly prone, but it is down and does not Stand.
    let prone = |c: &Combatant| !c.is_down() && c.conditions.get(CK::Prone).is_some();
    let a_prone = prone(a);
    let t_prone = prone(t);
    let a_fright = if a.is_down() { 0 } else { a.conditions.value(CK::Frightened) };
    let t_fright = if t.is_down() { 0 } else { t.conditions.value(CK::Frightened) };
    if !a_prone && !t_prone && a_fright == 0 && t_fright == 0 {
        return round_damage(state, attacker, target, include_spells);
    }

    let mut base = state.clone();
    for id in [attacker, target] {
        if let Some(c) = base.combatants.iter_mut().find(|c| c.id == id) {
            if prone(c) {
                c.conditions.remove(CK::Prone);
            }
            if !c.is_down() {
                c.conditions.remove(CK::Frightened);
            }
        }
    }
    let steady = round_damage(&base, attacker, target, include_spells);
    let mut total = steady;
    let mut variant = |w: f64, edit: &dyn Fn(&mut CombatState), actions: usize| {
        let mut v = base.clone();
        edit(&mut v);
        total += w * (round_damage_with_actions(&v, attacker, target, include_spells, actions) - steady);
    };
    let restore = |id: CombatantId, kind: CK, value: u8| {
        move |s: &mut CombatState| {
            if let Some(c) = s.combatants.iter_mut().find(|c| c.id == id) {
                c.conditions.add(pf2e_core::conditions::Condition::new(kind, value));
            }
        }
    };
    if a_prone {
        variant(STAND_TAX_WEIGHT, &|_| {}, 2);
    }
    if t_prone && attacker != target {
        variant(PRONE_TARGET_WEIGHT, &restore(target, CK::Prone, 0), 3);
    }
    if a_fright > 0 {
        variant(frightened_weight(a_fright), &restore(attacker, CK::Frightened, a_fright), 3);
    }
    if t_fright > 0 && attacker != target {
        variant(frightened_weight(t_fright), &restore(target, CK::Frightened, t_fright), 3);
    }
    total.max(0.0)
}

/// Fraction of a first round the attacker keeps after closing to `target`.
pub fn engagement_factor(state: &CombatState, attacker: CombatantId, target: CombatantId) -> f64 {
    let a = state.combatant(attacker).expect("attacker");
    let t = state.combatant(target).expect("target");
    let dist = creature_distance(a.position, a.statblock.size, t.position, t.statblock.size);
    let mut best_reach = 0u32;
    let mut has_ranged = !a.statblock.spellcasting.is_empty();
    for s in &a.statblock.strikes {
        match s.range {
            StrikeRange::Melee { reach_ft } => {
                if within_reach(a.position, a.statblock.size, t.position, t.statblock.size, reach_ft) {
                    return 1.0;
                }
                best_reach = best_reach.max(reach_ft);
            }
            StrikeRange::Ranged { increment_ft } => {
                if dist <= increment_ft * 2 {
                    has_ranged = true;
                }
            }
        }
    }
    if has_ranged {
        return 0.9;
    }
    let speed = a.statblock.speed.max(5) as f64;
    let moves = ((dist.saturating_sub(best_reach.max(5))) as f64 / speed).ceil().min(3.0);
    0.6 + 0.4 * (3.0 - moves) / 3.0
}

/// Actions a creature will have on its next turn relative to 3.
pub fn action_factor(c: &Combatant) -> f64 {
    use pf2e_core::conditions::ConditionKind as CK;
    if !c.can_act() {
        return 0.0;
    }
    let mut a = 3.0 + if c.has(CK::Quickened) { 1.0 } else { 0.0 };
    let stunned = c.conditions.value(CK::Stunned) as f64;
    if stunned > 0.0 {
        a -= stunned.min(a);
    } else {
        a -= c.conditions.value(CK::Slowed) as f64;
    }
    (a / 3.0).max(0.0)
}
