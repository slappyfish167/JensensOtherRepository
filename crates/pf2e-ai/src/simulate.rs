//! PF2e Monte Carlo policy. The driver (seeded dice, play-out, statistics)
//! is system-neutral and lives in `tactics_ai::simulate`; this module only
//! supplies the fast scripted policy the simulated creatures follow.

use crate::analysis::{expected_spell, expected_strike, typical_targets};
use crate::movegen::{candidates, MoveGenConfig};
use pf2e_core::combat::{Action, CombatState};
use pf2e_core::grid::CoverLevel;
use pf2e_core::stats::Side;
pub use tactics_ai::simulate::{CombatantOdds, CombatantTally, SimulationConfig, SimulationReport, SimulationTally};

/// Fast heuristic policy: highest immediate expected value, otherwise close
/// the distance, otherwise end the turn.
pub fn scripted_action(s: &CombatState) -> Option<Action> {
    let actor = s.current_actor()?;
    let a = s.combatant(actor)?;
    if a.actions_remaining == 0 {
        return None;
    }
    let legal = s.legal_actions();
    let mut best: Option<(f64, Action)> = None;
    for act in &legal {
        let v = match act {
            Action::Strike { actor_strike, target, .. } => {
                let t = s.combatant(*target)?;
                let ev = expected_strike(s, actor, *target, *actor_strike, a.attacks_this_turn, CoverLevel::None);
                // Prefer finishing blows and wounded targets.
                let kill = if ev >= t.hp as f64 { 3.0 } else { 0.0 };
                if t.is_down() { -1.0 } else { ev + kill + (1.0 - t.hp as f64 / t.statblock.max_hp.max(1) as f64) }
            }
            Action::CastSpell { entry, spell_id, rank, targets, .. } => {
                let Some(spell) = s.spells.get(spell_id) else { continue };
                if let Some(h) = &spell.healing {
                    let t = targets.first().and_then(|t| s.combatant(*t));
                    match t {
                        Some(t) if (t.hp as f64) < 0.5 * t.statblock.max_hp as f64 => h.at_rank(spell.rank, *rank).average() * 1.2,
                        _ => -1.0,
                    }
                } else if spell.damage.is_some() {
                    let n = if targets.is_empty() { typical_targets(s, spell_id) } else { targets.len() as f64 };
                    let tgt = targets.first().copied().or_else(|| s.enemies_of(actor).find(|e| !e.is_down()).map(|e| e.id));
                    tgt.map(|t| expected_spell(s, actor, t, *entry, spell_id, *rank) * n.max(1.0) * 0.5).unwrap_or(0.0)
                } else if a.attacks_this_turn == 0 && !spell.cantrip && targets.iter().any(|t| s.combatant(*t).map(|c| a.side.is_hostile_to(c.side)).unwrap_or(false)) {
                    3.0
                } else {
                    -1.0
                }
            }
            Action::RaiseShield => if a.actions_remaining == 1 { 1.0 } else { -1.0 },
            Action::Stand => 4.0,
            _ => continue,
        };
        if v > 0.75 && best.as_ref().map(|b| v > b.0).unwrap_or(true) {
            best = Some((v, act.clone()));
        }
    }
    if let Some((_, act)) = best {
        return Some(act);
    }
    // Close in: first movement candidate is the best approach square.
    let approach = candidates(s, &MoveGenConfig { max_moves: 1, focus: None, committed: false })
        .into_iter()
        .find(|c| matches!(c, Action::Stride { .. }))
        .filter(|_| a.attacks_this_turn == 0);
    if approach.is_some() || a.side != Side::Enemy {
        return approach;
    }
    // Hostile creatures never idle while a legal action remains: take the
    // best remaining attack on a standing foe, else shore up defence, else
    // anything legal.
    let attack = legal
        .iter()
        .filter_map(|act| match act {
            Action::Strike { actor_strike, target, .. } => {
                let t = s.combatant(*target)?;
                (!t.is_down()).then(|| (expected_strike(s, actor, *target, *actor_strike, a.attacks_this_turn, CoverLevel::None), act))
            }
            Action::Demoralize { target } | Action::Trip { target } => s.combatant(*target).filter(|t| !t.is_down()).map(|_| (0.1, act)),
            _ => None,
        })
        .filter(|(v, _)| *v > 0.0)
        .max_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, act)| act.clone());
    attack
        .or_else(|| legal.iter().find(|x| matches!(x, Action::RaiseShield | Action::TakeCover)).cloned())
        .or_else(|| candidates(s, &MoveGenConfig { max_moves: 1, focus: None, committed: true }).into_iter().next())
}

/// Plays run `run` of `cfg` to the end and returns the final state.
pub fn play_out(initial: &CombatState, cfg: &SimulationConfig, run: u32) -> CombatState {
    tactics_ai::simulate::play_out::<crate::system::Pf2e>(initial, cfg, run)
}

/// Runs `start .. start + count` of the simulation described by `cfg`.
pub fn simulate_range(initial: &CombatState, cfg: SimulationConfig, start: u32, count: u32) -> SimulationTally {
    tactics_ai::simulate::simulate_range::<crate::system::Pf2e>(initial, cfg, start, count)
}

/// Monte Carlo over the PF2e combat (see `tactics_ai::simulate`).
pub fn simulate(initial: &CombatState, cfg: SimulationConfig) -> SimulationReport {
    tactics_ai::simulate::simulate::<crate::system::Pf2e>(initial, cfg)
}
