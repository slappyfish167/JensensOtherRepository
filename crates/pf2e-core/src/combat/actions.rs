//! Action legality, declaration, and legal-action generation.
//!
//! `check_action` is side-effect free and is the single source of truth for
//! legality: `declare_action` calls it first, and `legal_actions` (used by
//! UIs and the AI) is built from it.

use super::command::Action;
use super::engine::{CombatError, CombatResult};
use super::event::Event;
use super::pending::*;
use super::rules;
use super::state::{CombatState, LifeState};
use crate::conditions::{ConditionKind as CK, Duration};
use crate::effects::{Effect, EffectTag};
use crate::grid::{area_squares, creature_distance, distance_ft, footprint, movement_map, within_reach, MoveMap, MoveRules, Pos};
use crate::ids::CombatantId;
use crate::models::battlefield::Terrain;
use crate::models::spell::{Spell, SpellTargeting};
use crate::models::statblock::{ReactionKind, StrikeRange};
use crate::models::weapon::WeaponTrait;
use std::collections::HashSet;

fn illegal<T>(msg: impl Into<String>) -> CombatResult<T> {
    Err(CombatError::IllegalAction(msg.into()))
}

impl CombatState {
    /// Movement options for a combatant from its current square.
    pub fn movement(&self, id: CombatantId) -> MoveMap {
        let c = self.c(id);
        let mut rules = MoveRules { speed_ft: c.statblock.speed, size: c.statblock.size, ..Default::default() };
        for o in &self.combatants {
            if o.id == id || o.life != LifeState::Active || o.is_hazard() {
                continue;
            }
            for sq in o.footprint() {
                rules.occupied.insert(sq);
                if c.side.is_hostile_to(o.side) && !o.is_down() {
                    rules.impassable.insert(sq);
                }
            }
        }
        movement_map(&self.battlefield, c.position, &rules)
    }

    fn occupied_by_others(&self, id: CombatantId) -> HashSet<Pos> {
        self.combatants
            .iter()
            .filter(|o| o.id != id && o.life == LifeState::Active && !o.is_hazard())
            .flat_map(|o| o.footprint())
            .collect()
    }

    /// Validates an action for `actor` (whose turn it must be) and returns its cost.
    pub fn check_action(&self, actor: CombatantId, action: &Action) -> CombatResult<u8> {
        self.check_action_with(actor, action, None)
    }

    /// [`CombatState::check_action`] reusing an already computed movement
    /// map for the actor (Stride validation is a pathfinding query).
    fn check_action_with(&self, actor: CombatantId, action: &Action, movement: Option<&MoveMap>) -> CombatResult<u8> {
        self.ensure_can_declare(actor)?;
        let a = self.c(actor);
        if !a.can_act() {
            return illegal(format!("{} cannot act", a.name));
        }
        let cost: u8 = match action {
            Action::CastSpell { spell_id, .. } => self.spells.get(spell_id).map(|s| s.actions).unwrap_or(1),
            Action::DisableHazard { .. } => 2,
            _ => 1,
        };
        if a.actions_remaining < cost {
            return Err(CombatError::NotEnoughActions { needed: cost, have: a.actions_remaining });
        }
        if a.conditions.has(CK::Fleeing) && !matches!(action, Action::Stride { .. } | Action::Step { .. }) {
            return illegal("a fleeing creature can only move away");
        }
        let target_ok = |t: CombatantId| -> CombatResult<&super::state::Combatant> {
            let tc = self.combatant(t).ok_or(CombatError::UnknownCombatant(t))?;
            if t == actor {
                return illegal("cannot target yourself");
            }
            if tc.life != LifeState::Active {
                return illegal(format!("{} is not a valid target", tc.name));
            }
            Ok(tc)
        };

        match action {
            Action::Strike { actor_strike, target, .. } => {
                let tc = target_ok(*target)?;
                if tc.is_hazard() {
                    return illegal("Strike hazards is not modelled; use Disable a Device");
                }
                if a.conditions.has(CK::Restrained) {
                    return illegal("restrained creatures cannot Strike");
                }
                rules::strike_reachable(self, actor, *target, *actor_strike).map_err(CombatError::IllegalAction)?;
            }
            Action::Stride { to } => {
                if a.conditions.has(CK::Immobilized) {
                    return illegal("immobilized");
                }
                if a.conditions.has(CK::Prone) {
                    return illegal("prone creatures must Stand (or Crawl) first");
                }
                let reachable = match movement {
                    Some(mm) => mm.cost_to(*to).is_some(),
                    None => self.movement(actor).cost_to(*to).is_some(),
                };
                if !reachable {
                    return illegal(format!("cannot reach ({}, {}) with a Stride", to.x, to.y));
                }
            }
            Action::Step { to } => {
                if a.conditions.has(CK::Immobilized) || a.conditions.has(CK::Prone) {
                    return illegal("cannot Step while immobilized or prone");
                }
                if distance_ft(to.x - a.position.x, to.y - a.position.y) != 5 {
                    return illegal("Step moves exactly 5 feet");
                }
                let fp = footprint(*to, a.statblock.size);
                let occupied = self.occupied_by_others(actor);
                for sq in &fp {
                    let t = self.battlefield.terrain(*sq);
                    if !matches!(t, Terrain::Open | Terrain::Obscured) {
                        return illegal("cannot Step into difficult terrain or walls");
                    }
                    if occupied.contains(sq) {
                        return illegal("square is occupied");
                    }
                }
            }
            Action::Stand => {
                if !a.conditions.has(CK::Prone) {
                    return illegal("not prone");
                }
            }
            Action::RaiseShield => {
                if a.statblock.shield.is_none() {
                    return illegal("no shield");
                }
                if a.shield_broken() {
                    return illegal("shield is broken");
                }
                if a.has_tag(&EffectTag::ShieldRaised) {
                    return illegal("shield already raised");
                }
            }
            Action::TakeCover => {
                if a.has_tag(&EffectTag::TakingCover) {
                    return illegal("already taking cover");
                }
            }
            Action::Demoralize { target } => {
                let tc = target_ok(*target)?;
                if !a.side.is_hostile_to(tc.side) || tc.is_hazard() {
                    return illegal("Demoralize targets an enemy creature");
                }
                if creature_distance(a.position, a.statblock.size, tc.position, tc.statblock.size) > 30 {
                    return illegal("target must be within 30 feet");
                }
                if tc.has_tag(&EffectTag::ImmuneTo(format!("demoralize:{}", actor.0))) {
                    return illegal(format!("{} is temporarily immune to your Demoralize", tc.name));
                }
            }
            Action::Trip { target } => {
                let tc = target_ok(*target)?;
                if tc.is_hazard() {
                    return illegal("cannot Trip a hazard");
                }
                if tc.conditions.has(CK::Prone) {
                    return illegal("target is already prone");
                }
                let reach = a.statblock.melee_reach().unwrap_or(5);
                if !within_reach(a.position, a.statblock.size, tc.position, tc.statblock.size, reach) {
                    return illegal("target is out of reach");
                }
            }
            Action::CastSpell { entry, spell_id, rank, targets, aim } => {
                let e = a.statblock.spellcasting.get(*entry).ok_or_else(|| CombatError::IllegalAction("no such spellcasting entry".into()))?;
                let spell = self.spells.get(spell_id).ok_or_else(|| CombatError::IllegalAction(format!("unknown spell '{spell_id}'")))?;
                if spell.cantrip {
                    if !e.cantrips.contains(spell_id) {
                        return illegal(format!("{} does not know {}", a.name, spell.name));
                    }
                    if *rank != cantrip_rank(a.statblock.level) {
                        return illegal(format!("cantrips are cast at rank {}", cantrip_rank(a.statblock.level)));
                    }
                } else {
                    if !e.spells.iter().any(|k| k.spell_id == *spell_id && k.rank == *rank) {
                        return illegal(format!("{} has no {} at rank {rank}", a.name, spell.name));
                    }
                    let left = a.spell_slots.get(*entry).and_then(|s| s.get(rank)).copied().unwrap_or(0);
                    if left == 0 {
                        return illegal(format!("no rank-{rank} slots left"));
                    }
                }
                self.spell_targets(actor, spell, targets, *aim).map_err(CombatError::IllegalAction)?;
            }
            Action::DisableHazard { hazard } => {
                let h = self.combatant(*hazard).ok_or(CombatError::UnknownCombatant(*hazard))?;
                let Some(profile) = &h.statblock.hazard else { return illegal("not a hazard") };
                if h.life != LifeState::Active {
                    return illegal("hazard is already inactive");
                }
                if profile.disable.is_empty() {
                    return illegal("this hazard cannot be disabled with a skill");
                }
                let near = std::iter::once(h.position)
                    .chain(profile.trigger_squares(h.position))
                    .any(|p| creature_distance(a.position, a.statblock.size, p, crate::stats::Size::Medium) <= 5);
                if !near {
                    return illegal("must be adjacent to the hazard");
                }
            }
        }
        Ok(cost)
    }

    /// Resolves the creatures a spell affects, validating range and line of effect.
    pub fn spell_targets(&self, caster: CombatantId, spell: &Spell, targets: &[CombatantId], aim: Option<Pos>) -> Result<Vec<CombatantId>, String> {
        let a = self.c(caster);
        let range = spell.range_ft.unwrap_or(5);
        match spell.targeting {
            SpellTargeting::SelfOnly => Ok(vec![caster]),
            SpellTargeting::Creatures { max, allies } => {
                if targets.is_empty() {
                    return Err("choose at least one target".into());
                }
                if targets.len() > max as usize {
                    return Err(format!("{} takes at most {max} target(s)", spell.name));
                }
                let mut seen = HashSet::new();
                for &t in targets {
                    if !seen.insert(t) {
                        return Err("duplicate target".into());
                    }
                    let tc = self.combatant(t).ok_or("unknown target")?;
                    if tc.life != LifeState::Active || tc.is_hazard() {
                        return Err(format!("{} is not a valid target", tc.name));
                    }
                    if allies && a.side.is_hostile_to(tc.side) {
                        return Err(format!("{} targets allies only", spell.name));
                    }
                    if t != caster {
                        let d = creature_distance(a.position, a.statblock.size, tc.position, tc.statblock.size);
                        if d > range {
                            return Err(format!("{} is {d} ft away (range {range} ft)", tc.name));
                        }
                        if !rules::cover_between(self, caster, t, true).line_of_effect {
                            return Err(format!("no line of effect to {}", tc.name));
                        }
                    }
                }
                Ok(targets.to_vec())
            }
            SpellTargeting::Area { area, allies_only } => {
                let aim = match area {
                    crate::grid::AreaShape::Emanation(_) => a.position,
                    _ => aim.ok_or("choose a point to aim the area")?,
                };
                if let crate::grid::AreaShape::Burst(_) = area {
                    let d = distance_ft(aim.x - a.position.x, aim.y - a.position.y);
                    if d > range {
                        return Err(format!("burst centre is {d} ft away (range {range} ft)"));
                    }
                }
                let squares: HashSet<Pos> = area_squares(&self.battlefield, area, a.position, a.statblock.size, aim).into_iter().collect();
                Ok(self
                    .combatants
                    .iter()
                    .filter(|c| c.life == LifeState::Active && !c.is_hazard())
                    .filter(|c| !allies_only || !a.side.is_hostile_to(c.side))
                    .filter(|c| c.footprint().iter().any(|p| squares.contains(p)))
                    .map(|c| c.id)
                    .collect())
            }
        }
    }

    /// Declares (and starts resolving) an action. Assumes `ensure_can_declare`.
    pub(crate) fn declare_action(&mut self, actor: CombatantId, action: &Action, ev: &mut Vec<Event>) -> CombatResult<()> {
        // One pathfinding pass serves both Stride validation and its path.
        let stride_map = matches!(action, Action::Stride { .. }).then(|| self.movement(actor));
        let cost = self.check_action_with(actor, action, stride_map.as_ref())?;
        let tag = self.alloc_tag();
        self.cm(actor).actions_remaining -= cost;
        ev.push(Event::ActionDeclared { actor, action: action.name().to_string(), cost });

        match action {
            Action::Strike { actor_strike, target, cover_override } => {
                let a = self.c(actor);
                let strike = &a.statblock.strikes[*actor_strike];
                let agile = strike.is_agile();
                let melee = strike.is_melee();
                let sweep = strike.traits.contains(&WeaponTrait::Sweep) && a.struck_this_turn.iter().any(|t| t != target);
                let cover = match cover_override {
                    Some(c) => *c,
                    None => rules::cover_between(self, actor, *target, !melee).cover,
                };
                let ctx = AttackCtx {
                    attacker: actor,
                    target: *target,
                    source: AttackSource::Strike { index: *actor_strike },
                    map_penalty: rules::map_penalty(a.attacks_this_turn, agile),
                    attack_number: a.attacks_this_turn,
                    cover,
                    disrupts: None,
                    sweep,
                };
                let am = self.cm(actor);
                am.attacks_this_turn += 1;
                am.struck_this_turn.push(*target);
                am.effects.retain(|e| !e.tags.contains(&EffectTag::TakingCover));
                let steps = self.attack_steps(tag, ctx);
                self.push_front_all(steps);
            }
            Action::Stride { to } => {
                // Movement depends on positions and conditions only, which
                // nothing above has changed since `stride_map` was computed.
                let path = stride_map.as_ref().and_then(|mm| mm.path_to(*to)).unwrap_or_default();
                self.cm(actor).effects.retain(|e| !e.tags.contains(&EffectTag::TakingCover));
                self.queue.push_front(Step::ContinueMove { tag, mover: actor, kind: MoveKind::Stride, remaining: path, offered: vec![] });
            }
            Action::Step { to } => {
                self.cm(actor).effects.retain(|e| !e.tags.contains(&EffectTag::TakingCover));
                self.queue.push_front(Step::ContinueMove { tag, mover: actor, kind: MoveKind::Step, remaining: vec![*to], offered: vec![] });
            }
            Action::Stand => {
                self.cm(actor).conditions.remove(CK::Prone);
                ev.push(Event::ConditionLost { target: actor, condition: CK::Prone });
                let steps: Vec<Step> = self.reactive_strike_offers(actor, None, "standing up (a move action)", &[]).into_iter().map(|(_, s)| s).collect();
                self.push_front_all(steps);
            }
            Action::RaiseShield => {
                let expires = self.expiry_for(Duration::UntilStartOfSourcesNextTurn, actor, actor);
                let id = self.alloc_effect_id();
                self.cm(actor).effects.push(Effect {
                    id,
                    name: "Shield Raised".into(),
                    source: Some(actor),
                    grants: vec![],
                    tags: vec![EffectTag::ShieldRaised],
                    expires,
                });
                ev.push(Event::EffectGained { target: actor, name: "Shield Raised".into() });
            }
            Action::TakeCover => {
                let id = self.alloc_effect_id();
                self.cm(actor).effects.push(Effect {
                    id,
                    name: "Taking Cover".into(),
                    source: Some(actor),
                    grants: vec![],
                    tags: vec![EffectTag::TakingCover],
                    expires: None,
                });
                ev.push(Event::EffectGained { target: actor, name: "Taking Cover".into() });
            }
            Action::Demoralize { target } => {
                self.queue.push_front(Step::SkillCheck { tag, actor, target: *target, action: SkillAction::Demoralize, map_penalty: 0 });
            }
            Action::Trip { target } => {
                let a = self.cm(actor);
                let map = rules::map_penalty(a.attacks_this_turn, false);
                a.attacks_this_turn += 1;
                self.queue.push_front(Step::SkillCheck { tag, actor, target: *target, action: SkillAction::Trip, map_penalty: map });
            }
            Action::CastSpell { entry, spell_id, rank, targets, aim } => {
                let spell = self.spells[spell_id].clone();
                let resolved = self.spell_targets(actor, &spell, targets, *aim).map_err(CombatError::IllegalAction)?;
                if !spell.cantrip {
                    if let Some(n) = self.cm(actor).spell_slots.get_mut(*entry).and_then(|s| s.get_mut(rank)) {
                        *n -= 1;
                    }
                }
                ev.push(Event::Note { text: format!("{} casts {} (rank {rank})", self.c(actor).name, spell.name) });
                let mut steps = Vec::new();
                if spell.has_trait("manipulate") {
                    steps.extend(self.reactive_strike_offers(actor, Some(tag), "casting a spell (manipulate)", &[]).into_iter().map(|(_, s)| s));
                }
                steps.push(Step::ResolveSpell { tag, caster: actor, entry: *entry, spell_id: spell_id.clone(), rank: *rank, targets: resolved });
                self.push_front_all(steps);
            }
            Action::DisableHazard { hazard } => {
                self.queue.push_front(Step::SkillCheck { tag, actor, target: *hazard, action: SkillAction::Disable, map_penalty: 0 });
            }
        }
        Ok(())
    }

    /// Concealment flat check (if needed) followed by the attack roll.
    pub(crate) fn attack_steps(&self, tag: u32, ctx: AttackCtx) -> Vec<Step> {
        match rules::concealment_dc(self, ctx.attacker, ctx.target) {
            Some(dc) => vec![Step::ConcealmentCheck { tag, ctx, dc }],
            None => vec![Step::AttackRoll { tag, ctx }],
        }
    }

    /// Reaction windows for Reactive Strike against `mover`.
    pub(crate) fn reactive_strike_offers(&mut self, mover: CombatantId, disrupts: Option<u32>, trigger: &str, skip: &[CombatantId]) -> Vec<(CombatantId, Step)> {
        let m = self.c(mover).clone();
        let mut out = Vec::new();
        let candidates: Vec<_> = self
            .combatants
            .iter()
            .filter(|r| {
                r.id != mover
                    && !skip.contains(&r.id)
                    && r.side.is_hostile_to(m.side)
                    && r.statblock.has_reaction(ReactionKind::ReactiveStrike)
                    && r.reaction_available
                    && r.can_act()
            })
            .filter_map(|r| {
                // Best melee strike that reaches the mover.
                r.statblock
                    .strikes
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| match s.range {
                        StrikeRange::Melee { reach_ft } => within_reach(r.position, r.statblock.size, m.position, m.statblock.size, reach_ft),
                        _ => false,
                    })
                    .max_by_key(|(_, s)| s.attack_bonus)
                    .map(|(i, _)| (r.id, r.name.clone(), i))
            })
            .collect();
        for (rid, rname, idx) in candidates {
            let rtag = self.alloc_tag();
            let cover = rules::cover_between(self, rid, mover, false).cover;
            let ctx = AttackCtx {
                attacker: rid,
                target: mover,
                source: AttackSource::Strike { index: idx },
                map_penalty: 0,
                attack_number: 0,
                cover,
                disrupts,
                sweep: false,
            };
            let on_accept = self.attack_steps(rtag, ctx);
            out.push((
                rid,
                Step::Decision {
                    tag: rtag,
                    decider: rid,
                    reaction: ReactionKind::ReactiveStrike,
                    prompt: format!("{} is {trigger}. Use {rname}'s Reactive Strike?", m.name),
                    on_accept,
                    on_decline: vec![],
                },
            ));
        }
        out
    }

    /// Every legal action for the current actor (empty when input is pending).
    /// Stride lists every reachable square; callers may prune.
    pub fn legal_actions(&self) -> Vec<Action> {
        let Some(actor) = self.current_actor() else { return vec![] };
        if self.pending().is_some() || !self.queue.is_empty() {
            return vec![];
        }
        let a = self.c(actor);
        let mut candidates: Vec<Action> = Vec::new();
        let others: Vec<_> = self.combatants.iter().filter(|c| c.id != actor && c.life == LifeState::Active).collect();
        for (i, _) in a.statblock.strikes.iter().enumerate() {
            for t in others.iter().filter(|t| a.side.is_hostile_to(t.side) && !t.is_hazard()) {
                candidates.push(Action::Strike { actor_strike: i, target: t.id, cover_override: None });
            }
        }
        for t in others.iter().filter(|t| a.side.is_hostile_to(t.side) && !t.is_hazard()) {
            candidates.push(Action::Demoralize { target: t.id });
            candidates.push(Action::Trip { target: t.id });
        }
        for t in others.iter().filter(|t| t.is_hazard()) {
            candidates.push(Action::DisableHazard { hazard: t.id });
        }
        candidates.push(Action::Stand);
        candidates.push(Action::RaiseShield);
        candidates.push(Action::TakeCover);
        // Every destination of the movement map is a legal Stride whenever
        // one is: check one representative instead of re-running the
        // pathfinder per square.
        let dests: Vec<Pos> = self.movement(actor).destinations().map(|(p, _)| p).collect();
        let mut strides = Vec::new();
        if let Some(first) = dests.first() {
            if self.check_action(actor, &Action::Stride { to: *first }).is_ok() {
                strides = dests.into_iter().map(|to| Action::Stride { to }).collect();
            }
        }
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx != 0 || dy != 0 {
                    candidates.push(Action::Step { to: Pos::new(a.position.x + dx, a.position.y + dy) });
                }
            }
        }
        for (ei, e) in a.statblock.spellcasting.iter().enumerate() {
            let mut options: Vec<(String, u8)> = e.cantrips.iter().map(|c| (c.clone(), cantrip_rank(a.statblock.level))).collect();
            options.extend(e.spells.iter().map(|k| (k.spell_id.clone(), k.rank)));
            options.dedup();
            for (sid, rank) in options {
                let Some(spell) = self.spells.get(&sid) else { continue };
                for (targets, aim) in self.spell_target_options(actor, spell) {
                    candidates.push(Action::CastSpell { entry: ei, spell_id: sid.clone(), rank, targets, aim });
                }
            }
        }
        let mut legal: Vec<Action> = candidates.into_iter().filter(|c| self.check_action(actor, c).is_ok()).collect();
        legal.extend(strides);
        legal
    }

    /// A reasonable set of target choices for a spell (not exhaustive for
    /// multi-target spells: singles plus "as many enemies as allowed").
    fn spell_target_options(&self, caster: CombatantId, spell: &Spell) -> Vec<(Vec<CombatantId>, Option<Pos>)> {
        let a = self.c(caster);
        let living: Vec<_> = self.combatants.iter().filter(|c| c.life == LifeState::Active && !c.is_hazard()).collect();
        match spell.targeting {
            SpellTargeting::SelfOnly => vec![(vec![], None)],
            SpellTargeting::Creatures { max, allies } => {
                let pool: Vec<CombatantId> = living
                    .iter()
                    .filter(|c| if allies { !a.side.is_hostile_to(c.side) } else { a.side.is_hostile_to(c.side) })
                    .map(|c| c.id)
                    .filter(|&id| self.spell_targets(caster, spell, &[id], None).is_ok())
                    .collect();
                let mut out: Vec<_> = pool.iter().map(|&t| (vec![t], None)).collect();
                if max > 1 && pool.len() > 1 {
                    out.push((pool.iter().copied().take(max as usize).collect(), None));
                }
                out
            }
            SpellTargeting::Area { area, .. } => match area {
                crate::grid::AreaShape::Emanation(_) => vec![(vec![], None)],
                _ => living
                    .iter()
                    .filter(|c| a.side.is_hostile_to(c.side))
                    .flat_map(|c| [c.position, Pos::new(c.position.x + 1, c.position.y + 1)])
                    .map(|p| (vec![], Some(p)))
                    .collect(),
            },
        }
    }
}

/// Cantrips are automatically heightened to half the caster's level, rounded up.
pub fn cantrip_rank(level: i32) -> u8 {
    ((level.max(1) + 1) / 2).clamp(1, 10) as u8
}
