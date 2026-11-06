//! Events are *outputs* describing what happened. They are derived data: the
//! log of record is the command list, so events never need to be stored.

use crate::model::HitLocation;
use serde::{Deserialize, Serialize};
use tactics_core::{CombatantId, Pos, Side};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Event {
    CombatStarted,
    /// Final initiative order, first to act first.
    InitiativeOrder { order: Vec<(CombatantId, i32)> },
    RoundStarted { round: u32 },
    RoundEnded { round: u32 },
    TurnStarted { combatant: CombatantId },
    TurnEnded { combatant: CombatantId },
    ActionDeclared { actor: CombatantId, action: String },
    Moved { combatant: CombatantId, from: Pos, to: Pos, cost: u32 },
    /// A d10 check (or initiative) finished. `beat`/`success` are `None` for
    /// rolls whose total is used later (initiative, defender rolls).
    CheckResolved {
        roller: Option<CombatantId>,
        purpose: String,
        natural: u8,
        total: i32,
        beat: Option<i32>,
        success: Option<bool>,
    },
    /// The target chose to dodge a ranged attack.
    DodgeDeclared { combatant: CombatantId, attacker: CombatantId },
    /// An attack missed.
    Missed { attacker: CombatantId, target: CombatantId },
    DamageRolled { purpose: String, faces: Vec<i32>, total: i32 },
    HitLocation { target: CombatantId, location: HitLocation },
    ArmorStopped { target: CombatantId, location: HitLocation, damage: i32, sp: i32 },
    ArmorPenetrated { target: CombatantId, location: HitLocation, damage: i32, sp_before: i32, sp_after: i32 },
    HpLost { target: CombatantId, amount: i32, hp: i32 },
    /// The GM must resolve the specific injury on their own table.
    CriticalInjury { target: CombatantId, location: HitLocation, note: String },
    CoverHit { pos: Pos, damage: i32, hp: i32 },
    CoverDestroyed { pos: Pos },
    Reloaded { combatant: CombatantId, weapon: usize, ammo: u32 },
    DeathSave { combatant: CombatantId, face: u8, threshold: i32, success: bool },
    MortallyWounded { combatant: CombatantId },
    Died { combatant: CombatantId },
    CombatEnded { winner: Option<Side> },
    Note { text: String },
}
