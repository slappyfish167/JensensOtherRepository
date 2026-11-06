//! Position evaluation on a Stockfish-like scale, for any game system:
//! `+10` overwhelming player advantage … `0` balanced … `-10` near-certain defeat.
//!
//! Model (Lanchester's square law): a side's fighting strength is
//! `H × O`, its effective hit points times its expected damage per round.
//! The log-ratio of the two strengths is the *logit*; the score is
//! `10 · tanh(logit / 2)` and the win estimate is `σ(1.6 · logit)`.
//!
//! `O` is measured against the opposing roster as a whole (downed members
//! included), so it is a property of the attackers, not of who happens to be
//! left standing. `H` and `O` come from the system ([`TacticalSystem::effective_hp`],
//! [`TacticalSystem::round_damage`], ...), so conditions, armor, cover and
//! resources enter through each system's own rules.

use crate::system::{TacticalSystem, Unit};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use tactics_core::Side;

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
pub trait Evaluator<S: TacticalSystem>: Send + Sync {
    fn evaluate(&self, state: &S::State) -> f64;
}

/// Switches the evaluator and the counterfactual probes can flip.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EvalOptions {
    /// Logit units per score unit (tanh scaling).
    pub scale: f64,
    /// Count limited resources (spell slots, ammunition).
    pub include_resources: bool,
    pub include_position: bool,
    pub include_tempo: bool,
}

impl Default for EvalOptions {
    fn default() -> Self {
        EvalOptions { scale: 2.0, include_resources: true, include_position: true, include_tempo: true }
    }
}

/// The Lanchester evaluator for system `S`.
pub struct HeuristicEvaluator<S> {
    pub options: EvalOptions,
    _system: PhantomData<fn() -> S>,
}

impl<S> HeuristicEvaluator<S> {
    pub fn new(options: EvalOptions) -> Self {
        HeuristicEvaluator { options, _system: PhantomData }
    }
}

impl<S> Default for HeuristicEvaluator<S> {
    fn default() -> Self {
        Self::new(EvalOptions::default())
    }
}

impl<S> Clone for HeuristicEvaluator<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for HeuristicEvaluator<S> {}

impl<S> std::fmt::Debug for HeuristicEvaluator<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeuristicEvaluator").field("options", &self.options).finish()
    }
}

const EPS: f64 = 0.5;

fn fighters(units: &[Unit]) -> impl Iterator<Item = &Unit> {
    units.iter().filter(|u| !u.ignored)
}

impl<S: TacticalSystem> HeuristicEvaluator<S> {
    fn sides(o: &EvalOptions, s: &S::State) -> (SideSummary, SideSummary) {
        let units = S::units(s);
        let mut out = [SideSummary::default(), SideSummary::default()];
        for (i, side) in [Side::Party, Side::Enemy].into_iter().enumerate() {
            // Offense is measured against the whole opposing roster, downed
            // members included: dropping the softest enemy must never make
            // your own side look weaker just because harder targets remain.
            let foes: Vec<_> = fighters(&units).filter(|c| side.is_hostile_to(c.side)).collect();
            let sum = &mut out[i];
            for c in fighters(&units).filter(|c| c.side == side) {
                if c.down {
                    sum.down += 1;
                    continue;
                }
                sum.standing += 1;
                sum.hit_points += S::effective_hp(s, c.id).max(1.0);
                if foes.is_empty() {
                    continue;
                }
                let mut total = 0.0;
                let mut best = 0.0f64;
                for f in &foes {
                    let mut d = S::round_damage(s, c.id, f.id, o.include_resources);
                    if o.include_position {
                        d *= S::engagement(s, c.id, f.id);
                    }
                    total += d;
                    best = best.max(d);
                }
                let mean = total / foes.len() as f64;
                sum.damage_per_round += 0.5 * (mean + best) * S::action_factor(s, c.id);
            }
        }
        let [p, e] = out;
        (p, e)
    }

    fn logit_of(o: &EvalOptions, s: &S::State, p: &SideSummary, e: &SideSummary) -> f64 {
        let (mut hp_p, mut hp_e) = (p.hit_points.max(EPS), e.hit_points.max(EPS));
        if o.include_tempo {
            if let Some(actor) = S::current_actor(s) {
                let units = S::units(s);
                if let Some(a) = units.iter().find(|u| u.id == actor).filter(|a| !a.down && !a.ignored) {
                    let remaining = S::tempo_fraction(s, actor);
                    let dmg = fighters(&units)
                        .filter(|f| a.side.is_hostile_to(f.side) && !f.down)
                        .map(|f| S::round_damage(s, actor, f.id, o.include_resources))
                        .fold(0.0, f64::max)
                        * remaining;
                    match a.side {
                        Side::Party => hp_e = (hp_e - dmg * 0.5).max(EPS),
                        Side::Enemy => hp_p = (hp_p - dmg * 0.5).max(EPS),
                        Side::Neutral => {}
                    }
                }
            }
        }
        (hp_p * p.damage_per_round.max(EPS)).ln() - (hp_e * e.damage_per_round.max(EPS)).ln()
    }

    fn score_of(o: &EvalOptions, logit: f64) -> f64 {
        10.0 * (logit / o.scale).tanh()
    }

    fn terminal(s: &S::State) -> Option<f64> {
        S::winner(s).map(|w| match w {
            Some(Side::Party) => 10.0,
            Some(Side::Enemy) => -10.0,
            _ => 0.0,
        })
    }

    /// Full evaluation with a factor breakdown (counterfactual analysis:
    /// each factor is the change in score when that aspect is neutralised).
    pub fn evaluate_detailed(&self, s: &S::State) -> Evaluation {
        let o = &self.options;
        let (p, e) = Self::sides(o, s);
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
        let logit = Self::logit_of(o, s, &p, &e);
        let score = Self::score_of(o, logit);
        let mut factors = vec![
            Factor {
                name: "Hit points".into(),
                value: Self::score_of(o, (p.hit_points.max(EPS) / e.hit_points.max(EPS)).ln()),
                detail: format!("party {:.0} HP vs enemies {:.0} HP", p.hit_points, e.hit_points),
            },
            Factor {
                name: "Offense vs defense".into(),
                value: Self::score_of(o, (p.damage_per_round.max(EPS) / e.damage_per_round.max(EPS)).ln()),
                detail: format!(
                    "expected damage/round: party {:.1} vs enemies {:.1} (attacks against current defenses)",
                    p.damage_per_round, e.damage_per_round
                ),
            },
        ];
        let probe = |name: &str, detail: &str, alt: &dyn Fn(&mut S::State, &mut EvalOptions)| {
            let mut s2 = s.clone();
            let mut o2 = *o;
            alt(&mut s2, &mut o2);
            let (p2, e2) = Self::sides(&o2, &s2);
            let l2 = Self::logit_of(&o2, &s2, &p2, &e2);
            Factor { name: name.into(), value: score - Self::score_of(&o2, l2), detail: detail.into() }
        };
        factors.push(probe("Position", "how quickly each side can bring its attacks to bear", &|_, o2| o2.include_position = false));
        for cf in S::counterfactuals() {
            factors.push(probe(cf.name, cf.detail, &cf.apply));
        }
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

impl<S: TacticalSystem> Evaluator<S> for HeuristicEvaluator<S> {
    fn evaluate(&self, s: &S::State) -> f64 {
        if let Some(t) = Self::terminal(s) {
            return t;
        }
        let o = &self.options;
        let (p, e) = Self::sides(o, s);
        if e.standing == 0 && e.down > 0 {
            return 10.0;
        }
        if p.standing == 0 && p.down > 0 {
            return -10.0;
        }
        Self::score_of(o, Self::logit_of(o, s, &p, &e))
    }
}
