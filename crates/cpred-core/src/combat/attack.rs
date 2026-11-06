//! Attack geometry and numbers shared by legality checks, resolution and the
//! analysis layer: range, line of sight, cover, DVs and check modifiers.

use super::pending::{stat_lines, wound_line, AttackCtx, CheckRole, CheckRoll};
use super::state::CombatState;
use crate::model::{HitLocation, Skill, Terrain, Weapon};
use crate::rules::{AttackMode, METRES_PER_SQUARE};
use tactics_core::geom::line_squares;
use tactics_core::{CombatantId, Pos};

/// The geometry-dependent facts of a possible attack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackPlan {
    pub weapon: Weapon,
    pub distance_m: u32,
    /// Range DV (ranged attacks only).
    pub dv: Option<i32>,
    /// Cover square that blocks the attack (ranged attacks only).
    pub cover: Option<Pos>,
    pub location: HitLocation,
    /// Rounds the attack consumes.
    pub ammo_needed: u32,
    /// Rounds currently loaded.
    pub ammo_loaded: u32,
}

impl AttackPlan {
    pub fn has_ammo(&self) -> bool {
        self.ammo_loaded >= self.ammo_needed
    }
}

impl CombatState {
    /// Whether nothing blocks sight between two squares.
    pub fn line_of_sight(&self, from: Pos, to: Pos) -> bool {
        line_squares(from, to).into_iter().all(|sq| !self.battlefield.terrain(sq).blocks_sight())
    }

    /// The cover square adjacent to `target` on the line from `from`, if any.
    pub fn blocking_cover(&self, from: Pos, target: Pos) -> Option<Pos> {
        line_squares(from, target).into_iter().find(|sq| {
            sq.chebyshev(target) == 1 && self.battlefield.terrain(*sq) == Terrain::Cover && self.cover_at(*sq).is_some()
        })
    }

    /// Validates the geometry of an attack (weapon, target, range, line of
    /// sight, mode) ignoring the turn economy and ammunition. The error is a
    /// human-readable reason.
    pub fn plan_attack(&self, attacker: CombatantId, weapon: usize, target: CombatantId, mode: AttackMode) -> Result<AttackPlan, String> {
        let a = self.combatant(attacker).ok_or_else(|| format!("unknown combatant {attacker}"))?;
        let t = self.combatant(target).ok_or_else(|| format!("unknown combatant {target}"))?;
        if attacker == target {
            return Err("cannot attack yourself".into());
        }
        if !t.alive {
            return Err(format!("{} is dead", t.name));
        }
        let w = a.statblock.weapons.get(weapon).ok_or_else(|| format!("{} has no weapon #{weapon}", a.name))?;
        let squares = a.position.chebyshev(t.position);
        let distance_m = squares * METRES_PER_SQUARE;
        let ammo_loaded = a.ammo.get(weapon).copied().unwrap_or(0);
        if w.melee {
            if mode != AttackMode::Single {
                return Err(format!("{} is a melee weapon: no aimed shots or autofire", w.name));
            }
            if squares != 1 {
                return Err(format!("{} must be adjacent to attack with {}", t.name, w.name));
            }
            return Ok(AttackPlan {
                weapon: w.clone(),
                distance_m,
                dv: None,
                cover: None,
                location: HitLocation::Body,
                ammo_needed: 0,
                ammo_loaded,
            });
        }
        let autofire = mode == AttackMode::Autofire;
        if autofire && w.autofire.is_none() {
            return Err(format!("{} cannot autofire", w.name));
        }
        if !self.line_of_sight(a.position, t.position) {
            return Err(format!("no line of sight from {} to {}", a.name, t.name));
        }
        let dv = self
            .rules
            .range_band(distance_m)
            .and_then(|band| self.rules.range_dv(w, band, autofire))
            .ok_or_else(|| format!("{} is out of range of {} ({distance_m} m)", t.name, w.name))?;
        let ammo_needed = match (w.uses_ammo(), autofire) {
            (false, _) => 0,
            (true, true) => self.rules.autofire_rounds,
            (true, false) => 1,
        };
        Ok(AttackPlan {
            weapon: w.clone(),
            distance_m,
            dv: Some(dv),
            cover: self.blocking_cover(a.position, t.position),
            location: if mode == AttackMode::Aimed { HitLocation::Head } else { HitLocation::Body },
            ammo_needed,
            ammo_loaded,
        })
    }

    /// Attacker check modifier and breakdown for an attack.
    pub fn attack_modifier(&self, attacker: CombatantId, weapon: usize, mode: AttackMode) -> (i32, Vec<(String, i32)>) {
        let a = self.c(attacker);
        let Some(w) = a.statblock.weapons.get(weapon) else { return (0, vec![]) };
        let skill = if mode == AttackMode::Autofire { Skill::Autofire } else { w.skill };
        let mut breakdown = vec![];
        let mut total = stat_lines(a, skill.stat(), &mut breakdown);
        let level = a.statblock.skill(skill);
        breakdown.push((skill.label().to_string(), level));
        total += level;
        total += wound_line(a, &mut breakdown);
        if mode == AttackMode::Aimed {
            let p = -self.rules.aimed_shot_penalty;
            breakdown.push(("aimed shot".to_string(), p));
            total += p;
        }
        (total, breakdown)
    }

    /// Defender's DEX + Evasion modifier and breakdown.
    pub fn evasion_modifier(&self, defender: CombatantId) -> (i32, Vec<(String, i32)>) {
        let d = self.c(defender);
        let mut breakdown = vec![];
        let mut total = stat_lines(d, Skill::Evasion.stat(), &mut breakdown);
        let level = d.statblock.skill(Skill::Evasion);
        breakdown.push((Skill::Evasion.label().to_string(), level));
        total += level;
        total += wound_line(d, &mut breakdown);
        (total, breakdown)
    }

    /// The defender's roll for a melee attack or a dodge.
    pub(crate) fn defense_check(&self, ctx: &AttackCtx, dodge: bool) -> CheckRoll {
        let (modifier, breakdown) = self.evasion_modifier(ctx.target);
        let what = if dodge { "dodge" } else { "defence" };
        CheckRoll {
            roller: ctx.target,
            purpose: format!(
                "{}'s {what} (DEX + Evasion) vs {}'s {}",
                self.name_of(ctx.target),
                self.name_of(ctx.attacker),
                self.weapon_name(ctx.attacker, ctx.weapon)
            ),
            modifier,
            breakdown,
            beat: None,
            role: CheckRole::Defense { ctx: ctx.clone(), dodge },
        }
    }

    /// The attacker's roll against `beat` (`beat_label`: "DV", "dodge", "defence").
    pub(crate) fn attack_check(&self, ctx: &AttackCtx, beat: i32, beat_label: &str) -> CheckRoll {
        let (modifier, breakdown) = self.attack_modifier(ctx.attacker, ctx.weapon, ctx.mode);
        let kind = match ctx.mode {
            AttackMode::Single => "attack",
            AttackMode::Aimed => "aimed shot",
            AttackMode::Autofire => "autofire",
        };
        CheckRoll {
            roller: ctx.attacker,
            purpose: format!(
                "{} {kind} ({} → {}, beat {beat_label} {beat})",
                self.weapon_name(ctx.attacker, ctx.weapon),
                self.name_of(ctx.attacker),
                self.name_of(ctx.target)
            ),
            modifier,
            breakdown,
            beat: Some(beat),
            role: CheckRole::Attack { ctx: ctx.clone() },
        }
    }
}
