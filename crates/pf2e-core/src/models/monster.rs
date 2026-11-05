//! Monsters (and hazards) as reusable assets, plus the elite/weak templates.

use crate::models::statblock::{DamageRider, HazardEffect, Statblock};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monster {
    pub id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: String,
    /// Templates applied in order ("elite", "weak").
    #[serde(default)]
    pub applied_templates: Vec<MonsterTemplate>,
    pub statblock: Statblock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonsterTemplate {
    Elite,
    Weak,
}

impl Monster {
    pub fn name(&self) -> &str {
        &self.statblock.name
    }

    pub fn level(&self) -> i32 {
        self.statblock.level
    }

    pub fn is_hazard(&self) -> bool {
        self.statblock.hazard.is_some()
    }

    /// Returns a new monster with the template applied (GM Core rules).
    pub fn with_template(&self, template: MonsterTemplate) -> Monster {
        let mut m = self.clone();
        apply_template(&mut m.statblock, template);
        m.applied_templates.push(template);
        m.id = format!("{}-{}", self.id, template_slug(template));
        m
    }
}

fn template_slug(t: MonsterTemplate) -> &'static str {
    match t {
        MonsterTemplate::Elite => "elite",
        MonsterTemplate::Weak => "weak",
    }
}

/// Elite: +2 to AC, attacks, DCs, saves, Perception and skills; +2 strike
/// damage; HP by starting level; level +1 (+2 from −1/0).
/// Weak: the mirror image.
pub fn apply_template(sb: &mut Statblock, t: MonsterTemplate) {
    let d: i32 = match t {
        MonsterTemplate::Elite => 2,
        MonsterTemplate::Weak => -2,
    };
    let level = sb.level;
    let hp_delta = match t {
        MonsterTemplate::Elite => match level {
            i32::MIN..=1 => 10,
            2..=4 => 15,
            5..=19 => 20,
            _ => 30,
        },
        MonsterTemplate::Weak => -match level {
            i32::MIN..=2 => 10,
            3..=5 => 15,
            6..=20 => 20,
            _ => 30,
        },
    };
    sb.max_hp = (sb.max_hp + hp_delta).max(1);
    sb.ac += d;
    sb.fortitude += d;
    sb.reflex += d;
    sb.will += d;
    sb.perception += d;
    for v in sb.skills.values_mut() {
        *v += d;
    }
    for s in &mut sb.strikes {
        s.attack_bonus += d;
        if let Some(first) = s.damage.first_mut() {
            first.dice.modifier += d;
        }
    }
    for e in &mut sb.spellcasting {
        e.attack_bonus += d;
        e.dc += d;
    }
    if let Some(dc) = sb.class_dc.as_mut() {
        *dc += d;
    }
    for r in &mut sb.damage_riders {
        if let DamageRider::FlatDamage { value, .. } = r {
            *value += d;
        }
    }
    if let Some(h) = sb.hazard.as_mut() {
        h.stealth_dc += d;
        for o in &mut h.disable {
            o.dc += d;
        }
        if let Some(HazardEffect::Save { dc, .. }) = h.trigger_effect.as_mut() {
            *dc += d;
        }
    }
    sb.level = match t {
        MonsterTemplate::Elite => level + if level <= 0 { 2 } else { 1 },
        MonsterTemplate::Weak => level - if level == 1 { 2 } else { 1 },
    };
    let prefix = match t {
        MonsterTemplate::Elite => "Elite",
        MonsterTemplate::Weak => "Weak",
    };
    sb.name = format!("{prefix} {}", sb.name);
}
