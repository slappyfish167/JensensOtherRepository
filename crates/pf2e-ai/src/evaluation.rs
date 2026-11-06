//! PF2e position evaluation: the system-neutral Lanchester evaluator
//! (`tactics_ai::evaluation`) fed by PF2e rules through [`crate::system::Pf2e`].
//!
//! Conditions (frightened, off-guard, slowed, ...), cover and spell slots
//! enter through the core rules that compute attack totals, AC and the
//! available actions, so the evaluator never hard-codes a condition.

pub use tactics_ai::evaluation::{EvalOptions, Evaluation, Evaluator, Factor, SideSummary};

pub type HeuristicEvaluator = tactics_ai::HeuristicEvaluator<crate::system::Pf2e>;

use crate::keys::{combat_profile, combine, side_context};
use crate::movegen::MoveGenCache;
use pf2e_core::combat::CombatState;
use pf2e_core::stats::Side;
use pf2e_core::CombatantId;
use std::collections::HashMap;

/// Per-search memo for PF2e: per-pair expected damage and move generation.
/// Pair keys hash every input the expected-damage helpers read (see
/// [`crate::keys::combat_profile`]), so a hit is exact.
#[derive(Default)]
pub struct EvalCache {
    pairs: HashMap<u64, f64>,
    /// Per combatant for the position being evaluated: (combat profile,
    /// context of its side).
    keys: HashMap<CombatantId, (u64, u64)>,
    pub hits: u64,
    pub misses: u64,
    pub movegen: MoveGenCache,
}

impl EvalCache {
    /// Entries beyond this are dropped wholesale (bounded memory in wasm).
    pub const MAX_ENTRIES: usize = 200_000;

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Recomputes the per-combatant keys for position `s`.
    pub fn prepare(&mut self, s: &CombatState) {
        let sides: HashMap<Side, u64> = [Side::Party, Side::Enemy, Side::Neutral].into_iter().map(|side| (side, side_context(s, side))).collect();
        self.keys = s.combatants.iter().map(|c| (c.id, (combat_profile(c), sides[&c.side]))).collect();
    }

    /// `compute()` for attacker `a` → target `f`, memoised.
    pub fn pair(&mut self, a: CombatantId, f: CombatantId, flags: u64, compute: impl FnOnce() -> f64) -> f64 {
        let (Some(&(pa, side_a)), Some(&(pf, _))) = (self.keys.get(&a), self.keys.get(&f)) else { return compute() };
        let key = combine(&[side_a, pa, pf, flags]);
        if let Some(v) = self.pairs.get(&key) {
            self.hits += 1;
            return *v;
        }
        self.misses += 1;
        let v = compute();
        if self.pairs.len() >= Self::MAX_ENTRIES {
            self.pairs.clear();
        }
        self.pairs.insert(key, v);
        v
    }
}
