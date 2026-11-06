//! Building the initial `CombatState`.

use super::state::{ArmorSp, CombatState, Combatant, CoverObject, Phase, TurnEconomy, STATE_SCHEMA_VERSION};
use crate::content::Content;
use crate::model::{ActorKind, Battlefield, HitLocation, Statblock, Terrain, Weapon};
use crate::rules::RulesOptions;
use std::collections::{BTreeMap, VecDeque};
use tactics_core::{CombatantId, Pos, Side};

/// One combatant to place in a new combat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupParticipant {
    pub name: String,
    pub side: Side,
    pub kind: ActorKind,
    pub statblock: Statblock,
    pub position: Pos,
    /// Starting HP if not at full health (clamped to 0..=max).
    pub current_hp: Option<i32>,
}

/// Why a combat could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetupError {
    #[error("invalid battlefield: {0}")]
    Battlefield(String),
    #[error("{0} is placed off the map or on a wall or cover square")]
    BadPosition(String),
    #[error("{0} and {1} share a square")]
    Overlap(String, String),
    #[error("invalid statblock: {0}")]
    Statblock(String),
}

/// Builds combats. Stateless: the rules tables come from the [`Content`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CombatSetup;

impl CombatSetup {
    /// Builds a combat in [`Phase::Setup`] with default [`RulesOptions`].
    pub fn build(battlefield: Battlefield, participants: Vec<SetupParticipant>, content: &Content) -> Result<CombatState, SetupError> {
        Self::build_with_options(battlefield, participants, content, RulesOptions::default())
    }

    /// Builds a combat with explicit table-rule options.
    ///
    /// Each combatant gets the implicit Brawling weapon appended (damage from
    /// BODY), full magazines, its armor SP and full (or `current_hp`) HP.
    /// Combatant ids are assigned 1, 2, ... in participant order.
    pub fn build_with_options(
        battlefield: Battlefield,
        participants: Vec<SetupParticipant>,
        content: &Content,
        options: RulesOptions,
    ) -> Result<CombatState, SetupError> {
        battlefield.validate().map_err(|e| SetupError::Battlefield(e.to_string()))?;
        let rules = content.rules.clone();
        let mut placed: BTreeMap<Pos, String> = BTreeMap::new();
        let mut combatants = Vec::with_capacity(participants.len());
        for (i, p) in participants.into_iter().enumerate() {
            p.statblock.validate().map_err(|e| SetupError::Statblock(e.to_string()))?;
            if !battlefield.in_bounds(p.position) || !battlefield.terrain(p.position).standable() {
                return Err(SetupError::BadPosition(p.name));
            }
            if let Some(other) = placed.get(&p.position) {
                return Err(SetupError::Overlap(p.name, other.clone()));
            }
            placed.insert(p.position, p.name.clone());
            let mut statblock = p.statblock;
            statblock.weapons.push(Weapon::brawling(rules.brawling_dice(statblock.stats.body)));
            let max = statblock.effective_max_hp();
            combatants.push(Combatant {
                id: CombatantId(i as u32 + 1),
                name: p.name,
                side: p.side,
                kind: p.kind,
                hp: p.current_hp.map(|hp| hp.clamp(0, max)).unwrap_or(max),
                position: p.position,
                economy: TurnEconomy::default(),
                ammo: statblock.weapons.iter().map(|w| w.magazine).collect(),
                armor_sp: ArmorSp { body: statblock.armor.sp(HitLocation::Body), head: statblock.armor.sp(HitLocation::Head) },
                injuries: Vec::new(),
                death_save_penalty: 0,
                alive: true,
                statblock,
            });
        }
        let cover = battlefield.squares_of(Terrain::Cover).into_iter().map(|pos| CoverObject { pos, hp: options.cover_hp.max(1) }).collect();
        Ok(CombatState {
            schema_version: STATE_SCHEMA_VERSION,
            battlefield,
            combatants,
            phase: Phase::Setup,
            round: 0,
            turn: 0,
            initiative: Vec::new(),
            queue: VecDeque::new(),
            input_seq: 1,
            options,
            rules,
            cover,
        })
    }
}
