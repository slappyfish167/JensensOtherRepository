//! Optional, explicitly-enabled automatic dice for PF2e roll requests.
//!
//! The generator itself is shared by every system (`tactics_core::rng`);
//! this module only knows how a PF2e roll request is answered: the natural
//! d20 for checks and flat checks, the total for damage and healing.

use crate::combat::pending::{RollKind, RollRequest};
pub use tactics_core::rng::SeededDice;

pub trait AnswerRoll {
    /// The value to submit for a roll request (natural die or total).
    fn answer(&mut self, req: &RollRequest) -> i32;
}

impl AnswerRoll for SeededDice {
    fn answer(&mut self, req: &RollRequest) -> i32 {
        match &req.kind {
            RollKind::Check { .. } | RollKind::Flat { .. } => self.die(20) as i32,
            RollKind::Total { dice } => self.total(dice),
        }
    }
}
