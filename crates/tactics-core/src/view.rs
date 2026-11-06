//! System-neutral read models. Any UI that can render these can run a
//! combat in any game system without knowing its rules.

use crate::geom::Pos;
use crate::ids::CombatantId;
use crate::side::Side;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseView {
    /// `setup` | `rolling_initiative` | `active` | `ended`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<Side>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegendEntry {
    pub glyph: String,
    /// CSS-ish class: `open`, `difficult`, `wall`, `cover`, `obscured`, ...
    pub class: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapView {
    pub width: u32,
    pub height: u32,
    pub rows: Vec<String>,
    pub legend: Vec<LegendEntry>,
    /// What one square measures, e.g. `5 ft` or `2 m`.
    pub square: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatLine {
    pub label: String,
    pub value: String,
}

impl StatLine {
    pub fn new(label: impl Into<String>, value: impl ToString) -> Self {
        StatLine { label: label.into(), value: value.to_string() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitView {
    pub id: CombatantId,
    pub name: String,
    pub side: Side,
    /// `pc` | `npc` | `hazard`.
    pub kind: String,
    pub position: Pos,
    /// Edge length of the creature's space in squares.
    pub squares: u32,
    pub hp: i32,
    pub max_hp: i32,
    pub temp_hp: i32,
    /// Out of the fight (unconscious, dying, mortally wounded, dead, disabled).
    pub down: bool,
    pub dead: bool,
    /// Current conditions/statuses, e.g. `frightened 1`, `seriously wounded`.
    pub statuses: Vec<String>,
    /// Compact stat block for the inspector panel.
    pub stats: Vec<StatLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitiativeView {
    pub combatant: CombatantId,
    pub value: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableView {
    pub system: String,
    pub system_name: String,
    pub phase: PhaseView,
    pub round: u32,
    pub current_actor: Option<CombatantId>,
    pub map: MapView,
    pub combatants: Vec<UnitView>,
    pub initiative: Vec<InitiativeView>,
    /// What the acting creature has left this turn, e.g. `◆◆◇` or `Move ✓ · Action ✗`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
}

/// How a human should enter a roll.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum InputSpec {
    /// Enter the natural face of one die.
    Die { sides: u32 },
    /// Enter the rolled total.
    Total { min: i32, max: i32 },
    /// Enter every die face (e.g. `4 6 6`), because individual faces matter.
    Faces { count: u32, sides: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptView {
    /// `roll` | `decision`.
    pub kind: String,
    pub request_id: u32,
    /// Who rolls / decides.
    pub actor: Option<CombatantId>,
    pub prompt: String,
    #[serde(default)]
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InputSpec>,
    #[serde(default)]
    pub breakdown: Vec<StatLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decline_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionView {
    /// The system's own action value; send it back unchanged in an `act` command.
    pub action: serde_json::Value,
    pub description: String,
    /// UI grouping, e.g. `Attacks`, `Movement`, `Other`.
    pub group: String,
    /// Movement actions name their destination so a UI can show them on the grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<Pos>,
}
