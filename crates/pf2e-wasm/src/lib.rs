//! WebAssembly entry points for the Foundry module.
//!
//! Design:
//! * **Thin**: every export forwards to [`pf2e_bridge::Bridge`], which is
//!   tested natively. Nothing here knows a rule.
//! * **JSON strings across the boundary**. A snapshot is tens of KB; parsing
//!   costs well under a millisecond and keeps the TypeScript types the only
//!   schema the host needs (no generated glue types to keep in sync).
//! * **Memory**: one `Engine` per worker keeps the converted position, so
//!   repeated analyses do not re-send it. Search caches live only for the
//!   duration of a call and are dropped afterwards, returning memory to the
//!   allocator (linear memory never shrinks, but is reused).
//! * **Errors** are thrown as JSON strings `{ "code": "...", "message": "..." }`.
//! * **Panics** trap the instance (`RuntimeError: unreachable`). The hook
//!   records the message first; the worker reads it with [`last_panic`],
//!   reports it, and instantiates a fresh module.
//! * **Time limits**: searches take a deadline in milliseconds, checked
//!   against `performance.now()` through the pure `Budget` trait.

use pf2e_ai::search::Budget;
use pf2e_bridge::api::{Bridge, BridgeError, ErrorCode};
use std::sync::Mutex;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn performance_now() -> f64;
}

static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut p) = LAST_PANIC.lock() {
            *p = Some(info.to_string());
        }
    }));
}

/// Message of the panic that trapped the instance, if any.
#[wasm_bindgen(js_name = lastPanic)]
pub fn last_panic() -> Option<String> {
    LAST_PANIC.lock().ok().and_then(|p| p.clone())
}

/// `"<crate version>+snapshot.<schema version>"`, checked by the host.
#[wasm_bindgen]
pub fn version() -> String {
    format!("{}+snapshot.{}", env!("CARGO_PKG_VERSION"), pf2e_bridge::snapshot::SNAPSHOT_VERSION)
}

/// Stops a search once `deadline` (a `performance.now()` value) passes.
struct Deadline(f64);

impl Budget for Deadline {
    fn exhausted(&self, _nodes: usize) -> bool {
        self.0 > 0.0 && performance_now() >= self.0
    }
}

fn deadline(ms: f64) -> Deadline {
    Deadline(if ms.is_finite() && ms > 0.0 { performance_now() + ms } else { 0.0 })
}

fn js(r: Result<String, BridgeError>) -> Result<String, JsValue> {
    r.map_err(|e| JsValue::from_str(&e.to_json()))
}

#[wasm_bindgen]
#[derive(Default)]
pub struct Engine {
    bridge: Bridge,
}

#[wasm_bindgen]
impl Engine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Engine {
        Engine::default()
    }

    /// Converts and stores a `CombatSnapshot` (JSON). Returns a load summary.
    pub fn load(&mut self, snapshot: &str) -> Result<String, JsValue> {
        js(self.bridge.load(snapshot))
    }

    /// Evaluation bar data for the loaded position.
    pub fn evaluate(&self) -> Result<String, JsValue> {
        js(self.bridge.evaluate())
    }

    /// Best turn for a token. `deadline_ms` ≤ 0 means no time limit.
    #[wasm_bindgen(js_name = bestTurn)]
    pub fn best_turn(&self, token_id: &str, options: &str, deadline_ms: f64) -> Result<String, JsValue> {
        js(self.bridge.best_turn(token_id, options, &deadline(deadline_ms)))
    }

    /// GM assistant: plans for every creature of a side (`"enemy"` default).
    /// Each creature's search gets its own `deadline_ms` budget.
    #[wasm_bindgen(js_name = sidePlan)]
    pub fn side_plan(&self, side: &str, options: &str, deadline_ms: f64) -> Result<String, JsValue> {
        js(self.bridge.side_plan(side, options, &|| Box::new(deadline(deadline_ms)) as Box<dyn Budget>))
    }

    /// One slice of the Monte Carlo forecast (raw tally JSON).
    #[wasm_bindgen(js_name = forecastSlice)]
    pub fn forecast_slice(&self, options: &str) -> Result<String, JsValue> {
        js(self.bridge.forecast_slice(options))
    }

    /// Combines tallies (JSON array) into the forecast report.
    #[wasm_bindgen(js_name = mergeForecast)]
    pub fn merge_forecast(&self, tallies: &str) -> Result<String, JsValue> {
        js(self.bridge.merge_forecast(tallies))
    }

    /// Why the evaluation changed since the previous load (top `max` reasons).
    pub fn explain(&self, max: u32) -> Result<String, JsValue> {
        js(self.bridge.explain(max as usize))
    }
}

/// Error JSON for host-side failures (kept identical in shape).
pub fn internal_error(message: &str) -> String {
    BridgeError::new(ErrorCode::Internal, message).to_json()
}
