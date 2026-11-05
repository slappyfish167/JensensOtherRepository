//! Pure rule computations over a `CombatState`: modifiers from conditions
//! and effects, attack totals, AC, DCs, flanking, cover, MAP and strike
//! damage. Nothing here mutates state, so the AI layer can call it freely.

use super::pending::{AttackCtx, AttackSource, DamageRollSpec};
use super::state::{CombatState, Combatant};
use crate::conditions::ConditionKind as CK;
use crate::effects::{EffectTag, Selector};
use crate::grid::{compute_cover, creature_distance, flanks, within_reach, CoverInfo, CoverLevel};
use crate::ids::CombatantId;
use crate::modifiers::{Modifier, ModifierSet};
use crate::models::statblock::{DamageRider, StrikeProfile, StrikeRange};
use crate::models::weapon::{deadly_die, fatal_die, WeaponTrait};
use crate::stats::{Attribute, Save, Skill};
use crate::dice::DiceExpr;
use std::collections::HashSet;

/// What kind of roll or DC a modifier might apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckType {
    Attack { attribute: Attribute },
    SpellAttack { attribute: Attribute },
    Save(Save),
    Perception,
    Skill(Skill),
    Ac,
    SpellDc { attribute: Attribute },
}

impl CheckType {
    fn attribute(self) -> Option<Attribute> {
        match self {
            CheckType::Attack { attribute } | CheckType::SpellAttack { attribute } | CheckType::SpellDc { attribute } => {
                Some(attribute)
            }
            CheckType::Save(s) => Some(s.attribute()),
            CheckType::Perception => Some(Attribute::Wisdom),
            CheckType::Skill(s) => Some(s.attribute()),
            CheckType::Ac => Some(Attribute::Dexterity),
        }
    }

    fn is_attack(self) -> bool {
        matches!(self, CheckType::Attack { .. } | CheckType::SpellAttack { .. })
    }
}

/// Status/circumstance modifiers a combatant's conditions and effects impose
/// on a check or DC.
pub fn situational_modifiers(c: &Combatant, check: CheckType) -> ModifierSet {
    let mut m = ModifierSet::new();
    let conds = &c.conditions;
    let v = |k: CK| conds.value(k) as i32;

    if v(CK::Frightened) > 0 {
        m.push(Modifier::status(-v(CK::Frightened), format!("frightened {}", v(CK::Frightened))));
    }
    if v(CK::Sickened) > 0 {
        m.push(Modifier::status(-v(CK::Sickened), format!("sickened {}", v(CK::Sickened))));
    }
    let attr = check.attribute();
    if attr == Some(Attribute::Dexterity) && v(CK::Clumsy) > 0 {
        m.push(Modifier::status(-v(CK::Clumsy), format!("clumsy {}", v(CK::Clumsy))));
    }
    if attr == Some(Attribute::Strength) && v(CK::Enfeebled) > 0 {
        m.push(Modifier::status(-v(CK::Enfeebled), format!("enfeebled {}", v(CK::Enfeebled))));
    }
    if attr == Some(Attribute::Constitution) && v(CK::Drained) > 0 {
        m.push(Modifier::status(-v(CK::Drained), format!("drained {}", v(CK::Drained))));
    }
    if matches!(attr, Some(Attribute::Intelligence | Attribute::Wisdom | Attribute::Charisma)) && v(CK::Stupefied) > 0 {
        m.push(Modifier::status(-v(CK::Stupefied), format!("stupefied {}", v(CK::Stupefied))));
    }
    match check {
        CheckType::Ac => {
            if conds.has(CK::OffGuard) {
                m.push(Modifier::circumstance(-2, "off-guard"));
            }
            if conds.has(CK::Unconscious) {
                m.push(Modifier::status(-4, "unconscious"));
            }
        }
        CheckType::Save(Save::Reflex) if conds.has(CK::Unconscious) => m.push(Modifier::status(-4, "unconscious")),
        CheckType::Perception => {
            if conds.has(CK::Unconscious) {
                m.push(Modifier::status(-4, "unconscious"));
            }
            if conds.has(CK::Blinded) {
                m.push(Modifier::status(-4, "blinded"));
            }
        }
        _ => {}
    }
    if check.is_attack() && conds.has(CK::Prone) {
        m.push(Modifier::circumstance(-2, "prone"));
    }

    for e in &c.effects {
        for g in &e.grants {
            let applies = match g.selector {
                Selector::AttackRolls => check.is_attack(),
                Selector::Ac => check == CheckType::Ac,
                Selector::AllSaves => matches!(check, CheckType::Save(_)),
                Selector::Save(s) => check == CheckType::Save(s),
                Selector::Perception => check == CheckType::Perception,
                Selector::Skills => matches!(check, CheckType::Skill(_)),
                Selector::AllChecks => !matches!(check, CheckType::Ac | CheckType::SpellDc { .. }),
                Selector::SpellDc => matches!(check, CheckType::SpellDc { .. }),
                Selector::Damage => false,
            };
            if applies {
                m.push(g.modifier.clone());
            }
        }
    }
    m
}

pub fn total_with(base: i32, mods: &ModifierSet) -> i32 {
    base + mods.total()
}

pub fn save_modifier(c: &Combatant, save: Save) -> (i32, ModifierSet) {
    let m = situational_modifiers(c, CheckType::Save(save));
    (c.statblock.save(save) + m.total(), m)
}

pub fn save_dc(c: &Combatant, save: Save) -> i32 {
    10 + save_modifier(c, save).0
}

pub fn skill_modifier(c: &Combatant, skill: Skill) -> (i32, ModifierSet) {
    let m = situational_modifiers(c, CheckType::Skill(skill));
    (c.statblock.skill(skill) + m.total(), m)
}

pub fn perception_modifier(c: &Combatant) -> (i32, ModifierSet) {
    let m = situational_modifiers(c, CheckType::Perception);
    (c.statblock.perception + m.total(), m)
}

pub fn initiative_modifier(c: &Combatant) -> (i32, ModifierSet) {
    let (p, mut m) = perception_modifier(c);
    if c.statblock.initiative_bonus != 0 {
        m.push(Modifier::untyped(c.statblock.initiative_bonus, "initiative bonus"));
    }
    (p + c.statblock.initiative_bonus, m)
}

pub fn spell_dc(c: &Combatant, entry: usize) -> i32 {
    let Some(e) = c.statblock.spellcasting.get(entry) else { return 10 };
    e.dc + situational_modifiers(c, CheckType::SpellDc { attribute: e.attribute }).total()
}

/// Multiple attack penalty for the next attack.
pub fn map_penalty(attacks_made: u8, agile: bool) -> i32 {
    match (attacks_made, agile) {
        (0, _) => 0,
        (1, false) => -5,
        (1, true) => -4,
        (_, false) => -10,
        (_, true) => -8,
    }
}

/// Is `target` off-guard against this attacker (conditions or flanking)?
pub fn is_off_guard_to(state: &CombatState, attacker: CombatantId, target: CombatantId, melee: bool) -> bool {
    let t = state.c(target);
    if t.conditions.has(CK::OffGuard) {
        return true;
    }
    melee && state.options.flanking && is_flanked_by(state, attacker, target)
}

pub fn is_flanked_by(state: &CombatState, attacker: CombatantId, target: CombatantId) -> bool {
    let a = state.c(attacker);
    let t = state.c(target);
    let Some(a_reach) = a.statblock.melee_reach() else { return false };
    if !within_reach(a.position, a.statblock.size, t.position, t.statblock.size, a_reach) {
        return false;
    }
    state.combatants.iter().any(|ally| {
        ally.id != attacker
            && ally.id != target
            && ally.side == a.side
            && ally.can_act()
            && !ally.is_hazard()
            && ally
                .statblock
                .melee_reach()
                .map(|r| within_reach(ally.position, ally.statblock.size, t.position, t.statblock.size, r))
                .unwrap_or(false)
            && flanks(a.position, a.statblock.size, ally.position, ally.statblock.size, t.position, t.statblock.size)
    })
}

/// Map-based cover between two combatants, with Take Cover upgrades.
pub fn cover_between(state: &CombatState, attacker: CombatantId, target: CombatantId, ranged: bool) -> CoverInfo {
    let a = state.c(attacker);
    let t = state.c(target);
    let others: HashSet<_> = state
        .combatants
        .iter()
        .filter(|c| c.id != attacker && c.id != target && c.life == super::state::LifeState::Active && !c.is_hazard())
        .flat_map(|c| c.footprint())
        .collect();
    let mut info = compute_cover(&state.battlefield, a.position, a.statblock.size, t.position, t.statblock.size, &others);
    if !state.options.auto_cover && info.line_of_effect {
        info.cover = CoverLevel::None;
    }
    if t.has_tag(&EffectTag::TakingCover) {
        if info.cover == CoverLevel::Standard {
            info.cover = CoverLevel::Greater;
        } else if ranged && t.conditions.has(CK::Prone) && info.cover < CoverLevel::Standard {
            info.cover = CoverLevel::Standard;
        }
    }
    info
}

/// Breakdown of an attack: total modifier and target AC.
#[derive(Debug, Clone)]
pub struct AttackNumbers {
    pub modifier: i32,
    pub modifier_breakdown: ModifierSet,
    pub ac: i32,
    pub ac_breakdown: ModifierSet,
}

pub fn strike_of(c: &Combatant, index: usize) -> Option<&StrikeProfile> {
    c.statblock.strikes.get(index)
}

pub fn range_penalty(distance_ft: u32, increment_ft: u32) -> Option<i32> {
    if increment_ft == 0 {
        return Some(0);
    }
    let increments = distance_ft.saturating_sub(1) / increment_ft; // 0 within first increment
    if increments >= 6 {
        None
    } else {
        Some(-2 * increments as i32)
    }
}

pub fn attack_numbers(state: &CombatState, ctx: &AttackCtx) -> AttackNumbers {
    let a = state.c(ctx.attacker);
    let t = state.c(ctx.target);
    let mut mods = ModifierSet::new();
    let (base, check, melee) = match &ctx.source {
        AttackSource::Strike { index } => {
            let s = &a.statblock.strikes[*index];
            let melee = s.is_melee();
            if let StrikeRange::Ranged { increment_ft } = s.range {
                let d = creature_distance(a.position, a.statblock.size, t.position, t.statblock.size);
                if let Some(p) = range_penalty(d, increment_ft) {
                    if p != 0 {
                        mods.push(Modifier::untyped(p, "range increment"));
                    }
                }
            }
            (s.attack_bonus, CheckType::Attack { attribute: s.attribute }, melee)
        }
        AttackSource::Spell { entry, .. } => {
            let e = &a.statblock.spellcasting[*entry];
            (e.attack_bonus, CheckType::SpellAttack { attribute: e.attribute }, false)
        }
    };
    if ctx.map_penalty != 0 {
        mods.push(Modifier::untyped(ctx.map_penalty, "multiple attack penalty"));
    }
    if ctx.sweep {
        mods.push(Modifier::circumstance(1, "sweep"));
    }
    mods.extend(situational_modifiers(a, check));

    let mut ac_mods = situational_modifiers(t, CheckType::Ac);
    if !t.conditions.has(CK::OffGuard) && melee && state.options.flanking && is_flanked_by(state, ctx.attacker, ctx.target) {
        ac_mods.push(Modifier::circumstance(-2, "off-guard (flanked)"));
    }
    if ctx.cover != CoverLevel::None {
        ac_mods.push(Modifier::circumstance(ctx.cover.ac_bonus(), format!("{:?} cover", ctx.cover).to_lowercase()));
    }
    if t.has_tag(&EffectTag::ShieldRaised) {
        if let Some(sh) = &t.statblock.shield {
            ac_mods.push(Modifier::circumstance(sh.ac_bonus, "raised shield"));
        }
    }
    AttackNumbers {
        modifier: base + mods.total(),
        modifier_breakdown: mods,
        ac: t.statblock.ac + ac_mods.total(),
        ac_breakdown: ac_mods,
    }
}

/// Builds the damage roll specs for a successful Strike.
pub fn strike_damage_specs(state: &CombatState, ctx: &AttackCtx, critical: bool) -> Vec<DamageRollSpec> {
    let AttackSource::Strike { index } = ctx.source else { return Vec::new() };
    let a = state.c(ctx.attacker);
    let s = &a.statblock.strikes[index];
    let mut specs = Vec::new();
    let Some(base) = s.damage.first() else { return specs };

    let mut dice = base.dice.clone();
    let weapon_dice = dice.dice_count();
    if critical {
        if let Some(f) = fatal_die(&s.traits) {
            dice = dice.with_die_size(f);
        }
    }
    let mut flat = 0;
    if s.traits.contains(&WeaponTrait::Forceful) && ctx.attack_number >= 1 {
        flat += weapon_dice as i32 * if ctx.attack_number == 1 { 1 } else { 2 };
    }
    for r in &a.statblock.damage_riders {
        if let DamageRider::FlatDamage { value, .. } = r {
            flat += value;
        }
    }
    let enfeebled = a.conditions.value(CK::Enfeebled) as i32;
    if s.strength_damage && enfeebled > 0 {
        flat -= enfeebled;
    }
    for e in &a.effects {
        for g in &e.grants {
            if g.selector == Selector::Damage {
                flat += g.modifier.value;
            }
        }
    }
    dice.modifier += flat;
    specs.push(DamageRollSpec {
        label: s.name.clone(),
        dice,
        damage_type: base.damage_type,
        precision: base.precision,
        doubled: critical,
    });
    if critical {
        if let Some(f) = fatal_die(&s.traits) {
            specs.push(DamageRollSpec {
                label: format!("{} (fatal die)", s.name),
                dice: DiceExpr::new(1, f, 0),
                damage_type: base.damage_type,
                precision: false,
                doubled: false,
            });
        }
        if let Some(d) = deadly_die(&s.traits) {
            let n = match weapon_dice {
                0..=2 => 1,
                3 => 2,
                _ => 3,
            };
            specs.push(DamageRollSpec {
                label: format!("{} (deadly)", s.name),
                dice: DiceExpr::new(n, d, 0),
                damage_type: base.damage_type,
                precision: false,
                doubled: false,
            });
        }
    }
    for extra in s.damage.iter().skip(1) {
        specs.push(DamageRollSpec {
            label: format!("{} ({})", s.name, extra.damage_type.label()),
            dice: extra.dice.clone(),
            damage_type: extra.damage_type,
            precision: extra.precision,
            doubled: critical,
        });
    }
    for r in &a.statblock.damage_riders {
        if let DamageRider::SneakAttack { dice } = r {
            if is_off_guard_to(state, ctx.attacker, ctx.target, s.is_melee()) {
                specs.push(DamageRollSpec {
                    label: "Sneak Attack".into(),
                    dice: dice.clone(),
                    damage_type: base.damage_type,
                    precision: true,
                    doubled: critical,
                });
            }
        }
    }
    specs
}

/// Can `attacker` Strike `target` with strike `index` from where it stands?
pub fn strike_reachable(state: &CombatState, attacker: CombatantId, target: CombatantId, index: usize) -> Result<CoverInfo, String> {
    let a = state.c(attacker);
    let t = state.c(target);
    let s = strike_of(a, index).ok_or("no such strike")?;
    let dist = creature_distance(a.position, a.statblock.size, t.position, t.statblock.size);
    match s.range {
        StrikeRange::Melee { reach_ft } => {
            if !within_reach(a.position, a.statblock.size, t.position, t.statblock.size, reach_ft) {
                return Err(format!("{} is out of reach ({dist} ft, reach {reach_ft} ft)", t.name));
            }
        }
        StrikeRange::Ranged { increment_ft } => {
            if range_penalty(dist, increment_ft).is_none() {
                return Err(format!("{} is beyond maximum range", t.name));
            }
        }
    }
    let info = cover_between(state, attacker, target, !s.is_melee());
    if !info.line_of_effect {
        return Err(format!("no line of effect to {}", t.name));
    }
    Ok(info)
}

/// Flat-check DC imposed by the target's concealment, from the attacker's view.
pub fn concealment_dc(state: &CombatState, attacker: CombatantId, target: CombatantId) -> Option<i32> {
    let a = state.c(attacker);
    let t = state.c(target);
    let hidden = t.conditions.has(CK::Hidden) || a.conditions.has(CK::Blinded);
    if hidden {
        return Some(11);
    }
    let in_fog = t
        .footprint()
        .iter()
        .any(|p| state.battlefield.terrain(*p) == crate::models::battlefield::Terrain::Obscured);
    if t.conditions.has(CK::Concealed) || a.conditions.has(CK::Dazzled) || in_fog {
        return Some(5);
    }
    None
}
