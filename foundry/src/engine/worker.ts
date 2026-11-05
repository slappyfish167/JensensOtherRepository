// Analysis worker: owns one WASM engine instance.
//
// Lifecycle: boot → post hello → serve requests in order. A WebAssembly
// trap (Rust panic) leaves the instance unusable: the worker reports the
// panic message, flags `restarted`, and closes itself; the client spawns a
// fresh worker and replays the last snapshot.

import init, { Engine, lastPanic, version } from '../../wasm-pkg/pf2e_stockfish.js';
import type { WorkerRequest, WorkerResponse } from '../protocol/rpc';
import { dispatch, type EngineLike } from './dispatch';

declare const self: DedicatedWorkerGlobalScope;

let engine: EngineLike | null = null;

function reply(msg: WorkerResponse): void {
  self.postMessage(msg);
}

async function boot(): Promise<void> {
  try {
    await init();
    engine = new Engine() as unknown as EngineLike;
    self.postMessage({ hello: true, version: version() });
  } catch (e) {
    self.postMessage({ hello: true, failed: String(e) });
  }
}

self.onmessage = (ev: MessageEvent<WorkerRequest>) => {
  const req = ev.data;
  if (!engine) {
    reply({ id: req.id, ok: false, error: JSON.stringify({ code: 'worker_unavailable', message: 'engine not initialised' }) });
    return;
  }
  try {
    reply({ id: req.id, ok: true, result: dispatch(engine, req, version) });
  } catch (e) {
    if (e instanceof WebAssembly.RuntimeError) {
      const panic = lastPanic() ?? e.message;
      reply({ id: req.id, ok: false, restarted: true, error: JSON.stringify({ code: 'internal', message: `engine panic: ${panic}` }) });
      engine = null;
      self.close();
      return;
    }
    // Engine errors are thrown as JSON strings by the WASM layer.
    reply({ id: req.id, ok: false, error: typeof e === 'string' ? e : JSON.stringify({ code: 'internal', message: String(e) }) });
  }
};

void boot();
