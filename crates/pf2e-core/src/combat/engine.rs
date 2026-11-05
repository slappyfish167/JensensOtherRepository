//! Command application and the resolution-queue processor.
//!
//! Invariants:
//! * `apply` is atomic: on error the state is unchanged.
//! * After `apply` returns, the queue is either empty or its front step needs
//!   human input (`Step::needs_input`). `pending()` describes that input.
//! * No randomness exists anywhere in this module.

use super::command::{Command, GmOp};
use super::event::Event;
use super::pending::*;
use super::rules::{self, attack_numbers, save_modifier, skill_modifier};
use super::state::{CombatState, InitiativeEntry, LifeState, Phase, PersistentDamage};
use crate::checks::{degree_of_success, flat_check_succeeds, DegreeOfSuccess};
use crate::conditions::{Condition, ConditionKind as CK, Duration, Expiry, TurnBoundary};
use crate::damage::{apply_iwr, DamagePart, DamageType};
use crate::dice::DiceExpr;
use crate::effects::{Effect, EffectTag};
use crate::grid::CoverLevel;
use crate::ids::CombatantId;
use crate::models::spell::{SpellDefense, SpellOutcome};
use crate::models::statblock::{HazardEffect, ReactionKind};
use crate::stats::{Save, Side, Skill};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, serde::Serialize)]
pub enum CombatError {
    #[error("combat has not started")]
    NotStarted,
    #[error("combat already started")]
    AlreadyStarted,
    #[error("combat is over")]
    CombatOver,
    #[error("it is {expected}'s turn, not {got}'s")]
    NotYourTurn { expected: CombatantId, got: CombatantId },
    #[error("resolve the pending {0} first")]
    InputPending(String),
    #[error("nothing is waiting for input")]
    NoPendingInput,
    #[error("request {got} does not match pending request {expected}")]
    WrongRequest { expected: u32, got: u32 },
    #[error("pending input is a {0}, not this kind of answer")]
    WrongInputKind(String),
    #[error("invalid roll: {0}")]
    InvalidRoll(String),
    #[error("illegal action: {0}")]
    IllegalAction(String),
    #[error("unknown combatant {0}")]
    UnknownCombatant(CombatantId),
    #[error("{needed} action(s) needed, {have} remaining")]
    NotEnoughActions { needed: u8, have: u8 },
}

pub type CombatResult<T> = Result<T, CombatError>;

impl CombatState {
    /// Applies a command atomically and returns the resulting events.
    pub fn apply(&mut self, cmd: &Command) -> CombatResult<Vec<Event>> {
        let mut next = self.clone();
        let events = next.apply_in_place(cmd)?;
        *self = next;
        Ok(events)
    }

    /// Applies a command directly. On error the state may be partially
    /// modified; callers that need atomicity (everyone except throwaway
    /// search clones) should use [`CombatState::apply`].
    pub fn apply_in_place(&mut self, cmd: &Command) -> CombatResult<Vec<Event>> {
        let mut ev = Vec::new();
        match cmd {
            Command::StartCombat => {
                if self.phase != Phase::Setup {
                    return Err(CombatError::AlreadyStarted);
                }
                self.phase = Phase::RollingInitiative;
                ev.push(Event::CombatStarted);
                let ids: Vec<_> = self.combatants.iter().filter(|c| c.in_initiative && c.life == LifeState::Active).map(|c| c.id).collect();
                for id in ids {
                    self.queue.push_back(Step::RollInitiative { combatant: id });
                }
                self.queue.push_back(Step::FinalizeInitiative);
            }
            Command::ProvideRoll { request_id, value, extra_modifier } => {
                self.check_request(*request_id)?;
                let step = self.queue.front().cloned().ok_or(CombatError::NoPendingInput)?;
                if matches!(step, Step::Decision { .. }) {
                    return Err(CombatError::WrongInputKind("decision".into()));
                }
                self.validate_roll_value(*value)?;
                self.queue.pop_front();
                self.input_seq += 1;
                self.resolve_roll(step, *value, *extra_modifier, &mut ev)?;
            }
            Command::Decide { request_id, accept } => {
                self.check_request(*request_id)?;
                let step = self.queue.front().cloned().ok_or(CombatError::NoPendingInput)?;
                let Step::Decision { decider, reaction, on_accept, on_decline, .. } = step else {
                    return Err(CombatError::WrongInputKind("roll".into()));
                };
                self.queue.pop_front();
                self.input_seq += 1;
                if *accept {
                    let r = self.cm(decider);
                    r.reaction_available = false;
                    ev.push(Event::ReactionUsed { reactor: decider, reaction: format!("{reaction:?}") });
                    self.push_front_all(on_accept);
                } else {
                    self.push_front_all(on_decline);
                }
            }
            Command::Act { actor, action } => {
                self.ensure_can_declare(*actor)?;
                self.declare_action(*actor, action, &mut ev)?;
            }
            Command::EndTurn { actor } => {
                self.ensure_can_declare(*actor)?;
                self.queue.push_back(Step::EndTurn { combatant: *actor });
            }
            Command::Gm { op } => self.apply_gm(op, &mut ev)?,
        }
        self.run_automatic(&mut ev)?;
        Ok(ev)
    }

    fn check_request(&self, request_id: u32) -> CombatResult<()> {
        if self.is_over() {
            return Err(CombatError::CombatOver);
        }
        match self.queue.front() {
            Some(s) if s.needs_input() => {
                if request_id != self.input_seq {
                    return Err(CombatError::WrongRequest { expected: self.input_seq, got: request_id });
                }
                Ok(())
            }
            _ => Err(CombatError::NoPendingInput),
        }
    }

    fn validate_roll_value(&self, value: i32) -> CombatResult<()> {
        match self.pending() {
            Some(PendingInput::Roll(r)) => match r.kind {
                RollKind::Check { .. } | RollKind::Flat { .. } => {
                    if !(1..=20).contains(&value) {
                        return Err(CombatError::InvalidRoll(format!("enter the natural d20 face (1–20), got {value}")));
                    }
                    Ok(())
                }
                RollKind::Total { dice } => {
                    if !dice.accepts_total(value) {
                        return Err(CombatError::InvalidRoll(format!(
                            "{dice} can only total {}–{}, got {value}",
                            dice.min(),
                            dice.max()
                        )));
                    }
                    Ok(())
                }
            },
            _ => Err(CombatError::NoPendingInput),
        }
    }

    pub(crate) fn ensure_can_declare(&self, actor: CombatantId) -> CombatResult<()> {
        match self.phase {
            Phase::Setup | Phase::RollingInitiative => return Err(CombatError::NotStarted),
            Phase::Ended { .. } => return Err(CombatError::CombatOver),
            Phase::Active => {}
        }
        if self.combatant(actor).is_none() {
            return Err(CombatError::UnknownCombatant(actor));
        }
        if let Some(p) = self.pending() {
            return Err(CombatError::InputPending(match p {
                PendingInput::Roll(r) => format!("roll ({})", r.purpose),
                PendingInput::Decision(_) => "reaction decision".into(),
            }));
        }
        let current = self.current_actor().ok_or(CombatError::NotStarted)?;
        if current != actor {
            return Err(CombatError::NotYourTurn { expected: current, got: actor });
        }
        Ok(())
    }

    pub(crate) fn push_front_all(&mut self, steps: Vec<Step>) {
        for s in steps.into_iter().rev() {
            self.queue.push_front(s);
        }
    }

    // -----------------------------------------------------------------------
    // Pending input description
    // -----------------------------------------------------------------------

    /// What the engine is waiting for, if anything.
    pub fn pending(&self) -> Option<PendingInput> {
        let step = self.queue.front()?;
        if !step.needs_input() {
            return None;
        }
        let id = self.input_seq;
        let name = |c: CombatantId| self.combatant(c).map(|x| x.name.clone()).unwrap_or_default();
        let check = |roller: CombatantId, modifier: i32, dc: i32, purpose: String, breakdown: Vec<crate::modifiers::Modifier>| {
            let expr = DiceExpr::d20(modifier).to_string();
            PendingInput::Roll(RollRequest {
                request_id: id,
                roller: Some(roller),
                kind: RollKind::Check { modifier, dc },
                prompt: format!("Roll {expr} for {purpose} — enter the natural d20"),
                expression: expr,
                purpose,
                breakdown,
            })
        };
        let flat = |roller: CombatantId, dc: i32, purpose: String| {
            PendingInput::Roll(RollRequest {
                request_id: id,
                roller: Some(roller),
                kind: RollKind::Flat { dc },
                expression: "1d20".into(),
                prompt: format!("Roll a DC {dc} flat check (1d20) for {purpose}"),
                purpose,
                breakdown: vec![],
            })
        };
        let total = |roller: Option<CombatantId>, dice: DiceExpr, purpose: String, note: &str| {
            PendingInput::Roll(RollRequest {
                request_id: id,
                roller,
                prompt: format!("Roll {dice} for {purpose}{note} — enter the total"),
                expression: dice.to_string(),
                kind: RollKind::Total { dice },
                purpose,
                breakdown: vec![],
            })
        };
        Some(match step {
            Step::RollInitiative { combatant } => {
                let (m, b) = rules::initiative_modifier(self.c(*combatant));
                check(*combatant, m, 0, format!("initiative ({})", name(*combatant)), b.applied().into_iter().cloned().collect())
            }
            Step::RecoveryCheck { combatant } => {
                let dc = 10 + self.c(*combatant).conditions.value(CK::Dying) as i32;
                flat(*combatant, dc, format!("{}'s recovery check", name(*combatant)))
            }
            Step::PersistentDamage { combatant, damage_type } => {
                let dice = self.persistent_dice(*combatant, *damage_type).unwrap_or_default();
                total(Some(*combatant), dice, format!("persistent {} damage on {}", damage_type.label(), name(*combatant)), "")
            }
            Step::PersistentRecovery { combatant, damage_type } => {
                flat(*combatant, 15, format!("{} to end persistent {} damage", name(*combatant), damage_type.label()))
            }
            Step::ConcealmentCheck { ctx, dc, .. } => {
                flat(ctx.attacker, *dc, format!("{} to target {} through concealment", name(ctx.attacker), name(ctx.target)))
            }
            Step::AttackRoll { ctx, .. } => {
                let n = attack_numbers(self, ctx);
                let what = match &ctx.source {
                    AttackSource::Strike { index } => self.c(ctx.attacker).statblock.strikes[*index].name.clone(),
                    AttackSource::Spell { spell_id, .. } => self.spells.get(spell_id).map(|s| s.name.clone()).unwrap_or_default(),
                };
                let mut breakdown: Vec<_> = n.modifier_breakdown.applied().into_iter().cloned().collect();
                breakdown.extend(n.ac_breakdown.applied().into_iter().map(|m| {
                    let mut m = m.clone();
                    m.source = format!("target AC: {}", m.source);
                    m
                }));
                check(
                    ctx.attacker,
                    n.modifier,
                    n.ac,
                    format!("{what} attack ({} → {}, AC {})", name(ctx.attacker), name(ctx.target), n.ac),
                    breakdown,
                )
            }
            Step::DamageRolls { specs, rolled, source, purpose, .. } => {
                let spec = &specs[rolled.len()];
                let note = if spec.doubled { " (critical: the engine doubles it)" } else { "" };
                total(
                    Some(*source),
                    spec.dice.clone(),
                    format!("{} {} damage ({purpose})", spec.label, spec.damage_type.label()),
                    note,
                )
            }
            Step::SaveSequence { source, save, pending, .. } => {
                let t = pending[0];
                let (m, b) = save_modifier(self.c(t), *save);
                check(
                    t,
                    m,
                    source.dc(),
                    format!("{}'s {save} save (DC {}) vs {}", name(t), source.dc(), name(source.origin())),
                    b.applied().into_iter().cloned().collect(),
                )
            }
            Step::HealRoll { caster, dice, spell_id, .. } => {
                let sname = self.spells.get(spell_id).map(|s| s.name.clone()).unwrap_or_default();
                total(Some(*caster), dice.clone(), format!("{sname} healing"), "")
            }
            Step::SkillCheck { actor, target, action, map_penalty, .. } => {
                let (m, dc, b, label) = self.skill_check_numbers(*actor, *target, *action, *map_penalty);
                check(*actor, m, dc, format!("{label} ({} → {}, DC {dc})", name(*actor), name(*target)), b)
            }
            Step::Decision { decider, reaction, prompt, .. } => PendingInput::Decision(DecisionRequest {
                request_id: id,
                decider: *decider,
                reaction: *reaction,
                prompt: prompt.clone(),
            }),
            _ => return None,
        })
    }

    pub(crate) fn skill_check_numbers(
        &self,
        actor: CombatantId,
        target: CombatantId,
        action: SkillAction,
        map_penalty: i32,
    ) -> (i32, i32, Vec<crate::modifiers::Modifier>, String) {
        let a = self.c(actor);
        let t = self.c(target);
        let (skill, label) = match action {
            SkillAction::Demoralize => (Skill::Intimidation, "Demoralize (Intimidation vs Will DC)"),
            SkillAction::Trip => (Skill::Athletics, "Trip (Athletics vs Reflex DC)"),
            SkillAction::Disable => {
                let s = t.statblock.hazard.as_ref().and_then(|h| h.disable.first()).map(|d| d.skill).unwrap_or(Skill::Thievery);
                (s, "Disable a Device")
            }
        };
        let (mut m, mods) = skill_modifier(a, skill);
        m += map_penalty;
        let dc = match action {
            SkillAction::Demoralize => rules::save_dc(t, Save::Will),
            SkillAction::Trip => rules::save_dc(t, Save::Reflex),
            SkillAction::Disable => t.statblock.hazard.as_ref().and_then(|h| h.disable.first()).map(|d| d.dc).unwrap_or(20),
        };
        let mut b: Vec<_> = mods.applied().into_iter().cloned().collect();
        if map_penalty != 0 {
            b.push(crate::modifiers::Modifier::untyped(map_penalty, "multiple attack penalty"));
        }
        (m, dc, b, label.to_string())
    }

    fn persistent_dice(&self, c: CombatantId, t: DamageType) -> Option<DiceExpr> {
        self.c(c).persistent.iter().find(|p| p.damage_type == t).map(|p| p.dice.clone())
    }

    // -----------------------------------------------------------------------
    // Automatic processing
    // -----------------------------------------------------------------------

    pub(crate) fn run_automatic(&mut self, ev: &mut Vec<Event>) -> CombatResult<()> {
        // Generous guard against runaway loops (bug protection, not a rule).
        for _ in 0..10_000 {
            if self.check_victory(ev) {
                return Ok(());
            }
            match self.queue.front() {
                None => return Ok(()),
                Some(s) if s.needs_input() => return Ok(()),
                Some(_) => {
                    let step = self.queue.pop_front().expect("front exists");
                    self.process_auto(step, ev)?;
                }
            }
        }
        Err(CombatError::IllegalAction("resolution loop did not terminate".into()))
    }

    fn check_victory(&mut self, ev: &mut Vec<Event>) -> bool {
        if self.is_over() {
            return true;
        }
        if self.phase != Phase::Active {
            return false;
        }
        let side_down = |side: Side| {
            let mut members = self.combatants.iter().filter(|c| c.side == side && !c.is_hazard()).peekable();
            members.peek().is_some() && members.all(|c| c.is_down())
        };
        let winner = if side_down(Side::Enemy) {
            Some(Side::Party)
        } else if side_down(Side::Party) {
            Some(Side::Enemy)
        } else {
            return false;
        };
        self.phase = Phase::Ended { winner };
        self.queue.clear();
        ev.push(Event::CombatEnded { winner });
        true
    }

    fn process_auto(&mut self, step: Step, ev: &mut Vec<Event>) -> CombatResult<()> {
        match step {
            Step::FinalizeInitiative => {
                let sides: std::collections::HashMap<CombatantId, Side> = self.combatants.iter().map(|c| (c.id, c.side)).collect();
                self.initiative.sort_by(|a, b| {
                    b.value
                        .cmp(&a.value)
                        // Ties: adversaries of the PCs act first.
                        .then_with(|| (sides[&a.combatant] == Side::Party).cmp(&(sides[&b.combatant] == Side::Party)))
                        .then_with(|| a.combatant.cmp(&b.combatant))
                });
                ev.push(Event::InitiativeOrder { order: self.initiative.iter().map(|e| (e.combatant, e.value)).collect() });
                self.phase = Phase::Active;
                self.round = 1;
                self.turn = 0;
                ev.push(Event::RoundStarted { round: 1 });
                if self.initiative.is_empty() {
                    self.phase = Phase::Ended { winner: None };
                    ev.push(Event::CombatEnded { winner: None });
                } else {
                    self.queue.push_back(Step::BeginTurn);
                }
            }
            Step::BeginTurn => self.begin_turn(ev),
            Step::EndTurn { combatant } => {
                let mut steps = Vec::new();
                let mut types: Vec<DamageType> = self.c(combatant).persistent.iter().map(|p| p.damage_type).collect();
                types.sort();
                for t in types {
                    steps.push(Step::PersistentDamage { combatant, damage_type: t });
                    steps.push(Step::PersistentRecovery { combatant, damage_type: t });
                }
                steps.push(Step::EndTurnTail { combatant });
                self.push_front_all(steps);
            }
            Step::EndTurnTail { combatant } => {
                let c = self.cm(combatant);
                let fr = c.conditions.value(CK::Frightened);
                if fr > 0 {
                    c.conditions.set_value(CK::Frightened, fr - 1);
                    if fr == 1 {
                        ev.push(Event::ConditionLost { target: combatant, condition: CK::Frightened });
                    }
                }
                self.expire(combatant, TurnBoundary::End, ev);
                ev.push(Event::TurnEnded { combatant });
                self.queue.push_back(Step::AdvanceTurn);
            }
            Step::AdvanceTurn => {
                let n = self.initiative.len();
                for _ in 0..=n {
                    self.turn += 1;
                    if self.turn >= n {
                        self.turn = 0;
                        self.round += 1;
                        ev.push(Event::RoundStarted { round: self.round });
                    }
                    let id = self.initiative[self.turn].combatant;
                    if self.c(id).life == LifeState::Active {
                        break;
                    }
                }
                self.queue.push_back(Step::BeginTurn);
            }
            Step::DeliverDamage { tag, source, target, parts, critical, from_strike, shield_offered, shield_blocked } => {
                let t = self.c(target);
                let eligible = from_strike
                    && !shield_offered
                    && t.statblock.has_reaction(ReactionKind::ShieldBlock)
                    && t.reaction_available
                    && t.can_act()
                    && t.has_tag(&EffectTag::ShieldRaised)
                    && !t.shield_broken()
                    && parts.iter().any(|p| p.damage_type.is_physical() && p.amount > 0);
                if eligible {
                    let total: i32 = parts.iter().map(|p| p.amount).sum();
                    let hardness = t.statblock.shield.as_ref().map(|s| s.hardness).unwrap_or(0);
                    let mk = |blocked| Step::DeliverDamage {
                        tag,
                        source,
                        target,
                        parts: parts.clone(),
                        critical,
                        from_strike,
                        shield_offered: true,
                        shield_blocked: blocked,
                    };
                    self.queue.push_front(Step::Decision {
                        tag,
                        decider: target,
                        reaction: ReactionKind::ShieldBlock,
                        prompt: format!(
                            "{} is about to take {total} damage. Use Shield Block (hardness {hardness})?",
                            t.name
                        ),
                        on_accept: vec![mk(true)],
                        on_decline: vec![mk(false)],
                    });
                } else {
                    self.deliver_damage(source, target, parts, critical, from_strike, shield_blocked, ev);
                }
            }
            Step::SaveSequence { tag, source, results, .. } => self.after_saves(tag, source, results, ev),
            Step::ResolveSpell { tag, caster, entry, spell_id, rank, targets } => {
                self.resolve_spell(tag, caster, entry, &spell_id, rank, targets, ev)
            }
            Step::SpellOutcomes { caster, spell_id, rank, target, degree, .. } => {
                self.apply_spell_outcomes(caster, &spell_id, rank, target, degree, ev)
            }
            Step::ContinueMove { tag, mover, kind, remaining, offered } => {
                self.continue_move(tag, mover, kind, remaining, offered, ev)
            }
            Step::HazardTrigger { hazard, victim } => self.trigger_hazard(hazard, victim, ev),
            other => {
                return Err(CombatError::IllegalAction(format!("step {other:?} cannot run automatically")));
            }
        }
        Ok(())
    }

    /// A copy of this state in which `id` is starting its turn right now,
    /// with normal start-of-turn processing. Used for "what would this
    /// creature do?" analysis. Returns `None` if `id` does not take turns,
    /// the combat is not running, or the turn would immediately need input
    /// (e.g. a recovery check).
    pub fn hypothetical_turn(&self, id: CombatantId) -> Option<CombatState> {
        if self.phase != Phase::Active || self.combatant(id)?.life != LifeState::Active {
            return None;
        }
        let idx = self.initiative_index(id)?;
        let mut s = self.clone();
        s.queue.clear();
        s.turn = idx;
        let mut ev = Vec::new();
        s.begin_turn(&mut ev);
        if s.run_automatic(&mut ev).is_err() || s.pending().is_some() || s.current_actor() != Some(id) {
            return None;
        }
        Some(s)
    }

    fn begin_turn(&mut self, ev: &mut Vec<Event>) {
        let id = self.initiative[self.turn].combatant;
        self.expire(id, TurnBoundary::Start, ev);
        let round = self.round;
        let c = self.cm(id);
        c.attacks_this_turn = 0;
        c.struck_this_turn.clear();
        c.reaction_available = true;
        let mut actions: i32 = 3;
        if c.conditions.has(CK::Quickened) {
            actions += 1;
        }
        let stunned = c.conditions.value(CK::Stunned) as i32;
        if stunned > 0 {
            let lost = stunned.min(actions);
            actions -= lost;
            c.conditions.set_value(CK::Stunned, (stunned - lost) as u8);
        } else {
            actions -= c.conditions.value(CK::Slowed) as i32;
        }
        let can_act = c.can_act();
        c.actions_remaining = if can_act { actions.max(0) as u8 } else { 0 };
        let dying = c.conditions.value(CK::Dying);
        ev.push(Event::TurnStarted { combatant: id, actions: c.actions_remaining });
        let _ = round;
        if dying > 0 {
            self.queue.push_back(Step::RecoveryCheck { combatant: id });
            self.queue.push_back(Step::EndTurn { combatant: id });
        } else if !can_act {
            self.queue.push_back(Step::EndTurn { combatant: id });
        }
    }

    /// Removes effects and conditions whose expiry matches this boundary.
    fn expire(&mut self, who: CombatantId, boundary: TurnBoundary, ev: &mut Vec<Event>) {
        let round = self.round;
        let due = |e: &Option<Expiry>| {
            e.map(|x| x.combatant == who && x.boundary == boundary && x.round <= round).unwrap_or(false)
        };
        for c in &mut self.combatants {
            let id = c.id;
            let (gone, keep): (Vec<_>, Vec<_>) = c.effects.drain(..).partition(|e| due(&e.expires));
            c.effects = keep;
            for e in gone {
                ev.push(Event::EffectEnded { target: id, name: e.name });
            }
            let (gone, keep): (Vec<_>, Vec<_>) = c.conditions.0.drain(..).partition(|x| due(&x.expires));
            c.conditions.0 = keep;
            for x in gone {
                ev.push(Event::ConditionLost { target: id, condition: x.kind });
            }
        }
    }

    /// Converts a rules-text duration into a concrete expiry.
    pub fn expiry_for(&self, d: Duration, source: CombatantId, target: CombatantId) -> Option<Expiry> {
        let next_turn_round = |id: CombatantId| match self.initiative_index(id) {
            Some(i) if i > self.turn => self.round,
            _ => self.round + 1,
        };
        let src = if self.initiative_index(source).is_some() { source } else { target };
        match d {
            Duration::Rounds(n) => Some(Expiry { combatant: src, round: self.round + n, boundary: TurnBoundary::Start }),
            Duration::UntilEndOfTargetsNextTurn => {
                Some(Expiry { combatant: target, round: next_turn_round(target), boundary: TurnBoundary::End })
            }
            Duration::UntilStartOfSourcesNextTurn => {
                Some(Expiry { combatant: src, round: next_turn_round(src), boundary: TurnBoundary::Start })
            }
            Duration::Unlimited => None,
        }
    }

    // -----------------------------------------------------------------------
    // Roll resolution
    // -----------------------------------------------------------------------

    fn resolve_roll(&mut self, step: Step, value: i32, extra: i32, ev: &mut Vec<Event>) -> CombatResult<()> {
        let nat = value.clamp(1, 20) as u8;
        match step {
            Step::RollInitiative { combatant } => {
                let (m, _) = rules::initiative_modifier(self.c(combatant));
                let total = value + m + extra;
                self.initiative.push(InitiativeEntry { combatant, value: total, natural: nat });
                ev.push(Event::CheckResolved {
                    roller: Some(combatant),
                    purpose: "initiative".into(),
                    natural: nat,
                    total,
                    dc: 0,
                    degree: DegreeOfSuccess::Success,
                });
            }
            Step::RecoveryCheck { combatant } => {
                let c = self.cm(combatant);
                let dying = c.conditions.value(CK::Dying) as i32;
                let dc = 10 + dying;
                let degree = degree_of_success(nat, nat as i32 + extra, dc);
                let delta = match degree {
                    DegreeOfSuccess::CriticalSuccess => -2,
                    DegreeOfSuccess::Success => -1,
                    DegreeOfSuccess::Failure => 1,
                    DegreeOfSuccess::CriticalFailure => 2,
                };
                ev.push(Event::CheckResolved {
                    roller: Some(combatant),
                    purpose: "recovery check".into(),
                    natural: nat,
                    total: nat as i32,
                    dc,
                    degree,
                });
                let new = dying + delta;
                if new <= 0 {
                    c.conditions.remove(CK::Dying);
                    let w = c.conditions.value(CK::Wounded);
                    c.conditions.set_value(CK::Wounded, w + 1);
                    ev.push(Event::ConditionLost { target: combatant, condition: CK::Dying });
                    ev.push(Event::Note { text: format!("{} is stable (unconscious, wounded {})", c.name, w + 1) });
                } else {
                    c.conditions.set_value(CK::Dying, new as u8);
                    self.check_death(combatant, ev);
                }
            }
            Step::PersistentDamage { combatant, damage_type } => {
                ev.push(Event::DamageRolled {
                    purpose: format!("persistent {}", damage_type.label()),
                    expression: self.persistent_dice(combatant, damage_type).map(|d| d.to_string()).unwrap_or_default(),
                    rolled: value,
                    counted: value + extra,
                });
                let parts = vec![DamagePart { amount: (value + extra).max(0), damage_type, precision: false }];
                let src = self.c(combatant).persistent.iter().find(|p| p.damage_type == damage_type).and_then(|p| p.source);
                self.deliver_damage(src.unwrap_or(combatant), combatant, parts, false, false, false, ev);
            }
            Step::PersistentRecovery { combatant, damage_type } => {
                let ok = flat_check_succeeds(nat, 15);
                ev.push(Event::FlatCheck {
                    combatant,
                    purpose: format!("end persistent {}", damage_type.label()),
                    natural: nat,
                    dc: 15,
                    success: ok,
                });
                if ok {
                    self.cm(combatant).persistent.retain(|p| p.damage_type != damage_type);
                    ev.push(Event::Note { text: format!("persistent {} damage ends", damage_type.label()) });
                }
            }
            Step::ConcealmentCheck { tag, ctx, dc } => {
                let ok = flat_check_succeeds(nat, dc);
                ev.push(Event::FlatCheck { combatant: ctx.attacker, purpose: "concealment".into(), natural: nat, dc, success: ok });
                if ok {
                    self.queue.push_front(Step::AttackRoll { tag, ctx });
                } else {
                    ev.push(Event::Note { text: "the attack misses due to concealment".into() });
                    if let AttackSource::Spell { spell_id, rank, .. } = &ctx.source {
                        self.queue.push_front(Step::SpellOutcomes {
                            tag,
                            caster: ctx.attacker,
                            spell_id: spell_id.clone(),
                            rank: *rank,
                            target: ctx.target,
                            degree: DegreeOfSuccess::Failure,
                        });
                    }
                }
            }
            Step::AttackRoll { tag, ctx } => {
                let n = attack_numbers(self, &ctx);
                let total = value + n.modifier + extra;
                let degree = degree_of_success(nat, total, n.ac);
                ev.push(Event::CheckResolved {
                    roller: Some(ctx.attacker),
                    purpose: "attack".into(),
                    natural: nat,
                    total,
                    dc: n.ac,
                    degree,
                });
                self.after_attack(tag, ctx, degree, ev);
            }
            Step::DamageRolls { tag, source, shares, specs, mut rolled, critical, from_strike, purpose } => {
                let spec = &specs[rolled.len()];
                let mut amount = value + extra;
                if spec.doubled {
                    amount *= 2;
                }
                ev.push(Event::DamageRolled {
                    purpose: format!("{} ({})", spec.label, purpose),
                    expression: spec.dice.to_string(),
                    rolled: value,
                    counted: amount,
                });
                rolled.push(DamagePart { amount, damage_type: spec.damage_type, precision: spec.precision });
                if rolled.len() < specs.len() {
                    self.queue.push_front(Step::DamageRolls { tag, source, shares, specs, rolled, critical, from_strike, purpose });
                } else {
                    // Minimum 1 damage on a damaging hit.
                    let sum: i32 = rolled.iter().map(|p| p.amount).sum();
                    if sum < 1 {
                        if let Some(first) = rolled.first_mut() {
                            first.amount += 1 - sum;
                        }
                    }
                    let mut steps = Vec::new();
                    for share in shares {
                        let parts: Vec<DamagePart> = rolled
                            .iter()
                            .map(|p| DamagePart {
                                amount: p.amount * share.numerator as i32 / share.denominator.max(1) as i32,
                                damage_type: p.damage_type,
                                precision: p.precision,
                            })
                            .collect();
                        steps.push(Step::DeliverDamage {
                            tag,
                            source,
                            target: share.target,
                            parts,
                            critical: critical || share.degree == Some(DegreeOfSuccess::CriticalFailure),
                            from_strike,
                            shield_offered: false,
                            shield_blocked: false,
                        });
                    }
                    self.push_front_all(steps);
                }
            }
            Step::SaveSequence { tag, source, save, mut pending, mut results } => {
                let t = pending.remove(0);
                let (m, _) = save_modifier(self.c(t), save);
                let total = value + m + extra;
                let degree = degree_of_success(nat, total, source.dc());
                ev.push(Event::CheckResolved {
                    roller: Some(t),
                    purpose: format!("{save} save"),
                    natural: nat,
                    total,
                    dc: source.dc(),
                    degree,
                });
                results.push((t, degree));
                self.queue.push_front(Step::SaveSequence { tag, source, save, pending, results });
            }
            Step::HealRoll { caster, targets, .. } => {
                let amount = (value + extra).max(0);
                for t in targets {
                    self.heal(t, amount, ev);
                }
                let _ = caster;
            }
            Step::SkillCheck { tag, actor, target, action, map_penalty } => {
                let (m, dc, _, label) = self.skill_check_numbers(actor, target, action, map_penalty);
                let total = value + m + extra;
                let degree = degree_of_success(nat, total, dc);
                ev.push(Event::CheckResolved { roller: Some(actor), purpose: label, natural: nat, total, dc, degree });
                self.after_skill_check(tag, actor, target, action, degree, ev);
            }
            other => return Err(CombatError::IllegalAction(format!("step {other:?} does not take a roll"))),
        }
        Ok(())
    }

    fn after_attack(&mut self, tag: u32, ctx: AttackCtx, degree: DegreeOfSuccess, ev: &mut Vec<Event>) {
        let crit = degree == DegreeOfSuccess::CriticalSuccess;
        if crit {
            if let Some(dtag) = ctx.disrupts {
                let before = self.queue.len();
                self.queue.retain(|s| s.tag() != Some(dtag));
                if self.queue.len() != before {
                    ev.push(Event::ActionDisrupted { actor: ctx.target });
                }
            }
        }
        match &ctx.source {
            AttackSource::Strike { .. } => {
                if degree.is_success() {
                    let specs = rules::strike_damage_specs(self, &ctx, crit);
                    let name = self.c(ctx.attacker).name.clone();
                    self.queue.push_front(Step::DamageRolls {
                        tag,
                        source: ctx.attacker,
                        shares: vec![DamageShare { target: ctx.target, numerator: 1, denominator: 1, degree: None }],
                        specs,
                        rolled: vec![],
                        critical: crit,
                        from_strike: true,
                        purpose: format!("{name}'s Strike"),
                    });
                }
            }
            AttackSource::Spell { spell_id, rank, .. } => {
                let mut steps = Vec::new();
                if degree.is_success() {
                    if let Some(spell) = self.spells.get(spell_id) {
                        if let Some(dmg) = &spell.damage {
                            steps.push(Step::DamageRolls {
                                tag,
                                source: ctx.attacker,
                                shares: vec![DamageShare { target: ctx.target, numerator: 1, denominator: 1, degree: None }],
                                specs: vec![DamageRollSpec {
                                    label: spell.name.clone(),
                                    dice: dmg.at_rank(spell.rank, *rank),
                                    damage_type: dmg.damage_type,
                                    precision: false,
                                    doubled: crit,
                                }],
                                rolled: vec![],
                                critical: crit,
                                from_strike: false,
                                purpose: spell.name.clone(),
                            });
                        }
                    }
                }
                steps.push(Step::SpellOutcomes {
                    tag,
                    caster: ctx.attacker,
                    spell_id: spell_id.clone(),
                    rank: *rank,
                    target: ctx.target,
                    degree,
                });
                self.push_front_all(steps);
            }
        }
    }

    fn after_skill_check(&mut self, tag: u32, actor: CombatantId, target: CombatantId, action: SkillAction, d: DegreeOfSuccess, ev: &mut Vec<Event>) {
        use DegreeOfSuccess::*;
        match action {
            SkillAction::Demoralize => {
                let value = match d {
                    CriticalSuccess => 2,
                    Success => 1,
                    _ => 0,
                };
                if value > 0 {
                    self.add_condition(target, Condition { kind: CK::Frightened, value, source: Some(actor), expires: None }, ev);
                }
                let expires = self.expiry_for(Duration::Rounds(100), actor, target);
                let id = self.alloc_effect_id();
                let name = format!("Immune to Demoralize ({})", self.c(actor).name);
                self.cm(target).effects.push(Effect {
                    id,
                    name,
                    source: Some(actor),
                    grants: vec![],
                    tags: vec![EffectTag::ImmuneTo(format!("demoralize:{}", actor.0))],
                    expires,
                });
            }
            SkillAction::Trip => match d {
                CriticalSuccess | Success => {
                    self.add_condition(target, Condition::new(CK::Prone, 0), ev);
                    if d == CriticalSuccess {
                        self.queue.push_front(Step::DamageRolls {
                            tag,
                            source: actor,
                            shares: vec![DamageShare { target, numerator: 1, denominator: 1, degree: None }],
                            specs: vec![DamageRollSpec {
                                label: "Trip".into(),
                                dice: DiceExpr::new(1, 6, 0),
                                damage_type: DamageType::Bludgeoning,
                                precision: false,
                                doubled: false,
                            }],
                            rolled: vec![],
                            critical: false,
                            from_strike: false,
                            purpose: "critical Trip".into(),
                        });
                    }
                }
                CriticalFailure => self.add_condition(actor, Condition::new(CK::Prone, 0), ev),
                Failure => {}
            },
            SkillAction::Disable => match d {
                CriticalSuccess | Success => {
                    self.cm(target).life = LifeState::Inactive;
                    ev.push(Event::HazardDisabled { hazard: target });
                }
                CriticalFailure => self.queue.push_front(Step::HazardTrigger { hazard: target, victim: actor }),
                Failure => {}
            },
        }
    }

    // -----------------------------------------------------------------------
    // Damage, healing, dying
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn deliver_damage(
        &mut self,
        source: CombatantId,
        target: CombatantId,
        parts: Vec<DamagePart>,
        critical: bool,
        _from_strike: bool,
        shield_blocked: bool,
        ev: &mut Vec<Event>,
    ) {
        let t = self.c(target);
        if t.life == LifeState::Dead {
            return;
        }
        let result = apply_iwr(&parts, &t.statblock.iwr);
        let mut total = result.total;
        if shield_blocked {
            let hardness = t.statblock.shield.as_ref().map(|s| s.hardness).unwrap_or(0);
            let prevented = hardness.min(total);
            total -= prevented;
            let tm = self.cm(target);
            tm.shield_hp = (tm.shield_hp - total).max(0);
            let broken = tm.shield_broken();
            if broken {
                tm.effects.retain(|e| !e.tags.contains(&EffectTag::ShieldRaised));
            }
            ev.push(Event::ShieldBlocked { target, prevented, shield_hp: tm.shield_hp, broken });
        }
        let _ = source;
        self.take_damage(target, total, critical, result.lines, ev);
    }

    pub(crate) fn take_damage(&mut self, target: CombatantId, amount: i32, critical: bool, lines: Vec<crate::damage::DamageLine>, ev: &mut Vec<Event>) {
        let c = self.cm(target);
        let mut remaining = amount.max(0);
        let absorbed = c.temp_hp.min(remaining);
        c.temp_hp -= absorbed;
        remaining -= absorbed;
        let was_up = c.hp > 0;
        c.hp = (c.hp - remaining).max(0);
        let max = c.effective_max_hp();
        let hp = c.hp;
        let temp = c.temp_hp;
        ev.push(Event::DamageTaken { target, amount, lines, hp, temp_hp: temp });
        if amount >= 2 * max && amount > 0 {
            self.kill(target, ev);
            return;
        }
        if hp > 0 || remaining == 0 {
            return;
        }
        let c = self.cm(target);
        if !c.statblock.uses_dying_rules {
            self.kill(target, ev);
            return;
        }
        let step = if critical { 2 } else { 1 };
        let dying = c.conditions.value(CK::Dying);
        let new = if dying > 0 {
            dying + step
        } else {
            step + c.conditions.value(CK::Wounded)
        };
        c.conditions.set_value(CK::Dying, new);
        c.conditions.add(Condition::new(CK::Unconscious, 0));
        c.effects.retain(|e| !e.tags.contains(&EffectTag::ShieldRaised));
        if was_up {
            ev.push(Event::KnockedOut { combatant: target, dying: new });
        } else {
            ev.push(Event::ConditionGained { target, condition: CK::Dying, value: new });
        }
        self.check_death(target, ev);
    }

    fn check_death(&mut self, target: CombatantId, ev: &mut Vec<Event>) {
        let c = self.c(target);
        let threshold = 4 - c.conditions.value(CK::Doomed) as i32;
        if c.conditions.value(CK::Dying) as i32 >= threshold {
            self.kill(target, ev);
        }
    }

    fn kill(&mut self, target: CombatantId, ev: &mut Vec<Event>) {
        let c = self.cm(target);
        if c.life == LifeState::Dead {
            return;
        }
        c.life = LifeState::Dead;
        c.hp = 0;
        c.persistent.clear();
        c.effects.clear();
        ev.push(Event::Died { combatant: target });
        // Pending steps that only concern the dead creature's own turn/actions stay;
        // steps targeting it simply become no-ops.
    }

    pub(crate) fn heal(&mut self, target: CombatantId, amount: i32, ev: &mut Vec<Event>) {
        let c = self.cm(target);
        if c.life != LifeState::Active || amount <= 0 {
            return;
        }
        let max = c.effective_max_hp();
        let was_zero = c.hp == 0;
        c.hp = (c.hp + amount).min(max);
        let hp = c.hp;
        ev.push(Event::Healed { target, amount, hp });
        if was_zero && hp > 0 {
            if c.conditions.value(CK::Dying) > 0 {
                c.conditions.remove(CK::Dying);
                let w = c.conditions.value(CK::Wounded);
                c.conditions.set_value(CK::Wounded, w + 1);
                ev.push(Event::ConditionLost { target, condition: CK::Dying });
            }
            if c.conditions.remove(CK::Unconscious) {
                ev.push(Event::ConditionLost { target, condition: CK::Unconscious });
            }
        }
    }

    pub(crate) fn add_condition(&mut self, target: CombatantId, cond: Condition, ev: &mut Vec<Event>) {
        let kind = cond.kind;
        let value = cond.value;
        let c = self.cm(target);
        if c.life != LifeState::Active {
            return;
        }
        if c.conditions.add(cond) {
            ev.push(Event::ConditionGained { target, condition: kind, value });
        }
    }

    // -----------------------------------------------------------------------
    // Spells
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn resolve_spell(&mut self, tag: u32, caster: CombatantId, entry: usize, spell_id: &str, rank: u8, targets: Vec<CombatantId>, ev: &mut Vec<Event>) {
        let Some(spell) = self.spells.get(spell_id).cloned() else { return };
        if !self.c(caster).can_act() {
            ev.push(Event::Note { text: format!("{} fizzles", spell.name) });
            return;
        }
        let targets: Vec<_> = targets.into_iter().filter(|t| self.c(*t).life == LifeState::Active).collect();
        let mut steps = Vec::new();
        match spell.defense {
            SpellDefense::Attack => {
                let c = self.c(caster);
                let map = rules::map_penalty(c.attacks_this_turn, false);
                let number = c.attacks_this_turn;
                for &t in &targets {
                    let cover = rules::cover_between(self, caster, t, true).cover;
                    let ctx = AttackCtx {
                        attacker: caster,
                        target: t,
                        source: AttackSource::Spell { spell_id: spell_id.to_string(), rank, entry },
                        map_penalty: map,
                        attack_number: number,
                        cover,
                        disrupts: None,
                        sweep: false,
                    };
                    steps.extend(self.attack_steps(tag, ctx));
                }
                self.cm(caster).attacks_this_turn += 1;
            }
            SpellDefense::Save { save, .. } => {
                if !targets.is_empty() {
                    let dc = rules::spell_dc(self.c(caster), entry);
                    steps.push(Step::SaveSequence {
                        tag,
                        source: SaveSource::Spell { caster, spell_id: spell_id.to_string(), rank, dc },
                        save,
                        pending: targets.clone(),
                        results: vec![],
                    });
                }
            }
            SpellDefense::None => {
                if let Some(dmg) = &spell.damage {
                    steps.push(Step::DamageRolls {
                        tag,
                        source: caster,
                        shares: targets.iter().map(|&t| DamageShare { target: t, numerator: 1, denominator: 1, degree: None }).collect(),
                        specs: vec![DamageRollSpec {
                            label: spell.name.clone(),
                            dice: dmg.at_rank(spell.rank, rank),
                            damage_type: dmg.damage_type,
                            precision: false,
                            doubled: false,
                        }],
                        rolled: vec![],
                        critical: false,
                        from_strike: false,
                        purpose: spell.name.clone(),
                    });
                }
                if let Some(h) = &spell.healing {
                    steps.push(Step::HealRoll { tag, caster, spell_id: spell_id.to_string(), dice: h.at_rank(spell.rank, rank), targets: targets.clone() });
                }
                for &t in &targets {
                    steps.push(Step::SpellOutcomes { tag, caster, spell_id: spell_id.to_string(), rank, target: t, degree: DegreeOfSuccess::Success });
                }
            }
        }
        self.push_front_all(steps);
    }

    fn after_saves(&mut self, tag: u32, source: SaveSource, results: Vec<(CombatantId, DegreeOfSuccess)>, _ev: &mut Vec<Event>) {
        let mut steps = Vec::new();
        match &source {
            SaveSource::Spell { caster, spell_id, rank, .. } => {
                let Some(spell) = self.spells.get(spell_id).cloned() else { return };
                let basic = matches!(spell.defense, SpellDefense::Save { basic: true, .. });
                if let Some(dmg) = &spell.damage {
                    let shares: Vec<DamageShare> = results
                        .iter()
                        .filter_map(|&(t, d)| {
                            let (n, den) = damage_multiplier(spell.outcomes.for_degree(d), basic, d);
                            (n > 0).then_some(DamageShare { target: t, numerator: n, denominator: den, degree: Some(d) })
                        })
                        .collect();
                    if !shares.is_empty() {
                        steps.push(Step::DamageRolls {
                            tag,
                            source: *caster,
                            shares,
                            specs: vec![DamageRollSpec {
                                label: spell.name.clone(),
                                dice: dmg.at_rank(spell.rank, *rank),
                                damage_type: dmg.damage_type,
                                precision: false,
                                doubled: false,
                            }],
                            rolled: vec![],
                            critical: false,
                            from_strike: false,
                            purpose: spell.name.clone(),
                        });
                    }
                }
                for &(t, d) in &results {
                    steps.push(Step::SpellOutcomes { tag, caster: *caster, spell_id: spell_id.clone(), rank: *rank, target: t, degree: d });
                }
            }
            SaveSource::Hazard { hazard, .. } => {
                let effect = self.c(*hazard).statblock.hazard.as_ref().and_then(|h| h.trigger_effect.clone());
                if let Some(HazardEffect::Save { damage, basic, .. }) = effect {
                    let shares: Vec<DamageShare> = results
                        .iter()
                        .filter_map(|&(t, d)| {
                            let (n, den) = damage_multiplier(&[], basic, d);
                            (n > 0).then_some(DamageShare { target: t, numerator: n, denominator: den, degree: Some(d) })
                        })
                        .collect();
                    if !shares.is_empty() {
                        steps.push(Step::DamageRolls {
                            tag,
                            source: *hazard,
                            shares,
                            specs: vec![DamageRollSpec {
                                label: self.c(*hazard).name.clone(),
                                dice: damage.dice.clone(),
                                damage_type: damage.damage_type,
                                precision: false,
                                doubled: false,
                            }],
                            rolled: vec![],
                            critical: false,
                            from_strike: false,
                            purpose: "hazard".into(),
                        });
                    }
                }
            }
        }
        self.push_front_all(steps);
    }

    fn apply_spell_outcomes(&mut self, caster: CombatantId, spell_id: &str, rank: u8, target: CombatantId, degree: DegreeOfSuccess, ev: &mut Vec<Event>) {
        let Some(spell) = self.spells.get(spell_id).cloned() else { return };
        let _ = rank;
        for o in spell.outcomes.for_degree(degree) {
            match o {
                SpellOutcome::DamageMultiplier { .. } => {}
                SpellOutcome::ApplyCondition { condition, value, duration } => {
                    let expires = self.expiry_for(*duration, caster, target);
                    self.add_condition(target, Condition { kind: *condition, value: *value, source: Some(caster), expires }, ev);
                }
                SpellOutcome::Persistent { dice, damage_type } => {
                    let c = self.cm(target);
                    if c.life != LifeState::Active {
                        continue;
                    }
                    let new = PersistentDamage { dice: dice.clone(), damage_type: *damage_type, source: Some(caster) };
                    match c.persistent.iter_mut().find(|p| p.damage_type == *damage_type) {
                        Some(existing) if existing.dice.average() >= new.dice.average() => {}
                        Some(existing) => *existing = new,
                        None => c.persistent.push(new),
                    }
                    ev.push(Event::Note { text: format!("{} takes {dice} persistent {}", c.name, damage_type.label()) });
                }
                SpellOutcome::GrantEffect { name, grants, duration } => {
                    let expires = self.expiry_for(*duration, caster, target);
                    let id = self.alloc_effect_id();
                    let c = self.cm(target);
                    if c.life != LifeState::Active {
                        continue;
                    }
                    c.effects.retain(|e| e.name != *name);
                    c.effects.push(Effect { id, name: name.clone(), source: Some(caster), grants: grants.clone(), tags: vec![], expires });
                    ev.push(Event::EffectGained { target, name: name.clone() });
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Movement & hazards
    // -----------------------------------------------------------------------

    fn continue_move(&mut self, tag: u32, mover: CombatantId, kind: MoveKind, mut remaining: Vec<crate::grid::Pos>, mut offered: Vec<CombatantId>, ev: &mut Vec<Event>) {
        while !remaining.is_empty() {
            if !self.c(mover).can_act() || self.c(mover).conditions.has(CK::Immobilized) {
                return;
            }
            if kind == MoveKind::Stride {
                let offers = self.reactive_strike_offers(mover, None, "moving out of a square within reach", &offered);
                if !offers.is_empty() {
                    let mut steps = Vec::new();
                    for (who, step) in offers {
                        offered.push(who);
                        steps.push(step);
                    }
                    steps.push(Step::ContinueMove { tag, mover, kind, remaining, offered });
                    self.push_front_all(steps);
                    return;
                }
            }
            let next = remaining.remove(0);
            let from = self.c(mover).position;
            self.cm(mover).position = next;
            ev.push(Event::Moved { combatant: mover, from, to: next });
            let triggered: Vec<CombatantId> = self
                .combatants
                .iter()
                .filter(|h| {
                    h.life == LifeState::Active
                        && h.statblock
                            .hazard
                            .as_ref()
                            .map(|hz| hz.trigger_squares(h.position).iter().any(|sq| self.c(mover).footprint().contains(sq)))
                            .unwrap_or(false)
                })
                .map(|h| h.id)
                .collect();
            if !triggered.is_empty() {
                let steps = triggered.into_iter().map(|h| Step::HazardTrigger { hazard: h, victim: mover }).collect();
                self.push_front_all(steps);
                return; // triggering a hazard ends the movement
            }
        }
    }

    fn trigger_hazard(&mut self, hazard: CombatantId, victim: CombatantId, ev: &mut Vec<Event>) {
        let h = self.c(hazard);
        if h.life != LifeState::Active {
            return;
        }
        let Some(profile) = h.statblock.hazard.clone() else { return };
        ev.push(Event::HazardTriggered { hazard, victim });
        let tag = self.alloc_tag();
        match profile.trigger_effect {
            Some(HazardEffect::Save { save, dc, .. }) => {
                self.queue.push_front(Step::SaveSequence {
                    tag,
                    source: SaveSource::Hazard { hazard, dc },
                    save,
                    pending: vec![victim],
                    results: vec![],
                });
            }
            Some(HazardEffect::Strike { strike_index }) => {
                let ctx = AttackCtx {
                    attacker: hazard,
                    target: victim,
                    source: AttackSource::Strike { index: strike_index },
                    map_penalty: 0,
                    attack_number: 0,
                    cover: CoverLevel::None,
                    disrupts: None,
                    sweep: false,
                };
                self.queue.push_front(Step::AttackRoll { tag, ctx });
            }
            None => {}
        }
        if !profile.resets {
            self.cm(hazard).life = LifeState::Inactive;
        }
    }

    // -----------------------------------------------------------------------
    // GM overrides
    // -----------------------------------------------------------------------

    fn apply_gm(&mut self, op: &GmOp, ev: &mut Vec<Event>) -> CombatResult<()> {
        let check = |s: &Self, id: CombatantId| s.combatant(id).map(|_| ()).ok_or(CombatError::UnknownCombatant(id));
        match op {
            GmOp::AdjustHp { target, delta } => {
                check(self, *target)?;
                if *delta >= 0 {
                    self.heal(*target, *delta, ev);
                } else {
                    self.take_damage(*target, -delta, false, vec![], ev);
                }
            }
            GmOp::SetTempHp { target, value } => {
                check(self, *target)?;
                self.cm(*target).temp_hp = (*value).max(0);
            }
            GmOp::AddCondition { target, condition } => {
                check(self, *target)?;
                self.add_condition(*target, condition.clone(), ev);
            }
            GmOp::RemoveCondition { target, condition } => {
                check(self, *target)?;
                if self.cm(*target).conditions.remove(*condition) {
                    ev.push(Event::ConditionLost { target: *target, condition: *condition });
                }
            }
            GmOp::Move { target, to } => {
                check(self, *target)?;
                if !self.battlefield.in_bounds(*to) {
                    return Err(CombatError::IllegalAction("destination is off the map".into()));
                }
                let from = self.c(*target).position;
                self.cm(*target).position = *to;
                ev.push(Event::Moved { combatant: *target, from, to: *to });
            }
            GmOp::ClearPending => {
                self.queue.retain(|s| s.tag().is_none());
                ev.push(Event::Note { text: "GM cleared the pending resolution".into() });
            }
        }
        Ok(())
    }
}

/// Damage multiplier for a save result: explicit outcome text wins, then the
/// basic-save table, otherwise full damage only on a failure.
pub(crate) fn damage_multiplier(outcomes: &[SpellOutcome], basic: bool, d: DegreeOfSuccess) -> (u8, u8) {
    if let Some((n, den)) = outcomes.iter().find_map(|o| match o {
        SpellOutcome::DamageMultiplier { numerator, denominator } => Some((*numerator, *denominator)),
        _ => None,
    }) {
        return (n, den);
    }
    use DegreeOfSuccess::*;
    match (basic, d) {
        (_, CriticalSuccess) => (0, 1),
        (true, Success) => (1, 2),
        (false, Success) => (0, 1),
        (_, Failure) => (1, 1),
        (true, CriticalFailure) => (2, 1),
        (false, CriticalFailure) => (1, 1),
    }
}
