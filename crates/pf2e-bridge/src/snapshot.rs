//! `CombatSnapshot`: the JSON contract between the Foundry module's
//! TypeScript extractor (`foundry/src/protocol/snapshot.ts`, kept field for
//! field in sync and checked by a shared fixture) and this crate.
//!
//! Principles:
//! * **Numbers come from Foundry**, already computed by the PF2e system
//!   (AC, saves, strike totals, spell DCs), with *condition-derived*
//!   modifiers stripped by the extractor: conditions are sent separately
//!   and `pf2e-core` applies them, so nothing is counted twice.
//! * **Geometry is in grid squares**, relative to the scene's top-left
//!   (including scene padding is the extractor's job).
//! * **Identity is the token id.** Engine ids are assigned during
//!   conversion and mapped back in every report.
//! * Everything optional has a serde default, so older extractors keep
//!   working; `version` is bumped on breaking changes only.

use pf2e_core::combat::CombatantKind;
use pf2e_core::stats::{Attribute, AttributeSet, Save, Side, Size, Skill};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombatSnapshot {
    pub version: u32,
    pub scene: SceneSnapshot,
    pub combatants: Vec<CombatantSnapshot>,
    pub turn: TurnSnapshot,
    #[serde(default)]
    pub options: SnapshotOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapshotOptions {
    #[serde(default = "yes")]
    pub flanking: bool,
    #[serde(default = "yes")]
    pub auto_cover: bool,
    /// Squares of map kept around the combatants when cropping the scene.
    #[serde(default = "default_margin")]
    pub crop_margin: u32,
}

fn yes() -> bool {
    true
}

fn default_margin() -> u32 {
    12
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        SnapshotOptions { flanking: true, auto_cover: true, crop_margin: default_margin() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridKind {
    Square,
    Gridless,
    Hex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneSnapshot {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub grid: GridKind,
    /// `scene.grid.size`: pixels per square (reports convert back with it).
    pub grid_size_px: f64,
    /// `scene.grid.distance`: feet per square; the engine requires 5.
    pub grid_distance_ft: f64,
    /// Top-left of the scene's playable area in pixels (`scene.dimensions.sceneX/Y`).
    #[serde(default)]
    pub origin_px: [f64; 2],
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub walls: Vec<WallSnapshot>,
    #[serde(default)]
    pub terrain: Vec<TerrainCell>,
}

/// A wall segment in fractional squares (`px / grid_size`, scene-relative).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WallSnapshot {
    pub a: [f64; 2],
    pub b: [f64; 2],
    /// `wall.document.move !== NONE`.
    pub blocks_movement: bool,
    /// `wall.document.sight !== NONE` (informational in v1: movement walls
    /// also block line of effect; sight-only walls are ignored).
    #[serde(default)]
    pub blocks_sight: bool,
    /// Door (`door > 0`) currently open (`ds === 1`): does not block.
    #[serde(default)]
    pub open_door: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerrainKind {
    Difficult,
    GreaterDifficult,
    Obscured,
    Wall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainCell {
    pub x: i32,
    pub y: i32,
    pub kind: TerrainKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnSnapshot {
    /// `combat.started`; when false there is no current creature and turn
    /// analysis uses a hypothetical turn for the requested token.
    pub started: bool,
    #[serde(default)]
    pub round: u32,
    /// Token id of `combat.combatant`.
    #[serde(default)]
    pub current: Option<String>,
    /// Token ids in `combat.turns` order (initiative order).
    #[serde(default)]
    pub order: Vec<String>,
    /// Actions the current creature has already used this turn
    /// (module turn tracker; the PF2e system does not track this).
    #[serde(default)]
    pub actions_spent: u8,
    /// Attacks made this turn, for the multiple attack penalty.
    #[serde(default)]
    pub attacks_made: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombatantSnapshot {
    pub token_id: String,
    #[serde(default)]
    pub actor_id: String,
    pub name: String,
    pub kind: CombatantKind,
    pub side: Side,
    /// Top-left square of the token (scene-relative).
    pub x: i32,
    pub y: i32,
    pub level: i32,
    #[serde(default)]
    pub size: Size,
    #[serde(default)]
    pub traits: Vec<String>,
    pub hp: i32,
    pub max_hp: i32,
    #[serde(default)]
    pub temp_hp: i32,
    pub ac: i32,
    pub fortitude: i32,
    pub reflex: i32,
    pub will: i32,
    pub perception: i32,
    #[serde(default = "default_speed")]
    pub speed: u32,
    #[serde(default)]
    pub attributes: AttributeSet,
    #[serde(default)]
    pub skills: BTreeMap<Skill, i32>,
    #[serde(default)]
    pub strikes: Vec<StrikeSnapshot>,
    #[serde(default)]
    pub spellcasting: Vec<SpellcastingSnapshot>,
    #[serde(default)]
    pub iwr: IwrSnapshot,
    #[serde(default)]
    pub reactions: ReactionsSnapshot,
    #[serde(default)]
    pub shield: Option<ShieldSnapshot>,
    /// PF2e condition slugs (`off-guard`, `frightened`, ...) with values.
    #[serde(default)]
    pub conditions: Vec<ConditionSnapshot>,
    #[serde(default)]
    pub persistent: Vec<PersistentSnapshot>,
    #[serde(default)]
    pub class_dc: Option<i32>,
    /// Rogue sneak attack dice, e.g. `"1d6"`.
    #[serde(default)]
    pub sneak_attack: Option<String>,
    #[serde(default)]
    pub initiative: Option<f64>,
    #[serde(default)]
    pub hazard: Option<HazardSnapshot>,
    /// Reaction already spent this round (module turn tracker).
    #[serde(default)]
    pub reaction_used: bool,
    /// Player characters use the dying rules; NPCs die at 0 HP.
    #[serde(default)]
    pub uses_dying_rules: bool,
}

fn default_speed() -> u32 {
    25
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrikeSnapshot {
    pub name: String,
    /// `strike.totalModifier` minus condition-derived modifiers.
    pub attack_bonus: i32,
    /// `await strike.damage({ getFormula: true })` (normal hit), e.g.
    /// `"{(1d8 + 4)[slashing],(1d6)[fire]}"`.
    pub damage_formula: String,
    /// Fallback type for formula instances without a `[type]` flavor.
    #[serde(default)]
    pub damage_type: Option<String>,
    #[serde(default)]
    pub traits: Vec<String>,
    pub melee: bool,
    #[serde(default)]
    pub reach_ft: Option<u32>,
    #[serde(default)]
    pub range_increment_ft: Option<u32>,
    #[serde(default = "default_attr")]
    pub attribute: Attribute,
    /// The formula already reflects condition penalties to damage (PF2e
    /// bakes enfeebled into strike damage), so the engine must not apply
    /// them a second time.
    #[serde(default)]
    pub damage_includes_conditions: bool,
}

fn default_attr() -> Attribute {
    Attribute::Strength
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpellcastingSnapshot {
    pub name: String,
    #[serde(default)]
    pub tradition: Option<String>,
    /// `prepared` | `spontaneous` | `innate` | `focus` | `ritual` | `items`.
    pub kind: String,
    #[serde(default = "default_cast_attr")]
    pub attribute: Attribute,
    pub attack_bonus: i32,
    pub dc: i32,
    /// Remaining slots per rank (prepared: unexpended prepared copies).
    #[serde(default)]
    pub slots: BTreeMap<u8, u8>,
    #[serde(default)]
    pub spells: Vec<SpellSnapshot>,
}

fn default_cast_attr() -> Attribute {
    Attribute::Charisma
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpellSnapshot {
    /// `spell.slug`; matched against the engine's spell library first.
    pub slug: String,
    pub name: String,
    /// Rank it is prepared / known at (cantrips: ignored).
    pub rank: u8,
    /// The spell's own base rank (`spell.baseRank`).
    #[serde(default)]
    pub base_rank: Option<u8>,
    #[serde(default)]
    pub cantrip: bool,
    /// `system.time.value` as actions; `None` for reactions/free/long casts.
    #[serde(default)]
    pub actions: Option<u8>,
    #[serde(default)]
    pub range_ft: Option<u32>,
    #[serde(default)]
    pub area: Option<SpellAreaSnapshot>,
    /// Maximum number of creature targets (1 when unspecified).
    #[serde(default)]
    pub target_count: Option<u8>,
    #[serde(default)]
    pub targets_allies: bool,
    #[serde(default)]
    pub targets_self: bool,
    #[serde(default)]
    pub defense: Option<SpellDefenseSnapshot>,
    #[serde(default)]
    pub damage: Vec<FormulaSnapshot>,
    #[serde(default)]
    pub healing: Option<String>,
    #[serde(default)]
    pub heightening: Option<HeighteningSnapshot>,
    #[serde(default)]
    pub traits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpellAreaSnapshot {
    /// `burst` | `cone` | `emanation` | `line`.
    pub kind: String,
    pub size_ft: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SpellDefenseSnapshot {
    Attack,
    Save { save: Save, basic: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormulaSnapshot {
    pub formula: String,
    #[serde(default)]
    pub damage_type: Option<String>,
}

/// `system.heightening` of type `interval`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeighteningSnapshot {
    pub interval: u8,
    #[serde(default)]
    pub damage: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IwrSnapshot {
    #[serde(default)]
    pub immunities: Vec<String>,
    #[serde(default)]
    pub weaknesses: Vec<IwrValueSnapshot>,
    #[serde(default)]
    pub resistances: Vec<IwrValueSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IwrValueSnapshot {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ReactionsSnapshot {
    #[serde(default)]
    pub reactive_strike: bool,
    #[serde(default)]
    pub shield_block: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShieldSnapshot {
    pub name: String,
    pub ac_bonus: i32,
    pub hardness: i32,
    pub hp: i32,
    pub max_hp: i32,
    pub broken_threshold: i32,
    #[serde(default)]
    pub raised: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionSnapshot {
    pub slug: String,
    #[serde(default)]
    pub value: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistentSnapshot {
    pub formula: String,
    pub damage_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HazardSnapshot {
    #[serde(default)]
    pub complex: bool,
    #[serde(default)]
    pub stealth_dc: i32,
    #[serde(default)]
    pub disable: Vec<DisableSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisableSnapshot {
    pub skill: Skill,
    pub dc: i32,
}
