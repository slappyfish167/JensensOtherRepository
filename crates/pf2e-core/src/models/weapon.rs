//! Weapons, armor and shields.

use crate::damage::DamageType;
use crate::dice::parse_die_size;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponCategory {
    Unarmed,
    Simple,
    Martial,
    Advanced,
}

/// Weapon traits the engine understands. Serialized as compact strings
/// (`"agile"`, `"deadly-d10"`, `"reach-10"`, `"thrown-20"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum WeaponTrait {
    Agile,
    Finesse,
    Deadly(u32),
    Fatal(u32),
    Reach(u32),
    Thrown(u32),
    Forceful,
    Sweep,
    Backswing,
    Propulsive,
    TwoHand(u32),
    Versatile(DamageType),
    Volley(u32),
    Nonlethal,
    Other(String),
}

impl fmt::Display for WeaponTrait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WeaponTrait::Agile => write!(f, "agile"),
            WeaponTrait::Finesse => write!(f, "finesse"),
            WeaponTrait::Deadly(d) => write!(f, "deadly-d{d}"),
            WeaponTrait::Fatal(d) => write!(f, "fatal-d{d}"),
            WeaponTrait::Reach(r) => write!(f, "reach-{r}"),
            WeaponTrait::Thrown(r) => write!(f, "thrown-{r}"),
            WeaponTrait::Forceful => write!(f, "forceful"),
            WeaponTrait::Sweep => write!(f, "sweep"),
            WeaponTrait::Backswing => write!(f, "backswing"),
            WeaponTrait::Propulsive => write!(f, "propulsive"),
            WeaponTrait::TwoHand(d) => write!(f, "two-hand-d{d}"),
            WeaponTrait::Versatile(t) => write!(f, "versatile-{}", t.label()),
            WeaponTrait::Volley(r) => write!(f, "volley-{r}"),
            WeaponTrait::Nonlethal => write!(f, "nonlethal"),
            WeaponTrait::Other(s) => write!(f, "{s}"),
        }
    }
}

impl FromStr for WeaponTrait {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();
        let num = |rest: &str| -> Result<u32, String> {
            parse_die_size(rest).ok_or_else(|| format!("bad trait value in '{s}'"))
        };
        Ok(match s.as_str() {
            "agile" => WeaponTrait::Agile,
            "finesse" => WeaponTrait::Finesse,
            "forceful" => WeaponTrait::Forceful,
            "sweep" => WeaponTrait::Sweep,
            "backswing" => WeaponTrait::Backswing,
            "propulsive" => WeaponTrait::Propulsive,
            "nonlethal" => WeaponTrait::Nonlethal,
            _ => {
                if let Some(r) = s.strip_prefix("deadly-") {
                    WeaponTrait::Deadly(num(r)?)
                } else if let Some(r) = s.strip_prefix("fatal-") {
                    WeaponTrait::Fatal(num(r)?)
                } else if let Some(r) = s.strip_prefix("reach-") {
                    WeaponTrait::Reach(num(r)?)
                } else if let Some(r) = s.strip_prefix("thrown-") {
                    WeaponTrait::Thrown(num(r)?)
                } else if let Some(r) = s.strip_prefix("two-hand-") {
                    WeaponTrait::TwoHand(num(r)?)
                } else if let Some(r) = s.strip_prefix("volley-") {
                    WeaponTrait::Volley(num(r)?)
                } else if let Some(r) = s.strip_prefix("versatile-") {
                    WeaponTrait::Versatile(match r {
                        "b" | "bludgeoning" => DamageType::Bludgeoning,
                        "p" | "piercing" => DamageType::Piercing,
                        "s" | "slashing" => DamageType::Slashing,
                        other => return Err(format!("unknown versatile type '{other}'")),
                    })
                } else {
                    WeaponTrait::Other(s.clone())
                }
            }
        })
    }
}

impl TryFrom<String> for WeaponTrait {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<WeaponTrait> for String {
    fn from(value: WeaponTrait) -> Self {
        value.to_string()
    }
}

pub fn has_trait(traits: &[WeaponTrait], t: &WeaponTrait) -> bool {
    traits.contains(t)
}

pub fn deadly_die(traits: &[WeaponTrait]) -> Option<u32> {
    traits.iter().find_map(|t| if let WeaponTrait::Deadly(d) = t { Some(*d) } else { None })
}

pub fn fatal_die(traits: &[WeaponTrait]) -> Option<u32> {
    traits.iter().find_map(|t| if let WeaponTrait::Fatal(d) = t { Some(*d) } else { None })
}

pub fn reach_of(traits: &[WeaponTrait]) -> Option<u32> {
    traits.iter().find_map(|t| if let WeaponTrait::Reach(d) = t { Some(*d) } else { None })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Weapon {
    pub id: String,
    pub name: String,
    pub category: WeaponCategory,
    #[serde(default)]
    pub group: String,
    pub damage_die: u32,
    pub damage_type: DamageType,
    #[serde(default = "one")]
    pub hands: u8,
    /// Present for ranged weapons.
    #[serde(default)]
    pub range_increment: Option<u32>,
    #[serde(default)]
    pub traits: Vec<WeaponTrait>,
    #[serde(default)]
    pub level: u8,
}

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmorCategory {
    Unarmored,
    Light,
    Medium,
    Heavy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Armor {
    pub id: String,
    pub name: String,
    pub category: ArmorCategory,
    pub ac_bonus: i32,
    /// `None` = no cap.
    #[serde(default)]
    pub dex_cap: Option<i32>,
    #[serde(default)]
    pub check_penalty: i32,
    #[serde(default)]
    pub speed_penalty: u32,
    #[serde(default)]
    pub strength: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shield {
    pub id: String,
    pub name: String,
    pub ac_bonus: i32,
    pub hardness: i32,
    pub hp: i32,
    pub broken_threshold: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trait_strings_roundtrip() {
        for s in ["agile", "deadly-d10", "reach-10", "thrown-20", "versatile-piercing", "two-hand-d12", "shove"] {
            let t: WeaponTrait = s.parse().unwrap();
            assert_eq!(t.to_string(), s);
        }
        assert_eq!("versatile-p".parse::<WeaponTrait>().unwrap(), WeaponTrait::Versatile(DamageType::Piercing));
    }
}
