//! `CombatSnapshot` → `pf2e_core::CombatState`.
//!
//! Steps (each one data mapping; every rule is applied by `pf2e-core`):
//! 1. Validate the snapshot (version, grid, ids, numbers).
//! 2. Crop the scene to the combatants and rasterize walls and terrain.
//! 3. Map each combatant to a `Statblock` (strikes from Foundry damage
//!    formulas, spells via [`crate::spells`], IWR, shield, reactions).
//! 4. Build the state with `CombatSetup` (placement validation), nudging
//!    stacked tokens apart.
//! 5. Apply live state: HP, temporary HP, conditions, persistent damage,
//!    raised shield, spent reactions, remaining spell slots.
//! 6. Apply the turn: initiative order, round, current creature. The
//!    current creature's start-of-turn processing comes from
//!    `CombatState::hypothetical_turn`; actions already spent and attacks
//!    already made (the module's turn tracker) are then deducted.

use crate::api::{BridgeError, ErrorCode};
use crate::formula::{damage_type, parse_damage, parse_dice};
use crate::snapshot::*;
use crate::spells::{tradition, translate, Translation};
use crate::walls::{crop_for, rasterize, Crop};
use pf2e_core::builder::content::Content;
use pf2e_core::combat::state::PersistentDamage;
use pf2e_core::combat::{CombatSetup, CombatState, CombatantKind, InitiativeEntry, LifeState, Phase, SetupParticipant};
use pf2e_core::conditions::{Condition, ConditionKind, Duration};
use pf2e_core::damage::{DamageSelector, Iwr, IwrValue};
use pf2e_core::effects::{Effect, EffectTag};
use pf2e_core::grid::{footprint, Pos};
use pf2e_core::models::battlefield::{Battlefield, Terrain};
use pf2e_core::models::spell::Spell;
use pf2e_core::models::statblock::*;
use pf2e_core::models::weapon::WeaponTrait;
use pf2e_core::CombatantId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::OnceLock;

/// Engine id ↔ Foundry token id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IdMap {
    /// Index `i` holds the token of `CombatantId(i + 1)`.
    pub tokens: Vec<String>,
}

impl IdMap {
    pub fn token(&self, id: CombatantId) -> Option<&str> {
        self.tokens.get((id.0 as usize).checked_sub(1)?).map(String::as_str)
    }
    pub fn id(&self, token: &str) -> Option<CombatantId> {
        self.tokens.iter().position(|t| t == token).map(|i| CombatantId(i as u32 + 1))
    }
}

/// A square in scene terms, for drawing recommendations on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScenePoint {
    /// Scene square (column, row).
    pub square: [i32; 2],
    /// Canvas pixel position of the square's top-left corner.
    pub px: [f64; 2],
}

#[derive(Debug, Clone)]
pub struct Converted {
    pub state: CombatState,
    pub ids: IdMap,
    pub crop: Crop,
    pub grid_size_px: f64,
    pub origin_px: [f64; 2],
    /// A Foundry combat is running with a current creature.
    pub turn_active: bool,
    pub warnings: Vec<String>,
}

impl Converted {
    pub fn to_scene(&self, p: Pos) -> ScenePoint {
        let square = [p.x + self.crop.x, p.y + self.crop.y];
        ScenePoint {
            square,
            px: [self.origin_px[0] + square[0] as f64 * self.grid_size_px, self.origin_px[1] + square[1] as f64 * self.grid_size_px],
        }
    }
    pub fn token(&self, id: CombatantId) -> String {
        self.ids.token(id).unwrap_or_default().to_string()
    }
}

/// The engine's bundled spell definitions, parsed once.
pub fn spell_library() -> &'static BTreeMap<String, Spell> {
    static LIB: OnceLock<BTreeMap<String, Spell>> = OnceLock::new();
    LIB.get_or_init(|| Content::bundled().map(|c| c.spells.into_iter().map(|s| (s.id.clone(), s)).collect()).unwrap_or_default())
}

fn invalid(msg: impl Into<String>) -> BridgeError {
    BridgeError::new(ErrorCode::InvalidSnapshot, msg)
}

/// Upper bounds that keep a hostile or corrupt snapshot from exhausting
/// memory or time in the analysis worker.
pub const MAX_COMBATANTS: usize = 64;
pub const MAX_WALLS: usize = 20_000;
pub const MAX_STRIKES: usize = 16;
pub const MAX_SPELLS: usize = 200;

fn validate(s: &CombatSnapshot) -> Result<(), BridgeError> {
    if s.version != SNAPSHOT_VERSION {
        return Err(BridgeError::new(
            ErrorCode::UnsupportedVersion,
            format!("snapshot version {} is not supported (expected {SNAPSHOT_VERSION}); update the module", s.version),
        ));
    }
    match s.scene.grid {
        GridKind::Square => {}
        GridKind::Hex => return Err(BridgeError::new(ErrorCode::UnsupportedScene, "hex grids are not supported; PF2e tactical analysis needs a square grid")),
        GridKind::Gridless => return Err(BridgeError::new(ErrorCode::UnsupportedScene, "gridless scenes are not supported; enable a square grid")),
    }
    if (s.scene.grid_distance_ft - 5.0).abs() > 1e-6 {
        return Err(BridgeError::new(ErrorCode::UnsupportedScene, format!("grid squares must be 5 ft (scene uses {} ft)", s.scene.grid_distance_ft)));
    }
    if !(s.scene.grid_size_px.is_finite() && s.scene.grid_size_px > 0.0) {
        return Err(invalid("grid size must be positive"));
    }
    if s.combatants.is_empty() {
        return Err(BridgeError::new(ErrorCode::NothingToAnalyse, "no combatants on the scene"));
    }
    if s.combatants.len() > MAX_COMBATANTS {
        return Err(invalid(format!("{} combatants exceed the limit of {MAX_COMBATANTS}", s.combatants.len())));
    }
    if s.scene.walls.len() > MAX_WALLS {
        return Err(invalid(format!("{} walls exceed the limit of {MAX_WALLS}", s.scene.walls.len())));
    }
    let mut seen = HashSet::new();
    for c in &s.combatants {
        if !seen.insert(&c.token_id) {
            return Err(invalid(format!("token {} appears twice", c.token_id)));
        }
        if c.max_hp <= 0 || c.max_hp > 100_000 || c.hp > 100_000 || c.level < -1 || c.level > 30 {
            return Err(invalid(format!("{} has implausible statistics", c.name)));
        }
        if c.strikes.len() > MAX_STRIKES || c.spellcasting.iter().map(|e| e.spells.len()).sum::<usize>() > MAX_SPELLS {
            return Err(invalid(format!("{} has too many strikes or spells", c.name)));
        }
        if c.speed > 500 {
            return Err(invalid(format!("{} has an implausible speed", c.name)));
        }
    }
    Ok(())
}

fn condition_kind(slug: &str) -> Option<ConditionKind> {
    use ConditionKind::*;
    Some(match slug {
        "blinded" => Blinded,
        "clumsy" => Clumsy,
        "concealed" => Concealed,
        "dazzled" => Dazzled,
        "doomed" => Doomed,
        "drained" => Drained,
        "dying" => Dying,
        "enfeebled" => Enfeebled,
        "fleeing" => Fleeing,
        "frightened" => Frightened,
        "grabbed" => Grabbed,
        // The engine has no separate "undetected"; hidden is the nearest.
        "hidden" | "undetected" => Hidden,
        "immobilized" => Immobilized,
        "invisible" => Invisible,
        "off-guard" | "flat-footed" => OffGuard,
        "paralyzed" => Paralyzed,
        "prone" => Prone,
        "quickened" => Quickened,
        "restrained" => Restrained,
        "sickened" => Sickened,
        "slowed" => Slowed,
        "stunned" => Stunned,
        "stupefied" => Stupefied,
        "unconscious" => Unconscious,
        "wounded" => Wounded,
        _ => return None,
    })
}

/// Conditions with no tactical effect the engine could model, which are
/// ignored without a warning (attitudes, perception bookkeeping).
const QUIET_CONDITIONS: &[&str] = &["observed", "unnoticed", "friendly", "helpful", "hostile", "indifferent", "unfriendly", "encumbered"];

fn iwr_selector(name: &str) -> Option<DamageSelector> {
    Some(match name {
        "physical" => DamageSelector::Physical,
        "energy" => DamageSelector::Energy,
        "precision" => DamageSelector::Precision,
        "all-damage" | "all" => DamageSelector::All,
        other => DamageSelector::Type(damage_type(other)?),
    })
}

fn statblock(c: &CombatantSnapshot, spells: &mut BTreeMap<String, Spell>, warnings: &mut Vec<String>) -> Statblock {
    let mut strikes = Vec::new();
    for s in &c.strikes {
        let fallback = s.damage_type.as_deref().and_then(damage_type);
        let parsed = match parse_damage(&s.damage_formula, fallback) {
            Ok(p) if !p.components.is_empty() => p,
            Ok(_) | Err(_) => {
                warnings.push(format!("{}: strike '{}' skipped (damage '{}' not understood)", c.name, s.name, s.damage_formula));
                continue;
            }
        };
        if !parsed.skipped.is_empty() {
            warnings.push(format!("{}: '{}' riders not modelled: {}", c.name, s.name, parsed.skipped.join(", ")));
        }
        let traits: Vec<WeaponTrait> = s.traits.iter().filter_map(|t| t.parse().ok()).collect();
        let range = if s.melee {
            StrikeRange::Melee { reach_ft: s.reach_ft.unwrap_or(5).clamp(5, 100) }
        } else {
            StrikeRange::Ranged { increment_ft: s.range_increment_ft.unwrap_or(30).clamp(5, 1000) }
        };
        strikes.push(StrikeProfile {
            name: s.name.clone(),
            attack_bonus: s.attack_bonus,
            damage: parsed.components,
            traits,
            range,
            attribute: s.attribute,
            // `strength_damage` lets the engine apply enfeebled to damage;
            // skip it when Foundry's formula already includes the penalty.
            strength_damage: s.melee && s.attribute == pf2e_core::stats::Attribute::Strength && !s.damage_includes_conditions,
        });
    }

    let mut spellcasting = Vec::new();
    for e in &c.spellcasting {
        let kind = match e.kind.as_str() {
            "prepared" => SpellcastingKind::Prepared,
            "spontaneous" => SpellcastingKind::Spontaneous,
            // Focus spells: the extractor reports focus points as slots of
            // the heightened rank, which the engine treats like innate uses.
            "innate" | "focus" => SpellcastingKind::Innate,
            _ => continue,
        };
        let trad = tradition(e.tradition.as_deref());
        let mut known = Vec::new();
        let mut cantrips = Vec::new();
        let mut skipped = Vec::new();
        for sp in &e.spells {
            match translate(sp, trad, spell_library()) {
                Translation::Library(def) | Translation::Approximate(def) => {
                    let id = def.id.clone();
                    spells.entry(id.clone()).or_insert(def);
                    if sp.cantrip {
                        if !cantrips.contains(&id) {
                            cantrips.push(id);
                        }
                    } else {
                        known.push(KnownSpell { spell_id: id, rank: sp.rank.max(1) });
                    }
                }
                Translation::Unsupported(why) => skipped.push(why),
            }
        }
        if !skipped.is_empty() {
            warnings.push(format!("{}: {} spell(s) not modelled ({})", c.name, skipped.len(), skipped.join("; ")));
        }
        if known.is_empty() && cantrips.is_empty() {
            continue;
        }
        spellcasting.push(SpellcastingEntry {
            name: e.name.clone(),
            tradition: trad.unwrap_or(pf2e_core::models::spell::Tradition::Arcane),
            kind,
            attribute: e.attribute,
            attack_bonus: e.attack_bonus,
            dc: e.dc,
            slots: e.slots.clone(),
            spells: known,
            cantrips,
        });
    }

    let mut iwr = Iwr::default();
    let mut ignored = Vec::new();
    for i in &c.iwr.immunities {
        match iwr_selector(i) {
            Some(sel) => iwr.immunities.push(sel),
            None => ignored.push(format!("immunity {i}")),
        }
    }
    for (list, out, label) in [(&c.iwr.weaknesses, &mut iwr.weaknesses, "weakness"), (&c.iwr.resistances, &mut iwr.resistances, "resistance")] {
        for v in list {
            match iwr_selector(&v.kind) {
                Some(sel) => out.push(IwrValue { applies_to: sel, value: v.value }),
                None => ignored.push(format!("{label} {}", v.kind)),
            }
        }
    }
    // Condition/effect immunities (sleep, fear, ...) are not damage; only
    // report damage-like entries the engine cannot express.
    ignored.retain(|s| !s.starts_with("immunity "));
    if !ignored.is_empty() {
        warnings.push(format!("{}: not modelled: {}", c.name, ignored.join(", ")));
    }

    let mut reactions = Vec::new();
    if c.reactions.reactive_strike {
        reactions.push(ReactionKind::ReactiveStrike);
    }
    if c.reactions.shield_block && c.shield.is_some() {
        reactions.push(ReactionKind::ShieldBlock);
    }
    let mut damage_riders = Vec::new();
    if let Some(sa) = &c.sneak_attack {
        match parse_dice(sa) {
            Ok(dice) => damage_riders.push(DamageRider::SneakAttack { dice }),
            Err(_) => warnings.push(format!("{}: sneak attack '{sa}' not understood", c.name)),
        }
    }
    Statblock {
        name: c.name.clone(),
        level: c.level,
        traits: c.traits.clone(),
        size: c.size,
        attributes: c.attributes,
        max_hp: c.max_hp,
        ac: c.ac,
        fortitude: c.fortitude,
        reflex: c.reflex,
        will: c.will,
        perception: c.perception,
        speed: c.speed,
        skills: c.skills.clone(),
        strikes,
        spellcasting,
        iwr,
        reactions,
        shield: c.shield.as_ref().map(|s| ShieldStats {
            name: s.name.clone(),
            ac_bonus: s.ac_bonus,
            hardness: s.hardness,
            max_hp: s.max_hp,
            broken_threshold: s.broken_threshold,
        }),
        class_dc: c.class_dc,
        damage_riders,
        initiative_bonus: 0,
        uses_dying_rules: c.uses_dying_rules,
        hazard: c.hazard.as_ref().map(|h| HazardProfile {
            complex: h.complex,
            stealth_dc: h.stealth_dc,
            disable: h.disable.iter().map(|d| DisableOption { skill: d.skill, dc: d.dc }).collect(),
            trigger_offsets: vec![],
            trigger_effect: None,
            resets: false,
        }),
    }
}

/// Moves a token that overlaps another creature or sits in a wall to the
/// nearest free square (deterministic ring search).
fn place(bf: &Battlefield, pos: Pos, size: pf2e_core::stats::Size, taken: &HashSet<Pos>) -> Option<Pos> {
    let free = |p: Pos| footprint(p, size).iter().all(|sq| bf.in_bounds(*sq) && bf.terrain(*sq) != Terrain::Wall && !taken.contains(sq));
    if free(pos) {
        return Some(pos);
    }
    for r in 1..=6i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() == r || dy.abs() == r {
                    let p = Pos::new(pos.x + dx, pos.y + dy);
                    if free(p) {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

pub fn snapshot_to_state(snap: &CombatSnapshot) -> Result<Converted, BridgeError> {
    validate(snap)?;
    let mut warnings = Vec::new();

    // Dead NPCs are irrelevant to tactics and often lie under living tokens.
    let living: Vec<&CombatantSnapshot> = snap.combatants.iter().filter(|c| c.uses_dying_rules || c.hp > 0).collect();
    if living.is_empty() {
        return Err(BridgeError::new(ErrorCode::NothingToAnalyse, "every combatant is defeated"));
    }
    let owned: Vec<CombatantSnapshot> = living.iter().map(|c| (*c).clone()).collect();
    let crop = crop_for(&owned, snap.scene.width, snap.scene.height, snap.options.crop_margin).map_err(invalid)?;

    let mut bf = Battlefield::open(crop.width, crop.height);
    for t in &snap.scene.terrain {
        if crop.contains(t.x, t.y) {
            let terrain = match t.kind {
                TerrainKind::Difficult => Terrain::Difficult,
                TerrainKind::GreaterDifficult => Terrain::GreaterDifficult,
                TerrainKind::Obscured => Terrain::Obscured,
                TerrainKind::Wall => Terrain::Wall,
            };
            bf.set_terrain(Pos::new(t.x - crop.x, t.y - crop.y), terrain);
        }
    }
    bf.edges = rasterize(&snap.scene.walls, &crop);
    bf.notes = format!("Foundry scene {} ({}×{} squares at {},{})", snap.scene.name, crop.width, crop.height, crop.x, crop.y);

    let mut spells = BTreeMap::new();
    let mut participants = Vec::new();
    let mut taken: HashSet<Pos> = HashSet::new();
    let mut tokens = Vec::new();
    for c in &living {
        let sb = statblock(c, &mut spells, &mut warnings);
        let want = Pos::new(c.x - crop.x, c.y - crop.y);
        let is_hazard = c.kind == CombatantKind::Hazard;
        let pos = if is_hazard {
            want
        } else {
            match place(&bf, want, c.size, &taken) {
                Some(p) => {
                    if p != want {
                        warnings.push(format!("{} overlaps another creature or a wall; analysed one square away", c.name));
                    }
                    p
                }
                None => return Err(invalid(format!("no free space for {} near its token", c.name))),
            }
        };
        if !is_hazard {
            taken.extend(footprint(pos, c.size));
        }
        participants.push(SetupParticipant { name: c.name.clone(), side: c.side, kind: c.kind, statblock: sb, position: pos, current_hp: Some(c.hp.max(0)) });
        tokens.push(c.token_id.clone());
    }
    let mut setup = CombatSetup::default();
    setup.options.flanking = snap.options.flanking;
    setup.options.auto_cover = snap.options.auto_cover;
    let mut state = setup.build(bf, participants, &spells).map_err(|e| invalid(e.to_string()))?;
    let ids = IdMap { tokens };

    // Live state.
    for (i, c) in living.iter().enumerate() {
        let id = CombatantId(i as u32 + 1);
        let mut ignored = Vec::new();
        let cm = state.combatant_mut(id).expect("just built");
        cm.temp_hp = c.temp_hp.max(0);
        for cond in &c.conditions {
            match condition_kind(&cond.slug) {
                Some(kind) => {
                    let value = if kind.is_valued() { cond.value.unwrap_or(1).max(1) } else { 0 };
                    cm.conditions.add(Condition::new(kind, value));
                }
                None if cond.slug == "persistent-damage" || QUIET_CONDITIONS.contains(&cond.slug.as_str()) => {}
                None => ignored.push(cond.slug.clone()),
            }
        }
        for p in &c.persistent {
            match (parse_dice(&p.formula), damage_type(&p.damage_type)) {
                (Ok(dice), Some(damage_type)) => cm.persistent.push(PersistentDamage { dice, damage_type, source: None }),
                _ => ignored.push(format!("persistent {} {}", p.formula, p.damage_type)),
            }
        }
        if let Some(sh) = &c.shield {
            cm.shield_hp = sh.hp.clamp(0, sh.max_hp.max(0));
        }
        cm.reaction_available = !c.reaction_used;
        if c.hp <= 0 && !c.uses_dying_rules {
            cm.life = LifeState::Dead;
        }
        if !ignored.is_empty() {
            warnings.push(format!("{}: conditions not modelled: {}", c.name, ignored.join(", ")));
        }
    }

    // Turn order: Foundry's, then anyone missing (not in the combat tracker).
    let mut order: Vec<CombatantId> = snap.turn.order.iter().filter_map(|t| ids.id(t)).collect();
    let in_tracker: BTreeSet<CombatantId> = order.iter().copied().collect();
    for c in &state.combatants {
        if !in_tracker.contains(&c.id) && c.in_initiative {
            if snap.turn.started {
                // Not in the Foundry combat: present, but takes no turns.
                warnings.push(format!("{} is not in the combat tracker; it will not act in forecasts", c.name));
            } else {
                order.push(c.id);
            }
        }
    }
    let skip: BTreeSet<CombatantId> = state.combatants.iter().filter(|c| !c.in_initiative).map(|c| c.id).collect();
    order.retain(|id| !skip.contains(id));
    let tracked: BTreeSet<CombatantId> = order.iter().copied().collect();
    for c in &mut state.combatants {
        if !tracked.contains(&c.id) {
            c.in_initiative = false;
        }
    }
    if order.is_empty() {
        return Err(BridgeError::new(ErrorCode::NothingToAnalyse, "no creature takes turns"));
    }
    state.initiative = order
        .iter()
        .map(|id| {
            let init = living[(id.0 - 1) as usize].initiative.unwrap_or(0.0);
            InitiativeEntry { combatant: *id, value: init.floor() as i32, natural: 10 }
        })
        .collect();
    state.phase = Phase::Active;
    state.round = snap.turn.round.max(1);
    let current = snap.turn.current.as_deref().and_then(|t| ids.id(t)).filter(|id| tracked.contains(id));
    let turn_active = snap.turn.started && current.is_some();
    state.turn = current.and_then(|id| state.initiative_index(id)).unwrap_or(0);

    // Raised shields last until the start of their bearer's next turn.
    for (i, c) in living.iter().enumerate() {
        let id = CombatantId(i as u32 + 1);
        if c.shield.as_ref().map(|s| s.raised).unwrap_or(false) {
            let expires = state.expiry_for(Duration::UntilStartOfSourcesNextTurn, id, id);
            let eid = state.next_effect_id + 1;
            state.next_effect_id = eid;
            state.combatant_mut(id).expect("built").effects.push(Effect {
                id: eid,
                name: "Shield Raised".into(),
                source: Some(id),
                grants: vec![],
                tags: vec![EffectTag::ShieldRaised],
                expires,
            });
        }
    }

    if let Some(cur) = current.filter(|_| turn_active) {
        let spent = snap.turn.actions_spent;
        let attacks = snap.turn.attacks_made;
        let started = state.hypothetical_turn(cur).and_then(|h| h.combatant(cur).cloned());
        let c = state.combatant_mut(cur).expect("current");
        match started {
            Some(mut fresh) => {
                fresh.actions_remaining = fresh.actions_remaining.saturating_sub(spent);
                fresh.attacks_this_turn = attacks.min(10);
                fresh.reaction_available = !living[(cur.0 - 1) as usize].reaction_used;
                *c = fresh;
            }
            None => c.actions_remaining = 0,
        }
    }
    state.battlefield.validate().map_err(|e| invalid(e.to_string()))?;
    Ok(Converted {
        state,
        ids,
        crop,
        grid_size_px: snap.scene.grid_size_px,
        origin_px: snap.scene.origin_px,
        turn_active,
        warnings,
    })
}
