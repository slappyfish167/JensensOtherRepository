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

/// Raw counts from a range of runs. Tallies from disjoint run ranges
/// [`merge`](SimulationTally::merge) into exactly the result of one
/// [`simulate`] call over the union, because every run's dice seed depends
/// only on `cfg.seed` and the run index. Hosts use this to split a
/// simulation across workers (e.g. one WebAssembly instance per core).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SimulationTally {
    pub runs: u32,
    pub party_wins: u32,
    pub enemy_wins: u32,
    pub timeouts: u32,
    pub tpks: u32,
    pub any_pc_deaths: u32,
    pub total_rounds: u64,
    pub combatants: Vec<CombatantTally>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombatantTally {
    pub id: CombatantId,
    pub name: String,
    pub side: Side,
    pub deaths: u32,
    pub downs: u32,
}

impl SimulationTally {
    fn empty<S: TacticalSystem>(initial: &S::State) -> Self {
        SimulationTally {
            combatants: S::units(initial)
                .into_iter()
                .filter(|u| !u.ignored)
                .map(|u| CombatantTally { id: u.id, name: u.name, side: u.side, deaths: 0, downs: 0 })
                .collect(),
            ..Default::default()
        }
    }

    /// Adds another tally of the same combat (disjoint run range).
    pub fn merge(&mut self, other: &SimulationTally) {
        if self.combatants.is_empty() {
            self.combatants = other.combatants.iter().map(|c| CombatantTally { deaths: 0, downs: 0, ..c.clone() }).collect();
        }
        self.runs += other.runs;
        self.party_wins += other.party_wins;
        self.enemy_wins += other.enemy_wins;
        self.timeouts += other.timeouts;
        self.tpks += other.tpks;
        self.any_pc_deaths += other.any_pc_deaths;
        self.total_rounds += other.total_rounds;
        for c in &mut self.combatants {
            if let Some(o) = other.combatants.iter().find(|o| o.id == c.id) {
                c.deaths += o.deaths;
                c.downs += o.downs;
            }
        }
    }

    pub fn report(&self) -> SimulationReport {
        let n = self.runs.max(1) as f64;
        SimulationReport {
            runs: self.runs,
            party_win_rate: self.party_wins as f64 / n,
            enemy_win_rate: self.enemy_wins as f64 / n,
            timeout_rate: self.timeouts as f64 / n,
            tpk_probability: self.tpks as f64 / n,
            any_pc_death_probability: self.any_pc_deaths as f64 / n,
            average_rounds: self.total_rounds as f64 / n,
            combatants: self
                .combatants
                .iter()
                .map(|c| CombatantOdds { id: c.id, name: c.name.clone(), side: c.side, death_rate: c.deaths as f64 / n, down_rate: c.downs as f64 / n })
                .collect(),
        }
    }
}

/// Plays run number `run` of `cfg` to the end (or past `max_rounds`) and
/// returns the final state. The dice depend only on `cfg.seed` and `run`.
pub fn play_out<S: TacticalSystem>(initial: &S::State, cfg: &SimulationConfig, run: u32) -> S::State {
    let mut dice = SeededDice::new(cfg.seed.wrapping_mul(0x9E37_79B9).wrapping_add(run as u64 * 7919));
    let mut s = initial.clone();
    if S::phase(&s).kind == "setup" {
        let _ = S::apply_in_place(&mut s, &S::start_combat());
    }
    let mut guard = 0;
    while !S::is_over(&s) && S::round(&s) <= cfg.max_rounds && guard < 50_000 {
        guard += 1;
        let cmd = if S::pending(&s).is_some() {
            match S::node(&s) {
                Node::Decision { accept, .. } => accept,
                _ => match S::auto_answer(&s, &mut dice) {
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

/// Runs `start .. start + count` of the simulation described by `cfg`.
pub fn simulate_range<S: TacticalSystem>(initial: &S::State, cfg: SimulationConfig, start: u32, count: u32) -> SimulationTally {
    let mut t = SimulationTally::empty::<S>(initial);
    for run in start..start.saturating_add(count) {
        let s = play_out::<S>(initial, &cfg, run);
        t.runs += 1;
        t.total_rounds += S::round(&s) as u64;
        match S::winner(&s) {
            Some(Some(Side::Party)) => t.party_wins += 1,
            Some(Some(Side::Enemy)) => t.enemy_wins += 1,
            _ => t.timeouts += 1,
        }
        let units = S::units(&s);
        let party: Vec<_> = units.iter().filter(|c| c.side == Side::Party && !c.ignored).collect();
        if !party.is_empty() && party.iter().all(|c| c.down) {
            t.tpks += 1;
        }
        if party.iter().any(|c| c.dead) {
            t.any_pc_deaths += 1;
        }
        for ct in &mut t.combatants {
            if let Some(c) = units.iter().find(|u| u.id == ct.id) {
                ct.deaths += c.dead as u32;
                ct.downs += c.down as u32;
            }
        }
    }
    t
}

pub fn simulate<S: TacticalSystem>(initial: &S::State, cfg: SimulationConfig) -> SimulationReport {
    simulate_range::<S>(initial, cfg, 0, cfg.runs).report()
}
