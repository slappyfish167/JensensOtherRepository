// Keeps the engine in sync with Foundry and answers on-demand analyses.
//
// * Hooks call `invalidate()`; a debounce collapses bursts (a Strike fires
//   chat, actor and item hooks within milliseconds) into one refresh.
// * A refresh builds the snapshot, skips the engine entirely if the JSON is
//   unchanged, otherwise loads it and re-evaluates.
// * Refreshes never overlap: a refresh requested while one runs is
//   coalesced into a single follow-up.
// * On-demand analyses (best turn, GM plan, forecast, explain) first make
//   sure the engine holds the latest position.

import type { EngineClient } from '../engine/client';
import { EngineError } from '../engine/client';
import type { EvaluationReport, ExplainReport, ForecastReport, LoadSummary, SidePlanReport, TurnReport } from '../protocol/report';
import type { CombatSnapshot } from '../protocol/snapshot';

export interface ServiceDeps {
  /** Builds the snapshot for this client's viewer, or null if nothing to analyse. */
  snapshot(): Promise<{ snapshot: CombatSnapshot; warnings: string[] } | null>;
  /** Whether this client computes the evaluation bar itself. */
  evaluatesLocally(): boolean;
  searchMs(): number;
  monsterSearchMs(): number;
  forecastRuns(): number;
  debounceMs?: number;
}

export type EvaluationListener = (report: EvaluationReport | null, summary: LoadSummary | null) => void;

export class AnalysisService {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running: Promise<void> | null = null;
  private again = false;
  private lastJson: string | null = null;
  private listeners = new Set<EvaluationListener>();
  lastEvaluation: EvaluationReport | null = null;
  lastSummary: LoadSummary | null = null;
  lastError: EngineError | null = null;
  refreshes = 0;
  engineLoads = 0;

  constructor(
    private readonly engine: EngineClient,
    private readonly deps: ServiceDeps,
  ) {}

  onEvaluation(fn: EvaluationListener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private emit(): void {
    for (const fn of this.listeners) fn(this.lastEvaluation, this.lastSummary);
  }

  /** Schedules a refresh (debounced). */
  invalidate(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.refresh();
    }, this.deps.debounceMs ?? 150);
  }

  /** Brings the engine up to date now; resolves when done. */
  refresh(): Promise<void> {
    if (this.running) {
      this.again = true;
      return this.running;
    }
    this.running = (async () => {
      do {
        this.again = false;
        await this.refreshOnce();
      } while (this.again);
    })().finally(() => {
      this.running = null;
    });
    return this.running;
  }

  private async refreshOnce(): Promise<void> {
    this.refreshes += 1;
    try {
      const built = await this.deps.snapshot();
      if (!built) {
        this.lastJson = null;
        this.lastEvaluation = null;
        this.lastSummary = null;
        this.emit();
        return;
      }
      const json = JSON.stringify(built.snapshot);
      if (json === this.lastJson) return;
      this.lastSummary = await this.engine.load(json);
      this.lastSummary.warnings = [...built.warnings, ...this.lastSummary.warnings];
      this.lastJson = json;
      this.engineLoads += 1;
      this.lastError = null;
      this.lastEvaluation = this.deps.evaluatesLocally() ? await this.engine.evaluate() : null;
      this.emit();
    } catch (e) {
      this.lastError = e instanceof EngineError ? e : new EngineError('internal', String(e));
      this.lastJson = null;
      this.lastEvaluation = null;
      this.emit();
    }
  }

  private async ready(): Promise<void> {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    await this.refresh();
    if (!this.lastJson) throw this.lastError ?? new EngineError('nothing_to_analyse', 'nothing to analyse on this scene');
  }

  async bestTurn(tokenId: string): Promise<TurnReport> {
    await this.ready();
    return this.engine.bestTurn(tokenId, this.deps.searchMs());
  }

  async sidePlan(side: 'enemy' | 'party' = 'enemy'): Promise<SidePlanReport> {
    await this.ready();
    return this.engine.sidePlan(side, this.deps.monsterSearchMs());
  }

  async forecast(seed = 1): Promise<ForecastReport> {
    await this.ready();
    return this.engine.forecast(this.deps.forecastRuns(), seed);
  }

  async explain(max = 6): Promise<ExplainReport> {
    await this.ready();
    return this.engine.explain(max);
  }
}
