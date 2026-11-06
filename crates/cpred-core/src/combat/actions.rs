//! Movement, action legality, declaration, and legal-action generation.
//!
//! `check_action` is side-effect free and is the single source of truth for
//! legality: `declare_action` runs after it, and `legal_actions` (used by UIs
//! and the AI) is built from it.

use super::command::Action;
use super::engine::{CombatError, CombatResult};
use super::event::Event;
use super::pending::{AttackCtx, PendingInput, Step};
use super::state::{CombatState, Phase};
use crate::model::{Stat, Terrain};
use crate::rules::AttackMode;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use tactics_core::{CombatantId, Pos};

fn illegal<T>(msg: impl Into<String>) -> CombatResult<T> {
    Err(CombatError::IllegalAction(msg.into()))
}

/// Squares a combatant can end a move in, with their cost in squares.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MovementMap {
    /// Reachable destinations (start excluded), sorted by position.
    pub destinations: Vec<(Pos, u32)>,
}

impl MovementMap {
    /// Cost to reach `pos`, if reachable.
    pub fn cost_to(&self, pos: Pos) -> Option<u32> {
        self.destinations.binary_search_by(|(p, _)| p.cmp(&pos)).ok().map(|i| self.destinations[i].1)
    }
}

impl CombatState {
    /// Movement options for one Move Action (or Run) from the combatant's
    /// current square: Dijkstra over 8 neighbours, difficult terrain costs 2,
    /// walls/cover/standing enemies block, allies can be passed but not ended
    /// on, diagonal steps may not cut wall corners.
    pub fn movement(&self, id: CombatantId) -> MovementMap {
        let Some(c) = self.combatant(id) else { return MovementMap::default() };
        let budget = c.effective_stat(Stat::Move).max(0) as u32;
        let mut blocked = BTreeSet::new();
        let mut occupied = BTreeSet::new();
        for o in self.combatants.iter().filter(|o| o.id != id && o.alive) {
            occupied.insert(o.position);
            if c.side.is_hostile_to(o.side) && !o.is_down() {
                blocked.insert(o.position);
            }
        }
        let start = c.position;
        let mut dist: BTreeMap<Pos, u32> = BTreeMap::new();
        let mut heap = BinaryHeap::new();
        dist.insert(start, 0);
        heap.push(Reverse((0u32, start.y, start.x)));
        while let Some(Reverse((cost, y, x))) = heap.pop() {
            let pos = Pos::new(x, y);
            if dist.get(&pos).copied() != Some(cost) {
                continue;
            }
            for next in pos.neighbours() {
                if blocked.contains(&next) || self.cuts_corner(pos, next) {
                    continue;
                }
                let Some(step) = self.battlefield.terrain(next).enter_cost() else { continue };
                let nc = cost + step;
                if nc > budget || dist.get(&next).is_some_and(|&d| d <= nc) {
                    continue;
                }
                dist.insert(next, nc);
                heap.push(Reverse((nc, next.y, next.x)));
            }
        }
        let destinations = dist.into_iter().filter(|(p, _)| *p != start && !occupied.contains(p)).collect();
        MovementMap { destinations }
    }

    fn cuts_corner(&self, from: Pos, to: Pos) -> bool {
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        dx != 0
            && dy != 0
            && (self.battlefield.terrain(Pos::new(from.x + dx, from.y)) == Terrain::Wall
                || self.battlefield.terrain(Pos::new(from.x, from.y + dy)) == Terrain::Wall)
    }

    /// Errors unless `actor` may declare something right now.
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
                PendingInput::Decision(_) => "dodge decision".into(),
            }));
        }
        let current = self.current_actor().ok_or(CombatError::NotStarted)?;
        if current != actor {
            return Err(CombatError::NotYourTurn { expected: current, got: actor });
        }
        Ok(())
    }

    /// Validates an action for `actor` (whose turn it must be).
    pub fn check_action(&self, actor: CombatantId, action: &Action) -> CombatResult<()> {
        self.ensure_can_declare(actor)?;
        let a = self.c(actor);
        if !a.can_act() {
            return illegal(format!("{} cannot act", a.name));
        }
        let eco = a.economy;
        match action {
            Action::Move { to } => {
                if !eco.move_available {
                    return illegal("the Move Action is already used");
                }
                self.check_reach(actor, *to)
            }
            Action::Run { to } => {
                if !eco.action_available {
                    return illegal("the Action is already used");
                }
                self.check_reach(actor, *to)
            }
            Action::Attack { weapon, target } => {
                let continuing = !eco.action_available && eco.shots_left > 0 && eco.rof_weapon == Some(*weapon);
                if !eco.action_available && !continuing {
                    return illegal(if eco.shots_left > 0 {
                        "further attacks this Action must use the same weapon"
                    } else {
                        "the Action is already used"
                    });
                }
                self.check_attack(actor, *weapon, *target, AttackMode::Single)
            }
            Action::AimedShot { weapon, target } | Action::Autofire { weapon, target } => {
                if !eco.action_available {
                    return illegal("needs a whole Action");
                }
                let mode = if matches!(action, Action::AimedShot { .. }) { AttackMode::Aimed } else { AttackMode::Autofire };
                if mode == AttackMode::Aimed && a.statblock.weapons.get(*weapon).is_some_and(|w| w.melee) {
                    return illegal("aimed shots need a ranged weapon");
                }
                self.check_attack(actor, *weapon, *target, mode)
            }
            Action::Reload { weapon } => {
                if !eco.action_available {
                    return illegal("the Action is already used");
                }
                let Some(w) = a.statblock.weapons.get(*weapon) else {
                    return illegal(format!("{} has no weapon #{weapon}", a.name));
                };
                if !w.uses_ammo() {
                    return illegal(format!("{} does not use ammunition", w.name));
                }
                if a.ammo.get(*weapon).copied().unwrap_or(0) >= w.magazine {
                    return illegal(format!("{} is fully loaded", w.name));
                }
                Ok(())
            }
        }
    }

    fn check_reach(&self, actor: CombatantId, to: Pos) -> CombatResult<()> {
        if self.movement(actor).cost_to(to).is_none() {
            return illegal(format!("cannot reach ({}, {})", to.x, to.y));
        }
        Ok(())
    }

    fn check_attack(&self, actor: CombatantId, weapon: usize, target: CombatantId, mode: AttackMode) -> CombatResult<()> {
        let plan = self.plan_attack(actor, weapon, target, mode).map_err(CombatError::IllegalAction)?;
        if !plan.has_ammo() {
            return illegal(format!(
                "{} needs {} round(s), {} loaded",
                plan.weapon.name, plan.ammo_needed, plan.ammo_loaded
            ));
        }
        Ok(())
    }

    /// Mutates the state for a validated action and queues its resolution.
    pub(crate) fn declare_action(&mut self, actor: CombatantId, action: &Action, ev: &mut Vec<Event>) -> CombatResult<()> {
        ev.push(Event::ActionDeclared { actor, action: self.describe_action(actor, action) });
        match action {
            Action::Move { to } | Action::Run { to } => {
                let cost = self.movement(actor).cost_to(*to).unwrap_or(0);
                let c = self.cm(actor);
                let from = c.position;
                c.position = *to;
                if matches!(action, Action::Move { .. }) {
                    c.economy.move_available = false;
                } else {
                    c.economy.action_available = false;
                }
                ev.push(Event::Moved { combatant: actor, from, to: *to, cost });
            }
            Action::Attack { weapon, target } | Action::AimedShot { weapon, target } | Action::Autofire { weapon, target } => {
                let mode = action.attack().map(|(_, _, m)| m).unwrap_or(AttackMode::Single);
                let plan = self.plan_attack(actor, *weapon, *target, mode).map_err(CombatError::IllegalAction)?;
                let c = self.cm(actor);
                let eco = &mut c.economy;
                if eco.action_available {
                    eco.action_available = false;
                    let rof = plan.weapon.rof;
                    let more = if mode == AttackMode::Single { rof.saturating_sub(1) } else { 0 };
                    eco.shots_left = more;
                    eco.rof_weapon = (more > 0).then_some(*weapon);
                } else {
                    eco.shots_left = eco.shots_left.saturating_sub(1);
                    if eco.shots_left == 0 {
                        eco.rof_weapon = None;
                    }
                }
                if let Some(ammo) = c.ammo.get_mut(*weapon) {
                    *ammo = ammo.saturating_sub(plan.ammo_needed);
                }
                let ctx = AttackCtx {
                    attacker: actor,
                    target: *target,
                    weapon: *weapon,
                    mode,
                    location: plan.location,
                    dv: plan.dv,
                    cover: plan.cover,
                };
                self.queue.push_back(Step::BeginAttack { ctx });
            }
            Action::Reload { weapon } => {
                let c = self.cm(actor);
                let mag = c.statblock.weapons.get(*weapon).map(|w| w.magazine).unwrap_or(0);
                if let Some(ammo) = c.ammo.get_mut(*weapon) {
                    *ammo = mag;
                }
                c.economy.action_available = false;
                ev.push(Event::Reloaded { combatant: actor, weapon: *weapon, ammo: mag });
            }
        }
        Ok(())
    }

    /// Every legal action for the combatant whose turn it is (empty while
    /// input is pending). Attacks are only listed against hostile targets
    /// (`check_action` also accepts other living targets).
    pub fn legal_actions(&self) -> Vec<Action> {
        let Some(id) = self.current_actor() else { return vec![] };
        if self.ensure_can_declare(id).is_err() || !self.c(id).can_act() {
            return vec![];
        }
        let c = self.c(id);
        let eco = c.economy;
        let mut out = Vec::new();
        let moves = if eco.move_available || eco.action_available { self.movement(id) } else { MovementMap::default() };
        if eco.move_available {
            out.extend(moves.destinations.iter().map(|(p, _)| Action::Move { to: *p }));
        }
        let targets: Vec<CombatantId> = self.enemies_of(id).map(|t| t.id).collect();
        for weapon in 0..c.statblock.weapons.len() {
            for &target in &targets {
                for action in [
                    Action::Attack { weapon, target },
                    Action::AimedShot { weapon, target },
                    Action::Autofire { weapon, target },
                ] {
                    if self.check_action(id, &action).is_ok() {
                        out.push(action);
                    }
                }
            }
        }
        for weapon in 0..c.statblock.weapons.len() {
            let action = Action::Reload { weapon };
            if self.check_action(id, &action).is_ok() {
                out.push(action);
            }
        }
        if eco.action_available {
            out.extend(moves.destinations.iter().map(|(p, _)| Action::Run { to: *p }));
        }
        out
    }

    /// One-line human description of an action.
    pub fn describe_action(&self, actor: CombatantId, action: &Action) -> String {
        let me = self.name_of(actor);
        match action {
            Action::Move { to } => format!("{me} moves to ({}, {})", to.x, to.y),
            Action::Run { to } => format!("{me} runs to ({}, {})", to.x, to.y),
            Action::Attack { weapon, target } => {
                format!("{me} attacks {} with {}", self.name_of(*target), self.weapon_name(actor, *weapon))
            }
            Action::AimedShot { weapon, target } => {
                format!("{me} takes an aimed shot at {}'s head with {}", self.name_of(*target), self.weapon_name(actor, *weapon))
            }
            Action::Autofire { weapon, target } => {
                format!("{me} autofires {} at {}", self.weapon_name(actor, *weapon), self.name_of(*target))
            }
            Action::Reload { weapon } => format!("{me} reloads {}", self.weapon_name(actor, *weapon)),
        }
    }
}
