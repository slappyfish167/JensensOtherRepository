//! Content loading: weapons, armor, rules tables, actors and encounters.
//!
//! The bundled sample content lives in `content/cpred/*.json` and is compiled
//! into the crate ([`Content::bundled`]). In actor files a statblock may
//! reference catalogue entries by id (`"weapon_ids": [...]`,
//! `"armor_ids": {"body": "...", "head": "..."}`) instead of — or in addition
//! to — listing full `weapons` / `armor`; `Content` resolves them into full
//! structs at load time.

use crate::combat::{CombatSetup, CombatState, SetupError, SetupParticipant};
use crate::model::{Actor, ActorKind, Armor, ArmorPiece, Encounter, Skill, Statblock, Stats, Weapon};
use crate::rules::RulesTables;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// All loaded content, fully resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub weapons: Vec<Weapon>,
    pub armor: Vec<ArmorPiece>,
    pub rules: RulesTables,
    pub actors: Vec<Actor>,
    pub encounters: Vec<Encounter>,
}

/// Why content could not be loaded or resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentError {
    #[error("{file}: {error}")]
    Json { file: String, error: String },
    #[error("duplicate {kind} id '{id}'")]
    DuplicateId { kind: &'static str, id: String },
    #[error("actor '{actor}' references unknown weapon '{id}'")]
    UnknownWeapon { actor: String, id: String },
    #[error("actor '{actor}' references unknown armor '{id}'")]
    UnknownArmor { actor: String, id: String },
    #[error("encounter '{encounter}' references unknown actor '{actor}'")]
    UnknownActor { encounter: String, actor: String },
    #[error("encounter '{encounter}': participant #{index} has neither an actor_id nor a statblock")]
    EmptyParticipant { encounter: String, index: usize },
    #[error("invalid statblock: {0}")]
    Statblock(String),
    #[error("unknown encounter '{0}'")]
    UnknownEncounter(String),
    #[error("encounter '{encounter}' cannot be set up: {error}")]
    Setup { encounter: String, error: SetupError },
}

/// Catalogue armor references of an actor statblock.
#[derive(Debug, Clone, Default, Deserialize)]
struct ArmorIds {
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    head: Option<String>,
}

/// A statblock as written in `actors.json`.
#[derive(Debug, Clone, Deserialize)]
struct StatblockSpec {
    #[serde(default)]
    name: Option<String>,
    stats: Stats,
    #[serde(default)]
    skills: BTreeMap<Skill, i32>,
    #[serde(default)]
    weapons: Vec<Weapon>,
    #[serde(default)]
    weapon_ids: Vec<String>,
    #[serde(default)]
    armor: Option<Armor>,
    #[serde(default)]
    armor_ids: ArmorIds,
    #[serde(default)]
    max_hp: Option<i32>,
    #[serde(default)]
    notes: String,
}

/// An actor as written in `actors.json`.
#[derive(Debug, Clone, Deserialize)]
struct ActorSpec {
    id: String,
    name: String,
    #[serde(default)]
    kind: ActorKind,
    statblock: StatblockSpec,
}

fn parse<T: serde::de::DeserializeOwned>(file: &str, json: &str) -> Result<T, ContentError> {
    serde_json::from_str(json).map_err(|e| ContentError::Json { file: file.to_string(), error: e.to_string() })
}

fn ensure_unique<'a>(kind: &'static str, ids: impl Iterator<Item = &'a str>) -> Result<(), ContentError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !id.is_empty() && !seen.insert(id) {
            return Err(ContentError::DuplicateId { kind, id: id.to_string() });
        }
    }
    Ok(())
}

impl Content {
    /// The sample content shipped in `content/cpred/`.
    pub fn bundled() -> Result<Content, ContentError> {
        Self::from_json(
            include_str!("../../../content/cpred/weapons.json"),
            include_str!("../../../content/cpred/armor.json"),
            include_str!("../../../content/cpred/rules.json"),
            include_str!("../../../content/cpred/actors.json"),
            include_str!("../../../content/cpred/encounters.json"),
        )
    }

    /// Parses and resolves the five content files.
    pub fn from_json(weapons: &str, armor: &str, rules: &str, actors: &str, encounters: &str) -> Result<Content, ContentError> {
        let weapons: Vec<Weapon> = parse("weapons.json", weapons)?;
        let armor: Vec<ArmorPiece> = parse("armor.json", armor)?;
        let rules: RulesTables = parse("rules.json", rules)?;
        let specs: Vec<ActorSpec> = parse("actors.json", actors)?;
        let encounters: Vec<Encounter> = parse("encounters.json", encounters)?;
        ensure_unique("weapon", weapons.iter().map(|w| w.id.as_str()))?;
        ensure_unique("armor", armor.iter().map(|a| a.id.as_str()))?;
        ensure_unique("actor", specs.iter().map(|a| a.id.as_str()))?;
        ensure_unique("encounter", encounters.iter().map(|e| e.id.as_str()))?;
        let mut content = Content { weapons, armor, rules, actors: Vec::new(), encounters };
        for spec in specs {
            let actor = content.resolve_actor(spec)?;
            content.actors.push(actor);
        }
        for e in &content.encounters {
            content.participants(e)?;
        }
        Ok(content)
    }

    fn resolve_actor(&self, spec: ActorSpec) -> Result<Actor, ContentError> {
        let s = spec.statblock;
        let mut weapons = s.weapons;
        for id in &s.weapon_ids {
            let w = self.weapon(id).ok_or_else(|| ContentError::UnknownWeapon { actor: spec.id.clone(), id: id.clone() })?;
            weapons.push(w.clone());
        }
        let piece = |id: &Option<String>| -> Result<Option<ArmorPiece>, ContentError> {
            id.as_ref()
                .map(|id| self.armor_piece(id).cloned().ok_or_else(|| ContentError::UnknownArmor { actor: spec.id.clone(), id: id.clone() }))
                .transpose()
        };
        let mut armor = s.armor.unwrap_or_default();
        if let Some(body) = piece(&s.armor_ids.body)? {
            armor.body = Some(body);
        }
        if let Some(head) = piece(&s.armor_ids.head)? {
            armor.head = Some(head);
        }
        let statblock = Statblock {
            name: s.name.unwrap_or_else(|| spec.name.clone()),
            stats: s.stats,
            skills: s.skills,
            weapons,
            armor,
            max_hp: s.max_hp,
            notes: s.notes,
        };
        statblock.validate().map_err(|e| ContentError::Statblock(e.to_string()))?;
        Ok(Actor { id: spec.id, name: spec.name, kind: spec.kind, statblock })
    }

    pub fn weapon(&self, id: &str) -> Option<&Weapon> {
        self.weapons.iter().find(|w| w.id == id)
    }

    pub fn armor_piece(&self, id: &str) -> Option<&ArmorPiece> {
        self.armor.iter().find(|a| a.id == id)
    }

    pub fn actor(&self, id: &str) -> Option<&Actor> {
        self.actors.iter().find(|a| a.id == id)
    }

    pub fn encounter(&self, id: &str) -> Option<&Encounter> {
        self.encounters.iter().find(|e| e.id == id)
    }

    /// Resolves an encounter's participants into setup participants.
    /// Repeated unnamed actors get numbered names ("Street Ganger 2").
    pub fn participants(&self, encounter: &Encounter) -> Result<Vec<SetupParticipant>, ContentError> {
        let mut out = Vec::with_capacity(encounter.participants.len());
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for (index, p) in encounter.participants.iter().enumerate() {
            let (base_name, kind, statblock) = match (&p.statblock, &p.actor_id) {
                (Some(sb), _) => (sb.name.clone(), ActorKind::Npc, sb.clone()),
                (None, Some(id)) => {
                    let a = self
                        .actor(id)
                        .ok_or_else(|| ContentError::UnknownActor { encounter: encounter.id.clone(), actor: id.clone() })?;
                    (a.name.clone(), a.kind, a.statblock.clone())
                }
                (None, None) => return Err(ContentError::EmptyParticipant { encounter: encounter.id.clone(), index }),
            };
            let name = match &p.name {
                Some(n) => n.clone(),
                None => {
                    let n = counts.entry(base_name.clone()).or_insert(0);
                    *n += 1;
                    if *n == 1 {
                        base_name
                    } else {
                        format!("{base_name} {n}")
                    }
                }
            };
            out.push(SetupParticipant {
                name,
                side: p.side,
                kind: p.kind.unwrap_or(kind),
                statblock,
                position: p.position,
                current_hp: p.current_hp,
            });
        }
        Ok(out)
    }

    /// Builds a ready-to-start combat for a stored encounter.
    pub fn build_encounter(&self, id: &str) -> Result<CombatState, ContentError> {
        let e = self.encounter(id).ok_or_else(|| ContentError::UnknownEncounter(id.to_string()))?;
        let participants = self.participants(e)?;
        CombatSetup::build(e.battlefield.clone(), participants, self)
            .map_err(|error| ContentError::Setup { encounter: e.id.clone(), error })
    }
}
