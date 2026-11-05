//! Latency benchmark for the analyses the Foundry module runs.
//!
//! ```bash
//! cargo run --release -p pf2e-bridge --example bench
//! ```
//!
//! Targets (docs/FOUNDRY.md §Performance): best turn < 100 ms,
//! monster plan < 2 s, TPK simulation < 3 s. WebAssembly runs roughly
//! 1.3–2× slower than native, so native numbers should sit well below.

use pf2e_ai::evaluation::HeuristicEvaluator;
use pf2e_ai::queries::side_strategy;
use pf2e_ai::search::{best_turn, SearchConfig};
use pf2e_ai::simulate::{simulate, SimulationConfig};
use pf2e_bridge::fixtures::crypt_ambush_active;
use pf2e_core::stats::Side;
use std::time::Instant;

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    println!("{label:<44} {:>9.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    out
}

fn main() {
    let s = crypt_ambush_active(7);
    let eval = HeuristicEvaluator::default();
    time("evaluate_detailed", || eval.evaluate_detailed(&s));
    let mut worst = 0.0f64;
    for c in s.combatants.iter().filter(|c| c.in_initiative) {
        let Some(h) = s.hypothetical_turn(c.id) else { continue };
        let t = Instant::now();
        let plan = best_turn(&h, &eval, SearchConfig::default()).expect("plan");
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        worst = worst.max(ms);
        println!(
            "best_turn {:<34} {:>9.1} ms  nodes={:<6} gain={:+.2} depth={} tt={} cache={} truncated={} actions={}",
            c.name, ms, plan.nodes, plan.gain, plan.depth, plan.transposition_hits, plan.eval_cache_hits, plan.truncated, plan.actions.len()
        );
    }
    println!("{:<44} {:>9.1} ms", "best_turn worst case", worst);
    time("side_strategy(enemy)", || side_strategy(&s, Side::Enemy, SearchConfig::default()));
    let r = time("simulate 200 runs", || simulate(&s, SimulationConfig::default()));
    println!("  party win {:.0}%  TPK {:.0}%", r.party_win_rate * 100.0, r.tpk_probability * 100.0);
}
