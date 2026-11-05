//! Encounters: who fights where, plus the XP budget / threat calculator.

use crate::grid::Pos;
use crate::models::battlefield::Battlefield;
use crate::models::statblock::Statblock;
use crate::stats::Side;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ParticipantSource {
    /// A stored character (derived into a stat block at combat start).
    Character(String),
    /// A stored monster or hazard.
    Monster(String),
    /// An ad-hoc stat block embedded in the encounter.
    Inline(Box<Statblock>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterParticipant {
    pub source: ParticipantSource,
    pub side: Side,
    pub position: Pos,
    /// Display-name override ("Goblin 2").
    #[serde(default)]
    pub name: Option<String>,
    /// Starting HP override (pre-damaged foes, resting PCs).
    #[serde(default)]
    pub current_hp: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Encounter {
    pub id: String,
    pub name: String,
    /// Party level used for the XP budget.
    pub party_level: i32,
    pub participants: Vec<EncounterParticipant>,
    pub battlefield: Battlefield,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreatLevel {
    Trivial,
    Low,
    Moderate,
    Severe,
    Extreme,
    BeyondExtreme,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncounterBudget {
    pub party_size: usize,
    pub total_xp: i32,
    pub threat: ThreatLevel,
    /// Budgets adjusted for party size: trivial, low, moderate, severe, extreme.
    pub budgets: [i32; 5],
    pub warnings: Vec<String>,
}

/// XP for a creature (or complex hazard) relative to party level.
pub fn creature_xp(creature_level: i32, party_level: i32) -> Option<i32> {
    Some(match creature_level - party_level {
        d if d <= -5 => 0,
        -4 => 10,
        -3 => 15,
        -2 => 20,
        -1 => 30,
        0 => 40,
        1 => 60,
        2 => 80,
        3 => 120,
        4 => 160,
        _ => return None,
    })
}

/// Simple hazards are worth one fifth of a creature of the same level.
pub fn simple_hazard_xp(level: i32, party_level: i32) -> Option<i32> {
    Some(match level - party_level {
        d if d <= -5 => 0,
        -4 => 2,
        -3 => 3,
        -2 => 4,
        -1 => 6,
        0 => 8,
        1 => 12,
        2 => 16,
        3 => 24,
        4 => 32,
        _ => return None,
    })
}

/// GM Core encounter budget. `opponents` = (level, is_simple_hazard).
pub fn encounter_budget(party_level: i32, party_size: usize, opponents: &[(i32, bool)]) -> EncounterBudget {
    let base = [40, 60, 80, 120, 160];
    let per_pc = [10, 15, 20, 30, 40];
    let delta = party_size as i32 - 4;
    let budgets = [0, 1, 2, 3, 4].map(|i| base[i] + per_pc[i] * delta);
    let mut warnings = Vec::new();
    let mut total = 0;
    for &(level, simple) in opponents {
        let xp = if simple { simple_hazard_xp(level, party_level) } else { creature_xp(level, party_level) };
        match xp {
            Some(x) => total += x,
            None => {
                warnings.push(format!(
                    "level {level} opponent is more than 4 levels above the party; it is effectively unbeatable"
                ));
                total += 200;
            }
        }
    }
    if party_size == 0 {
        warnings.push("encounter has no party members".into());
    }
    let threat = if total <= budgets[0] {
        ThreatLevel::Trivial
    } else if total <= budgets[1] {
        ThreatLevel::Low
    } else if total <= budgets[2] {
        ThreatLevel::Moderate
    } else if total <= budgets[3] {
        ThreatLevel::Severe
    } else if total <= budgets[4] {
        ThreatLevel::Extreme
    } else {
        ThreatLevel::BeyondExtreme
    };
    EncounterBudget { party_size, total_xp: total, threat, budgets, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moderate_encounter_for_four() {
        let b = encounter_budget(1, 4, &[(1, false), (0, false), (-1, false)]);
        assert_eq!(b.total_xp, 40 + 30 + 20);
        assert_eq!(b.threat, ThreatLevel::Severe);
        let b = encounter_budget(3, 5, &[(3, false), (3, false)]);
        assert_eq!(b.budgets[2], 100);
        assert_eq!(b.threat, ThreatLevel::Moderate);
    }
}
