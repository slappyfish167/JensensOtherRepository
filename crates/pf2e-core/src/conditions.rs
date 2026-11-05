//! Conditions, their implications, and durations.

use crate::ids::CombatantId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionKind {
    Blinded,
    Clumsy,
    Concealed,
    Dazzled,
    Doomed,
    Drained,
    Dying,
    Enfeebled,
    Fleeing,
    Frightened,
    Grabbed,
    Hidden,
    Immobilized,
    Invisible,
    OffGuard,
    Paralyzed,
    Prone,
    Quickened,
    Restrained,
    Sickened,
    Slowed,
    Stunned,
    Stupefied,
    Unconscious,
    Wounded,
}

impl ConditionKind {
    pub fn is_valued(self) -> bool {
        use ConditionKind::*;
        matches!(
            self,
            Clumsy | Doomed | Drained | Dying | Enfeebled | Frightened | Sickened | Slowed | Stunned | Stupefied
                | Wounded
        )
    }

    /// Conditions that a condition automatically imposes.
    pub fn implies(self) -> &'static [ConditionKind] {
        use ConditionKind::*;
        match self {
            Unconscious => &[Prone, OffGuard],
            Dying => &[Unconscious, Prone, OffGuard],
            Grabbed => &[OffGuard, Immobilized],
            Restrained => &[OffGuard, Immobilized],
            Paralyzed => &[OffGuard],
            Invisible => &[Hidden],
            Prone => &[OffGuard],
            _ => &[],
        }
    }

    pub fn label(self) -> String {
        format!("{self:?}").to_lowercase()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnBoundary {
    Start,
    End,
}

/// "Until the start/end of `combatant`'s turn in `round`".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Expiry {
    pub combatant: CombatantId,
    pub round: u32,
    pub boundary: TurnBoundary,
}

/// Durations as written in rules text; converted to an [`Expiry`] when applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Duration {
    /// Ends at the start of the source's turn after `n` rounds (1 round, 1 minute = 10).
    Rounds(u32),
    /// Ends at the end of the affected creature's next turn.
    UntilEndOfTargetsNextTurn,
    /// Ends at the start of the source's next turn.
    UntilStartOfSourcesNextTurn,
    Unlimited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub kind: ConditionKind,
    #[serde(default)]
    pub value: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CombatantId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Expiry>,
}

impl Condition {
    pub fn new(kind: ConditionKind, value: u8) -> Self {
        Condition { kind, value, source: None, expires: None }
    }
}

/// A combatant's set of conditions. Valued conditions keep the higher value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConditionSet(pub Vec<Condition>);

impl ConditionSet {
    /// Directly present (not implied).
    pub fn get(&self, kind: ConditionKind) -> Option<&Condition> {
        self.0.iter().find(|c| c.kind == kind)
    }

    pub fn value(&self, kind: ConditionKind) -> u8 {
        self.get(kind).map(|c| c.value.max(if kind.is_valued() { 0 } else { 1 })).unwrap_or(0)
    }

    /// Present directly or implied by another condition.
    pub fn has(&self, kind: ConditionKind) -> bool {
        self.0.iter().any(|c| c.kind == kind || implies_transitively(c.kind, kind))
    }

    /// Adds a condition. Returns true if anything changed.
    pub fn add(&mut self, cond: Condition) -> bool {
        if let Some(existing) = self.0.iter_mut().find(|c| c.kind == cond.kind) {
            if cond.kind.is_valued() && cond.value > existing.value {
                existing.value = cond.value;
                existing.expires = cond.expires;
                existing.source = cond.source;
                return true;
            }
            if !cond.kind.is_valued() && existing.expires.is_some() && cond.expires.is_none() {
                existing.expires = None;
                return true;
            }
            return false;
        }
        if cond.kind.is_valued() && cond.value == 0 {
            return false;
        }
        self.0.push(cond);
        true
    }

    /// Sets a valued condition to an exact value (0 removes it).
    pub fn set_value(&mut self, kind: ConditionKind, value: u8) {
        if value == 0 {
            self.remove(kind);
        } else if let Some(existing) = self.0.iter_mut().find(|c| c.kind == kind) {
            existing.value = value;
        } else {
            self.0.push(Condition::new(kind, value));
        }
    }

    pub fn remove(&mut self, kind: ConditionKind) -> bool {
        let before = self.0.len();
        self.0.retain(|c| c.kind != kind);
        before != self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Condition> {
        self.0.iter()
    }
}

fn implies_transitively(from: ConditionKind, target: ConditionKind) -> bool {
    from.implies().iter().any(|&k| k == target || implies_transitively(k, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConditionKind::*;

    #[test]
    fn valued_conditions_keep_highest() {
        let mut s = ConditionSet::default();
        s.add(Condition::new(Frightened, 2));
        s.add(Condition::new(Frightened, 1));
        assert_eq!(s.value(Frightened), 2);
        s.set_value(Frightened, 0);
        assert!(!s.has(Frightened));
    }

    #[test]
    fn implications_are_transitive() {
        let mut s = ConditionSet::default();
        s.add(Condition::new(Dying, 1));
        assert!(s.has(Unconscious));
        assert!(s.has(Prone));
        assert!(s.has(OffGuard));
        assert!(!s.has(Grabbed));
    }
}
