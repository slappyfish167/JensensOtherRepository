//! Stable 64-bit hashes of combat states for the search caches.
//!
//! `DefaultHasher::new()` uses fixed SipHash keys, so hashes are identical
//! across runs and platforms (native and wasm32) for one build — which keeps
//! search results reproducible. Hashes are cache keys only; nothing persists.

use pf2e_core::combat::{CombatState, Combatant};
use pf2e_core::effects::Effect;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn hash_effect<H: Hasher>(e: &Effect, h: &mut H) {
    // `id` is deliberately skipped: equal effects applied along different
    // paths get different ids but are the same position.
    e.name.hash(h);
    e.source.hash(h);
    e.tags.hash(h);
    e.expires.hash(h);
    for g in &e.grants {
        g.selector.hash(h);
        g.modifier.kind.hash(h);
        g.modifier.value.hash(h);
    }
}

fn hash_conditions<H: Hasher>(c: &Combatant, h: &mut H) {
    for x in &c.conditions.0 {
        x.kind.hash(h);
        x.value.hash(h);
        x.source.hash(h);
        x.expires.hash(h);
    }
}

/// Everything about a combatant that can change attack numbers, AC, saves,
/// damage, spell options or movement — i.e. all mutable fields except hit
/// points and per-turn bookkeeping that the expected-damage helpers never
/// read. `attacks_this_turn` *is* included: spell-attack MAP reads it.
pub fn combat_profile(c: &Combatant) -> u64 {
    let mut h = DefaultHasher::new();
    c.id.hash(&mut h);
    c.position.hash(&mut h);
    c.life.hash(&mut h);
    hash_conditions(c, &mut h);
    for e in &c.effects {
        hash_effect(e, &mut h);
    }
    c.spell_slots.hash(&mut h);
    c.shield_hp.hash(&mut h);
    c.attacks_this_turn.hash(&mut h);
    h.finish()
}

/// Positions and capability of every creature: drives flanking, reach,
/// engagement and concealment terrain.
pub fn positional_context(s: &CombatState) -> u64 {
    let mut h = DefaultHasher::new();
    for c in &s.combatants {
        c.id.hash(&mut h);
        c.position.hash(&mut h);
        c.life.hash(&mut h);
        hash_conditions(c, &mut h);
    }
    h.finish()
}

/// Positions and capability of one side's creatures. Expected damage of
/// attacker → target depends on the two creatures' profiles plus the
/// attacker's *allies* (flanking, and sneak attack through flanking); see
/// `pf2e_core::combat::rules::is_flanked_by`. Movement by the target's
/// allies cannot change it, so keying on this instead of the whole board
/// keeps far more cache entries valid after a Stride.
pub fn side_context(s: &CombatState, side: pf2e_core::stats::Side) -> u64 {
    let mut h = DefaultHasher::new();
    for c in s.combatants.iter().filter(|c| c.side == side) {
        c.id.hash(&mut h);
        c.position.hash(&mut h);
        c.life.hash(&mut h);
        hash_conditions(c, &mut h);
    }
    h.finish()
}

/// Full position key for transposition detection. Two states with equal
/// keys have the same future: same creatures, HP, conditions, effects,
/// resources, turn and actions. Internal counters (`input_seq`, tag and
/// effect id allocators) are excluded because they differ between
/// transposed move orders without affecting the rules.
pub fn state_key(s: &CombatState) -> u64 {
    let mut h = DefaultHasher::new();
    s.round.hash(&mut h);
    s.turn.hash(&mut h);
    std::mem::discriminant(&s.phase).hash(&mut h);
    s.queue.len().hash(&mut h);
    for c in &s.combatants {
        combat_profile(c).hash(&mut h);
        c.hp.hash(&mut h);
        c.temp_hp.hash(&mut h);
        c.actions_remaining.hash(&mut h);
        c.reaction_available.hash(&mut h);
        c.struck_this_turn.hash(&mut h);
        for p in &c.persistent {
            p.dice.hash(&mut h);
            p.damage_type.hash(&mut h);
        }
    }
    h.finish()
}

/// Everything `CombatState::legal_actions` and the actor's movement map
/// depend on: positions, life, conditions, effects, resources of every
/// creature, plus whose turn it is and how many actions remain. Hit points
/// and attack counters are excluded (legality never reads them).
pub fn legality_key(s: &CombatState, actor: pf2e_core::CombatantId) -> u64 {
    let mut h = DefaultHasher::new();
    actor.hash(&mut h);
    s.combatant(actor).map(|c| c.actions_remaining).hash(&mut h);
    for c in &s.combatants {
        c.id.hash(&mut h);
        c.position.hash(&mut h);
        c.life.hash(&mut h);
        hash_conditions(c, &mut h);
        for e in &c.effects {
            hash_effect(e, &mut h);
        }
        c.spell_slots.hash(&mut h);
        c.shield_hp.hash(&mut h);
    }
    h.finish()
}

pub fn combine(parts: &[u64]) -> u64 {
    let mut h = DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}
