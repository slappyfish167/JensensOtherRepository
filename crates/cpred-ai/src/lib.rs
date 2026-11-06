//! # cpred-ai
//!
//! The Cyberpunk RED adapter for the system-neutral analysis layer
//! (`tactics-ai`). It **contains no game rules**: [`CyberpunkRed`] exposes
//! `cpred-core` through the `GameSystem` / `TacticalSystem` contracts, and
//! the expected-value helpers in [`ev`] are built on core rule functions.
//!
//! * [`system`] — the `CyberpunkRed` adapter.
//! * [`ev`] — expected damage per Action / round, engagement, kill odds.
//! * [`movegen`] — cover-aware candidate actions and the Monte Carlo policy.

pub mod ev;
pub mod movegen;
pub mod system;

pub use system::CyberpunkRed;

#[cfg(test)]
mod tests;
