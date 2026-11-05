//! Foundry spells → engine spell definitions.
//!
//! Resolution order for each spell on a creature's spellcasting entry:
//! 1. **Library match by slug** — the engine's hand-authored definition
//!    (exact outcomes, e.g. *fear*'s frightened values).
//! 2. **Structured translation** — damage or healing spells whose Foundry
//!    data (`system.damage`, `defense`, `area`, `range`, `heightening`) maps
//!    onto engine fields. Basic saves get their damage table from the engine;
//!    prose-only riders (conditions on a failed save) are not modelled and
//!    the spell is flagged `approximate`.
//! 3. Otherwise the spell is skipped with a warning (analysis ignores it).

use crate::formula::{damage_type, parse_damage, parse_dice};
use crate::snapshot::{SpellDefenseSnapshot, SpellSnapshot};
use pf2e_core::grid::AreaShape;
use pf2e_core::models::spell::{DegreeOutcomes, Spell, SpellDamage, SpellDefense, SpellTargeting, Tradition};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Translation {
    Library(Spell),
    Approximate(Spell),
    Unsupported(String),
}

pub fn tradition(name: Option<&str>) -> Option<Tradition> {
    Some(match name?.to_ascii_lowercase().as_str() {
        "arcane" => Tradition::Arcane,
        "divine" => Tradition::Divine,
        "occult" => Tradition::Occult,
        "primal" => Tradition::Primal,
        _ => return None,
    })
}

fn area(kind: &str, size_ft: u32) -> Option<AreaShape> {
    Some(match kind {
        "burst" => AreaShape::Burst(size_ft),
        "cone" => AreaShape::Cone(size_ft),
        "emanation" => AreaShape::Emanation(size_ft),
        "line" => AreaShape::Line(size_ft),
        _ => return None,
    })
}

/// Translates one spell. `library` holds the engine's bundled definitions.
pub fn translate(s: &SpellSnapshot, trad: Option<Tradition>, library: &BTreeMap<String, Spell>) -> Translation {
    if let Some(def) = library.get(&s.slug) {
        return Translation::Library(def.clone());
    }
    let Some(actions) = s.actions.filter(|a| (1..=3).contains(a)) else {
        return Translation::Unsupported(format!("{}: not cast with 1–3 actions", s.name));
    };
    if s.damage.is_empty() && s.healing.is_none() {
        return Translation::Unsupported(format!("{}: no damage or healing to model", s.name));
    }
    let base_rank = s.base_rank.unwrap_or(s.rank).max(1);
    let targeting = if let Some(a) = &s.area {
        match area(&a.kind, a.size_ft) {
            Some(shape) => SpellTargeting::Area { area: shape, allies_only: s.targets_allies },
            None => return Translation::Unsupported(format!("{}: unknown area '{}'", s.name, a.kind)),
        }
    } else if s.targets_self {
        SpellTargeting::SelfOnly
    } else {
        SpellTargeting::Creatures { max: s.target_count.unwrap_or(1).clamp(1, 10), allies: s.targets_allies }
    };
    let defense = match s.defense {
        None => SpellDefense::None,
        Some(SpellDefenseSnapshot::Attack) => SpellDefense::Attack,
        Some(SpellDefenseSnapshot::Save { save, basic }) => SpellDefense::Save { save, basic },
    };
    let heighten_every = s.heightening.as_ref().map(|h| h.interval).unwrap_or(0);
    let heighten_add = |i: usize| -> Option<pf2e_core::dice::DiceExpr> {
        let h = s.heightening.as_ref()?;
        parse_dice(h.damage.get(i)?).ok()
    };
    let damage = match s.damage.first() {
        None => None,
        Some(d) => {
            let fallback = d.damage_type.as_deref().and_then(damage_type);
            match parse_damage(&d.formula, fallback) {
                Ok(p) if !p.components.is_empty() => {
                    let c = &p.components[0];
                    Some(SpellDamage {
                        dice: c.dice.clone(),
                        damage_type: c.damage_type,
                        heighten_every: if heighten_add(0).is_some() { heighten_every } else { 0 },
                        heighten_add: heighten_add(0).unwrap_or_default(),
                    })
                }
                _ => return Translation::Unsupported(format!("{}: cannot read damage '{}'", s.name, d.formula)),
            }
        }
    };
    let healing = match &s.healing {
        None => None,
        Some(f) => match parse_dice(f) {
            Ok(dice) => Some(SpellDamage {
                dice,
                damage_type: pf2e_core::damage::DamageType::Vitality,
                heighten_every: if heighten_add(0).is_some() && damage.is_none() { heighten_every } else { 0 },
                heighten_add: if damage.is_none() { heighten_add(0).unwrap_or_default() } else { Default::default() },
            }),
            Err(_) => return Translation::Unsupported(format!("{}: cannot read healing '{f}'", s.name)),
        },
    };
    Translation::Approximate(Spell {
        id: s.slug.clone(),
        name: s.name.clone(),
        rank: if s.cantrip { 1 } else { base_rank },
        cantrip: s.cantrip,
        traditions: trad.into_iter().collect(),
        traits: s.traits.clone(),
        actions,
        range_ft: s.range_ft,
        targeting,
        defense,
        damage,
        healing,
        outcomes: DegreeOutcomes::default(),
        description: "Translated from Foundry spell data (damage only; other effects not modelled).".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{FormulaSnapshot, HeighteningSnapshot, SpellAreaSnapshot};
    use pf2e_core::builder::content::Content;
    use pf2e_core::stats::Save;

    fn snap(slug: &str) -> SpellSnapshot {
        SpellSnapshot {
            slug: slug.into(),
            name: slug.into(),
            rank: 1,
            base_rank: Some(1),
            cantrip: false,
            actions: Some(2),
            range_ft: Some(30),
            area: None,
            target_count: None,
            targets_allies: false,
            targets_self: false,
            defense: None,
            damage: vec![],
            healing: None,
            heightening: None,
            traits: vec![],
        }
    }

    fn library() -> BTreeMap<String, Spell> {
        Content::bundled().unwrap().spells.into_iter().map(|s| (s.id.clone(), s)).collect()
    }

    #[test]
    fn library_spells_win() {
        let t = translate(&snap("fear"), None, &library());
        assert!(matches!(t, Translation::Library(s) if s.id == "fear" && !s.outcomes.failure.is_empty()));
    }

    #[test]
    fn damage_spell_translates_with_heightening() {
        let mut s = snap("thunderstrike-variant");
        s.defense = Some(SpellDefenseSnapshot::Save { save: Save::Reflex, basic: true });
        s.damage = vec![FormulaSnapshot { formula: "1d12".into(), damage_type: Some("electricity".into()) }];
        s.heightening = Some(HeighteningSnapshot { interval: 1, damage: vec!["1d12".into()] });
        s.area = Some(SpellAreaSnapshot { kind: "burst".into(), size_ft: 10 });
        let Translation::Approximate(sp) = translate(&s, Some(Tradition::Primal), &library()) else { panic!() };
        let d = sp.damage.unwrap();
        assert_eq!(d.at_rank(1, 3).to_string(), "3d12");
        assert_eq!(d.damage_type, pf2e_core::damage::DamageType::Electricity);
        assert!(matches!(sp.targeting, SpellTargeting::Area { area: AreaShape::Burst(10), .. }));
    }

    #[test]
    fn utility_and_reaction_spells_are_skipped() {
        assert!(matches!(translate(&snap("invisibility"), None, &library()), Translation::Unsupported(_)));
        let mut r = snap("feather-fall");
        r.actions = None;
        r.healing = Some("1".into());
        assert!(matches!(translate(&r, None, &library()), Translation::Unsupported(_)));
    }
}
