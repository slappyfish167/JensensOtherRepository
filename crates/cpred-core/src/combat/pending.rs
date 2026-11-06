//! The resolution queue and the public description of pending input.
//!
//! Every multi-part rule is a sequence of [`Step`]s. An attack is
//! `BeginAttack → [DodgeDecision] → [defender Check → CritFollowUp?] →
//! attacker Check → [CritFollowUp] → DamageRoll`. Steps that need a human (a
//! die or a yes/no decision) stop the queue until the matching command
//! arrives; all other steps run automatically.

use super::state::CombatState;
use crate::model::{HitLocation, Stat};
use crate::rules::{self, AttackMode};
use serde::{Deserialize, Serialize};
use tactics_core::{CombatantId, DiceExpr, Pos};

/// Everything fixed when an attack is declared.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AttackCtx {
    pub attacker: CombatantId,
    pub target: CombatantId,
    pub weapon: usize,
    pub mode: AttackMode,
    pub location: HitLocation,
    /// Range DV (ranged attacks only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dv: Option<i32>,
    /// Cover square that blocks the attack (ranged attacks only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<Pos>,
}

/// What a check's total is used for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "role")]
pub enum CheckRole {
    /// Defender's DEX + Evasion: melee defence, or a dodge (`dodge = true`).
    Defense { ctx: AttackCtx, dodge: bool },
    /// The attacker's roll against `CheckRoll::beat`.
    Attack { ctx: AttackCtx },
}

/// A d10 check waiting to be rolled. Modifier and breakdown are fixed when
/// the step is created.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckRoll {
    pub roller: CombatantId,
    pub purpose: String,
    pub modifier: i32,
    pub breakdown: Vec<(String, i32)>,
    /// The number the total must exceed (`None` for defender rolls).
    pub beat: Option<i32>,
    pub role: CheckRole,
}

impl CheckRoll {
    /// Autofire attacker roll: `Some(max multiplier)`.
    pub fn autofire_max(&self, state: &CombatState) -> Option<u8> {
        match &self.role {
            CheckRole::Attack { ctx } if ctx.mode == AttackMode::Autofire => state
                .combatant(ctx.attacker)
                .and_then(|a| a.statblock.weapons.get(ctx.weapon))
                .and_then(|w| w.autofire),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "step")]
pub enum Step {
    // ----- turn structure -----
    /// Needs a natural d10.
    RollInitiative { combatant: CombatantId },
    FinalizeInitiative,
    BeginTurn,
    /// Needs a natural d10.
    DeathSave { combatant: CombatantId },
    EndTurn { combatant: CombatantId },
    AdvanceTurn,

    // ----- attack resolution -----
    BeginAttack { ctx: AttackCtx },
    /// Needs a yes/no decision from the target.
    DodgeDecision { ctx: AttackCtx },
    /// Needs a natural d10.
    Check { roll: CheckRoll },
    /// Needs a d10 that is added (`sign = 1`) or subtracted (`sign = -1`).
    CritFollowUp { roll: CheckRoll, natural: u8, subtotal: i32, sign: i32 },
    /// Needs every face of `dice`.
    DamageRoll {
        ctx: AttackCtx,
        dice: DiceExpr,
        /// Autofire multiplier.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        multiplier: Option<i32>,
        purpose: String,
    },
}

impl Step {
    pub fn needs_input(&self) -> bool {
        matches!(
            self,
            Step::RollInitiative { .. }
                | Step::DeathSave { .. }
                | Step::DodgeDecision { .. }
                | Step::Check { .. }
                | Step::CritFollowUp { .. }
                | Step::DamageRoll { .. }
        )
    }

    /// Steps that belong to the turn structure (kept by `GmOp::ClearPending`).
    pub fn is_turn_structure(&self) -> bool {
        matches!(
            self,
            Step::RollInitiative { .. } | Step::FinalizeInitiative | Step::BeginTurn | Step::EndTurn { .. } | Step::AdvanceTurn
        )
    }
}

// ---------------------------------------------------------------------------
// Public view of what the engine is waiting for.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RollKind {
    /// STAT + skill + 1d10: enter the natural d10. `beat` is the number the
    /// total must exceed (`None` for defender/evasion rolls).
    Check { modifier: i32, beat: Option<i32> },
    /// Enter a d10 to add (`sign = 1`) or subtract (`sign = -1`).
    CritFollowUp { sign: i32 },
    /// Enter a natural d10: success iff the face is ≤ `threshold` (a 10 always fails).
    DeathSave { threshold: i32 },
    /// REF + 1d10: enter the natural d10 (no critical follow-up).
    Initiative { modifier: i32 },
    /// Enter every die face of `dice`.
    Damage { dice: DiceExpr },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollRequest {
    pub request_id: u32,
    pub roller: Option<CombatantId>,
    pub kind: RollKind,
    /// The dice to roll, e.g. `1d10+14`.
    pub expression: String,
    pub purpose: String,
    /// Human-readable prompt.
    pub prompt: String,
    /// What goes into the modifier, for transparency.
    #[serde(default)]
    pub breakdown: Vec<(String, i32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub request_id: u32,
    pub decider: CombatantId,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PendingInput {
    Roll(RollRequest),
    Decision(DecisionRequest),
}

impl PendingInput {
    pub fn request_id(&self) -> u32 {
        match self {
            PendingInput::Roll(r) => r.request_id,
            PendingInput::Decision(d) => d.request_id,
        }
    }
}

fn d10(modifier: i32) -> String {
    DiceExpr::new(1, 10, modifier).to_string()
}

impl CombatState {
    /// What the engine is waiting for, if anything.
    pub fn pending(&self) -> Option<PendingInput> {
        let step = self.queue.front()?;
        if !step.needs_input() {
            return None;
        }
        let id = self.input_seq;
        let roll = |roller, kind, expression: String, purpose: String, prompt: String, breakdown| {
            PendingInput::Roll(RollRequest { request_id: id, roller, kind, expression, purpose, prompt, breakdown })
        };
        Some(match step {
            Step::RollInitiative { combatant } => {
                let (modifier, breakdown) = self.initiative_modifier(*combatant);
                let expr = d10(modifier);
                let purpose = format!("initiative ({})", self.name_of(*combatant));
                let prompt = format!("Roll {expr} for {purpose} — enter the natural d10");
                roll(Some(*combatant), RollKind::Initiative { modifier }, expr, purpose, prompt, breakdown)
            }
            Step::DeathSave { combatant } => {
                let threshold = self.death_save_threshold(*combatant);
                let c = self.c(*combatant);
                let purpose = format!("{}'s death save", c.name);
                let prompt = format!(
                    "Roll 1d10 for {purpose} — succeeds on {threshold} or lower, a 10 always fails (BODY {}, penalty {})",
                    c.statblock.stats.body, c.death_save_penalty
                );
                roll(Some(*combatant), RollKind::DeathSave { threshold }, "1d10".into(), purpose, prompt, vec![])
            }
            Step::DodgeDecision { ctx } => PendingInput::Decision(DecisionRequest {
                request_id: id,
                decider: ctx.target,
                prompt: self.dodge_prompt(ctx),
            }),
            Step::Check { roll: r } => {
                let expr = d10(r.modifier);
                let prompt = format!("Roll {expr} for {} — enter the natural d10", r.purpose);
                let kind = RollKind::Check { modifier: r.modifier, beat: r.beat };
                roll(Some(r.roller), kind, expr, r.purpose.clone(), prompt, r.breakdown.clone())
            }
            Step::CritFollowUp { roll: r, sign, .. } => {
                let prompt = if *sign > 0 {
                    format!("Critical success! Roll 1d10 and ADD it ({})", r.purpose)
                } else {
                    format!("Critical failure! Roll 1d10 and SUBTRACT it ({})", r.purpose)
                };
                let purpose = format!("critical follow-up for {}", r.purpose);
                roll(Some(r.roller), RollKind::CritFollowUp { sign: *sign }, "1d10".into(), purpose, prompt, vec![])
            }
            Step::DamageRoll { ctx, dice, multiplier, purpose } => {
                let times = multiplier.map(|m| format!(", the engine multiplies by {m}")).unwrap_or_default();
                let prompt = format!("Roll {dice} for {purpose} — enter every die face{times}");
                roll(Some(ctx.attacker), RollKind::Damage { dice: dice.clone() }, dice.to_string(), purpose.clone(), prompt, vec![])
            }
            _ => return None,
        })
    }

    /// Initiative modifier: effective REF plus wound penalty.
    pub(crate) fn initiative_modifier(&self, id: CombatantId) -> (i32, Vec<(String, i32)>) {
        let c = self.c(id);
        let mut breakdown = vec![];
        let total = stat_lines(c, Stat::Ref, &mut breakdown) + wound_line(c, &mut breakdown);
        (total, breakdown)
    }

    /// Highest face that passes this combatant's death save.
    pub(crate) fn death_save_threshold(&self, id: CombatantId) -> i32 {
        let c = self.c(id);
        rules::death_save_threshold(c.statblock.stats.body, c.death_save_penalty, self.options.death_save_inclusive)
    }

    fn dodge_prompt(&self, ctx: &AttackCtx) -> String {
        let weapon = self.weapon_name(ctx.attacker, ctx.weapon);
        format!(
            "{} attacks {} with {weapon} (DV {}). Does {} dodge (roll DEX + Evasion instead of the DV)?",
            self.name_of(ctx.attacker),
            self.name_of(ctx.target),
            ctx.dv.unwrap_or(0),
            self.name_of(ctx.target)
        )
    }

    pub(crate) fn weapon_name(&self, id: CombatantId, weapon: usize) -> String {
        self.combatant(id).and_then(|c| c.statblock.weapons.get(weapon)).map(|w| w.name.clone()).unwrap_or_default()
    }
}

/// Appends the stat line(s) for `stat` (base value and armor penalty) and
/// returns the effective value.
pub(crate) fn stat_lines(c: &super::state::Combatant, stat: Stat, breakdown: &mut Vec<(String, i32)>) -> i32 {
    let raw = c.statblock.stats.get(stat);
    let eff = c.effective_stat(stat);
    breakdown.push((stat.label().to_string(), raw));
    if eff != raw {
        breakdown.push(("armor penalty".to_string(), eff - raw));
    }
    eff
}

/// Appends the wound penalty line (if any) and returns it.
pub(crate) fn wound_line(c: &super::state::Combatant, breakdown: &mut Vec<(String, i32)>) -> i32 {
    let p = c.wound_penalty();
    if c.is_mortally_wounded() {
        breakdown.push(("mortally wounded".to_string(), p));
    } else if c.is_seriously_wounded() {
        breakdown.push(("seriously wounded".to_string(), p));
    }
    p
}
