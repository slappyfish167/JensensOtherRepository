import { beforeAll, describe, expect, it } from 'vitest';
import { AnalysisService } from '../src/analysis/service';
import { EngineClient, EngineError } from '../src/engine/client';
import type { CombatSnapshot } from '../src/protocol/snapshot';
import { fixtureJson, InProcessWorker, loadWasm } from './helpers';

beforeAll(loadWasm);

const client = (forecastWorkers = 0, timeoutMs = 20_000, silent = false) =>
  new EngineClient(() => new InProcessWorker({ silent }), { forecastWorkers, timeoutMs });

describe('EngineClient with the real WASM engine', () => {
  it('runs every analysis on the shared fixture', async () => {
    const e = client();
    expect(await e.start()).toMatch(/\+snapshot\.1$/);
    const summary = await e.load(fixtureJson());
    expect(summary.turn_active).toBe(true);
    const ev = await e.evaluate();
    expect(ev.score).toBeGreaterThanOrEqual(-10);
    expect(ev.score).toBeLessThanOrEqual(10);
    const turn = await e.bestTurn(summary.current_token!, 0);
    expect(turn.steps.length).toBeGreaterThan(0);
    expect(turn.hypothetical).toBe(false);
    const plan = await e.sidePlan('enemy', 0);
    expect(plan.creatures.length).toBe(3);
    const ex = await e.explain(5);
    expect(ex.previous_score).toBeNull();
    e.dispose();
  });

  it('splitting a forecast across workers gives the identical result', async () => {
    const single = client(0);
    const split = client(3);
    await single.load(fixtureJson());
    await split.load(fixtureJson());
    const a = await single.forecast(60, 11, 10);
    const b = await split.forecast(60, 11, 10);
    expect(b).toEqual(a);
    expect(a.runs).toBe(60);
    single.dispose();
    split.dispose();
  });

  it('maps engine errors to codes', async () => {
    const e = client();
    await expect(e.evaluate()).rejects.toMatchObject({ code: 'no_snapshot' });
    await e.load(fixtureJson());
    await expect(e.bestTurn('nope', 0)).rejects.toBeInstanceOf(EngineError);
    await expect(e.bestTurn('nope', 0)).rejects.toMatchObject({ code: 'unknown_token' });
    const bad = JSON.parse(fixtureJson()) as CombatSnapshot;
    bad.scene.grid = 'hex';
    await expect(e.load(JSON.stringify(bad))).rejects.toMatchObject({ code: 'unsupported_scene' });
    e.dispose();
  });

  it('times out a hung worker and restarts it', async () => {
    const e = client(0, 50, true);
    await expect(e.load(fixtureJson())).rejects.toMatchObject({ code: 'timeout' });
    expect(e.restarts).toBe(1);
    e.dispose();
  });
});

describe('AnalysisService', () => {
  it('debounces, skips unchanged positions and re-evaluates changes', async () => {
    const e = client();
    const snap = JSON.parse(fixtureJson()) as CombatSnapshot;
    let current = snap;
    let builds = 0;
    const svc = new AnalysisService(e, {
      snapshot: async () => {
        builds++;
        return { snapshot: current, warnings: [] };
      },
      evaluatesLocally: () => true,
      searchMs: () => 100,
      monsterSearchMs: () => 200,
      forecastRuns: () => 20,
      debounceMs: 5,
    });
    const seen: number[] = [];
    svc.onEvaluation((r) => r && seen.push(r.score));
    for (let i = 0; i < 10; i++) svc.invalidate();
    await new Promise((r) => setTimeout(r, 40));
    await svc.refresh();
    expect(builds).toBe(2); // one debounced burst + one explicit refresh
    expect(svc.engineLoads).toBe(1); // second build was identical: engine untouched
    // Wound a goblin: the evaluation must move toward the party.
    current = structuredClone(snap);
    const gob = current.combatants.find((c) => c.name.startsWith('Goblin Cutter 1'))!;
    gob.hp = 1;
    await svc.refresh();
    expect(svc.engineLoads).toBe(2);
    expect(seen.length).toBe(2);
    expect(seen[1]!).toBeGreaterThan(seen[0]!);
    const ex = await svc.explain(3);
    expect(ex.reasons[0]?.token_id).toBe(gob.token_id);
    e.dispose();
  });
});
