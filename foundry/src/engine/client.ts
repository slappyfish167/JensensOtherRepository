// Promise-based client for the analysis workers.
//
// * One **primary** worker serves interactive requests (evaluation, best
//   turn, GM plan, explain) so they never queue behind a forecast.
// * **Forecast** workers split Monte Carlo runs; per-run seeds make the
//   merged result identical to a single-threaded run.
// * Every worker keeps the last snapshot it loaded; after a crash or a
//   timeout the client restarts it and replays that snapshot.

import type { EngineErrorShape, ErrorCode, EvaluationReport, ExplainReport, ForecastReport, LoadSummary, SidePlanReport, SimulationTally, TurnReport } from '../protocol/report';
import { isHello, type EngineOp, type WorkerResponse } from '../protocol/rpc';

export class EngineError extends Error {
  constructor(
    readonly code: ErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'EngineError';
  }

  static from(raw: string): EngineError {
    try {
      const e = JSON.parse(raw) as EngineErrorShape;
      if (e && typeof e.code === 'string') return new EngineError(e.code, e.message);
    } catch {
      /* not JSON */
    }
    return new EngineError('internal', raw);
  }
}

/** What the client needs from a Worker (lets tests inject a fake). */
export interface WorkerLike {
  postMessage(msg: unknown): void;
  terminate(): void;
  onmessage: ((ev: MessageEvent) => void) | null;
  onerror: ((ev: ErrorEvent | Event) => void) | null;
}

export type WorkerFactory = () => WorkerLike;

interface Pending {
  resolve: (v: string) => void;
  reject: (e: EngineError) => void;
  timer: ReturnType<typeof setTimeout> | undefined;
}

/** One worker with request/response bookkeeping and self-healing. */
export class WorkerChannel {
  private worker: WorkerLike | null = null;
  private ready: Promise<string> | null = null;
  private pending = new Map<number, Pending>();
  private nextId = 1;
  private lastSnapshot: string | null = null;
  restarts = 0;

  constructor(
    private readonly factory: WorkerFactory,
    private readonly timeoutMs: number,
  ) {}

  start(): Promise<string> {
    if (this.ready) return this.ready;
    const worker = this.factory();
    this.worker = worker;
    this.ready = new Promise<string>((resolve, reject) => {
      worker.onmessage = (ev: MessageEvent) => {
        const data: unknown = ev.data;
        if (isHello(data)) {
          if ('failed' in data) reject(new EngineError('worker_unavailable', data.failed));
          else resolve(data.version);
          return;
        }
        this.onResponse(data as WorkerResponse);
      };
      worker.onerror = (ev) => {
        const msg = 'message' in ev ? String((ev as ErrorEvent).message) : 'worker error';
        reject(new EngineError('worker_unavailable', msg));
        this.failAll(new EngineError('worker_unavailable', msg));
        this.reset();
      };
    });
    return this.ready;
  }

  private onResponse(r: WorkerResponse): void {
    const p = this.pending.get(r.id);
    if (!p) return;
    this.pending.delete(r.id);
    if (p.timer) clearTimeout(p.timer);
    if (r.ok) p.resolve(r.result);
    else {
      p.reject(EngineError.from(r.error));
      if (r.restarted) this.reset();
    }
  }

  private failAll(e: EngineError): void {
    for (const [, p] of this.pending) {
      if (p.timer) clearTimeout(p.timer);
      p.reject(e);
    }
    this.pending.clear();
  }

  /** Drops the worker; the next call starts a fresh one. */
  private reset(): void {
    this.worker?.terminate();
    this.worker = null;
    this.ready = null;
    this.restarts += 1;
  }

  async call(op: EngineOp, timeoutMs = this.timeoutMs): Promise<string> {
    if (!this.worker) {
      await this.start();
      // Restore the position after a restart (not for the load itself).
      if (this.lastSnapshot && op.op !== 'load') await this.send({ op: 'load', snapshot: this.lastSnapshot }, timeoutMs);
    } else {
      await this.start();
    }
    if (op.op === 'load') this.lastSnapshot = op.snapshot;
    return this.send(op, timeoutMs);
  }

  private send(op: EngineOp, timeoutMs: number): Promise<string> {
    const id = this.nextId++;
    return new Promise<string>((resolve, reject) => {
      const timer =
        timeoutMs > 0
          ? setTimeout(() => {
              this.pending.delete(id);
              // A runaway search cannot be interrupted inside WASM: kill it.
              this.failAll(new EngineError('timeout', `analysis took longer than ${timeoutMs} ms`));
              reject(new EngineError('timeout', `analysis took longer than ${timeoutMs} ms`));
              this.reset();
            }, timeoutMs)
          : undefined;
      this.pending.set(id, { resolve, reject, timer });
      this.worker?.postMessage({ id, ...op });
    });
  }

  dispose(): void {
    this.failAll(new EngineError('cancelled', 'engine shut down'));
    this.worker?.terminate();
    this.worker = null;
    this.ready = null;
  }
}

export interface EngineOptions {
  /** Workers dedicated to forecasts (0 = share the primary worker). */
  forecastWorkers: number;
  /** Hard timeout for any single request. */
  timeoutMs: number;
}

/** Typed façade over the primary and forecast workers. */
export class EngineClient {
  private readonly primary: WorkerChannel;
  private readonly forecasters: WorkerChannel[];
  private snapshot: string | null = null;
  private forecastLoaded = new WeakSet<WorkerChannel>();

  constructor(factory: WorkerFactory, opts: EngineOptions) {
    this.primary = new WorkerChannel(factory, opts.timeoutMs);
    this.forecasters = Array.from({ length: Math.max(0, opts.forecastWorkers) }, () => new WorkerChannel(factory, opts.timeoutMs * 4));
  }

  start(): Promise<string> {
    return this.primary.start();
  }

  get restarts(): number {
    return this.primary.restarts + this.forecasters.reduce((n, f) => n + f.restarts, 0);
  }

  async load(snapshotJson: string): Promise<LoadSummary> {
    this.snapshot = snapshotJson;
    this.forecastLoaded = new WeakSet();
    return JSON.parse(await this.primary.call({ op: 'load', snapshot: snapshotJson })) as LoadSummary;
  }

  async evaluate(): Promise<EvaluationReport> {
    return JSON.parse(await this.primary.call({ op: 'evaluate' })) as EvaluationReport;
  }

  async bestTurn(token: string, deadlineMs: number, options: { node_budget?: number; beam?: number } = {}): Promise<TurnReport> {
    return JSON.parse(await this.primary.call({ op: 'bestTurn', token, options: JSON.stringify(options), deadlineMs })) as TurnReport;
  }

  async sidePlan(side: 'enemy' | 'party', deadlineMsPerCreature: number): Promise<SidePlanReport> {
    return JSON.parse(await this.primary.call({ op: 'sidePlan', side: JSON.stringify(side), options: '', deadlineMs: deadlineMsPerCreature })) as SidePlanReport;
  }

  async explain(max = 6): Promise<ExplainReport> {
    return JSON.parse(await this.primary.call({ op: 'explain', max })) as ExplainReport;
  }

  /** Monte Carlo forecast split across forecast workers (or the primary). */
  async forecast(runs: number, seed: number, maxRounds = 20): Promise<ForecastReport> {
    const snapshot = this.snapshot;
    if (!snapshot) throw new EngineError('no_snapshot', 'load a snapshot first');
    const workers = this.forecasters.length ? this.forecasters : [this.primary];
    const per = Math.ceil(runs / workers.length);
    const slices = workers.map(async (w, i) => {
      const start = i * per;
      const count = Math.max(0, Math.min(per, runs - start));
      if (count === 0) return null;
      if (w !== this.primary && !this.forecastLoaded.has(w)) {
        await w.call({ op: 'load', snapshot });
        this.forecastLoaded.add(w);
      }
      return w.call({ op: 'forecastSlice', options: JSON.stringify({ runs, seed, max_rounds: maxRounds, start, count }) });
    });
    const tallies = (await Promise.all(slices)).filter((t): t is string => t !== null);
    const merged: SimulationTally[] = tallies.map((t) => JSON.parse(t) as SimulationTally);
    return JSON.parse(await this.primary.call({ op: 'mergeForecast', tallies: JSON.stringify(merged) })) as ForecastReport;
  }

  dispose(): void {
    this.primary.dispose();
    for (const f of this.forecasters) f.dispose();
  }
}
