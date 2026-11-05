// Maps an RPC request onto the WASM `Engine` API. Shared by the worker and
// by tests, which run the real WASM engine in-process.

import type { EngineOp } from '../protocol/rpc';

/** The subset of the wasm-bindgen `Engine` class the module uses. */
export interface EngineLike {
  load(snapshot: string): string;
  evaluate(): string;
  bestTurn(token: string, options: string, deadlineMs: number): string;
  sidePlan(side: string, options: string, deadlineMs: number): string;
  forecastSlice(options: string): string;
  mergeForecast(tallies: string): string;
  explain(max: number): string;
}

export function dispatch(engine: EngineLike, req: EngineOp, version: () => string): string {
  switch (req.op) {
    case 'load':
      return engine.load(req.snapshot);
    case 'evaluate':
      return engine.evaluate();
    case 'bestTurn':
      return engine.bestTurn(req.token, req.options, req.deadlineMs);
    case 'sidePlan':
      return engine.sidePlan(req.side, req.options, req.deadlineMs);
    case 'forecastSlice':
      return engine.forecastSlice(req.options);
    case 'mergeForecast':
      return engine.mergeForecast(req.tallies);
    case 'explain':
      return engine.explain(req.max);
    case 'version':
      return JSON.stringify(version());
  }
}
