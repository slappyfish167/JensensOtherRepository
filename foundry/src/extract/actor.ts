// PF2e actor + token → one snapshot combatant.

import type { CombatantKind, CombatantSnapshot, ShieldSnapshot, Side, Size, Skill } from '../protocol/snapshot';
import { toArray, type ActorLike, type TokenDocLike } from '../types/pf2e';
import { extractConditions } from './conditions';
import { checkBase, dcBase } from './modifiers';
import { extractSpellcasting } from './spells';
import { extractStrikes, type StrikeExtraction } from './strikes';

const SIZES: Record<string, Size> = { tiny: 'tiny', sm: 'small', med: 'medium', lg: 'large', huge: 'huge', grg: 'gargantuan' };

const SKILLS: Skill[] = [
  'acrobatics', 'arcana', 'athletics', 'crafting', 'deception', 'diplomacy', 'intimidation', 'medicine',
  'nature', 'occultism', 'performance', 'religion', 'society', 'stealth', 'survival', 'thievery',
];

const REACTIVE_STRIKE_SLUGS = new Set(['reactive-strike', 'attack-of-opportunity']);
const SHIELD_BLOCK_SLUGS = new Set(['shield-block']);

/** Token disposition (CONST.TOKEN_DISPOSITIONS) → side. */
export function sideOf(actor: ActorLike, disposition: number | undefined): Side {
  if (actor.type === 'character' || actor.hasPlayerOwner) return 'party';
  if (disposition === 1) return 'party';
  if (disposition === -1) return 'enemy';
  return 'neutral';
}

export function kindOf(actor: ActorLike): CombatantKind | null {
  switch (actor.type) {
    case 'character':
    case 'familiar':
      return 'character';
    case 'npc':
      return 'monster';
    case 'hazard':
      return 'hazard';
    default:
      return null; // loot, vehicle, party, army: not combatants
  }
}

/**
 * Rogue sneak attack dice by level. This is class progression *data* (the
 * same kind of table as content/classes.json), not a combat rule.
 */
export function sneakAttackDice(level: number): string {
  return level >= 17 ? '4d6' : level >= 11 ? '3d6' : level >= 5 ? '2d6' : '1d6';
}

function num(v: unknown, fallback: number): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : fallback;
}

function speedOf(actor: ActorLike): number {
  const sys = actor.system;
  return num(sys?.movement?.speeds?.land?.value, num(sys?.attributes?.speed?.total, num(sys?.attributes?.speed?.value, num(actor.attributes?.speed?.total, num(actor.attributes?.speed?.value, 25)))));
}

function shieldOf(actor: ActorLike): ShieldSnapshot | null {
  const s = actor.attributes?.shield ?? actor.system?.attributes?.shield;
  if (!s || !s.itemId || s.destroyed) return null;
  return {
    name: s.name ?? 'Shield',
    ac_bonus: num(s.ac, 2),
    hardness: num(s.hardness, 0),
    hp: num(s.hp?.value, 0),
    max_hp: num(s.hp?.max, 0),
    broken_threshold: num(s.brokenThreshold, 0),
    raised: !!s.raised && !s.broken,
  };
}

export interface CombatantContext {
  /** Top-left square of the token, scene-relative. */
  square: [number, number];
  initiative: number | null;
  reactionUsed: boolean;
  /** Per-actor cache of strike extraction (formulas are async and costly). */
  strikeCache?: Map<string, StrikeExtraction>;
}

export interface CombatantExtraction {
  combatant: CombatantSnapshot | null;
  warnings: string[];
}

export async function extractCombatant(token: TokenDocLike, ctx: CombatantContext): Promise<CombatantExtraction> {
  const actor = token.actor;
  const warnings: string[] = [];
  if (!actor || !token.id) return { combatant: null, warnings };
  const kind = kindOf(actor);
  if (!kind) return { combatant: null, warnings };
  const name = token.name ?? actor.name ?? 'Creature';
  const level = num(actor.level, num(actor.system?.details?.level?.value, 0));
  const hp = actor.hitPoints ?? actor.attributes?.hp ?? actor.system?.attributes?.hp ?? {};
  const ac = dcBase(actor.getStatistic?.('ac') ?? actor.armorClass ?? null) ?? num(actor.attributes?.ac?.value, 10);
  const save = (k: 'fortitude' | 'reflex' | 'will') => checkBase(actor.saves?.[k]) ?? 0;
  const items = toArray(actor.items);
  const slugs = new Set(items.map((i) => i.slug ?? i.system?.slug ?? '').filter(Boolean) as string[]);
  const cacheKey = actor.uuid ?? actor.id;
  let extracted = cacheKey ? ctx.strikeCache?.get(cacheKey) : undefined;
  if (!extracted) {
    extracted = await extractStrikes(actor);
    if (cacheKey) ctx.strikeCache?.set(cacheKey, extracted);
  }
  const { strikes, warnings: sw } = extracted;
  warnings.push(...sw);
  const { conditions, persistent } = extractConditions(actor);
  const iwrSrc = actor.attributes ?? actor.system?.attributes ?? {};
  const skills: Partial<Record<Skill, number>> = {};
  for (const sk of SKILLS) {
    const v = checkBase(actor.skills?.[sk]);
    if (v !== null) skills[sk] = v;
  }
  const abilities = actor.abilities ?? actor.system?.abilities ?? {};
  const mod = (k: string) => num(abilities[k]?.mod, 0);
  const traits = actor.traits ? [...actor.traits] : (actor.system?.traits?.value ?? []);
  const sizeKey = actor.size ?? actor.system?.traits?.size?.value ?? 'med';
  const combatant: CombatantSnapshot = {
    token_id: token.id,
    actor_id: actor.id ?? '',
    name,
    kind,
    side: sideOf(actor, token.disposition),
    x: ctx.square[0],
    y: ctx.square[1],
    level,
    size: SIZES[sizeKey] ?? 'medium',
    traits,
    hp: num(hp.value, 0),
    max_hp: Math.max(1, num(hp.max, 1)),
    temp_hp: num(hp.temp, 0),
    ac,
    fortitude: save('fortitude'),
    reflex: save('reflex'),
    will: save('will'),
    perception: checkBase(actor.perception) ?? 0,
    speed: speedOf(actor),
    attributes: { strength: mod('str'), dexterity: mod('dex'), constitution: mod('con'), intelligence: mod('int'), wisdom: mod('wis'), charisma: mod('cha') },
    skills,
    strikes,
    spellcasting: extractSpellcasting(actor, level),
    iwr: {
      immunities: (iwrSrc.immunities ?? []).map((i) => i.type).filter((t): t is string => !!t),
      weaknesses: (iwrSrc.weaknesses ?? []).filter((w) => w.type).map((w) => ({ type: String(w.type), value: num(w.value, 0) })),
      resistances: (iwrSrc.resistances ?? []).filter((w) => w.type).map((w) => ({ type: String(w.type), value: num(w.value, 0) })),
    },
    reactions: {
      reactive_strike: [...slugs].some((s) => REACTIVE_STRIKE_SLUGS.has(s)),
      shield_block: [...slugs].some((s) => SHIELD_BLOCK_SLUGS.has(s)),
    },
    shield: shieldOf(actor),
    conditions,
    persistent,
    class_dc: dcBase(actor.classDC ?? null),
    sneak_attack: slugs.has('sneak-attack') ? sneakAttackDice(level) : null,
    initiative: ctx.initiative,
    hazard: kind === 'hazard' ? { complex: actor.isComplex ?? strikes.length > 0, stealth_dc: 0, disable: [] } : null,
    reaction_used: ctx.reactionUsed,
    uses_dying_rules: actor.type === 'character',
  };
  if (strikes.length === 0 && (combatant.spellcasting ?? []).length === 0 && kind !== 'hazard') {
    warnings.push(`${name}: no usable strikes or spells were found`);
  }
  return { combatant, warnings };
}
