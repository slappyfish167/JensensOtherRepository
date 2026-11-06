use serde::{Deserialize, Serialize};
use std::fmt;

/// Identifier of a combatant inside one combat. Stable for the whole combat
/// (assigned in encounter order), so command logs can refer to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CombatantId(pub u32);

impl fmt::Display for CombatantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}
