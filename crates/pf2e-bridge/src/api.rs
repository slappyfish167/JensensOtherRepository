//! String-in / string-out façade used by the WebAssembly module (and by
//! native tests). One `Bridge` holds the most recent converted snapshot so
//! repeated analyses of the same position do not re-send or re-convert it,
//! plus the previous one so "explain" can diff two positions.
//!
//! Every error is a [`BridgeError`] with a stable [`ErrorCode`] that the
//! TypeScript side maps to a localized message.

use crate::convert::{snapshot_to_state, Converted};
use crate::report::{self, BudgetFactory, EvaluationReport, ExplainReport, ForecastReport, SidePlanReport, TurnReport};
use crate::snapshot::CombatSnapshot;
use pf2e_ai::evaluation::HeuristicEvaluator;
use pf2e_ai::explain::{explain_change, explain_position};
use pf2e_ai::search::{Budget, SearchConfig, Unlimited};
use pf2e_ai::simulate::{simulate_range, SimulationConfig, SimulationTally};
use pf2e_core::stats::Side;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request or snapshot is not valid JSON for its schema.
    InvalidJson,
    UnsupportedVersion,
    /// Hex/gridless grid or non-5-ft squares.
    UnsupportedScene,
    InvalidSnapshot,
    NothingToAnalyse,
    /// An analysis was requested before any snapshot was loaded.
    NoSnapshot,
    UnknownToken,
    /// The creature cannot act now (unconscious, paralyzed, dying, ...).
    CannotAct,
    /// A Rust panic was caught; the module instance must be recreated.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct BridgeError {
    pub code: ErrorCode,
    pub message: String,
}

impl BridgeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        BridgeError { code, message: message.into() }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| format!("{{\"code\":\"internal\",\"message\":{:?}}}", self.message))
    }
}

fn parse<T: for<'de> Deserialize<'de>>(json: &str, what: &str) -> Result<T, BridgeError> {
    if json.len() > 8 * 1024 * 1024 {
        return Err(BridgeError::new(ErrorCode::InvalidJson, format!("{what} exceeds 8 MiB")));
    }
    serde_json::from_str(json).map_err(|e| BridgeError::new(ErrorCode::InvalidJson, format!("{what}: {e}")))
}

fn to_json<T: Serialize>(v: &T) -> Result<String, BridgeError> {
    serde_json::to_string(v).map_err(|e| BridgeError::new(ErrorCode::Internal, e.to_string()))
}

/// Summary returned by [`Bridge::load`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadSummary {
    pub combatants: usize,
    pub crop: crate::walls::Crop,
    pub wall_edges: usize,
    pub turn_active: bool,
    pub current_token: Option<String>,
    pub warnings: Vec<String>,
    /// Stable hash of the converted position (cache key for the host).
    pub fingerprint: String,
}

/// Options for turn searches; all optional on the wire.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TurnOptions {
    #[serde(default)]
    pub node_budget: Option<usize>,
    #[serde(default)]
    pub beam: Option<usize>,
}

impl TurnOptions {
    fn config(&self) -> SearchConfig {
        let d = SearchConfig::default();
        SearchConfig {
            node_budget: self.node_budget.unwrap_or(d.node_budget).clamp(100, 500_000),
            beam: self.beam.unwrap_or(d.beam).clamp(2, 32),
            ..d
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ForecastOptions {
    pub runs: u32,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_rounds")]
    pub max_rounds: u32,
    /// First run index of this slice (for splitting across workers).
    #[serde(default)]
    pub start: u32,
    /// Runs in this slice; defaults to all of `runs`.
    #[serde(default)]
    pub count: Option<u32>,
}

fn default_seed() -> u64 {
    1
}
fn default_rounds() -> u32 {
    20
}

#[derive(Default)]
pub struct Bridge {
    current: Option<Converted>,
    previous: Option<Converted>,
}

impl Bridge {
    pub fn new() -> Self {
        Self::default()
    }

    fn cur(&self) -> Result<&Converted, BridgeError> {
        self.current.as_ref().ok_or_else(|| BridgeError::new(ErrorCode::NoSnapshot, "load a snapshot first"))
    }

    /// Converts and stores a snapshot. The previously loaded one is kept
    /// for [`Bridge::explain`] if it describes the same tokens.
    pub fn load(&mut self, snapshot_json: &str) -> Result<String, BridgeError> {
        let snap: CombatSnapshot = parse(snapshot_json, "snapshot")?;
        let conv = snapshot_to_state(&snap)?;
        let summary = LoadSummary {
            combatants: conv.state.combatants.len(),
            crop: conv.crop,
            wall_edges: conv.state.battlefield.edges.len(),
            turn_active: conv.turn_active,
            current_token: if conv.turn_active { conv.state.current_actor().map(|id| conv.token(id)) } else { None },
            warnings: conv.warnings.clone(),
            fingerprint: conv.state.fingerprint(),
        };
        if let Some(old) = self.current.take() {
            if old.ids == conv.ids && old.crop == conv.crop {
                if old.state.fingerprint() != conv.state.fingerprint() {
                    self.previous = Some(old);
                }
            } else {
                self.previous = None;
            }
        }
        self.current = Some(conv);
        to_json(&summary)
    }

    pub fn evaluate(&self) -> Result<String, BridgeError> {
        let c = self.cur()?;
        let ev = HeuristicEvaluator::default().evaluate_detailed(&c.state);
        to_json(&report::evaluation(c, &ev))
    }

    pub fn evaluation(&self) -> Result<EvaluationReport, BridgeError> {
        let c = self.cur()?;
        Ok(report::evaluation(c, &HeuristicEvaluator::default().evaluate_detailed(&c.state)))
    }

    pub fn best_turn_report(&self, token_id: &str, opts: TurnOptions, budget: &dyn Budget) -> Result<TurnReport, BridgeError> {
        let c = self.cur()?;
        let id = c.ids.id(token_id).ok_or_else(|| BridgeError::new(ErrorCode::UnknownToken, format!("token {token_id} is not in the analysed position")))?;
        report::best_turn(c, id, opts.config(), budget)
            .ok_or_else(|| BridgeError::new(ErrorCode::CannotAct, format!("{} cannot act right now", c.state.combatant(id).map(|x| x.name.as_str()).unwrap_or(token_id))))
    }

    pub fn best_turn(&self, token_id: &str, options_json: &str, budget: &dyn Budget) -> Result<String, BridgeError> {
        let opts: TurnOptions = if options_json.trim().is_empty() { TurnOptions::default() } else { parse(options_json, "options")? };
        to_json(&self.best_turn_report(token_id, opts, budget)?)
    }

    /// `budgets` is called once per creature, so each gets a fresh limit.
    pub fn side_plan_report(&self, side: Side, opts: TurnOptions, budgets: BudgetFactory) -> Result<SidePlanReport, BridgeError> {
        Ok(report::side_plan(self.cur()?, side, opts.config(), budgets))
    }

    /// GM assistant for the monsters (`side` = `"enemy"`, default) or party.
    pub fn side_plan(&self, side_json: &str, options_json: &str, budgets: BudgetFactory) -> Result<String, BridgeError> {
        let side: Side = if side_json.trim().is_empty() { Side::Enemy } else { parse(side_json, "side")? };
        let opts: TurnOptions = if options_json.trim().is_empty() { TurnOptions::default() } else { parse(options_json, "options")? };
        to_json(&self.side_plan_report(side, opts, budgets)?)
    }

    /// Runs one slice of the Monte Carlo forecast; returns a raw tally that
    /// [`Bridge::merge_forecast`] combines (possibly from several workers).
    pub fn forecast_slice(&self, options_json: &str) -> Result<String, BridgeError> {
        let o: ForecastOptions = parse(options_json, "forecast options")?;
        let runs = o.runs.clamp(1, 20_000);
        let cfg = SimulationConfig { runs, seed: o.seed, max_rounds: o.max_rounds.clamp(1, 100) };
        let count = o.count.unwrap_or(runs).min(runs.saturating_sub(o.start));
        to_json(&simulate_range(&self.cur()?.state, cfg, o.start, count))
    }

    pub fn merge_forecast_report(&self, tallies: &[SimulationTally]) -> Result<ForecastReport, BridgeError> {
        let mut total = SimulationTally::default();
        for t in tallies {
            total.merge(t);
        }
        Ok(report::forecast(self.cur()?, &total.report()))
    }

    pub fn merge_forecast(&self, tallies_json: &str) -> Result<String, BridgeError> {
        let tallies: Vec<SimulationTally> = parse(tallies_json, "tallies")?;
        to_json(&self.merge_forecast_report(&tallies)?)
    }

    pub fn explain_report(&self, max: usize) -> Result<ExplainReport, BridgeError> {
        let c = self.cur()?;
        let ev = HeuristicEvaluator::default();
        let ex = match &self.previous {
            Some(p) => explain_change(&p.state, &c.state, &ev, max),
            None => explain_position(&c.state, &ev, max),
        };
        Ok(report::explanation(c, ex))
    }

    /// Why the evaluation changed since the previous snapshot (or, without
    /// one, why it is what it is).
    pub fn explain(&self, max: usize) -> Result<String, BridgeError> {
        to_json(&self.explain_report(max.clamp(1, 20))?)
    }
}

/// Budget factory for callers without a clock.
pub fn unlimited() -> Box<dyn Budget> {
    Box::new(Unlimited)
}
