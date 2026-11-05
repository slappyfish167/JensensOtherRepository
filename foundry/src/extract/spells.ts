// PF2e spellcasting entries → snapshot spellcasting.
//
// Only data is read here; how spells resolve is decided by the engine
// (library definitions by slug, or translation of the structured damage
// data in Rust — see crates/pf2e-bridge/src/spells.rs).

import type { Attribute, Save, SpellSnapshot, SpellcastingSnapshot } from '../protocol/snapshot';
import { toArray, type ActorLike, type SpellLike, type SpellcastingEntryLike } from '../types/pf2e';
import { checkBase, dcBase } from './modifiers';

const ATTRIBUTES: Record<string, Attribute> = {
  str: 'strength',
  dex: 'dexterity',
  con: 'constitution',
  int: 'intelligence',
  wis: 'wisdom',
  cha: 'charisma',
};

const SAVES: Record<string, Save> = { fortitude: 'fortitude', reflex: 'reflex', will: 'will' };

export function parseFeet(text: string | undefined): number | null {
  if (!text) return null;
  const t = text.toLowerCase();
  if (t.includes('touch')) return 5;
  const m = /(\d+)\s*(?:feet|foot|ft)/.exec(t);
  return m?.[1] ? Number.parseInt(m[1], 10) : null;
}

export function parseActions(time: string | undefined): number | null {
  return time && /^[123]$/.test(time.trim()) ? Number.parseInt(time.trim(), 10) : null;
}

export function parseTargets(text: string | undefined): { count: number | null; allies: boolean; self: boolean } {
  const t = (text ?? '').toLowerCase();
  const m = /(?:up to\s+)?(\d+|one|two|three|four|five|six)\s+(?:willing\s+)?(?:creature|ally|allies|target)/.exec(t);
  const words: Record<string, number> = { one: 1, two: 2, three: 3, four: 4, five: 5, six: 6 };
  const count = m?.[1] ? (words[m[1]] ?? Number.parseInt(m[1], 10)) : null;
  return { count: Number.isFinite(count) ? count : null, allies: /\b(ally|allies|willing)\b/.test(t), self: t.trim() === 'you' || t.trim() === 'self' };
}

function traitList(s: SpellLike): string[] {
  if (s.traits instanceof Set) return [...s.traits];
  if (Array.isArray(s.traits)) return s.traits.map(String);
  return s.system?.traits?.value ?? [];
}

function kindsOf(k: string[] | Set<string> | undefined): string[] {
  return k instanceof Set ? [...k] : (k ?? []);
}

export function spellSnapshot(s: SpellLike): SpellSnapshot | null {
  if (!s.slug || !s.name) return null;
  const sys = s.system ?? {};
  const traits = traitList(s);
  const damageEntries = Object.values(sys.damage ?? {});
  const damage = damageEntries
    .filter((d) => !kindsOf(d.kinds).includes('healing') && typeof d.formula === 'string')
    .map((d) => ({ formula: String(d.formula), damage_type: d.type ?? null }));
  const heal = damageEntries.find((d) => kindsOf(d.kinds).includes('healing') && typeof d.formula === 'string');
  const def = sys.defense;
  const defense = def?.passive?.statistic === 'ac'
    ? ({ kind: 'attack' } as const)
    : def?.save?.statistic && SAVES[def.save.statistic]
      ? ({ kind: 'save', save: SAVES[def.save.statistic] as Save, basic: !!def.save.basic } as const)
      : null;
  const targets = parseTargets(sys.target?.value);
  const h = sys.heightening;
  return {
    slug: s.slug,
    name: s.name,
    rank: s.rank ?? 1,
    base_rank: s.baseRank ?? null,
    cantrip: !!s.isCantrip,
    actions: parseActions(sys.time?.value),
    range_ft: parseFeet(sys.range?.value),
    area: sys.area?.type && sys.area.value ? { kind: sys.area.type, size_ft: sys.area.value } : null,
    target_count: targets.count,
    targets_allies: targets.allies || traits.includes('healing'),
    targets_self: targets.self,
    defense,
    damage,
    healing: heal?.formula ?? null,
    heightening: h?.type === 'interval' && h.interval ? { interval: h.interval, damage: Object.values(h.damage ?? {}) } : null,
    traits,
  };
}

function remainingSlots(entry: SpellcastingEntryLike, spells: SpellLike[], actor: ActorLike, level: number): Record<string, number> {
  const slots: Record<string, number> = {};
  const raw = entry.system?.slots ?? {};
  switch (entry.category) {
    case 'spontaneous':
      for (let r = 1; r <= 10; r++) {
        const v = raw[`slot${r}`]?.value ?? 0;
        if (v > 0) slots[String(r)] = v;
      }
      break;
    case 'prepared':
      for (let r = 1; r <= 10; r++) {
        const n = (raw[`slot${r}`]?.prepared ?? []).filter((p) => p.id && !p.expended).length;
        if (n > 0) slots[String(r)] = n;
      }
      break;
    case 'innate':
      for (const s of spells) {
        if (s.isCantrip) continue;
        const r = String(s.rank ?? 1);
        const uses = s.system?.location?.uses;
        slots[r] = (slots[r] ?? 0) + (uses?.max ? (uses.value ?? 0) : 1);
      }
      break;
    case 'focus': {
      // Focus spells heighten to half level rounded up; points are uses.
      const pts = actor.system?.resources?.focus?.value ?? 0;
      if (pts > 0) slots[String(Math.max(1, Math.ceil(level / 2)))] = pts;
      break;
    }
  }
  return slots;
}

export function extractSpellcasting(actor: ActorLike, level: number): SpellcastingSnapshot[] {
  const out: SpellcastingSnapshot[] = [];
  for (const entry of actor.spellcasting?.regular ?? []) {
    const kind = entry.category ?? '';
    if (!['prepared', 'spontaneous', 'innate', 'focus'].includes(kind)) continue;
    let spells = toArray(entry.spells as Iterable<SpellLike> | { contents?: SpellLike[] } | null);
    if (kind === 'prepared') {
      const prepared = new Set<string>();
      for (const slot of Object.values(entry.system?.slots ?? {})) for (const p of slot.prepared ?? []) if (p.id) prepared.add(p.id);
      spells = spells.filter((s) => s.isCantrip || (s.id !== undefined && prepared.has(s.id)));
    }
    const focusRank = Math.max(1, Math.ceil(level / 2));
    const snaps = spells
      .map(spellSnapshot)
      .filter((s): s is SpellSnapshot => s !== null)
      .map((s) => (kind === 'focus' && !s.cantrip ? { ...s, rank: focusRank } : s));
    if (snaps.length === 0) continue;
    const stat = entry.statistic;
    const attack = checkBase(stat) ?? 0;
    const dc = dcBase(stat ? { dc: stat.dc } : null) ?? 10 + attack;
    out.push({
      name: entry.name ?? 'Spells',
      tradition: entry.tradition ?? null,
      kind,
      attribute: ATTRIBUTES[entry.attribute ?? ''] ?? 'charisma',
      attack_bonus: attack,
      dc,
      slots: remainingSlots(entry, spells, actor, level),
      spells: snaps,
    });
  }
  return out;
}
