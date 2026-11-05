//! # pf2e-bridge
//!
//! Adapter between a virtual tabletop and the engine:
//!
//! ```text
//!  Foundry (actors, tokens, walls, combat)        ← source of truth
//!        │  TypeScript extractor (foundry/src/extract)
//!        ▼
//!  CombatSnapshot (JSON, this crate's `snapshot` module)
//!        │  convert::snapshot_to_state   — validation, walls, spells, turn state
//!        ▼
//!  pf2e_core::CombatState                        ← rules authority
//!        │  pf2e_ai (evaluation, search, Monte Carlo)
//!        ▼
//!  report::* (token-addressed recommendations)   → Foundry UI
//! ```
//!
//! The crate contains **no game rules**: it maps data into core types and
//! asks `pf2e-core`/`pf2e-ai` everything else. It performs no I/O and holds
//! no global state, so the same code is unit-tested natively and shipped in
//! the `pf2e-wasm` module.

pub mod api;
pub mod convert;
pub mod fixtures;
pub mod formula;
pub mod report;
pub mod snapshot;
pub mod spells;
pub mod walls;

pub use api::{Bridge, BridgeError, ErrorCode};
pub use convert::{snapshot_to_state, Converted, IdMap};
pub use snapshot::CombatSnapshot;

#[cfg(test)]
mod tests;
