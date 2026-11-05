//! Typed bonuses and penalties with PF2e stacking rules.
//!
//! * Circumstance, status and item: only the highest bonus and the worst
//!   penalty of each type apply.
//! * Untyped penalties all stack; untyped bonuses (rare) also stack.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModifierType {
    Untyped,
    Circumstance,
    Status,
    Item,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifier {
    pub kind: ModifierType,
    pub value: i32,
    pub source: String,
}

impl Modifier {
    pub fn new(kind: ModifierType, value: i32, source: impl Into<String>) -> Self {
        Modifier { kind, value, source: source.into() }
    }
    pub fn circumstance(value: i32, source: impl Into<String>) -> Self {
        Self::new(ModifierType::Circumstance, value, source)
    }
    pub fn status(value: i32, source: impl Into<String>) -> Self {
        Self::new(ModifierType::Status, value, source)
    }
    pub fn item(value: i32, source: impl Into<String>) -> Self {
        Self::new(ModifierType::Item, value, source)
    }
    pub fn untyped(value: i32, source: impl Into<String>) -> Self {
        Self::new(ModifierType::Untyped, value, source)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModifierSet {
    pub modifiers: Vec<Modifier>,
}

impl ModifierSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, m: Modifier) {
        if m.value != 0 {
            self.modifiers.push(m);
        }
    }

    pub fn extend(&mut self, other: ModifierSet) {
        for m in other.modifiers {
            self.push(m);
        }
    }

    /// The modifiers that actually count after stacking (for explanations).
    pub fn applied(&self) -> Vec<&Modifier> {
        let mut out: Vec<&Modifier> = Vec::new();
        for kind in [ModifierType::Circumstance, ModifierType::Status, ModifierType::Item] {
            let of_kind = self.modifiers.iter().filter(|m| m.kind == kind);
            if let Some(best) = of_kind.clone().filter(|m| m.value > 0).max_by_key(|m| m.value) {
                out.push(best);
            }
            if let Some(worst) = of_kind.filter(|m| m.value < 0).min_by_key(|m| m.value) {
                out.push(worst);
            }
        }
        out.extend(self.modifiers.iter().filter(|m| m.kind == ModifierType::Untyped));
        out
    }

    pub fn total(&self) -> i32 {
        self.applied().iter().map(|m| m.value).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_modifiers_do_not_stack() {
        let mut s = ModifierSet::new();
        s.push(Modifier::status(-1, "frightened 1"));
        s.push(Modifier::status(-2, "sickened 2"));
        s.push(Modifier::status(1, "bless"));
        s.push(Modifier::circumstance(2, "cover"));
        s.push(Modifier::circumstance(1, "lesser cover"));
        s.push(Modifier::untyped(-2, "range"));
        s.push(Modifier::untyped(-2, "range again"));
        // status: +1 -2, circumstance: +2, untyped: -4
        assert_eq!(s.total(), 1 - 2 + 2 - 4);
    }
}
