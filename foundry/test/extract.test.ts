import { beforeAll, describe, expect, it } from 'vitest';
import { Engine } from '../wasm-pkg/pf2e_stockfish.js';
import { extractCombatant, sneakAttackDice } from '../src/extract/actor';
import { baseValue } from '../src/extract/modifiers';
import { extractScene, wallSnapshot, geometry, sampleTerrain } from '../src/extract/scene';
import { buildSnapshot } from '../src/extract/snapshot';
import { parseFeet, parseTargets } from '../src/extract/spells';
import type { LoadSummary } from '../src/protocol/report';
import type { CombatLike, SceneLike } from '../src/types/pf2e';
import { fakeToken, loadWasm } from './helpers';

const scene: SceneLike = {
  id: 'sc1',
  name: 'Test',
  grid: { type: 1, size: 100, distance: 5 },
  dimensions: { sceneX: 0, sceneY: 0, sceneWidth: 2000, sceneHeight: 1500, size: 100, distance: 5 },
  walls: [
    { c: [800, 0, 800, 500], move: 20, sight: 20, door: 0, ds: 0 },
    { c: [800, 500, 800, 600], move: 20, sight: 20, door: 1, ds: 1 },
  ],
  tokens: [],
};

describe('modifier stripping', () => {
  it('removes only engine-applied, enabled modifiers', () => {
    const v = baseValue(10, [
      { slug: 'frightened', modifier: -2, enabled: true },
      { slug: 'bless', modifier: 1, enabled: true },
      { slug: 'off-guard', modifier: -2, enabled: false },
      { slug: 'clumsy', modifier: -1, enabled: true, ignored: true },
    ]);
    expect(v).toBe(12);
  });
});

describe('combatant extraction', () => {
  it('reads a PF2e actor into base numbers plus conditions', async () => {
    const t = fakeToken({ id: 't1', name: 'Kira', type: 'character', ac: 18, attack: 9, frightenedValue: 2, spells: true, shield: true });
    const { combatant, warnings } = await extractCombatant(t, { square: [3, 4], initiative: 17, reactionUsed: false });
    expect(warnings).toEqual([]);
    expect(combatant).not.toBeNull();
    const c = combatant!;
    expect(c.side).toBe('party');
    expect(c.uses_dying_rules).toBe(true);
    // Totals had frightened 2 folded in; the snapshot carries the base and the condition.
    expect(c.ac).toBe(18);
    expect(c.strikes?.[0]?.attack_bonus).toBe(9);
    expect(c.fortitude).toBe(7);
    expect(c.conditions).toEqual([{ slug: 'frightened', value: 2 }]);
    expect(c.strikes?.[0]?.damage_formula).toBe('{(1d8 + 4)[slashing]}');
    expect(c.reactions).toEqual({ reactive_strike: false, shield_block: true });
    expect(c.shield?.raised).toBe(true);
    const sc = c.spellcasting?.[0];
    expect(sc?.slots).toEqual({ '1': 1 }); // one unexpended prepared copy
    expect(sc?.spells?.map((s) => s.slug)).toEqual(['electric-arc', 'force-barrage']); // unprepared Fear excluded
  });

  it('skips non-combatant actors and caches strike formulas per actor', async () => {
    const loot = fakeToken({ id: 'l', name: 'Chest' });
    loot.actor!.type = 'loot';
    expect((await extractCombatant(loot, { square: [0, 0], initiative: null, reactionUsed: false })).combatant).toBeNull();
    const t = fakeToken({ id: 'g', name: 'Goblin' });
    let calls = 0;
    const strike = t.actor!.system!.actions![0]!;
    strike.damage = async () => {
      calls++;
      return '1d6+1';
    };
    const cache = new Map();
    await extractCombatant(t, { square: [0, 0], initiative: null, reactionUsed: false, strikeCache: cache });
    await extractCombatant(t, { square: [0, 0], initiative: null, reactionUsed: false, strikeCache: cache });
    expect(calls).toBe(1);
  });

  it('parses spell text fields', () => {
    expect(parseFeet('30 feet')).toBe(30);
    expect(parseFeet('touch')).toBe(5);
    expect(parseTargets('up to 3 creatures').count).toBe(3);
    expect(parseTargets('1 willing creature').allies).toBe(true);
    expect(sneakAttackDice(5)).toBe('2d6');
  });
});

describe('scene extraction', () => {
  it('converts walls to squares and respects open doors', () => {
    const s = extractScene(scene);
    expect(s.grid).toBe('square');
    expect(s.width).toBe(20);
    expect(s.walls?.[0]).toMatchObject({ a: [8, 0], b: [8, 5], blocks_movement: true, open_door: false });
    expect(s.walls?.[1]?.open_door).toBe(true);
    expect(wallSnapshot({ c: [0, 0] }, geometry(scene))).toBeNull();
  });

  it('samples terrain at square centres inside a window', () => {
    const cells = sampleTerrain(geometry(scene), { x0: 0, y0: 0, x1: 4, y1: 1 }, (px) => (px < 200 ? 'difficult' : null));
    expect(cells).toEqual([
      { x: 0, y: 0, kind: 'difficult' },
      { x: 1, y: 0, kind: 'difficult' },
    ]);
  });
});

describe('snapshot → Rust engine contract', () => {
  beforeAll(loadWasm);

  const tokens = () => [
    fakeToken({ id: 'hero', name: 'Hero', type: 'character', x: 3, y: 3, ac: 18, attack: 9, spells: true, shield: true }),
    fakeToken({ id: 'orc', name: 'Orc', x: 6, y: 3, hp: 30, maxHp: 30, attack: 8 }),
    fakeToken({ id: 'spy', name: 'Hidden Spy', x: 2, y: 8, hidden: true }),
  ];

  function combat(ts: ReturnType<typeof tokens>): CombatLike {
    const turns = ts.map((t, i) => ({ id: `c${i}`, tokenId: t.id, token: t, initiative: 20 - i, defeated: false }));
    return { id: 'cb', started: true, round: 1, turn: 0, combatant: turns[0], turns, scene: { id: 'sc1' } };
  }

  it('the engine accepts what the extractor produces', async () => {
    const ts = tokens();
    const { snapshot, warnings } = await buildSnapshot({
      scene: { ...scene, tokens: ts },
      combat: combat(ts),
      turn: { combatId: 'cb', round: 1, combatantToken: 'hero', actionsSpent: 1, attacksMade: 1, reactionsUsed: [] },
      viewer: { isGM: true, canSee: () => true },
      options: { flanking: true, autoCover: true, cropMargin: 12 },
    });
    expect(warnings).toEqual([]);
    expect(snapshot.turn).toMatchObject({ started: true, current: 'hero', actions_spent: 1, attacks_made: 1 });
    const engine = new Engine();
    const summary = JSON.parse(engine.load(JSON.stringify(snapshot))) as LoadSummary;
    expect(summary.combatants).toBe(3);
    expect(summary.turn_active).toBe(true);
    expect(summary.wall_edges).toBeGreaterThan(0);
    // MAP and spent actions carried into the engine: two actions left.
    const turn = JSON.parse(engine.bestTurn('hero', '', 0)) as { steps: unknown[] };
    expect(turn.steps.length).toBeLessThanOrEqual(2);
  });

  it('players never send hidden or unseen tokens', async () => {
    const ts = tokens();
    const res = await buildSnapshot({
      scene: { ...scene, tokens: ts },
      combat: combat(ts),
      turn: null,
      viewer: { isGM: false, canSee: (t) => t.id !== 'orc' },
      options: { flanking: true, autoCover: true, cropMargin: 12 },
    });
    expect(res.snapshot.combatants.map((c) => c.token_id)).toEqual(['hero']);
    expect(res.withheld).toBe(2);
  });
});
