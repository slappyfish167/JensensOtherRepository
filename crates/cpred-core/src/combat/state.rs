//! Runtime combat models: `Combatant`, `InitiativeEntry`, `CombatState`.

use super::pending::Step;
use crate::model::{ActorKind, Battlefield, HitLocation, Stat, Statblock};
use crate::rules::{self, RulesOptions, RulesTables};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tactics_core::{CombatantId, Pos, Side};

/// Version of the serialized state layout.
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// What the acting combatant has left this turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct TurnEconomy {
    pub move_available: bool,
    pub action_available: bool,
    /// Further attacks allowed with `rof_weapon` from the Action already spent.
    pub shots_left: u8,
    pub rof_weapon: Option<usize>,
}

impl TurnEconomy {
    /// A full turn: Move Action and Action available.
    pub fn fresh() -> Self {
        TurnEconomy { move_available: true, action_available: true, shots_left: 0, rof_weapon: None }
    }
}

/// Current (possibly ablated) SP per location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ArmorSp {
    pub body: i32,
    pub head: i32,
}

impl ArmorSp {
    pub fn get(&self, location: HitLocation) -> i32 {
        match location {
            HitLocation::Body => self.body,
            HitLocation::Head => self.head,
        }
    }

    pub fn set(&mut self, location: HitLocation, value: i32) {
        match location {
            HitLocation::Body => self.body = value,
            HitLocation::Head => self.head = value,
        }
    }
}

/// A recorded Critical Injury. The specific injury is resolved by the GM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CriticalInjury {
    pub location: HitLocation,
    pub round: u32,
}

/// One participant of a combat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combatant {
    pub id: CombatantId,
    pub name: String,
    pub side: Side,
    pub kind: ActorKind,
    /// Includes the implicit Brawling weapon (always the last weapon).
    pub statblock: Statblock,
    pub hp: i32,
    pub position: Pos,
    pub economy: TurnEconomy,
    /// Rounds loaded, per weapon (0 for weapons without a magazine).
    pub ammo: Vec<u32>,
    pub armor_sp: ArmorSp,
    pub injuries: Vec<CriticalInjury>,
    pub death_save_penalty: u8,
    pub alive: bool,
}

impl Combatant {
    pub fn max_hp(&self) -> i32 {
        self.statblock.effective_max_hp()
    }

    pub fn is_dead(&self) -> bool {
        !self.alive
    }

    /// Alive at 0 HP.
    pub fn is_mortally_wounded(&self) -> bool {
        self.alive && self.hp <= 0
    }

    /// Alive, above 0 HP but below half (rounded up) of max HP.
    pub fn is_seriously_wounded(&self) -> bool {
        self.alive && self.hp > 0 && self.hp < rules::seriously_wounded_threshold(self.max_hp())
    }

    /// Dead or mortally wounded (out of the fight for victory purposes).
    pub fn is_down(&self) -> bool {
        self.is_dead() || self.is_mortally_wounded()
    }

    /// Wound penalty to all checks: 0, −2 or −4.
    pub fn wound_penalty(&self) -> i32 {
        if self.is_mortally_wounded() {
            rules::MORTALLY_WOUNDED_PENALTY
        } else if self.is_seriously_wounded() {
            rules::SERIOUSLY_WOUNDED_PENALTY
        } else {
            0
        }
    }

    /// Worst armor penalty of the worn pieces.
    pub fn armor_penalty(&self) -> i32 {
        self.statblock.armor.penalty()
    }

    /// A statistic after armor penalties (REF, DEX, MOVE) and the mortally
    /// wounded MOVE penalty. Other statistics are returned unchanged.
    pub fn effective_stat(&self, stat: Stat) -> i32 {
        let raw = self.statblock.stats.get(stat);
        match stat {
            Stat::Ref | Stat::Dex => (raw - self.armor_penalty()).max(0),
            Stat::Move => {
                let mortal = if self.is_mortally_wounded() { rules::MORTALLY_WOUNDED_MOVE_PENALTY } else { 0 };
                (raw - self.armor_penalty() - mortal).max(1)
            }
            _ => raw,
        }
    }

    /// Able to take a turn. Mortally wounded combatants still act (with
    /// penalties); only the dead cannot.
    pub fn can_act(&self) -> bool {
        self.alive
    }

    /// Whether this combatant may dodge ranged attacks right now.
    pub fn can_dodge(&self, tables: &RulesTables) -> bool {
        self.alive && !self.is_mortally_wounded() && self.effective_stat(Stat::Ref) >= tables.dodge_min_ref
    }
}

/// A rolled initiative value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitiativeEntry {
    pub combatant: CombatantId,
    pub value: i32,
    pub natural: u8,
}

/// Where the combat is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Phase {
    /// Created; waiting for `StartCombat`.
    Setup,
    RollingInitiative,
    Active,
    Ended { winner: Option<Side> },
}

/// A cover object standing on a `%` square.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CoverObject {
    pub pos: Pos,
    pub hp: i32,
}

/// The complete, serializable state of one combat. Everything the rules need
/// is in here (including the rules tables), so `initial state + command log`
/// reproduces the combat exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatState {
    pub schema_version: u32,
    pub battlefield: Battlefield,
    pub combatants: Vec<Combatant>,
    pub phase: Phase,
    pub round: u32,
    /// Index into `initiative` of the combatant whose turn it is.
    pub turn: usize,
    pub initiative: Vec<InitiativeEntry>,
    /// Resolution queue: the front step is what happens next.
    pub queue: VecDeque<Step>,
    /// Sequence number of the next human input (roll or decision).
    pub input_seq: u32,
    pub options: RulesOptions,
    pub rules: RulesTables,
    /// Cover objects still standing, sorted by position.
    pub cover: Vec<CoverObject>,
}

impl CombatState {
    pub fn combatant(&self, id: CombatantId) -> Option<&Combatant> {
        self.combatants.iter().find(|c| c.id == id)
    }

    pub fn combatant_mut(&mut self, id: CombatantId) -> Option<&mut Combatant> {
        self.combatants.iter_mut().find(|c| c.id == id)
    }

    /// Panicking accessor for ids already validated by the engine.
    pub(crate) fn c(&self, id: CombatantId) -> &Combatant {
        self.combatant(id).expect("validated combatant id")
    }

    pub(crate) fn cm(&mut self, id: CombatantId) -> &mut Combatant {
        self.combatant_mut(id).expect("validated combatant id")
    }

    /// Display name of a combatant (empty if unknown).
    pub fn name_of(&self, id: CombatantId) -> String {
        self.combatant(id).map(|c| c.name.clone()).unwrap_or_default()
    }

    /// The combatant whose turn it is.
    pub fn current_actor(&self) -> Option<CombatantId> {
        if self.phase != Phase::Active {
            return None;
        }
        self.initiative.get(self.turn).map(|e| e.combatant)
    }

    pub fn is_over(&self) -> bool {
        matches!(self.phase, Phase::Ended { .. })
    }

    /// Initiative index of a combatant.
    pub fn initiative_index(&self, id: CombatantId) -> Option<usize> {
        self.initiative.iter().position(|e| e.combatant == id)
    }

    /// Living combatants hostile to `id`.
    pub fn enemies_of(&self, id: CombatantId) -> impl Iterator<Item = &Combatant> {
        let side = self.combatant(id).map(|c| c.side).unwrap_or(Side::Neutral);
        self.combatants.iter().filter(move |c| c.alive && side.is_hostile_to(c.side))
    }

    /// The cover object on a square, if one is standing there.
    pub fn cover_at(&self, pos: Pos) -> Option<&CoverObject> {
        self.cover.iter().find(|c| c.pos == pos)
    }

    /// The combatant standing on a square (dead bodies excluded).
    pub fn occupant(&self, pos: Pos) -> Option<&Combatant> {
        self.combatants.iter().find(|c| c.alive && c.position == pos)
    }

    /// Stable FNV-1a hash of the canonical JSON form; used to verify replays.
    pub fn fingerprint(&self) -> String {
        tactics_core::system::fingerprint_json(&serde_json::to_string(self).unwrap_or_default())
    }
}
