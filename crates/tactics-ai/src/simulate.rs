//! Monte Carlo outcome estimation ("How likely is a TPK?") for any system.
//!
//! Each run plays the combat to the end with seeded dice (explicitly opted
//! into — this is analysis, not the live table) and the system's fast
//! scripted policy for every creature. The engine resolves everything; the
//! policy only picks among legal actions.

use crate::system::{Node, TacticalSystem};
use serde::{Deserialize, Serialize};
use tactics_core::{CombatantId, SeededDice, Side};

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
    /// Every party member down at the end.
    pub tpk_probability: f64,
    /// At least one party member dead at the end.
    pub any_pc_death_probability: f64,
    pub average_rounds: f64,
    pub combatants: Vec<CombatantOdds>,
}

/// Plays one combat to the end (or past `max_rounds`) with seeded dice.
pub fn play_out<S: TacticalSystem>(initial: &S::State, dice: &mut SeededDice, max_rounds: u32) -> S::State {
    let mut s = initial.clone();
    if S::phase(&s).kind == "setup" {
        let _ = S::apply_in_place(&mut s, &S::start_combat());
    }
    let mut guard = 0;
    while !S::is_over(&s) && S::round(&s) <= max_rounds && guard < 50_000 {
        guard += 1;
        let cmd = if S::pending(&s).is_some() {
            match S::node(&s) {
                Node::Decision { accept, .. } => accept,
                _ => match S::auto_answer(&s, dice) {
                    Some(c) => c,
                    None => break,
                },
            }
        } else {
            let Some(actor) = S::current_actor(&s) else { break };
            match S::scripted_action(&s) {
                Some(action) => S::act(actor, action),
                None => S::end_turn(actor),
            }
        };
        if S::apply_in_place(&mut s, &cmd).is_err() {
            // A scripted action the engine refused: end the turn instead.
            let Some(actor) = S::current_actor(&s) else { break };
            if S::apply_in_place(&mut s, &S::end_turn(actor)).is_err() {
                break;
            }
        }
    }
    s
}

pub fn simulate<S: TacticalSystem>(initial: &S::State, cfg: SimulationConfig) -> SimulationReport {
    let roster: Vec<_> = S::units(initial).into_iter().filter(|u| !u.ignored).collect();
    let mut deaths = vec![0u32; roster.len()];
    let mut downs = vec![0u32; roster.len()];
    let (mut pw, mut ew, mut to, mut tpk, mut any_death, mut rounds) = (0u32, 0u32, 0u32, 0u32, 0u32, 0f64);

    for run in 0..cfg.runs {
        let mut dice = SeededDice::new(cfg.seed.wrapping_mul(0x9E37_79B9).wrapping_add(run as u64 * 7919));
        let s = play_out::<S>(initial, &mut dice, cfg.max_rounds);
        rounds += S::round(&s) as f64;
        match S::winner(&s) {
            Some(Some(Side::Party)) => pw += 1,
            Some(Some(Side::Enemy)) => ew += 1,
            _ => to += 1,
        }
        let units = S::units(&s);
        let party: Vec<_> = units.iter().filter(|c| c.side == Side::Party && !c.ignored).collect();
        if !party.is_empty() && party.iter().all(|c| c.down) {
            tpk += 1;
        }
        if party.iter().any(|c| c.dead) {
            any_death += 1;
        }
        for (i, r) in roster.iter().enumerate() {
            if let Some(c) = units.iter().find(|u| u.id == r.id) {
                deaths[i] += c.dead as u32;
                downs[i] += c.down as u32;
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
        average_rounds: rounds / n,
        combatants: roster
            .into_iter()
            .enumerate()
            .map(|(i, u)| CombatantOdds { id: u.id, name: u.name, side: u.side, death_rate: deaths[i] as f64 / n, down_rate: downs[i] as f64 / n })
            .collect(),
    }
}
