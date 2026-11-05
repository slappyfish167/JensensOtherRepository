//! Position evaluation on a Stockfish-like scale:
//! `+10` overwhelming player advantage … `0` balanced … `-10` near-certain defeat.
//!
//! Model (Lanchester's square law): a side's fighting strength is
//! `H × O`, its effective hit points times its expected damage per round.
//! The log-ratio of the two strengths is the *logit*; the score is
//! `10 · tanh(logit / 2)` and the win estimate is `σ(1.6 · logit)`.
//!
//! Conditions (frightened, off-guard, slowed, ...), cover and spell slots
//! enter through the core rules that compute attack totals, AC and the
//! available actions, so the evaluator never hard-codes a condition.

use crate::analysis::{action_factor, engagement_factor, round_damage};
use pf2e_core::combat::{CombatState, Phase};
use pf2e_core::conditions::ConditionKind as CK;
use pf2e_core::stats::Side;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Factor {
    pub name: String,
    /// Contribution in evaluation units (positive favours the party).
    pub value: f64,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    /// −10..+10, positive favours the party.
    pub score: f64,
    pub win_probability: f64,
    pub logit: f64,
    pub terminal: bool,
    pub factors: Vec<Factor>,
    pub party: SideSummary,
    pub enemies: SideSummary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SideSummary {
    pub hit_points: f64,
    pub damage_per_round: f64,
    pub standing: usize,
    pub down: usize,
}

/// Anything that can score a position. Search depends only on this trait,
/// so alternative evaluators (learned, scripted) can be swapped in.
pub trait Evaluator: Send + Sync {
    fn evaluate(&self, state: &CombatState) -> f64;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HeuristicEvaluator {
    /// Logit units per score unit (tanh scaling).
    pub scale: f64,
    pub include_spells: bool,
    pub include_position: bool,
    pub include_tempo: bool,
}

impl Default for HeuristicEvaluator {
    fn default() -> Self {
        HeuristicEvaluator { scale: 2.0, include_spells: true, include_position: true, include_tempo: true }
    }
}

const EPS: f64 = 0.5;
/// Expected ticks of persistent damage with a DC 15 flat check (1 / 0.3).
const PERSISTENT_TICKS: f64 = 1.0 / 0.3;

impl HeuristicEvaluator {
    fn sides(&self, s: &CombatState) -> (SideSummary, SideSummary) {
        let mut out = [SideSummary::default(), SideSummary::default()];
        for (i, side) in [Side::Party, Side::Enemy].into_iter().enumerate() {
            let members: Vec<_> = s.combatants.iter().filter(|c| c.side == side && !c.is_hazard()).collect();
            let foes: Vec<_> = s.combatants.iter().filter(|c| side.is_hostile_to(c.side) && !c.is_down() && !c.is_hazard()).collect();
            let sum = &mut out[i];
            for c in members {
                if c.is_down() {
                    sum.down += 1;
                    continue;
                }
                sum.standing += 1;
                let persistent: f64 = c.persistent.iter().map(|p| p.dice.average() * PERSISTENT_TICKS).sum();
                sum.hit_points += ((c.hp + c.temp_hp) as f64 - persistent).max(1.0);
                if foes.is_empty() {
                    continue;
                }
                let mut total = 0.0;
                let mut best = 0.0f64;
                for f in &foes {
                    let mut d = round_damage(s, c.id, f.id, self.include_spells);
                    if self.include_position {
                        d *= engagement_factor(s, c.id, f.id);
                    }
                    total += d;
                    best = best.max(d);
                }
                let mean = total / foes.len() as f64;
                sum.damage_per_round += 0.5 * (mean + best) * action_factor(c);
            }
        }
        let [p, e] = out;
        (p, e)
    }

    fn logit_of(&self, s: &CombatState, p: &SideSummary, e: &SideSummary) -> f64 {
        let (mut hp_p, mut hp_e) = (p.hit_points.max(EPS), e.hit_points.max(EPS));
        if self.include_tempo {
            if let Some(actor) = s.current_actor().and_then(|id| s.combatant(id)) {
                if !actor.is_down() {
                    let remaining = actor.actions_remaining as f64 / 3.0;
                    let foes: Vec<_> = s.combatants.iter().filter(|c| actor.side.is_hostile_to(c.side) && !c.is_down() && !c.is_hazard()).collect();
                    if !foes.is_empty() {
                        let dmg = foes.iter().map(|f| round_damage(s, actor.id, f.id, self.include_spells)).fold(0.0, f64::max) * remaining;
                        match actor.side {
                            Side::Party => hp_e = (hp_e - dmg * 0.5).max(EPS),
                            Side::Enemy => hp_p = (hp_p - dmg * 0.5).max(EPS),
                            Side::Neutral => {}
                        }
                    }
                }
            }
        }
        (hp_p * p.damage_per_round.max(EPS)).ln() - (hp_e * e.damage_per_round.max(EPS)).ln()
    }

    fn score_of(&self, logit: f64) -> f64 {
        10.0 * (logit / self.scale).tanh()
    }

    fn terminal(s: &CombatState) -> Option<f64> {
        match s.phase {
            Phase::Ended { winner: Some(Side::Party) } => Some(10.0),
            Phase::Ended { winner: Some(Side::Enemy) } => Some(-10.0),
            Phase::Ended { .. } => Some(0.0),
            _ => None,
        }
    }

    /// Full evaluation with a factor breakdown (counterfactual analysis:
    /// each factor is the change in score when that aspect is neutralised).
    pub fn evaluate_detailed(&self, s: &CombatState) -> Evaluation {
        let (p, e) = self.sides(s);
        if let Some(t) = Self::terminal(s) {
            return Evaluation {
                score: t,
                win_probability: if t > 0.0 { 1.0 } else if t < 0.0 { 0.0 } else { 0.5 },
                logit: t,
                terminal: true,
                factors: vec![],
                party: p,
                enemies: e,
            };
        }
        let logit = self.logit_of(s, &p, &e);
        let score = self.score_of(logit);
        let mut factors = Vec::new();
        let hp_term = (p.hit_points.max(EPS) / e.hit_points.max(EPS)).ln();
        factors.push(Factor {
            name: "Hit points".into(),
            value: self.score_of(hp_term),
            detail: format!("party {:.0} HP vs enemies {:.0} HP", p.hit_points, e.hit_points),
        });
        let off_term = (p.damage_per_round.max(EPS) / e.damage_per_round.max(EPS)).ln();
        factors.push(Factor {
            name: "Offense vs defense".into(),
            value: self.score_of(off_term),
            detail: format!(
                "expected damage/round: party {:.1} vs enemies {:.1} (attack bonuses against current AC)",
                p.damage_per_round, e.damage_per_round
            ),
        });
        let counterfactual = |name: &str, detail: &str, alt: &dyn Fn(&mut CombatState, &mut HeuristicEvaluator)| {
            let mut s2 = s.clone();
            let mut ev2 = *self;
            alt(&mut s2, &mut ev2);
            let (p2, e2) = ev2.sides(&s2);
            let l2 = ev2.logit_of(&s2, &p2, &e2);
            Factor { name: name.into(), value: score - self.score_of(l2), detail: detail.into() }
        };
        factors.push(counterfactual(
            "Conditions",
            "effect of current conditions and effects (frightened, off-guard, bless, ...)",
            &|s2, _| {
                for c in &mut s2.combatants {
                    c.conditions.0.retain(|x| {
                        matches!(x.kind, CK::Dying | CK::Unconscious | CK::Wounded | CK::Slowed | CK::Stunned | CK::Quickened)
                    });
                    c.effects.clear();
                    c.persistent.clear();
                }
            },
        ));
        factors.push(counterfactual(
            "Action economy",
            "slowed / stunned / quickened and whose turn it is",
            &|s2, ev2| {
                ev2.include_tempo = false;
                for c in &mut s2.combatants {
                    c.conditions.0.retain(|x| !matches!(x.kind, CK::Slowed | CK::Stunned | CK::Quickened));
                }
            },
        ));
        factors.push(counterfactual("Position", "how quickly each side can bring its attacks to bear", &|_, ev2| {
            ev2.include_position = false;
        }));
        factors.push(counterfactual("Resources", "remaining spell slots", &|s2, _| {
            for c in &mut s2.combatants {
                for slots in &mut c.spell_slots {
                    for v in slots.values_mut() {
                        *v = 0;
                    }
                }
            }
        }));
        factors.push(counterfactual("Battlefield control", "cover and concealment from terrain", &|s2, _| {
            s2.options.auto_cover = false;
            for row in &mut s2.battlefield.map {
                *row = row.replace('%', ".");
            }
        }));
        Evaluation {
            score,
            win_probability: 1.0 / (1.0 + (-1.6 * logit).exp()),
            logit,
            terminal: false,
            factors,
            party: p,
            enemies: e,
        }
    }
}

impl Evaluator for HeuristicEvaluator {
    fn evaluate(&self, s: &CombatState) -> f64 {
        if let Some(t) = Self::terminal(s) {
            return t;
        }
        let (p, e) = self.sides(s);
        if e.standing == 0 && e.down > 0 {
            return 10.0;
        }
        if p.standing == 0 && p.down > 0 {
            return -10.0;
        }
        self.score_of(self.logit_of(s, &p, &e))
    }
}
