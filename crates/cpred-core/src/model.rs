//! Content models: stats, skills, weapons, armor, statblocks, stored actors,
//! battlefields and encounters. These are plain data; the rules that use
//! them live in [`crate::rules`] and [`crate::combat`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tactics_core::{DiceExpr, Pos, Side};

// ---------------------------------------------------------------------------
// Stats and skills
// ---------------------------------------------------------------------------

/// One of the ten Cyberpunk RED statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stat {
    Int,
    Ref,
    Dex,
    Tech,
    Cool,
    Will,
    Luck,
    Move,
    Body,
    Emp,
}

impl Stat {
    /// Upper-case abbreviation (`REF`, `DEX`, ...).
    pub fn label(self) -> &'static str {
        match self {
            Stat::Int => "INT",
            Stat::Ref => "REF",
            Stat::Dex => "DEX",
            Stat::Tech => "TECH",
            Stat::Cool => "COOL",
            Stat::Will => "WILL",
            Stat::Luck => "LUCK",
            Stat::Move => "MOVE",
            Stat::Body => "BODY",
            Stat::Emp => "EMP",
        }
    }
}

/// Raw statistics. `ref` and `move` are Rust keywords, hence the trailing
/// underscore; the JSON keys are `ref` and `move`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Stats {
    pub int: i32,
    #[serde(rename = "ref")]
    pub ref_: i32,
    pub dex: i32,
    pub tech: i32,
    pub cool: i32,
    pub will: i32,
    pub luck: i32,
    #[serde(rename = "move")]
    pub move_: i32,
    pub body: i32,
    pub emp: i32,
}

impl Stats {
    /// The raw value of one statistic.
    pub fn get(&self, stat: Stat) -> i32 {
        match stat {
            Stat::Int => self.int,
            Stat::Ref => self.ref_,
            Stat::Dex => self.dex,
            Stat::Tech => self.tech,
            Stat::Cool => self.cool,
            Stat::Will => self.will,
            Stat::Luck => self.luck,
            Stat::Move => self.move_,
            Stat::Body => self.body,
            Stat::Emp => self.emp,
        }
    }
}

/// The combat-relevant skills. Missing skills default to level 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skill {
    Handgun,
    ShoulderArms,
    HeavyWeapons,
    Archery,
    Autofire,
    MeleeWeapon,
    Brawling,
    MartialArts,
    Evasion,
    Athletics,
    Perception,
}

impl Skill {
    /// Every skill, in declaration order.
    pub const ALL: [Skill; 11] = [
        Skill::Handgun,
        Skill::ShoulderArms,
        Skill::HeavyWeapons,
        Skill::Archery,
        Skill::Autofire,
        Skill::MeleeWeapon,
        Skill::Brawling,
        Skill::MartialArts,
        Skill::Evasion,
        Skill::Athletics,
        Skill::Perception,
    ];

    /// The statistic a check with this skill adds.
    pub fn stat(self) -> Stat {
        match self {
            Skill::Handgun | Skill::ShoulderArms | Skill::HeavyWeapons | Skill::Archery | Skill::Autofire => Stat::Ref,
            Skill::MeleeWeapon | Skill::Brawling | Skill::MartialArts | Skill::Evasion | Skill::Athletics => Stat::Dex,
            Skill::Perception => Stat::Int,
        }
    }

    /// Human-readable name.
    pub fn label(self) -> &'static str {
        match self {
            Skill::Handgun => "Handgun",
            Skill::ShoulderArms => "Shoulder Arms",
            Skill::HeavyWeapons => "Heavy Weapons",
            Skill::Archery => "Archery",
            Skill::Autofire => "Autofire",
            Skill::MeleeWeapon => "Melee Weapon",
            Skill::Brawling => "Brawling",
            Skill::MartialArts => "Martial Arts",
            Skill::Evasion => "Evasion",
            Skill::Athletics => "Athletics",
            Skill::Perception => "Perception",
        }
    }
}

// ---------------------------------------------------------------------------
// Weapons
// ---------------------------------------------------------------------------

/// Weapon class. Ranged classes select a range table in
/// [`crate::rules::RulesTables`]; `Melee` and `Unarmed` have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponClass {
    Pistol,
    Smg,
    Shotgun,
    AssaultRifle,
    SniperRifle,
    Bow,
    GrenadeLauncher,
    RocketLauncher,
    /// Bladed or blunt melee weapons (halve the target's SP).
    Melee,
    /// Fists, kicks, martial arts (do not halve SP).
    Unarmed,
}

impl WeaponClass {
    /// Human-readable name.
    pub fn label(self) -> &'static str {
        match self {
            WeaponClass::Pistol => "Pistol",
            WeaponClass::Smg => "SMG",
            WeaponClass::Shotgun => "Shotgun",
            WeaponClass::AssaultRifle => "Assault Rifle",
            WeaponClass::SniperRifle => "Sniper Rifle",
            WeaponClass::Bow => "Bow",
            WeaponClass::GrenadeLauncher => "Grenade Launcher",
            WeaponClass::RocketLauncher => "Rocket Launcher",
            WeaponClass::Melee => "Melee",
            WeaponClass::Unarmed => "Unarmed",
        }
    }
}

/// Difficulty values per range band (`None` = cannot attack in that band).
/// Index `i` is the `i`-th band of [`crate::rules::RulesTables::range_bands_m`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct RangeTable {
    /// Single-shot (and aimed-shot) DVs.
    #[serde(default)]
    pub single: Vec<Option<i32>>,
    /// Autofire DVs (empty for weapons that cannot autofire).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub autofire: Vec<Option<i32>>,
}

impl RangeTable {
    /// True if the table has no entries at all (then the class table applies).
    pub fn is_empty(&self) -> bool {
        self.single.is_empty() && self.autofire.is_empty()
    }

    /// The DV for a band, or `None` if the weapon cannot attack in it.
    pub fn dv(&self, band: usize, autofire: bool) -> Option<i32> {
        let table = if autofire { &self.autofire } else { &self.single };
        table.get(band).copied().flatten()
    }
}

/// A weapon on a statblock.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Weapon {
    /// Content id (e.g. `heavy_pistol`).
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub class: WeaponClass,
    pub skill: Skill,
    pub damage: DiceExpr,
    /// Rate of fire: attacks per Action.
    #[serde(default = "one")]
    pub rof: u8,
    /// Magazine size; 0 = melee or no ammunition tracked.
    #[serde(default)]
    pub magazine: u32,
    /// Maximum autofire damage multiplier, if the weapon can autofire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autofire: Option<u8>,
    /// Weapon-specific DVs. Empty = use the class table from the rules.
    #[serde(default, skip_serializing_if = "RangeTable::is_empty")]
    pub range_table: RangeTable,
    /// Melee weapon (target must be adjacent; defender rolls evasion).
    #[serde(default)]
    pub melee: bool,
}

fn one() -> u8 {
    1
}

impl Weapon {
    /// Id of the implicit unarmed attack every combatant gets.
    pub const BRAWLING_ID: &'static str = "brawling";

    /// The implicit unarmed attack (damage depends on BODY).
    pub fn brawling(damage: DiceExpr) -> Weapon {
        Weapon {
            id: Self::BRAWLING_ID.into(),
            name: "Brawling".into(),
            class: WeaponClass::Unarmed,
            skill: Skill::Brawling,
            damage,
            rof: 2,
            magazine: 0,
            autofire: None,
            range_table: RangeTable::default(),
            melee: true,
        }
    }

    pub fn is_ranged(&self) -> bool {
        !self.melee
    }

    /// Melee weapons (not unarmed attacks) halve the target's SP.
    pub fn halves_armor(&self) -> bool {
        self.melee && self.class == WeaponClass::Melee
    }

    /// Whether the weapon tracks ammunition.
    pub fn uses_ammo(&self) -> bool {
        self.magazine > 0
    }
}

// ---------------------------------------------------------------------------
// Armor
// ---------------------------------------------------------------------------

/// Where a hit lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitLocation {
    Body,
    Head,
}

impl HitLocation {
    pub fn label(self) -> &'static str {
        match self {
            HitLocation::Body => "body",
            HitLocation::Head => "head",
        }
    }
}

/// One piece of armor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArmorPiece {
    /// Content id (e.g. `light_armorjack`); empty for ad-hoc pieces.
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Stopping power.
    pub sp: i32,
    /// Applied as −penalty to REF, DEX and MOVE.
    #[serde(default)]
    pub penalty: i32,
}

/// Body and head armor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Armor {
    #[serde(default)]
    pub body: Option<ArmorPiece>,
    #[serde(default)]
    pub head: Option<ArmorPiece>,
}

impl Armor {
    /// The worst penalty of the two pieces.
    pub fn penalty(&self) -> i32 {
        let p = |a: &Option<ArmorPiece>| a.as_ref().map(|x| x.penalty).unwrap_or(0);
        p(&self.body).max(p(&self.head)).max(0)
    }

    /// Full (unablated) SP at a location.
    pub fn sp(&self, location: HitLocation) -> i32 {
        let piece = match location {
            HitLocation::Body => &self.body,
            HitLocation::Head => &self.head,
        };
        piece.as_ref().map(|x| x.sp.max(0)).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Statblocks and actors
// ---------------------------------------------------------------------------

/// Everything the combat engine needs to know about a character or NPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Statblock {
    pub name: String,
    pub stats: Stats,
    #[serde(default)]
    pub skills: BTreeMap<Skill, i32>,
    #[serde(default)]
    pub weapons: Vec<Weapon>,
    #[serde(default)]
    pub armor: Armor,
    /// Fixed maximum HP (NPCs); `None` = derive from BODY and WILL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hp: Option<i32>,
    #[serde(default)]
    pub notes: String,
}

/// Why a statblock is unusable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatblockError {
    #[error("{name}: stat {stat} is negative")]
    NegativeStat { name: String, stat: &'static str },
    #[error("{name}: weapon '{weapon}' has ROF 0")]
    ZeroRof { name: String, weapon: String },
    #[error("{name}: weapon '{weapon}' has no damage dice")]
    NoDamage { name: String, weapon: String },
    #[error("{name}: maximum HP must be at least 1")]
    MaxHp { name: String },
}

impl Statblock {
    /// Skill level (0 if untrained).
    pub fn skill(&self, skill: Skill) -> i32 {
        self.skills.get(&skill).copied().unwrap_or(0)
    }

    /// HP from the formula `10 + 5 × ceil((BODY + WILL) / 2)`.
    pub fn derived_max_hp(&self) -> i32 {
        crate::rules::max_hp_from(self.stats.body, self.stats.will)
    }

    /// `max_hp` if given, otherwise the derived value.
    pub fn effective_max_hp(&self) -> i32 {
        self.max_hp.unwrap_or_else(|| self.derived_max_hp())
    }

    /// Basic sanity checks used at setup and when loading content.
    pub fn validate(&self) -> Result<(), StatblockError> {
        let name = || self.name.clone();
        for stat in [
            Stat::Int,
            Stat::Ref,
            Stat::Dex,
            Stat::Tech,
            Stat::Cool,
            Stat::Will,
            Stat::Luck,
            Stat::Move,
            Stat::Body,
            Stat::Emp,
        ] {
            if self.stats.get(stat) < 0 {
                return Err(StatblockError::NegativeStat { name: name(), stat: stat.label() });
            }
        }
        for w in &self.weapons {
            if w.rof == 0 {
                return Err(StatblockError::ZeroRof { name: name(), weapon: w.name.clone() });
            }
            if w.damage.is_flat() {
                return Err(StatblockError::NoDamage { name: name(), weapon: w.name.clone() });
            }
        }
        if self.effective_max_hp() < 1 {
            return Err(StatblockError::MaxHp { name: name() });
        }
        Ok(())
    }
}

/// Player character or GM-controlled NPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Pc,
    #[default]
    Npc,
}

/// A stored character or NPC template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ActorKind,
    pub statblock: Statblock,
}

// ---------------------------------------------------------------------------
// Battlefield
// ---------------------------------------------------------------------------

/// What occupies one map square.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terrain {
    /// `.`
    Open,
    /// `~` — costs 2 squares of movement to enter.
    Difficult,
    /// `#` — blocks movement and line of sight.
    Wall,
    /// `%` — a cover object: blocks movement, has HP, can be destroyed.
    Cover,
}

impl Terrain {
    pub fn from_glyph(c: char) -> Option<Terrain> {
        Some(match c {
            '.' => Terrain::Open,
            '~' => Terrain::Difficult,
            '#' => Terrain::Wall,
            '%' => Terrain::Cover,
            _ => return None,
        })
    }

    pub fn glyph(self) -> char {
        match self {
            Terrain::Open => '.',
            Terrain::Difficult => '~',
            Terrain::Wall => '#',
            Terrain::Cover => '%',
        }
    }

    /// Movement cost in squares to enter, `None` if impassable.
    pub fn enter_cost(self) -> Option<u32> {
        match self {
            Terrain::Open => Some(1),
            Terrain::Difficult => Some(2),
            Terrain::Wall | Terrain::Cover => None,
        }
    }

    /// Whether the square blocks line of sight.
    pub fn blocks_sight(self) -> bool {
        self == Terrain::Wall
    }

    /// Whether a combatant may stand here.
    pub fn standable(self) -> bool {
        self.enter_cost().is_some()
    }
}

/// A rectangular grid of 2 m squares. Row 0 is the top (`y = 0`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Battlefield {
    pub width: u32,
    pub height: u32,
    pub map: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

/// Why a battlefield is malformed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BattlefieldError {
    #[error("map has {rows} rows but height is {height}")]
    RowCount { rows: usize, height: u32 },
    #[error("row {row} has {len} squares but width is {width}")]
    RowWidth { row: usize, len: usize, width: u32 },
    #[error("unknown terrain glyph '{glyph}' at ({x},{y})")]
    Glyph { glyph: char, x: usize, y: usize },
    #[error("battlefield must be between 1x1 and 200x200")]
    Size,
}

impl Battlefield {
    /// An all-open map.
    pub fn open(width: u32, height: u32) -> Self {
        Battlefield {
            width,
            height,
            map: (0..height).map(|_| ".".repeat(width as usize)).collect(),
            notes: String::new(),
        }
    }

    /// Checks dimensions and glyphs.
    pub fn validate(&self) -> Result<(), BattlefieldError> {
        if self.width == 0 || self.height == 0 || self.width > 200 || self.height > 200 {
            return Err(BattlefieldError::Size);
        }
        if self.map.len() != self.height as usize {
            return Err(BattlefieldError::RowCount { rows: self.map.len(), height: self.height });
        }
        for (y, row) in self.map.iter().enumerate() {
            let len = row.chars().count();
            if len != self.width as usize {
                return Err(BattlefieldError::RowWidth { row: y, len, width: self.width });
            }
            if let Some((x, glyph)) = row.chars().enumerate().find(|(_, c)| Terrain::from_glyph(*c).is_none()) {
                return Err(BattlefieldError::Glyph { glyph, x, y });
            }
        }
        Ok(())
    }

    pub fn in_bounds(&self, p: Pos) -> bool {
        p.x >= 0 && p.y >= 0 && (p.x as u32) < self.width && (p.y as u32) < self.height
    }

    /// Terrain at a square. Out-of-bounds squares behave as walls.
    pub fn terrain(&self, p: Pos) -> Terrain {
        if !self.in_bounds(p) {
            return Terrain::Wall;
        }
        self.map
            .get(p.y as usize)
            .and_then(|row| row.chars().nth(p.x as usize))
            .and_then(Terrain::from_glyph)
            .unwrap_or(Terrain::Wall)
    }

    /// Replaces the terrain of one square (no-op out of bounds).
    pub fn set_terrain(&mut self, p: Pos, t: Terrain) {
        if !self.in_bounds(p) {
            return;
        }
        if let Some(row) = self.map.get_mut(p.y as usize) {
            *row = row.chars().enumerate().map(|(x, c)| if x == p.x as usize { t.glyph() } else { c }).collect();
        }
    }

    /// Every square of a given terrain, in row-major order.
    pub fn squares_of(&self, t: Terrain) -> Vec<Pos> {
        let mut out = Vec::new();
        for (y, row) in self.map.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                if Terrain::from_glyph(c) == Some(t) {
                    out.push(Pos::new(x as i32, y as i32));
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Encounters
// ---------------------------------------------------------------------------

/// One combatant in a stored encounter: either a reference to a stored
/// [`Actor`] (`actor_id`) or an inline `statblock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statblock: Option<Statblock>,
    /// Display name override (e.g. "Street Ganger 2").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub side: Side,
    pub position: Pos,
    /// Overrides the actor's PC/NPC kind (inline statblocks default to NPC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ActorKind>,
    /// Starting HP if not at full health.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_hp: Option<i32>,
}

/// A stored encounter: a map plus participants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Encounter {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    pub battlefield: Battlefield,
    pub participants: Vec<Participant>,
}
