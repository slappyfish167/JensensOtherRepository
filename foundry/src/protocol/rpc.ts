// Main thread ⇄ analysis worker messages.
//
// Every request carries an id; the worker answers exactly once with the
// same id. Payloads are JSON strings produced/consumed by the WASM engine,
// passed through untouched (no double parsing in the worker).

export type EngineOp =
  | { op: 'load'; snapshot: string }
  | { op: 'evaluate' }
  | { op: 'bestTurn'; token: string; options: string; deadlineMs: number }
  | { op: 'sidePlan'; side: string; options: string; deadlineMs: number }
  | { op: 'forecastSlice'; options: string }
  | { op: 'mergeForecast'; tallies: string }
  | { op: 'explain'; max: number }
  | { op: 'version' };

export type WorkerRequest = { id: number } & EngineOp;

export type WorkerResponse =
  | { id: number; ok: true; result: string }
  | { id: number; ok: false; error: string; restarted?: boolean };

/** Sent once by the worker when the WASM module is ready (or failed). */
export type WorkerHello = { hello: true; version: string } | { hello: true; failed: string };

export function isHello(m: unknown): m is WorkerHello {
  return typeof m === 'object' && m !== null && (m as { hello?: unknown }).hello === true;
}
