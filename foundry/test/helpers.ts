// Test helpers: the real WASM engine behind an in-process "worker", and
// PF2e-shaped fake documents.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import init, { Engine, version } from '../wasm-pkg/pf2e_stockfish.js';
import type { WorkerLike } from '../src/engine/client';
import { dispatch, type EngineLike } from '../src/engine/dispatch';
import type { WorkerRequest } from '../src/protocol/rpc';
import type { ActorLike, ModifierLike, StatisticLike, TokenDocLike } from '../src/types/pf2e';

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

let ready: Promise<unknown> | null = null;
export function loadWasm(): Promise<unknown> {
  ready ??= init({ module_or_path: readFileSync(here('../wasm-pkg/pf2e_stockfish_bg.wasm')) });
  return ready;
}

export const fixtureJson = (): string => readFileSync(here('./fixtures/crypt-ambush.snapshot.json'), 'utf8');

/** A Worker stand-in that runs the real engine on the same thread. */
export class InProcessWorker implements WorkerLike {
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: ((ev: ErrorEvent | Event) => void) | null = null;
  terminated = false;
  private engine: EngineLike = new Engine() as unknown as EngineLike;

  constructor(private readonly opts: { silent?: boolean } = {}) {
    setTimeout(() => this.onmessage?.({ data: { hello: true, version: version() } } as MessageEvent), 0);
  }

  postMessage(msg: unknown): void {
    const req = msg as WorkerRequest;
    if (this.opts.silent) return; // simulates a hung search
    setTimeout(() => {
      if (this.terminated) return;
      try {
        const result = dispatch(this.engine, req, version);
        this.onmessage?.({ data: { id: req.id, ok: true, result } } as MessageEvent);
      } catch (e) {
        this.onmessage?.({ data: { id: req.id, ok: false, error: typeof e === 'string' ? e : String(e) } } as MessageEvent);
      }
    }, 0);
  }

  terminate(): void {
    this.terminated = true;
  }
}

// ---------------------------------------------------------------------------
// PF2e-shaped fakes
// ---------------------------------------------------------------------------

export function stat(total: number, modifiers: ModifierLike[] = []): StatisticLike {
  return { mod: total, value: total, modifiers, check: { mod: total, modifiers }, dc: { value: total, modifiers } };
}

const frightened = (v: number): ModifierLike => ({ slug: 'frightened', modifier: -v, type: 'status', enabled: true });

export interface FakeOpts {
  id: string;
  name: string;
  type?: 'character' | 'npc' | 'hazard';
  level?: number;
  hp?: number;
  maxHp?: number;
  ac?: number;
  attack?: number;
  formula?: string;
  frightenedValue?: number;
  x?: number;
  y?: number;
  disposition?: number;
  hidden?: boolean;
  spells?: boolean;
  shield?: boolean;
}

/** Token + actor shaped like PF2e 7.x documents (only fields the extractor reads). */
export function fakeToken(o: FakeOpts): TokenDocLike {
  const fr = o.frightenedValue ?? 0;
  const mods = fr ? [frightened(fr)] : [];
  const conditions = fr ? [{ slug: 'frightened', value: fr, active: true }] : [];
  const actor: ActorLike = {
    id: `a-${o.id}`,
    uuid: `Actor.a-${o.id}`,
    name: o.name,
    type: o.type ?? 'npc',
    level: o.level ?? 1,
    size: 'med',
    hasPlayerOwner: o.type === 'character',
    hitPoints: { value: o.hp ?? 20, max: o.maxHp ?? 20, temp: 0 },
    getStatistic: (slug) => (slug === 'ac' ? stat((o.ac ?? 16) - fr, mods) : null),
    saves: { fortitude: stat(7 - fr, mods), reflex: stat(5 - fr, mods), will: stat(4 - fr, mods) },
    perception: stat(6 - fr, mods),
    skills: { athletics: stat(7 - fr, mods), intimidation: stat(3 - fr, mods) },
    abilities: { str: { mod: 4 }, dex: { mod: 2 }, con: { mod: 2 }, int: { mod: 0 }, wis: { mod: 1 }, cha: { mod: 0 } },
    getReach: () => 5,
    conditions,
    items: o.shield ? [{ slug: 'shield-block', type: 'feat' }] : [],
    attributes: {
      shield: o.shield ? { name: 'Steel Shield', ac: 2, hardness: 5, hp: { value: 20, max: 20 }, brokenThreshold: 10, raised: true, itemId: 'sh1' } : null,
      immunities: [],
      weaknesses: [],
      resistances: [],
    },
    system: {
      attributes: { speed: { total: 25 } },
      actions: [
        {
          type: 'strike',
          label: 'Longsword',
          ready: true,
          totalModifier: (o.attack ?? 9) - fr,
          modifiers: mods,
          item: { name: 'Longsword', isMelee: true, system: { traits: { value: ['versatile-p'] }, damage: { damageType: 'slashing' } } },
          damage: async () => o.formula ?? '{(1d8 + 4)[slashing]}',
        },
      ],
    },
    spellcasting: o.spells
      ? {
          regular: [
            {
              name: 'Arcane Prepared Spells',
              tradition: 'arcane',
              category: 'prepared',
              attribute: 'int',
              statistic: { check: { mod: 7, modifiers: [] }, dc: { value: 17, modifiers: [] } },
              system: { slots: { slot0: { prepared: [{ id: 's-arc' }] }, slot1: { prepared: [{ id: 's-fb', expended: false }, { id: 's-fb', expended: true }] } } },
              spells: [
                { id: 's-arc', slug: 'electric-arc', name: 'Electric Arc', rank: 1, isCantrip: true, system: { time: { value: '2' }, range: { value: '30 feet' } } },
                { id: 's-fb', slug: 'force-barrage', name: 'Force Barrage', rank: 1, system: { time: { value: '2' }, range: { value: '120 feet' } } },
                { id: 's-unprepared', slug: 'fear', name: 'Fear', rank: 1, system: { time: { value: '2' } } },
              ],
            },
          ],
        }
      : null,
  };
  return { id: o.id, name: o.name, x: (o.x ?? 0) * 100, y: (o.y ?? 0) * 100, hidden: !!o.hidden, disposition: o.disposition ?? (o.type === 'character' ? 1 : -1), actor };
}
