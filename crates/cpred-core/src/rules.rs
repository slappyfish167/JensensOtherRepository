//! Rule constants, data tables and small pure rule functions.
//!
//! # Rules assumptions
//!
//! This engine implements a *simplified* subset of Cyberpunk RED combat
//! mechanics. Items marked **VERIFY** are interpretations or numbers the GM
//! should check against their own rulebook; the numeric ones live in
//! `content/cpred/rules.json` ([`RulesTables`]) or in [`RulesOptions`], so they
//! can be corrected without code changes.
//!
//! **Grid.** One square = 2 m. Distances are Chebyshev (a diagonal step is one
//! square). Glyphs: `.` open, `~` difficult (2 squares to enter), `#` wall
//! (blocks movement and line of sight), `%` cover object (blocks movement,
//! does not block sight, has HP). Moving diagonally past the corner of a wall
//! is not allowed (same convention as the PF2e engine).
//!
//! **Stats and checks.** A check is `STAT + skill + 1d10 + modifiers`. Ranged
//! weapon skills and Autofire use REF; Melee Weapon, Brawling, Martial Arts,
//! Evasion and Athletics use DEX; Perception uses INT. The human enters the
//! natural d10. A natural 10 asks for one more d10 that is **added**; a
//! natural 1 asks for one more d10 that is **subtracted**; the follow-up never
//! explodes again. The roller succeeds only with a total strictly **greater**
//! than the number to beat — ties go to the defender.
//!
//! **Armor.** Body and head pieces each have SP and a penalty. The worst
//! penalty of the two applies to REF, DEX and MOVE (effective REF/DEX min 0,
//! MOVE min 1).
//!
//! **HP and wounds** (**VERIFY**). Max HP = `10 + 5 × ceil((BODY + WILL) / 2)`
//! unless the statblock gives `max_hp`. *Seriously Wounded*: HP below
//! `ceil(max / 2)` → −2 to all checks. *Mortally Wounded*: HP ≤ 0 (HP is
//! clamped at 0) → −4 to all checks and MOVE −6 (min 1); the combatant is
//! still alive and still takes turns.
//!
//! **Initiative.** REF (effective, with wound penalties) + 1d10, natural die
//! entered, no explosion. Highest first; ties: higher effective REF, then the
//! lower combatant id.
//!
//! **Turn.** One Move Action (up to effective MOVE squares) and one Action.
//! Run = spend the Action on a second move. ROF 2 weapons get two attacks per
//! Action (targets may differ). Aimed shots and autofire use the whole
//! Action.
//!
//! **Ranged attacks** (**VERIFY**: DV tables in `rules.json`). Need line of
//! sight (no wall on the line between the squares), ammunition and a range
//! band the weapon's table covers. The number to beat is the DV for the band
//! of the distance in metres (squares × 2). Bands: 0–6, 7–12, 13–25, 26–50,
//! 51–100, 101–200, 201–400, 401–800 m.
//!
//! **Dodging ranged attacks** (**VERIFY**). A living target that is not
//! mortally wounded and has effective REF ≥ 8 is asked whether it dodges. If
//! it does, it rolls DEX + Evasion + 1d10 and that total *replaces* the DV.
//! Dodging is free and unlimited.
//!
//! **Melee attacks.** The target must be adjacent. The defender always rolls
//! DEX + Evasion + 1d10 first (with its penalties, even if it cannot act),
//! then the attacker must beat that total.
//!
//! **Aimed shot.** Ranged only, −8 to hit, hits the head.
//!
//! **Autofire** (**VERIFY**). Needs an autofire-capable weapon with at least
//! 10 rounds, uses the Autofire skill against the autofire DV table (or a
//! dodge roll), costs 10 rounds. On a hit the damage is the sum of the 2d6
//! faces × `min(total − number to beat, weapon's max multiplier)`.
//!
//! **Damage.** The human enters every die face. Two or more 6s (on d6 damage
//! dice) cause a *Critical Injury* at the hit location: it is recorded (the GM
//! resolves the specific injury on their own table), deals 5 bonus damage
//! straight to HP (ignoring armor, even if the hit itself was stopped by SP)
//! and raises the target's death save penalty by 1.
//!
//! **Armor resolution.** Location is the body unless aimed (head). Melee
//! weapons (not unarmed attacks) halve SP, rounding up. If damage > SP: HP
//! loss = damage − SP (doubled for head hits) and that location's SP is
//! ablated by 1 (min 0). Otherwise nothing happens.
//!
//! **Cover** (**VERIFY**, simplification). Each `%` square is a cover object
//! with HP (default 20, [`RulesOptions::cover_hp`]). A ranged attack whose line
//! passes through a cover square adjacent to the target is *blocked*: it is
//! still rolled against the normal DV, but a hit damages the cover object
//! (full damage, no armor, no critical injury) instead of the target. At 0 HP
//! the square becomes open floor. Melee ignores cover.
//!
//! **Death saves** (**VERIFY**). At the start of a mortally wounded
//! combatant's turn it rolls a d10 (natural face): success iff the face is not
//! 10 and `face + death save penalty < BODY` (`≤` with
//! [`RulesOptions::death_save_inclusive`]). Failure = death. Each success
//! raises the penalty by 1 and the turn continues. A mortally wounded
//! combatant that takes damage immediately makes a death save
//! ([`RulesOptions::death_save_on_damage`]). Dead combatants skip their turns.
//!
//! **End of combat.** When every non-neutral combatant of one side is dead or
//! mortally wounded, the other side wins (both sides at once: no winner).

use crate::model::{HitLocation, RangeTable, Weapon, WeaponClass};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tactics_core::DiceExpr;

/// Metres per grid square.
pub const METRES_PER_SQUARE: u32 = 2;
/// Check penalty while seriously wounded.
pub const SERIOUSLY_WOUNDED_PENALTY: i32 = -2;
/// Check penalty while mortally wounded.
pub const MORTALLY_WOUNDED_PENALTY: i32 = -4;
/// MOVE penalty while mortally wounded.
pub const MORTALLY_WOUNDED_MOVE_PENALTY: i32 = 6;

/// How an attack is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackMode {
    /// An ordinary attack (one shot / one swing).
    Single,
    /// Aimed shot at the head.
    Aimed,
    /// Autofire.
    Autofire,
}

/// Brawling damage tier: applies when BODY ≥ `min_body`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BrawlingTier {
    pub min_body: i32,
    pub dice: DiceExpr,
}

/// Rules data loaded from `content/cpred/rules.json`. A copy lives in every
/// `CombatState`, so later edits of the file never change an old combat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesTables {
    /// Inclusive upper bound (metres) of each range band.
    pub range_bands_m: Vec<u32>,
    /// DV tables per weapon class.
    pub range_tables: BTreeMap<WeaponClass, RangeTable>,
    /// Brawling damage by BODY.
    pub brawling_damage: Vec<BrawlingTier>,
    /// Damage dice of an autofire hit (before the multiplier).
    #[serde(default = "default_autofire_damage")]
    pub autofire_damage: DiceExpr,
    /// Rounds an autofire attack costs.
    #[serde(default = "default_autofire_rounds")]
    pub autofire_rounds: u32,
    /// To-hit penalty of an aimed shot (positive number, subtracted).
    #[serde(default = "default_aimed_penalty")]
    pub aimed_shot_penalty: i32,
    /// Bonus damage of a critical injury (straight to HP).
    #[serde(default = "default_crit_damage")]
    pub critical_injury_damage: i32,
    /// Minimum effective REF needed to dodge ranged attacks.
    #[serde(default = "default_dodge_ref")]
    pub dodge_min_ref: i32,
}

fn default_autofire_damage() -> DiceExpr {
    DiceExpr::new(2, 6, 0)
}
fn default_autofire_rounds() -> u32 {
    10
}
fn default_aimed_penalty() -> i32 {
    8
}
fn default_crit_damage() -> i32 {
    5
}
fn default_dodge_ref() -> i32 {
    8
}

impl RulesTables {
    /// Index of the range band containing `distance_m`, if any.
    pub fn range_band(&self, distance_m: u32) -> Option<usize> {
        self.range_bands_m.iter().position(|&max| distance_m <= max)
    }

    /// DV for `weapon` in `band`: the weapon's own table if it has one,
    /// otherwise its class table. `None` = cannot attack at that range.
    pub fn range_dv(&self, weapon: &Weapon, band: usize, autofire: bool) -> Option<i32> {
        if !weapon.range_table.is_empty() {
            return weapon.range_table.dv(band, autofire);
        }
        self.range_tables.get(&weapon.class)?.dv(band, autofire)
    }

    /// Brawling damage dice for a BODY value.
    pub fn brawling_dice(&self, body: i32) -> DiceExpr {
        self.brawling_damage
            .iter()
            .filter(|t| body >= t.min_body)
            .max_by_key(|t| t.min_body)
            .map(|t| t.dice.clone())
            .unwrap_or_else(|| DiceExpr::new(1, 6, 0))
    }
}

/// Table-rule switches. Part of the state so replays honour them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RulesOptions {
    /// HP of each `%` cover object.
    #[serde(default = "default_cover_hp")]
    pub cover_hp: i32,
    /// Death save succeeds on `face + penalty ≤ BODY` instead of `< BODY`.
    #[serde(default)]
    pub death_save_inclusive: bool,
    /// A mortally wounded combatant that takes damage makes a death save.
    #[serde(default = "yes")]
    pub death_save_on_damage: bool,
}

fn default_cover_hp() -> i32 {
    20
}
fn yes() -> bool {
    true
}

impl Default for RulesOptions {
    fn default() -> Self {
        RulesOptions { cover_hp: default_cover_hp(), death_save_inclusive: false, death_save_on_damage: true }
    }
}

// ---------------------------------------------------------------------------
// Pure rule functions
// ---------------------------------------------------------------------------

/// `ceil(a / b)` for non-negative `b > 0`.
fn ceil_div(a: i32, b: i32) -> i32 {
    (a + b - 1).div_euclid(b)
}

/// Max HP = `10 + 5 × ceil((BODY + WILL) / 2)`.
pub fn max_hp_from(body: i32, will: i32) -> i32 {
    10 + 5 * ceil_div(body + will, 2)
}

/// HP strictly below this value is Seriously Wounded: `ceil(max / 2)`.
pub fn seriously_wounded_threshold(max_hp: i32) -> i32 {
    ceil_div(max_hp, 2)
}

/// The acting roller succeeds only with a total strictly greater than the
/// number to beat.
pub fn beats(total: i32, beat: i32) -> bool {
    total > beat
}

/// Autofire damage multiplier for a hit: `min(total − beat, max)`, 0 on a miss.
pub fn autofire_multiplier(total: i32, beat: i32, max: u8) -> i32 {
    (total - beat).clamp(0, max as i32)
}

/// Number of sixes among the faces of the d6 terms of `dice` (faces in term order).
pub fn count_sixes(dice: &DiceExpr, faces: &[i32]) -> usize {
    let mut i = 0;
    let mut sixes = 0;
    for t in &dice.terms {
        for _ in 0..t.count {
            if t.sides == 6 && faces.get(i) == Some(&6) {
                sixes += 1;
            }
            i += 1;
        }
    }
    sixes
}

/// Two or more 6s on the damage dice cause a Critical Injury.
pub fn is_critical_injury(dice: &DiceExpr, faces: &[i32]) -> bool {
    count_sixes(dice, faces) >= 2
}

/// SP that applies to a hit (melee weapons halve it, rounding up).
pub fn effective_sp(sp: i32, halves: bool) -> i32 {
    let sp = sp.max(0);
    if halves {
        ceil_div(sp, 2)
    } else {
        sp
    }
}

/// HP lost when `damage` meets `sp` at `location` (before critical-injury
/// bonus damage). Returns `(hp_loss, penetrated)`.
pub fn armor_result(damage: i32, sp: i32, location: HitLocation) -> (i32, bool) {
    if damage <= sp {
        return (0, false);
    }
    let loss = damage - sp;
    (if location == HitLocation::Head { loss * 2 } else { loss }, true)
}

/// Highest natural d10 face that passes a death save (a 10 always fails).
pub fn death_save_threshold(body: i32, penalty: u8, inclusive: bool) -> i32 {
    let t = body - penalty as i32 - if inclusive { 0 } else { 1 };
    t.min(9)
}

/// Death save outcome for a natural face.
pub fn death_save_succeeds(face: i32, body: i32, penalty: u8, inclusive: bool) -> bool {
    face != 10 && face <= death_save_threshold(body, penalty, inclusive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hp_formula() {
        assert_eq!(max_hp_from(7, 6), 45);
        assert_eq!(max_hp_from(5, 5), 35);
        assert_eq!(max_hp_from(8, 7), 50);
        assert_eq!(max_hp_from(0, 0), 10);
        assert_eq!(seriously_wounded_threshold(45), 23);
        assert_eq!(seriously_wounded_threshold(40), 20);
    }

    #[test]
    fn armor_and_sixes() {
        assert_eq!(armor_result(11, 11, HitLocation::Body), (0, false));
        assert_eq!(armor_result(15, 11, HitLocation::Body), (4, true));
        assert_eq!(armor_result(15, 11, HitLocation::Head), (8, true));
        assert_eq!(effective_sp(11, true), 6);
        assert_eq!(effective_sp(11, false), 11);
        let d: DiceExpr = "3d6".parse().unwrap();
        assert!(is_critical_injury(&d, &[6, 1, 6]));
        assert!(!is_critical_injury(&d, &[6, 5, 5]));
    }

    #[test]
    fn death_save_rule() {
        // BODY 6, penalty 0: faces 1..=5 succeed (face < 6).
        assert!(death_save_succeeds(5, 6, 0, false));
        assert!(!death_save_succeeds(6, 6, 0, false));
        assert!(death_save_succeeds(6, 6, 0, true));
        assert!(!death_save_succeeds(10, 12, 0, true));
        assert_eq!(death_save_threshold(6, 1, false), 4);
    }
}
