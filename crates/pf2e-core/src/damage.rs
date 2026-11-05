//! Damage types and immunities / weaknesses / resistances (IWR).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageType {
    Bludgeoning,
    Piercing,
    Slashing,
    Bleed,
    Acid,
    Cold,
    Electricity,
    Fire,
    Sonic,
    Force,
    Vitality,
    Void,
    Mental,
    Poison,
    Spirit,
    Untyped,
}

impl DamageType {
    pub fn is_physical(self) -> bool {
        matches!(self, DamageType::Bludgeoning | DamageType::Piercing | DamageType::Slashing)
    }

    pub fn is_energy(self) -> bool {
        matches!(
            self,
            DamageType::Acid
                | DamageType::Cold
                | DamageType::Electricity
                | DamageType::Fire
                | DamageType::Sonic
                | DamageType::Force
                | DamageType::Vitality
                | DamageType::Void
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            DamageType::Bludgeoning => "bludgeoning",
            DamageType::Piercing => "piercing",
            DamageType::Slashing => "slashing",
            DamageType::Bleed => "bleed",
            DamageType::Acid => "acid",
            DamageType::Cold => "cold",
            DamageType::Electricity => "electricity",
            DamageType::Fire => "fire",
            DamageType::Sonic => "sonic",
            DamageType::Force => "force",
            DamageType::Vitality => "vitality",
            DamageType::Void => "void",
            DamageType::Mental => "mental",
            DamageType::Poison => "poison",
            DamageType::Spirit => "spirit",
            DamageType::Untyped => "untyped",
        }
    }
}

/// What an immunity/weakness/resistance applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageSelector {
    Type(DamageType),
    Physical,
    Energy,
    Precision,
    All,
}

impl DamageSelector {
    pub fn matches(self, part: &DamagePart) -> bool {
        match self {
            DamageSelector::Type(t) => part.damage_type == t,
            DamageSelector::Physical => part.damage_type.is_physical(),
            DamageSelector::Energy => part.damage_type.is_energy(),
            DamageSelector::Precision => part.precision,
            DamageSelector::All => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IwrValue {
    pub applies_to: DamageSelector,
    pub value: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Iwr {
    #[serde(default)]
    pub immunities: Vec<DamageSelector>,
    #[serde(default)]
    pub weaknesses: Vec<IwrValue>,
    #[serde(default)]
    pub resistances: Vec<IwrValue>,
}

/// One rolled and typed piece of damage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamagePart {
    pub amount: i32,
    pub damage_type: DamageType,
    #[serde(default)]
    pub precision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageLine {
    pub damage_type: DamageType,
    pub rolled: i32,
    pub applied: i32,
    pub note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageResult {
    pub total: i32,
    pub lines: Vec<DamageLine>,
}

/// Applies IWR in rules order: immunity, then weakness, then resistance.
/// Only the single highest applicable weakness / resistance applies per part.
pub fn apply_iwr(parts: &[DamagePart], iwr: &Iwr) -> DamageResult {
    let mut result = DamageResult::default();
    for part in parts {
        let mut amount = part.amount.max(0);
        let mut notes: Vec<String> = Vec::new();
        if iwr.immunities.iter().any(|s| s.matches(part)) {
            notes.push("immune".into());
            amount = 0;
        } else if amount > 0 {
            if let Some(w) = iwr.weaknesses.iter().filter(|w| w.applies_to.matches(part)).map(|w| w.value).max() {
                amount += w;
                notes.push(format!("weakness {w}"));
            }
            if let Some(r) = iwr.resistances.iter().filter(|r| r.applies_to.matches(part)).map(|r| r.value).max() {
                amount = (amount - r).max(0);
                notes.push(format!("resistance {r}"));
            }
        }
        result.total += amount;
        result.lines.push(DamageLine {
            damage_type: part.damage_type,
            rolled: part.amount,
            applied: amount,
            note: notes.join(", "),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(amount: i32, t: DamageType) -> DamagePart {
        DamagePart { amount, damage_type: t, precision: false }
    }

    #[test]
    fn iwr_order() {
        let iwr = Iwr {
            immunities: vec![DamageSelector::Type(DamageType::Poison)],
            weaknesses: vec![IwrValue { applies_to: DamageSelector::Type(DamageType::Fire), value: 5 }],
            resistances: vec![
                IwrValue { applies_to: DamageSelector::Physical, value: 3 },
                IwrValue { applies_to: DamageSelector::Type(DamageType::Slashing), value: 5 },
            ],
        };
        let r = apply_iwr(
            &[part(10, DamageType::Slashing), part(4, DamageType::Fire), part(9, DamageType::Poison)],
            &iwr,
        );
        assert_eq!(r.lines[0].applied, 5); // highest resistance only
        assert_eq!(r.lines[1].applied, 9);
        assert_eq!(r.lines[2].applied, 0);
        assert_eq!(r.total, 14);
    }

    #[test]
    fn weakness_needs_damage() {
        let iwr = Iwr {
            weaknesses: vec![IwrValue { applies_to: DamageSelector::Type(DamageType::Fire), value: 5 }],
            ..Default::default()
        };
        assert_eq!(apply_iwr(&[part(0, DamageType::Fire)], &iwr).total, 0);
    }
}
