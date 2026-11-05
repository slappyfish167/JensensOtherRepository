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
    candidates(s, &MoveGenConfig { max_moves: 1, focus: None })
        .into_iter()
        .find(|c| matches!(c, Action::Stride { .. }))
        .filter(|_| a.attacks_this_turn == 0)
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
    fn empty(initial: &CombatState) -> Self {
        SimulationTally {
            combatants: initial
                .combatants
                .iter()
                .filter(|c| !c.is_hazard())
                .map(|c| CombatantTally { id: c.id, name: c.name.clone(), side: c.side, deaths: 0, downs: 0 })
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

/// Plays one run to the end (or the round limit) and returns the final state.
pub fn play_out(initial: &CombatState, cfg: &SimulationConfig, run: u32) -> CombatState {
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
    s
}

/// Runs `start .. start + count` of the simulation described by `cfg`.
pub fn simulate_range(initial: &CombatState, cfg: SimulationConfig, start: u32, count: u32) -> SimulationTally {
    let mut t = SimulationTally::empty(initial);
    for run in start..start.saturating_add(count) {
        let s = play_out(initial, &cfg, run);
        t.runs += 1;
        t.total_rounds += s.round as u64;
        match s.phase {
            Phase::Ended { winner: Some(Side::Party) } => t.party_wins += 1,
            Phase::Ended { winner: Some(Side::Enemy) } => t.enemy_wins += 1,
            _ => t.timeouts += 1,
        }
        let party: Vec<_> = s.combatants.iter().filter(|c| c.side == Side::Party && !c.is_hazard()).collect();
        if !party.is_empty() && party.iter().all(|c| c.is_down()) {
            t.tpks += 1;
        }
        if party.iter().any(|c| c.is_dead()) {
            t.any_pc_deaths += 1;
        }
        for ct in &mut t.combatants {
            if let Some(c) = s.combatant(ct.id) {
                if c.is_dead() {
                    ct.deaths += 1;
                }
                if c.is_down() {
                    ct.downs += 1;
                }
            }
        }
    }
    t
}

pub fn simulate(initial: &CombatState, cfg: SimulationConfig) -> SimulationReport {
    simulate_range(initial, cfg, 0, cfg.runs).report()
}
