//! Monte Carlo outcome estimation ("How likely is a TPK?").
//!
//! Each run plays the combat to the end with seeded dice (explicitly opted
//! into — this is analysis, not the live table) and a fast scripted policy
//! for every creature. The engine resolves everything; the policy only picks
//! among `legal_actions`.

use crate::analysis::{expected_spell, expected_strike, typical_targets};
use crate::movegen::{candidates, MoveGenConfig};
use pf2e_core::combat::{Action, CombatState, Command, PendingInput, Phase};
use pf2e_core::grid::CoverLevel;
use pf2e_core::rng::SeededDice;
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SimulationConfig {
    pub runs: u32,
    pub seed: u64,
    pub max_rounds: u32,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        SimulationConfig { runs: 200, seed: 1, max_rounds: 20 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatantOdds {
    pub id: CombatantId,
    pub name: String,
    pub side: Side,
    pub death_rate: f64,
    pub down_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationReport {
    pub runs: u32,
    pub party_win_rate: f64,
    pub enemy_win_rate: f64,
    pub timeout_rate: f64,
    /// Every party member down (unconscious or dead) at the end.
    pub tpk_probability: f64,
    /// At least one party member dead at the end.
    pub any_pc_death_probability: f64,
    pub average_rounds: f64,
    pub combatants: Vec<CombatantOdds>,
}

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

pub fn simulate(initial: &CombatState, cfg: SimulationConfig) -> SimulationReport {
    let ids: Vec<_> = initial.combatants.iter().filter(|c| !c.is_hazard()).map(|c| (c.id, c.name.clone(), c.side)).collect();
    let mut deaths = vec![0u32; ids.len()];
    let mut downs = vec![0u32; ids.len()];
    let (mut pw, mut ew, mut to, mut tpk, mut any_death, mut rounds) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u64);

    for run in 0..cfg.runs {
        let mut dice = SeededDice::new(cfg.seed.wrapping_mul(0x9E37_79B9).wrapping_add(run as u64 * 7919));
        let mut s = initial.clone();
        if s.phase == Phase::Setup {
            let _ = s.apply_in_place(&Command::StartCombat);
        }
        let mut guard = 0;
        while !s.is_over() && s.round <= cfg.max_rounds && guard < 20_000 {
            guard += 1;
            let cmd = match s.pending() {
                Some(PendingInput::Roll(r)) => Command::ProvideRoll { request_id: r.request_id, value: dice.answer(&r), extra_modifier: 0 },
                Some(PendingInput::Decision(d)) => Command::Decide { request_id: d.request_id, accept: true },
                None => {
                    let Some(actor) = s.current_actor() else { break };
                    match scripted_action(&s) {
                        Some(action) => Command::Act { actor, action },
                        None => Command::EndTurn { actor },
                    }
                }
            };
            if s.apply_in_place(&cmd).is_err() {
                if let Some(actor) = s.current_actor() {
                    if s.apply_in_place(&Command::EndTurn { actor }).is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        rounds += s.round as u64;
        match s.phase {
            Phase::Ended { winner: Some(Side::Party) } => pw += 1,
            Phase::Ended { winner: Some(Side::Enemy) } => ew += 1,
            _ => to += 1,
        }
        let party: Vec<_> = s.combatants.iter().filter(|c| c.side == Side::Party && !c.is_hazard()).collect();
        if !party.is_empty() && party.iter().all(|c| c.is_down()) {
            tpk += 1;
        }
        if party.iter().any(|c| c.is_dead()) {
            any_death += 1;
        }
        for (i, (id, _, _)) in ids.iter().enumerate() {
            if let Some(c) = s.combatant(*id) {
                if c.is_dead() {
                    deaths[i] += 1;
                }
                if c.is_down() {
                    downs[i] += 1;
                }
            }
        }
    }
    let n = cfg.runs.max(1) as f64;
    SimulationReport {
        runs: cfg.runs,
        party_win_rate: pw as f64 / n,
        enemy_win_rate: ew as f64 / n,
        timeout_rate: to as f64 / n,
        tpk_probability: tpk as f64 / n,
        any_pc_death_probability: any_death as f64 / n,
        average_rounds: rounds as f64 / n,
        combatants: ids
            .into_iter()
            .enumerate()
            .map(|(i, (id, name, side))| CombatantOdds { id, name, side, death_rate: deaths[i] as f64 / n, down_rate: downs[i] as f64 / n })
            .collect(),
    }
}
