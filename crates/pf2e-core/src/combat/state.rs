//! Runtime combat models: `Combatant`, `InitiativeEntry`, `CombatState`.

use crate::conditions::{ConditionKind, ConditionSet};
use crate::damage::DamageType;
use crate::dice::DiceExpr;
use crate::effects::{Effect, EffectTag};
use crate::grid::{footprint, Pos};
use crate::ids::CombatantId;
use crate::models::battlefield::Battlefield;
use crate::models::spell::Spell;
use crate::models::statblock::Statblock;
use crate::stats::Side;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

use super::pending::Step;

pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatantKind {
    Character,
    Monster,
    Hazard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentDamage {
    pub dice: DiceExpr,
    pub damage_type: DamageType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CombatantId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifeState {
    Active,
    Dead,
    /// Hazard disabled or spent.
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combatant {
    pub id: CombatantId,
    pub name: String,
    pub side: Side,
    pub kind: CombatantKind,
    pub statblock: Statblock,
    pub hp: i32,
    #[serde(default)]
    pub temp_hp: i32,
    pub position: Pos,
    #[serde(default)]
    pub conditions: ConditionSet,
    #[serde(default)]
    pub persistent: Vec<PersistentDamage>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub actions_remaining: u8,
    #[serde(default = "yes")]
    pub reaction_available: bool,
    #[serde(default)]
    pub attacks_this_turn: u8,
    /// Targets Struck this turn (for the sweep trait).
    #[serde(default)]
    pub struck_this_turn: Vec<CombatantId>,
    #[serde(default)]
    pub shield_hp: i32,
    /// Remaining slots per spellcasting entry (entry index → rank → count).
    #[serde(default)]
    pub spell_slots: Vec<BTreeMap<u8, u8>>,
    pub life: LifeState,
    /// Takes turns in initiative (false for simple hazards).
    pub in_initiative: bool,
}

fn yes() -> bool {
    true
}

impl Combatant {
    pub fn from_statblock(id: CombatantId, name: String, side: Side, kind: CombatantKind, sb: Statblock, position: Pos) -> Self {
        let in_initiative = sb.hazard.as_ref().map(|h| h.complex).unwrap_or(true);
        Combatant {
            id,
            name,
            side,
            kind,
            hp: sb.max_hp,
            temp_hp: 0,
            position,
            conditions: ConditionSet::default(),
            persistent: Vec::new(),
            effects: Vec::new(),
            actions_remaining: 0,
            reaction_available: true,
            attacks_this_turn: 0,
            struck_this_turn: Vec::new(),
            shield_hp: sb.shield.as_ref().map(|s| s.max_hp).unwrap_or(0),
            spell_slots: sb.spellcasting.iter().map(|e| e.slots.clone()).collect(),
            life: LifeState::Active,
            in_initiative,
            statblock: sb,
        }
    }

    pub fn is_dead(&self) -> bool {
        self.life == LifeState::Dead
    }

    pub fn is_hazard(&self) -> bool {
        self.kind == CombatantKind::Hazard
    }

    /// Dead, inactive, or unconscious.
    pub fn is_down(&self) -> bool {
        self.life != LifeState::Active || self.conditions.has(ConditionKind::Unconscious)
    }

    /// Able to take actions and reactions.
    pub fn can_act(&self) -> bool {
        !self.is_down() && !self.conditions.has(ConditionKind::Paralyzed)
    }

    pub fn has(&self, kind: ConditionKind) -> bool {
        self.conditions.has(kind)
    }

    pub fn has_tag(&self, tag: &EffectTag) -> bool {
        self.effects.iter().any(|e| e.tags.contains(tag))
    }

    pub fn footprint(&self) -> Vec<Pos> {
        footprint(self.position, self.statblock.size)
    }

    pub fn shield_broken(&self) -> bool {
        self.statblock.shield.as_ref().map(|s| self.shield_hp <= s.broken_threshold).unwrap_or(true)
    }

    /// Max HP after drained.
    pub fn effective_max_hp(&self) -> i32 {
        let drained = self.conditions.value(ConditionKind::Drained) as i32;
        (self.statblock.max_hp - drained * self.statblock.level.max(1)).max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitiativeEntry {
    pub combatant: CombatantId,
    pub value: i32,
    pub natural: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Phase {
    /// Created; waiting for `StartCombat`.
    Setup,
    RollingInitiative,
    Active,
    Ended { winner: Option<Side> },
}

/// Table-rule switches. Part of the state so replays honour them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesOptions {
    pub flanking: bool,
    /// Automatically compute cover from the map (GM can still override per Strike).
    pub auto_cover: bool,
}

impl Default for RulesOptions {
    fn default() -> Self {
        RulesOptions { flanking: true, auto_cover: true }
    }
}

/// The complete, serializable state of one combat. Everything the rules need
/// is in here (including spell definitions), so `initial state + command log`
/// reproduces the combat exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatState {
    pub schema_version: u32,
    pub battlefield: Battlefield,
    pub combatants: Vec<Combatant>,
    #[serde(default)]
    pub spells: BTreeMap<String, Spell>,
    pub phase: Phase,
    pub round: u32,
    /// Index into `initiative` of the creature whose turn it is.
    pub turn: usize,
    pub initiative: Vec<InitiativeEntry>,
    /// Resolution queue: the front step is what happens next.
    pub queue: VecDeque<Step>,
    /// Sequence number of the next human input (roll or decision).
    pub input_seq: u32,
    pub next_effect_id: u32,
    pub next_tag: u32,
    #[serde(default)]
    pub options: RulesOptions,
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

    pub fn current_actor(&self) -> Option<CombatantId> {
        if self.phase != Phase::Active {
            return None;
        }
        self.initiative.get(self.turn).map(|e| e.combatant)
    }

    pub fn is_over(&self) -> bool {
        matches!(self.phase, Phase::Ended { .. })
    }

    pub fn living_on_side(&self, side: Side) -> impl Iterator<Item = &Combatant> {
        self.combatants.iter().filter(move |c| c.side == side && !c.is_down() && !c.is_hazard())
    }

    pub fn enemies_of(&self, id: CombatantId) -> impl Iterator<Item = &Combatant> {
        let side = self.combatant(id).map(|c| c.side).unwrap_or(Side::Neutral);
        self.combatants.iter().filter(move |c| side.is_hostile_to(c.side))
    }

    /// Initiative index of a combatant, if it takes turns.
    pub fn initiative_index(&self, id: CombatantId) -> Option<usize> {
        self.initiative.iter().position(|e| e.combatant == id)
    }

    pub(crate) fn alloc_tag(&mut self) -> u32 {
        self.next_tag += 1;
        self.next_tag
    }

    pub(crate) fn alloc_effect_id(&mut self) -> u32 {
        self.next_effect_id += 1;
        self.next_effect_id
    }

    /// Stable FNV-1a hash of the canonical JSON form; used to verify replays.
    pub fn fingerprint(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        let mut h: u64 = 0xcbf29ce484222325;
        for b in json.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    }
}
