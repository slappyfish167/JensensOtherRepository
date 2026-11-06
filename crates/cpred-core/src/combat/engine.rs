//! Command application and the resolution-queue processor.
//!
//! Invariants:
//! * `apply` is atomic: on error the state is unchanged.
//! * After `apply` returns, the queue is either empty or its front step needs
//!   human input (`Step::needs_input`). `pending()` describes that input.
//! * No randomness exists anywhere in this module.

use super::command::{Command, GmOp};
use super::event::Event;
use super::pending::{AttackCtx, CheckRole, CheckRoll, Step};
use super::state::{CombatState, CriticalInjury, InitiativeEntry, Phase, TurnEconomy};
use crate::model::{HitLocation, Stat, Terrain};
use crate::rules::{self, AttackMode};
use tactics_core::{CombatantId, DiceExpr, Pos, Side};

/// Why a command was rejected. The state is unchanged when `apply` returns one.
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
    #[error("request {got} does not match pending request {expected} (stale answer?)")]
    WrongRequest { expected: u32, got: u32 },
    #[error("the pending input is a {0}, not this kind of answer")]
    WrongInputKind(String),
    #[error("invalid roll: {0}")]
    InvalidRoll(String),
    #[error("illegal action: {0}")]
    IllegalAction(String),
    #[error("unknown combatant {0}")]
    UnknownCombatant(CombatantId),
    #[error("invalid GM command: {0}")]
    InvalidGmOp(String),
    #[error("internal error: {0}")]
    Internal(String),
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
            Command::StartCombat => self.start_combat(&mut ev)?,
            Command::ProvideRoll { request_id, value, faces, extra_modifier } => {
                let step = self.take_input(*request_id)?;
                if matches!(step, Step::DodgeDecision { .. }) {
                    return Err(CombatError::WrongInputKind("decision".into()));
                }
                validate_roll(&step, *value, faces)?;
                self.consume_input();
                self.resolve_roll(step, *value, faces, *extra_modifier, &mut ev)?;
            }
            Command::Decide { request_id, accept } => {
                let step = self.take_input(*request_id)?;
                let Step::DodgeDecision { ctx } = step else {
                    return Err(CombatError::WrongInputKind("roll".into()));
                };
                self.consume_input();
                self.resolve_dodge_decision(ctx, *accept, &mut ev);
            }
            Command::Act { actor, action } => {
                self.check_action(*actor, action)?;
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

    fn start_combat(&mut self, ev: &mut Vec<Event>) -> CombatResult<()> {
        if self.phase != Phase::Setup {
            return Err(CombatError::AlreadyStarted);
        }
        self.phase = Phase::RollingInitiative;
        ev.push(Event::CombatStarted);
        let ids: Vec<_> = self.combatants.iter().filter(|c| c.alive).map(|c| c.id).collect();
        for id in ids {
            self.queue.push_back(Step::RollInitiative { combatant: id });
        }
        self.queue.push_back(Step::FinalizeInitiative);
        Ok(())
    }

    /// Validates the request id and returns (a copy of) the waiting step.
    fn take_input(&self, request_id: u32) -> CombatResult<Step> {
        if self.is_over() {
            return Err(CombatError::CombatOver);
        }
        match self.queue.front() {
            Some(s) if s.needs_input() => {
                if request_id != self.input_seq {
                    return Err(CombatError::WrongRequest { expected: self.input_seq, got: request_id });
                }
                Ok(s.clone())
            }
            _ => Err(CombatError::NoPendingInput),
        }
    }

    fn consume_input(&mut self) {
        self.queue.pop_front();
        self.input_seq += 1;
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
                None => {
                    // A combatant that died during its own turn ends it.
                    match self.current_actor() {
                        Some(id) if self.c(id).is_dead() => self.queue.push_back(Step::EndTurn { combatant: id }),
                        _ => return Ok(()),
                    }
                }
                Some(s) if s.needs_input() => return Ok(()),
                Some(_) => {
                    if let Some(step) = self.queue.pop_front() {
                        self.process_auto(step, ev)?;
                    }
                }
            }
        }
        Err(CombatError::Internal("resolution loop did not terminate".into()))
    }

    fn check_victory(&mut self, ev: &mut Vec<Event>) -> bool {
        if self.is_over() {
            return true;
        }
        if self.phase != Phase::Active {
            return false;
        }
        let lost = |side: Side| {
            let mut members = self.combatants.iter().filter(|c| c.side == side).peekable();
            members.peek().is_some() && members.all(|c| c.is_down())
        };
        let winner = match (lost(Side::Party), lost(Side::Enemy)) {
            (true, true) => None,
            (true, false) => Some(Side::Enemy),
            (false, true) => Some(Side::Party),
            (false, false) => {
                if self.combatants.iter().any(|c| c.alive) {
                    return false;
                }
                None
            }
        };
        self.phase = Phase::Ended { winner };
        self.queue.clear();
        ev.push(Event::CombatEnded { winner });
        true
    }

    fn process_auto(&mut self, step: Step, ev: &mut Vec<Event>) -> CombatResult<()> {
        match step {
            Step::FinalizeInitiative => self.finalize_initiative(ev),
            Step::BeginTurn => self.begin_turn(ev),
            Step::EndTurn { combatant } => {
                if let Some(c) = self.combatant_mut(combatant) {
                    c.economy = TurnEconomy::default();
                }
                ev.push(Event::TurnEnded { combatant });
                self.queue.push_back(Step::AdvanceTurn);
            }
            Step::AdvanceTurn => self.advance_turn(ev),
            Step::BeginAttack { ctx } => self.begin_attack(ctx),
            other => return Err(CombatError::Internal(format!("step {other:?} cannot run automatically"))),
        }
        Ok(())
    }

    fn finalize_initiative(&mut self, ev: &mut Vec<Event>) {
        let eff_ref = |s: &CombatState, id: CombatantId| s.combatant(id).map(|c| c.effective_stat(Stat::Ref)).unwrap_or(0);
        let mut order = self.initiative.clone();
        order.sort_by(|a, b| {
            b.value
                .cmp(&a.value)
                .then_with(|| eff_ref(self, b.combatant).cmp(&eff_ref(self, a.combatant)))
                .then_with(|| a.combatant.cmp(&b.combatant))
        });
        self.initiative = order;
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

    fn advance_turn(&mut self, ev: &mut Vec<Event>) {
        let n = self.initiative.len();
        if n == 0 {
            return;
        }
        for _ in 0..n {
            self.turn += 1;
            if self.turn >= n {
                ev.push(Event::RoundEnded { round: self.round });
                self.turn = 0;
                self.round += 1;
                ev.push(Event::RoundStarted { round: self.round });
            }
            let id = self.initiative[self.turn].combatant;
            if self.combatant(id).is_some_and(|c| c.alive) {
                break;
            }
        }
        self.queue.push_back(Step::BeginTurn);
    }

    /// Start of turn: fresh economy, death save if mortally wounded.
    pub(crate) fn begin_turn(&mut self, ev: &mut Vec<Event>) {
        let Some(id) = self.initiative.get(self.turn).map(|e| e.combatant) else { return };
        let c = self.cm(id);
        if !c.alive {
            return;
        }
        c.economy = TurnEconomy::fresh();
        let mortal = c.is_mortally_wounded();
        ev.push(Event::TurnStarted { combatant: id });
        if mortal {
            self.queue.push_back(Step::DeathSave { combatant: id });
        }
    }

    /// A copy of this state in which `id` is starting a fresh turn right now
    /// (full economy). `None` if the combat is not active, `id` is unknown or
    /// dead, or starting the turn would need human input (a death save).
    pub fn hypothetical_turn(&self, id: CombatantId) -> Option<CombatState> {
        if self.phase != Phase::Active || !self.combatant(id)?.alive {
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

    // -----------------------------------------------------------------------
    // Attack resolution
    // -----------------------------------------------------------------------

    fn begin_attack(&mut self, ctx: AttackCtx) {
        let melee = self.c(ctx.attacker).statblock.weapons.get(ctx.weapon).is_some_and(|w| w.melee);
        let step = if melee {
            Step::Check { roll: self.defense_check(&ctx, false) }
        } else if self.c(ctx.target).can_dodge(&self.rules) {
            Step::DodgeDecision { ctx }
        } else {
            let dv = ctx.dv.unwrap_or(0);
            Step::Check { roll: self.attack_check(&ctx, dv, "DV") }
        };
        self.queue.push_front(step);
    }

    fn resolve_dodge_decision(&mut self, ctx: AttackCtx, accept: bool, ev: &mut Vec<Event>) {
        let roll = if accept {
            ev.push(Event::DodgeDeclared { combatant: ctx.target, attacker: ctx.attacker });
            self.defense_check(&ctx, true)
        } else {
            self.attack_check(&ctx, ctx.dv.unwrap_or(0), "DV")
        };
        self.queue.push_front(Step::Check { roll });
    }

    fn resolve_roll(&mut self, step: Step, value: i32, faces: &[i32], extra: i32, ev: &mut Vec<Event>) -> CombatResult<()> {
        let nat = value.clamp(1, 10) as u8;
        match step {
            Step::RollInitiative { combatant } => {
                let (modifier, _) = self.initiative_modifier(combatant);
                let total = value + modifier + extra;
                self.initiative.push(InitiativeEntry { combatant, value: total, natural: nat });
                ev.push(Event::CheckResolved {
                    roller: Some(combatant),
                    purpose: "initiative".into(),
                    natural: nat,
                    total,
                    beat: None,
                    success: None,
                });
            }
            Step::DeathSave { combatant } => self.resolve_death_save(combatant, value, ev),
            Step::Check { roll } => {
                let subtotal = value + roll.modifier + extra;
                match value {
                    10 => self.queue.push_front(Step::CritFollowUp { roll, natural: nat, subtotal, sign: 1 }),
                    1 => self.queue.push_front(Step::CritFollowUp { roll, natural: nat, subtotal, sign: -1 }),
                    _ => self.finish_check(roll, nat, subtotal, ev),
                }
            }
            Step::CritFollowUp { roll, natural, subtotal, sign } => {
                let total = subtotal + sign * value + extra;
                self.finish_check(roll, natural, total, ev);
            }
            Step::DamageRoll { ctx, dice, multiplier, purpose } => {
                self.resolve_damage(&ctx, &dice, multiplier, faces, extra, purpose, ev);
            }
            other => return Err(CombatError::Internal(format!("step {other:?} does not take a roll"))),
        }
        Ok(())
    }

    fn finish_check(&mut self, roll: CheckRoll, natural: u8, total: i32, ev: &mut Vec<Event>) {
        let success = roll.beat.map(|b| rules::beats(total, b));
        ev.push(Event::CheckResolved {
            roller: Some(roll.roller),
            purpose: roll.purpose.clone(),
            natural,
            total,
            beat: roll.beat,
            success,
        });
        match roll.role {
            CheckRole::Defense { ctx, dodge } => {
                let label = if dodge { "dodge" } else { "defence" };
                let next = self.attack_check(&ctx, total, label);
                self.queue.push_front(Step::Check { roll: next });
            }
            CheckRole::Attack { ctx } => {
                if success != Some(true) {
                    ev.push(Event::Missed { attacker: ctx.attacker, target: ctx.target });
                    return;
                }
                self.queue.push_front(self.damage_step(ctx, total - roll.beat.unwrap_or(0)));
            }
        }
    }

    /// The damage roll after a hit by `margin`.
    fn damage_step(&self, ctx: AttackCtx, margin: i32) -> Step {
        let weapon = self.c(ctx.attacker).statblock.weapons.get(ctx.weapon).cloned();
        let name = weapon.as_ref().map(|w| w.name.clone()).unwrap_or_default();
        let who = format!("{} → {}", self.name_of(ctx.attacker), self.name_of(ctx.target));
        if ctx.mode == AttackMode::Autofire {
            let max = weapon.and_then(|w| w.autofire).unwrap_or(1);
            let m = rules::autofire_multiplier(margin, 0, max).max(1);
            Step::DamageRoll { dice: self.rules.autofire_damage.clone(), multiplier: Some(m), purpose: format!("{name} autofire damage ×{m} ({who})"), ctx }
        } else {
            let dice = weapon.map(|w| w.damage).unwrap_or_default();
            Step::DamageRoll { dice, multiplier: None, purpose: format!("{name} damage ({who})"), ctx }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_damage(&mut self, ctx: &AttackCtx, dice: &DiceExpr, multiplier: Option<i32>, faces: &[i32], extra: i32, purpose: String, ev: &mut Vec<Event>) {
        let base: i32 = faces.iter().sum::<i32>() + dice.modifier;
        let damage = (base * multiplier.unwrap_or(1) + extra).max(0);
        let crit = rules::is_critical_injury(dice, faces);
        ev.push(Event::DamageRolled { purpose, faces: faces.to_vec(), total: damage });
        if let Some(pos) = ctx.cover.filter(|p| self.cover_at(*p).is_some()) {
            self.damage_cover(pos, damage, ev);
            return;
        }
        self.damage_target(ctx, damage, crit, ev);
    }

    fn damage_cover(&mut self, pos: Pos, damage: i32, ev: &mut Vec<Event>) {
        let Some(obj) = self.cover.iter_mut().find(|c| c.pos == pos) else { return };
        obj.hp = (obj.hp - damage).max(0);
        let hp = obj.hp;
        ev.push(Event::CoverHit { pos, damage, hp });
        if hp == 0 {
            self.cover.retain(|c| c.pos != pos);
            self.battlefield.set_terrain(pos, Terrain::Open);
            ev.push(Event::CoverDestroyed { pos });
        }
    }

    fn damage_target(&mut self, ctx: &AttackCtx, damage: i32, crit: bool, ev: &mut Vec<Event>) {
        let target = ctx.target;
        let location = ctx.location;
        let halves = self.c(ctx.attacker).statblock.weapons.get(ctx.weapon).is_some_and(|w| w.halves_armor());
        let round = self.round;
        let crit_damage = self.rules.critical_injury_damage;
        let t = self.cm(target);
        if !t.alive {
            return;
        }
        ev.push(Event::HitLocation { target, location });
        let was_mortal = t.is_mortally_wounded();
        let sp_before = t.armor_sp.get(location);
        let sp = rules::effective_sp(sp_before, halves);
        let (loss, penetrated) = rules::armor_result(damage, sp, location);
        if penetrated {
            let sp_after = (sp_before - 1).max(0);
            t.armor_sp.set(location, sp_after);
            ev.push(Event::ArmorPenetrated { target, location, damage, sp_before, sp_after });
        } else {
            ev.push(Event::ArmorStopped { target, location, damage, sp });
        }
        let mut hurt = false;
        if loss > 0 {
            self.lose_hp(target, loss, ev);
            hurt = true;
        }
        if crit {
            self.add_critical_injury(target, location, round, ev);
            if crit_damage > 0 {
                self.lose_hp(target, crit_damage, ev);
                hurt = true;
            }
        }
        if was_mortal && hurt && self.options.death_save_on_damage && self.c(target).alive {
            self.queue.push_front(Step::DeathSave { combatant: target });
        }
    }

    fn add_critical_injury(&mut self, target: CombatantId, location: HitLocation, round: u32, ev: &mut Vec<Event>) {
        let t = self.cm(target);
        t.injuries.push(CriticalInjury { location, round });
        t.death_save_penalty = t.death_save_penalty.saturating_add(1);
        let note = format!(
            "{} suffers a Critical Injury to the {}: GM, resolve it on your critical injury table (death save penalty now {})",
            t.name,
            location.label(),
            t.death_save_penalty
        );
        ev.push(Event::CriticalInjury { target, location, note });
    }

    /// Removes HP (clamped at 0) and reports becoming mortally wounded.
    pub(crate) fn lose_hp(&mut self, target: CombatantId, amount: i32, ev: &mut Vec<Event>) {
        let t = self.cm(target);
        let was_up = t.hp > 0;
        t.hp = (t.hp - amount).max(0);
        let hp = t.hp;
        ev.push(Event::HpLost { target, amount, hp });
        if was_up && hp == 0 {
            ev.push(Event::MortallyWounded { combatant: target });
        }
    }

    fn resolve_death_save(&mut self, combatant: CombatantId, face: i32, ev: &mut Vec<Event>) {
        let threshold = self.death_save_threshold(combatant);
        let inclusive = self.options.death_save_inclusive;
        let c = self.cm(combatant);
        let success = rules::death_save_succeeds(face, c.statblock.stats.body, c.death_save_penalty, inclusive);
        ev.push(Event::DeathSave { combatant, face: face.clamp(1, 10) as u8, threshold, success });
        if success {
            c.death_save_penalty = c.death_save_penalty.saturating_add(1);
        } else {
            self.kill(combatant, ev);
        }
    }

    pub(crate) fn kill(&mut self, target: CombatantId, ev: &mut Vec<Event>) {
        let c = self.cm(target);
        if !c.alive {
            return;
        }
        c.alive = false;
        c.hp = 0;
        c.economy = TurnEconomy::default();
        ev.push(Event::Died { combatant: target });
    }

    // -----------------------------------------------------------------------
    // GM operations
    // -----------------------------------------------------------------------

    fn apply_gm(&mut self, op: &GmOp, ev: &mut Vec<Event>) -> CombatResult<()> {
        let gm_err = |m: String| Err(CombatError::InvalidGmOp(m));
        match op {
            GmOp::AdjustHp { target, delta } => {
                let c = self.combatant_mut(*target).ok_or(CombatError::UnknownCombatant(*target))?;
                if !c.alive {
                    return gm_err(format!("{} is dead", c.name));
                }
                let max = c.max_hp();
                let was_up = c.hp > 0;
                c.hp = (c.hp + delta).clamp(0, max);
                let (hp, name) = (c.hp, c.name.clone());
                ev.push(Event::Note { text: format!("GM: {name}'s HP changed by {delta} (now {hp})") });
                if was_up && hp == 0 {
                    ev.push(Event::MortallyWounded { combatant: *target });
                }
            }
            GmOp::SetSp { target, location, value } => {
                let c = self.combatant_mut(*target).ok_or(CombatError::UnknownCombatant(*target))?;
                c.armor_sp.set(*location, (*value).max(0));
                ev.push(Event::Note { text: format!("GM: {}'s {} SP set to {}", c.name, location.label(), (*value).max(0)) });
            }
            GmOp::SetAmmo { target, weapon, value } => {
                let c = self.combatant_mut(*target).ok_or(CombatError::UnknownCombatant(*target))?;
                let Some(w) = c.statblock.weapons.get(*weapon) else {
                    return gm_err(format!("{} has no weapon #{weapon}", c.name));
                };
                if !w.uses_ammo() {
                    return gm_err(format!("{} does not use ammunition", w.name));
                }
                let v = (*value).min(w.magazine);
                let text = format!("GM: {}'s {} loaded with {v}", c.name, w.name);
                c.ammo[*weapon] = v;
                ev.push(Event::Note { text });
            }
            GmOp::Move { target, to } => {
                let c = self.combatant(*target).ok_or(CombatError::UnknownCombatant(*target))?;
                if !self.battlefield.terrain(*to).standable() {
                    return gm_err(format!("({}, {}) is not a free square", to.x, to.y));
                }
                if self.occupant(*to).is_some_and(|o| o.id != *target) {
                    return gm_err(format!("({}, {}) is occupied", to.x, to.y));
                }
                let from = c.position;
                self.cm(*target).position = *to;
                ev.push(Event::Moved { combatant: *target, from, to: *to, cost: 0 });
            }
            GmOp::AddCriticalInjury { target, location } => {
                if self.combatant(*target).is_none() {
                    return Err(CombatError::UnknownCombatant(*target));
                }
                let round = self.round;
                self.add_critical_injury(*target, *location, round, ev);
            }
            GmOp::ClearPending => {
                let before = self.queue.len();
                self.queue.retain(|s| s.is_turn_structure());
                if self.queue.len() != before {
                    self.input_seq += 1;
                }
                ev.push(Event::Note { text: format!("GM cleared {} pending step(s)", before - self.queue.len()) });
            }
        }
        Ok(())
    }
}

/// Checks that an answer fits the waiting step.
fn validate_roll(step: &Step, value: i32, faces: &[i32]) -> CombatResult<()> {
    match step {
        Step::DamageRoll { dice, .. } => {
            let expected = dice.dice_count() as usize;
            if faces.len() != expected {
                return Err(CombatError::InvalidRoll(format!("{dice} needs {expected} die faces, got {}", faces.len())));
            }
            let mut i = 0;
            for t in &dice.terms {
                for _ in 0..t.count {
                    let f = faces[i];
                    if f < 1 || f > t.sides as i32 {
                        return Err(CombatError::InvalidRoll(format!("face #{} is {f}, but a d{} shows 1–{}", i + 1, t.sides, t.sides)));
                    }
                    i += 1;
                }
            }
            Ok(())
        }
        _ => {
            if !(1..=10).contains(&value) {
                return Err(CombatError::InvalidRoll(format!("enter the natural d10 face (1–10), got {value}")));
            }
            Ok(())
        }
    }
}
