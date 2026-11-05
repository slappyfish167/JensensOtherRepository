//! Building the initial `CombatState`.

use super::state::{CombatState, Combatant, CombatantKind, Phase, RulesOptions, STATE_SCHEMA_VERSION};
use crate::builder::content::SpellLibrary;
use crate::grid::Pos;
use crate::ids::CombatantId;
use crate::models::battlefield::Battlefield;
use crate::models::statblock::Statblock;
use crate::stats::Side;
use std::collections::{BTreeMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct SetupParticipant {
    pub name: String,
    pub side: Side,
    pub kind: CombatantKind,
    pub statblock: Statblock,
    pub position: Pos,
    pub current_hp: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetupError {
    #[error("invalid battlefield: {0}")]
    Battlefield(String),
    #[error("{0} is placed off the map or inside a wall")]
    BadPosition(String),
    #[error("{0} and {1} overlap")]
    Overlap(String, String),
    #[error("{name} knows unknown spell '{spell}'")]
    UnknownSpell { name: String, spell: String },
}

#[derive(Debug, Clone, Default)]
pub struct CombatSetup {
    pub options: RulesOptions,
}

impl CombatSetup {
    pub fn build(
        &self,
        battlefield: Battlefield,
        participants: Vec<SetupParticipant>,
        library: &dyn SpellLibrary,
    ) -> Result<CombatState, SetupError> {
        battlefield.validate().map_err(|e| SetupError::Battlefield(e.to_string()))?;
        let mut occupied: Vec<(HashSet<Pos>, String)> = Vec::new();
        let mut spells = BTreeMap::new();
        let mut combatants = Vec::new();
        for (i, p) in participants.into_iter().enumerate() {
            let fp = crate::grid::footprint(p.position, p.statblock.size);
            if fp.iter().any(|sq| battlefield.terrain(*sq) == crate::models::battlefield::Terrain::Wall) {
                return Err(SetupError::BadPosition(p.name));
            }
            if p.kind != CombatantKind::Hazard {
                let set: HashSet<Pos> = fp.into_iter().collect();
                if let Some((_, other)) = occupied.iter().find(|(o, _)| !o.is_disjoint(&set)) {
                    return Err(SetupError::Overlap(p.name, other.clone()));
                }
                occupied.push((set, p.name.clone()));
            }
            for e in &p.statblock.spellcasting {
                for id in e.cantrips.iter().chain(e.spells.iter().map(|k| &k.spell_id)) {
                    let s = library
                        .spell(id)
                        .ok_or_else(|| SetupError::UnknownSpell { name: p.name.clone(), spell: id.clone() })?;
                    spells.insert(id.clone(), s.clone());
                }
            }
            let mut c = Combatant::from_statblock(CombatantId(i as u32 + 1), p.name, p.side, p.kind, p.statblock, p.position);
            if let Some(hp) = p.current_hp {
                c.hp = hp.clamp(0, c.statblock.max_hp);
            }
            combatants.push(c);
        }
        Ok(CombatState {
            schema_version: STATE_SCHEMA_VERSION,
            battlefield: std::sync::Arc::new(battlefield),
            combatants,
            spells: std::sync::Arc::new(spells),
            phase: Phase::Setup,
            round: 0,
            turn: 0,
            initiative: Vec::new(),
            queue: VecDeque::new(),
            input_seq: 1,
            next_effect_id: 0,
            next_tag: 0,
            options: self.options,
        })
    }
}
