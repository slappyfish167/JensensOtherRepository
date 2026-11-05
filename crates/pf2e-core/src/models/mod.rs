//! Explicit data models. Content models (weapons, spells, feats, monsters,
//! characters, encounters) are plain serializable data; runtime combat models
//! (`Combatant`, `InitiativeEntry`) live in `crate::combat::state`.

pub mod battlefield;
pub mod character;
pub mod encounter;
pub mod feat;
pub mod monster;
pub mod spell;
pub mod statblock;
pub mod weapon;

pub use battlefield::{Battlefield, Terrain};
pub use character::Character;
pub use encounter::{Encounter, EncounterParticipant, ParticipantSource};
pub use feat::Feat;
pub use monster::{Monster, MonsterTemplate};
pub use spell::Spell;
pub use statblock::Statblock;
pub use weapon::{Armor, Shield, Weapon};
