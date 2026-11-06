use serde::{Deserialize, Serialize};

/// Which side of the table a combatant fights for. Every system uses the
/// same two opposing sides (players' party vs the GM's hostiles) plus
/// bystanders, so analysis can always tell friend from foe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Party,
    Enemy,
    Neutral,
}

impl Side {
    pub fn is_hostile_to(self, other: Side) -> bool {
        matches!((self, other), (Side::Party, Side::Enemy) | (Side::Enemy, Side::Party))
    }

    /// The side this one fights (Neutral maps to Enemy for display purposes).
    pub fn opposing(self) -> Side {
        match self {
            Side::Enemy => Side::Party,
            _ => Side::Enemy,
        }
    }
}
