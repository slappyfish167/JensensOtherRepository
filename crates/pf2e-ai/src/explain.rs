//! "Explain position": why is the evaluation what it is, and why did it
//! change?
//!
//! Every reason is measured **counterfactually** with the evaluator: take the
//! position, undo exactly one concrete difference (one creature's hit points,
//! its conditions, its square, a spent reaction, flanking), evaluate again,
//! and report the score difference as that reason's *impact*. Nothing here
//! knows a rule; facts such as "is flanked" are asked of `pf2e-core`.

use crate::evaluation::{Evaluator, HeuristicEvaluator};
use pf2e_core::combat::rules::is_flanked_by;
use pf2e_core::combat::{CombatState, Combatant};
use pf2e_core::conditions::{ConditionKind, ConditionKind as CK};
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonKind {
    HitPoints,
    Downed,
    Recovered,
    ConditionGained,
    ConditionLost,
    Flanked,
    Reaction,
    ActionEconomy,
    Resources,
    Position,
    PersistentDamage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reason {
    pub kind: ReasonKind,
    pub subject: Option<CombatantId>,
    pub text: String,
    /// Contribution to the current score (party-positive evaluation units).
    pub impact: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactorDelta {
    pub name: String,
    pub before: f64,
    pub after: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionExplanation {
    pub score: f64,
    pub previous_score: Option<f64>,
    pub delta: f64,
    /// Sorted by |impact|, largest first.
    pub reasons: Vec<Reason>,
    pub factors: Vec<FactorDelta>,
}

/// Ignore reasons smaller than this (evaluation units).
const NOISE: f64 = 0.05;

fn impact_of(ev: &HeuristicEvaluator, base: f64, s: &CombatState, undo: impl FnOnce(&mut CombatState)) -> f64 {
    let mut alt = s.clone();
    undo(&mut alt);
    base - ev.evaluate(&alt)
}

fn cond_label(kind: ConditionKind, value: u8) -> String {
    let name = kind.label().replace('_', "-");
    if kind.is_valued() && value > 0 {
        format!("{name} {value}")
    } else {
        name
    }
}

/// Creatures flanked by at least one hostile attacker, with one flanker.
fn flanked(s: &CombatState) -> Vec<(CombatantId, CombatantId)> {
    if !s.options.flanking {
        return vec![];
    }
    let mut out = Vec::new();
    for t in s.combatants.iter().filter(|c| !c.is_down() && !c.is_hazard()) {
        if let Some(a) = s
            .combatants
            .iter()
            .filter(|a| a.side.is_hostile_to(t.side) && a.can_act() && !a.is_hazard())
            .find(|a| is_flanked_by(s, a.id, t.id))
        {
            out.push((t.id, a.id));
        }
    }
    out
}

fn sort_and_trim(mut reasons: Vec<Reason>, max: usize) -> Vec<Reason> {
    reasons.retain(|r| r.impact.abs() >= NOISE || matches!(r.kind, ReasonKind::Downed | ReasonKind::Recovered));
    reasons.sort_by(|a, b| b.impact.abs().partial_cmp(&a.impact.abs()).unwrap_or(std::cmp::Ordering::Equal));
    reasons.truncate(max);
    reasons
}

fn flank_reasons(ev: &HeuristicEvaluator, base: f64, s: &CombatState, only_new_vs: Option<&CombatState>) -> Vec<Reason> {
    let before: Vec<CombatantId> = only_new_vs.map(|b| flanked(b).into_iter().map(|(t, _)| t).collect()).unwrap_or_default();
    let now = flanked(s);
    if now.is_empty() {
        return vec![];
    }
    // Impact of flanking as a whole, shared out across flanked creatures.
    let total = impact_of(ev, base, s, |alt| alt.options.flanking = false);
    let fresh: Vec<_> = now.iter().filter(|(t, _)| !before.contains(t)).collect();
    let share = total / now.len() as f64;
    fresh
        .into_iter()
        .map(|(t, a)| {
            let name = |id: &CombatantId| s.combatant(*id).map(|c| c.name.clone()).unwrap_or_default();
            Reason {
                kind: ReasonKind::Flanked,
                subject: Some(*t),
                text: format!("{} is flanked (off-guard to {} and allies)", name(t), name(a)),
                impact: share,
            }
        })
        .collect()
}

/// Reasons for the current evaluation of a single position.
pub fn explain_position(s: &CombatState, ev: &HeuristicEvaluator, max: usize) -> PositionExplanation {
    let base = ev.evaluate(s);
    let mut reasons = Vec::new();
    for c in s.combatants.iter().filter(|c| !c.is_hazard()) {
        let id = c.id;
        if c.is_down() {
            reasons.push(Reason {
                kind: ReasonKind::Downed,
                subject: Some(id),
                text: format!("{} is down", c.name),
                impact: impact_of(ev, base, s, |alt| revive(alt, id)),
            });
            continue;
        }
        let max_hp = c.effective_max_hp();
        if c.hp * 2 < max_hp {
            reasons.push(Reason {
                kind: ReasonKind::HitPoints,
                subject: Some(id),
                text: format!("{} is at {} / {} HP", c.name, c.hp, max_hp),
                impact: impact_of(ev, base, s, |alt| alt.combatant_mut(id).unwrap().hp = max_hp),
            });
        }
        let notable: Vec<_> = c.conditions.0.iter().filter(|x| !matches!(x.kind, CK::Wounded)).collect();
        if !notable.is_empty() {
            let names = notable.iter().map(|x| cond_label(x.kind, x.value)).collect::<Vec<_>>().join(", ");
            reasons.push(Reason {
                kind: ReasonKind::ConditionGained,
                subject: Some(id),
                text: format!("{} is {names}", c.name),
                impact: impact_of(ev, base, s, |alt| alt.combatant_mut(id).unwrap().conditions.0.clear()),
            });
        }
        if !c.persistent.is_empty() {
            reasons.push(Reason {
                kind: ReasonKind::PersistentDamage,
                subject: Some(id),
                text: format!("{} takes persistent damage", c.name),
                impact: impact_of(ev, base, s, |alt| alt.combatant_mut(id).unwrap().persistent.clear()),
            });
        }
        if !c.reaction_available && !c.statblock.reactions.is_empty() {
            reasons.push(Reason {
                kind: ReasonKind::Reaction,
                subject: Some(id),
                text: format!("{} has used its reaction", c.name),
                impact: impact_of(ev, base, s, |alt| alt.combatant_mut(id).unwrap().reaction_available = true),
            });
        }
    }
    reasons.extend(flank_reasons(ev, base, s, None));
    let factors = ev.evaluate_detailed(s).factors.into_iter().map(|f| FactorDelta { name: f.name, before: f.value, after: f.value }).collect();
    PositionExplanation { score: base, previous_score: None, delta: 0.0, reasons: sort_and_trim(reasons, max), factors }
}

fn revive(s: &mut CombatState, id: CombatantId) {
    if let Some(c) = s.combatant_mut(id) {
        c.life = pf2e_core::combat::LifeState::Active;
        c.conditions.0.retain(|x| !matches!(x.kind, CK::Dying | CK::Unconscious | CK::Prone | CK::OffGuard));
        c.hp = c.hp.max(1);
    }
}

fn restore<F: FnOnce(&mut Combatant, &Combatant)>(alt: &mut CombatState, before: &Combatant, f: F) {
    if let Some(c) = alt.combatant_mut(before.id) {
        f(c, before);
    }
}

/// Why the evaluation moved from `before` to `after` (same combat).
pub fn explain_change(before: &CombatState, after: &CombatState, ev: &HeuristicEvaluator, max: usize) -> PositionExplanation {
    let base = ev.evaluate(after);
    let prev = ev.evaluate(before);
    let mut reasons = Vec::new();
    for b in before.combatants.iter().filter(|c| !c.is_hazard()) {
        let Some(a) = after.combatant(b.id) else { continue };
        let name = &a.name;
        let id = a.id;
        if a.is_down() != b.is_down() {
            let (kind, text) = if a.is_down() {
                (ReasonKind::Downed, format!("{name} went down"))
            } else {
                (ReasonKind::Recovered, format!("{name} is back up"))
            };
            reasons.push(Reason {
                kind,
                subject: Some(id),
                text,
                impact: impact_of(ev, base, after, |alt| {
                    restore(alt, b, |c, b| {
                        c.hp = b.hp;
                        c.life = b.life;
                        c.conditions = b.conditions.clone();
                    })
                }),
            });
            continue;
        }
        let dhp = (a.hp + a.temp_hp) - (b.hp + b.temp_hp);
        if dhp != 0 {
            let max_hp = a.effective_max_hp().max(1);
            let text = if dhp < 0 {
                format!("{name} lost {} HP ({} / {max_hp} left)", -dhp, a.hp)
            } else {
                format!("{name} regained {dhp} HP ({} / {max_hp})", a.hp)
            };
            reasons.push(Reason {
                kind: ReasonKind::HitPoints,
                subject: Some(id),
                text,
                impact: impact_of(ev, base, after, |alt| {
                    restore(alt, b, |c, b| {
                        c.hp = b.hp;
                        c.temp_hp = b.temp_hp;
                    })
                }),
            });
        }
        // New conditions, or valued conditions that got worse.
        let gained: Vec<_> = a.conditions.0.iter().filter(|x| b.conditions.get(x.kind).map(|y| y.value < x.value).unwrap_or(true)).collect();
        let lost: Vec<_> = b.conditions.0.iter().filter(|x| !a.conditions.has(x.kind)).collect();
        if !gained.is_empty() || !lost.is_empty() {
            let mut parts = Vec::new();
            if !gained.is_empty() {
                parts.push(format!("is now {}", gained.iter().map(|x| cond_label(x.kind, x.value)).collect::<Vec<_>>().join(", ")));
            }
            if !lost.is_empty() {
                parts.push(format!("is no longer {}", lost.iter().map(|x| cond_label(x.kind, x.value)).collect::<Vec<_>>().join(", ")));
            }
            let kind = if gained.is_empty() { ReasonKind::ConditionLost } else { ReasonKind::ConditionGained };
            reasons.push(Reason {
                kind,
                subject: Some(id),
                text: format!("{name} {}", parts.join(" and ")),
                impact: impact_of(ev, base, after, |alt| restore(alt, b, |c, b| c.conditions = b.conditions.clone())),
            });
        }
        if a.position != b.position {
            reasons.push(Reason {
                kind: ReasonKind::Position,
                subject: Some(id),
                text: format!("{name} moved to ({}, {})", a.position.x, a.position.y),
                impact: impact_of(ev, base, after, |alt| restore(alt, b, |c, b| c.position = b.position)),
            });
        }
        if a.reaction_available != b.reaction_available && !a.statblock.reactions.is_empty() {
            let text = if a.reaction_available { format!("{name} has a reaction again") } else { format!("{name} used its reaction") };
            reasons.push(Reason {
                kind: ReasonKind::Reaction,
                subject: Some(id),
                text,
                impact: impact_of(ev, base, after, |alt| restore(alt, b, |c, b| c.reaction_available = b.reaction_available)),
            });
        }
        if a.spell_slots != b.spell_slots {
            reasons.push(Reason {
                kind: ReasonKind::Resources,
                subject: Some(id),
                text: format!("{name} spent spell slots"),
                impact: impact_of(ev, base, after, |alt| restore(alt, b, |c, b| c.spell_slots = b.spell_slots.clone())),
            });
        }
        if a.persistent != b.persistent {
            reasons.push(Reason {
                kind: ReasonKind::PersistentDamage,
                subject: Some(id),
                text: if a.persistent.is_empty() { format!("{name} is free of persistent damage") } else { format!("{name} is taking persistent damage") },
                impact: impact_of(ev, base, after, |alt| restore(alt, b, |c, b| c.persistent = b.persistent.clone())),
            });
        }
    }
    // Action economy of the creature whose turn it is (actions spent or lost).
    if let (Some(ida), Some(idb)) = (after.current_actor(), before.current_actor()) {
        if ida == idb {
            if let (Some(a), Some(b)) = (after.combatant(ida), before.combatant(idb)) {
                if a.actions_remaining < b.actions_remaining {
                    reasons.push(Reason {
                        kind: ReasonKind::ActionEconomy,
                        subject: Some(ida),
                        text: format!("{} has {} action(s) left", a.name, a.actions_remaining),
                        impact: impact_of(ev, base, after, |alt| alt.combatant_mut(ida).unwrap().actions_remaining = b.actions_remaining),
                    });
                }
            }
        }
    }
    reasons.extend(flank_reasons(ev, base, after, Some(before)));

    let fb = ev.evaluate_detailed(before).factors;
    let fa = ev.evaluate_detailed(after).factors;
    let factors = fa
        .into_iter()
        .map(|f| FactorDelta { before: fb.iter().find(|x| x.name == f.name).map(|x| x.value).unwrap_or(0.0), after: f.value, name: f.name })
        .collect();
    PositionExplanation { score: base, previous_score: Some(prev), delta: base - prev, reasons: sort_and_trim(reasons, max), factors }
}
