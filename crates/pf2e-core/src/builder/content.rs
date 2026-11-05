//! Rules content registry: ancestries, backgrounds, classes, feats, items,
//! spells. Content is data (JSON) so it can be corrected or extended without
//! touching engine code.

use crate::models::feat::Feat;
use crate::models::monster::Monster;
use crate::models::spell::{Spell, Tradition};
use crate::models::statblock::{ReactionKind, SpellcastingKind};
use crate::models::weapon::{Armor, Shield, Weapon};
use crate::stats::{Attribute, Proficiency, Size, Skill};
use crate::dice::DiceExpr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoostSlot {
    Fixed(Attribute),
    Free,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AncestryDef {
    pub id: String,
    pub name: String,
    pub hp: i32,
    #[serde(default)]
    pub size: Size,
    pub speed: u32,
    pub boosts: Vec<BoostSlot>,
    #[serde(default)]
    pub flaws: Vec<Attribute>,
    #[serde(default)]
    pub traits: Vec<String>,
    #[serde(default)]
    pub vision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundDef {
    pub id: String,
    pub name: String,
    /// The first background boost must be one of these.
    pub boost_options: Vec<Attribute>,
    #[serde(default)]
    pub trained_skills: Vec<Skill>,
    #[serde(default)]
    pub skill_feat: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressionStep {
    pub level: u8,
    pub rank: Proficiency,
}

pub fn rank_at(steps: &[ProgressionStep], level: u8) -> Proficiency {
    steps.iter().filter(|s| s.level <= level).map(|s| s.rank).max().unwrap_or(Proficiency::Untrained)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotProgression {
    /// Standard full caster: 3 slots in each lower rank, 2 (odd level) or 3
    /// (even level) in the highest rank; max rank = ceil(level / 2).
    Full,
}

impl SlotProgression {
    pub fn slots(self, level: u8) -> BTreeMap<u8, u8> {
        let mut out = BTreeMap::new();
        let level = level.clamp(1, 20);
        let max_rank = level.div_ceil(2).min(10);
        for r in 1..=max_rank.min(9) {
            let n = if r < max_rank { 3 } else if level % 2 == 1 { 2 } else { 3 };
            out.insert(r, n);
        }
        if level >= 19 {
            out.insert(10, 1);
            out.insert(9, 3);
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassSpellcasting {
    pub tradition: Tradition,
    pub kind: SpellcastingKind,
    pub attribute: Attribute,
    pub proficiency: Vec<ProgressionStep>,
    pub slots: SlotProgression,
    pub cantrips: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ClassFeatureKind {
    Reaction { reaction: ReactionKind },
    SneakAttack { dice: DiceExpr },
    WeaponSpecialization,
    GreaterWeaponSpecialization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassFeature {
    pub level: u8,
    pub name: String,
    pub feature: ClassFeatureKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassDef {
    pub id: String,
    pub name: String,
    pub hp_per_level: i32,
    pub key_attributes: Vec<Attribute>,
    pub perception: Vec<ProgressionStep>,
    pub fortitude: Vec<ProgressionStep>,
    pub reflex: Vec<ProgressionStep>,
    pub will: Vec<ProgressionStep>,
    #[serde(default)]
    pub unarmed: Vec<ProgressionStep>,
    #[serde(default)]
    pub simple: Vec<ProgressionStep>,
    #[serde(default)]
    pub martial: Vec<ProgressionStep>,
    #[serde(default)]
    pub advanced: Vec<ProgressionStep>,
    #[serde(default)]
    pub unarmored: Vec<ProgressionStep>,
    #[serde(default)]
    pub light: Vec<ProgressionStep>,
    #[serde(default)]
    pub medium: Vec<ProgressionStep>,
    #[serde(default)]
    pub heavy: Vec<ProgressionStep>,
    #[serde(default)]
    pub class_dc: Vec<ProgressionStep>,
    #[serde(default)]
    pub fixed_skills: Vec<Skill>,
    pub additional_skills: i32,
    pub class_feat_levels: Vec<u8>,
    pub skill_feat_levels: Vec<u8>,
    pub general_feat_levels: Vec<u8>,
    pub ancestry_feat_levels: Vec<u8>,
    pub skill_increase_levels: Vec<u8>,
    #[serde(default)]
    pub spellcasting: Option<ClassSpellcasting>,
    #[serde(default)]
    pub features: Vec<ClassFeature>,
}

/// All rules content known to the engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Content {
    pub ancestries: Vec<AncestryDef>,
    pub backgrounds: Vec<BackgroundDef>,
    pub classes: Vec<ClassDef>,
    pub feats: Vec<Feat>,
    pub weapons: Vec<Weapon>,
    pub armor: Vec<Armor>,
    pub shields: Vec<Shield>,
    pub spells: Vec<Spell>,
    /// Sample monsters/hazards shipped with the engine (seed data).
    #[serde(default)]
    pub monsters: Vec<Monster>,
}

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("failed to parse {file}: {source}")]
    Parse { file: String, source: serde_json::Error },
    #[error("duplicate {kind} id '{id}'")]
    Duplicate { kind: &'static str, id: String },
}

macro_rules! bundled {
    ($name:literal) => {
        ($name, include_str!(concat!("../../../../content/", $name)))
    };
}

impl Content {
    /// The sample content shipped in `/content`.
    pub fn bundled() -> Result<Content, ContentError> {
        let files = [
            bundled!("ancestries.json"),
            bundled!("backgrounds.json"),
            bundled!("classes.json"),
            bundled!("feats.json"),
            bundled!("weapons.json"),
            bundled!("armor.json"),
            bundled!("shields.json"),
            bundled!("spells.json"),
            bundled!("monsters.json"),
        ];
        Content::from_files(files.iter().map(|(n, s)| (n.to_string(), s.to_string())))
    }

    /// Builds content from `(file name, JSON array)` pairs; the file stem
    /// selects the collection.
    pub fn from_files(files: impl IntoIterator<Item = (String, String)>) -> Result<Content, ContentError> {
        let mut c = Content::default();
        for (name, text) in files {
            let p = |e| ContentError::Parse { file: name.clone(), source: e };
            match name.trim_end_matches(".json") {
                "ancestries" => c.ancestries.extend(serde_json::from_str::<Vec<AncestryDef>>(&text).map_err(p)?),
                "backgrounds" => c.backgrounds.extend(serde_json::from_str::<Vec<BackgroundDef>>(&text).map_err(p)?),
                "classes" => c.classes.extend(serde_json::from_str::<Vec<ClassDef>>(&text).map_err(p)?),
                "feats" => c.feats.extend(serde_json::from_str::<Vec<Feat>>(&text).map_err(p)?),
                "weapons" => c.weapons.extend(serde_json::from_str::<Vec<Weapon>>(&text).map_err(p)?),
                "armor" => c.armor.extend(serde_json::from_str::<Vec<Armor>>(&text).map_err(p)?),
                "shields" => c.shields.extend(serde_json::from_str::<Vec<Shield>>(&text).map_err(p)?),
                "spells" => c.spells.extend(serde_json::from_str::<Vec<Spell>>(&text).map_err(p)?),
                "monsters" => c.monsters.extend(serde_json::from_str::<Vec<Monster>>(&text).map_err(p)?),
                _ => {}
            }
        }
        c.check_unique()?;
        Ok(c)
    }

    fn check_unique(&self) -> Result<(), ContentError> {
        fn dup<'a>(kind: &'static str, ids: impl Iterator<Item = &'a str>) -> Result<(), ContentError> {
            let mut seen = std::collections::HashSet::new();
            for id in ids {
                if !seen.insert(id) {
                    return Err(ContentError::Duplicate { kind, id: id.to_string() });
                }
            }
            Ok(())
        }
        dup("ancestry", self.ancestries.iter().map(|x| x.id.as_str()))?;
        dup("background", self.backgrounds.iter().map(|x| x.id.as_str()))?;
        dup("class", self.classes.iter().map(|x| x.id.as_str()))?;
        dup("feat", self.feats.iter().map(|x| x.id.as_str()))?;
        dup("weapon", self.weapons.iter().map(|x| x.id.as_str()))?;
        dup("armor", self.armor.iter().map(|x| x.id.as_str()))?;
        dup("shield", self.shields.iter().map(|x| x.id.as_str()))?;
        dup("spell", self.spells.iter().map(|x| x.id.as_str()))?;
        dup("monster", self.monsters.iter().map(|x| x.id.as_str()))?;
        Ok(())
    }

    pub fn ancestry(&self, id: &str) -> Option<&AncestryDef> {
        self.ancestries.iter().find(|x| x.id == id)
    }
    pub fn background(&self, id: &str) -> Option<&BackgroundDef> {
        self.backgrounds.iter().find(|x| x.id == id)
    }
    pub fn class(&self, id: &str) -> Option<&ClassDef> {
        self.classes.iter().find(|x| x.id == id)
    }
    pub fn feat(&self, id: &str) -> Option<&Feat> {
        self.feats.iter().find(|x| x.id == id)
    }
    pub fn weapon(&self, id: &str) -> Option<&Weapon> {
        self.weapons.iter().find(|x| x.id == id)
    }
    pub fn armor_item(&self, id: &str) -> Option<&Armor> {
        self.armor.iter().find(|x| x.id == id)
    }
    pub fn shield(&self, id: &str) -> Option<&Shield> {
        self.shields.iter().find(|x| x.id == id)
    }
    pub fn spell(&self, id: &str) -> Option<&Spell> {
        self.spells.iter().find(|x| x.id == id)
    }
}

/// Lookup of spells by id, which is all the combat engine needs.
pub trait SpellLibrary {
    fn spell(&self, id: &str) -> Option<&Spell>;
}

impl SpellLibrary for Content {
    fn spell(&self, id: &str) -> Option<&Spell> {
        Content::spell(self, id)
    }
}

impl SpellLibrary for BTreeMap<String, Spell> {
    fn spell(&self, id: &str) -> Option<&Spell> {
        self.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_content_loads() {
        let c = Content::bundled().expect("bundled content must parse");
        assert!(c.classes.len() >= 4);
        assert!(c.spell("fireball").is_some());
    }

    #[test]
    fn full_caster_slots() {
        let s = SlotProgression::Full;
        assert_eq!(s.slots(1), BTreeMap::from([(1, 2)]));
        assert_eq!(s.slots(2), BTreeMap::from([(1, 3)]));
        assert_eq!(s.slots(5), BTreeMap::from([(1, 3), (2, 3), (3, 2)]));
    }
}
