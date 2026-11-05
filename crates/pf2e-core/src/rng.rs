//! Optional, explicitly-enabled automatic dice.
//!
//! The rules engine never calls this. A *driver* (the API server in
//! auto-dice mode, or the Monte Carlo simulator) uses it to answer roll
//! requests, and the answers are recorded as ordinary `ProvideRoll`
//! commands — so auto-rolled combats replay exactly like manual ones.
//!
//! SplitMix64 is implemented here (rather than pulled from a crate) so the
//! sequence for a given seed can never change under a dependency upgrade.

use crate::combat::pending::{RollKind, RollRequest};

#[derive(Debug, Clone)]
pub struct SeededDice {
    state: u64,
}

impl SeededDice {
    pub fn new(seed: u64) -> Self {
        SeededDice { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in 1..=sides (rejection sampling, no modulo bias).
    pub fn die(&mut self, sides: u32) -> u32 {
        let sides = sides.max(1) as u64;
        let zone = u64::MAX - (u64::MAX % sides);
        loop {
            let v = self.next_u64();
            if v < zone {
                return (v % sides) as u32 + 1;
            }
        }
    }

    /// The value to submit for a roll request (natural die or total).
    pub fn answer(&mut self, req: &RollRequest) -> i32 {
        match &req.kind {
            RollKind::Check { .. } | RollKind::Flat { .. } => self.die(20) as i32,
            RollKind::Total { dice } => {
                let mut total = dice.modifier;
                for t in &dice.terms {
                    for _ in 0..t.count {
                        total += self.die(t.sides) as i32;
                    }
                }
                total
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = SeededDice::new(42);
        let mut b = SeededDice::new(42);
        for _ in 0..100 {
            assert_eq!(a.die(20), b.die(20));
        }
    }

    #[test]
    fn faces_in_range_and_roughly_uniform() {
        let mut d = SeededDice::new(7);
        let mut counts = [0u32; 20];
        for _ in 0..20_000 {
            let v = d.die(20);
            assert!((1..=20).contains(&v));
            counts[v as usize - 1] += 1;
        }
        assert!(counts.iter().all(|&c| c > 800 && c < 1200));
    }
}
