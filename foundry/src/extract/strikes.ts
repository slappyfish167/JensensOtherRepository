// PF2e strike actions (`actor.system.actions`) → snapshot strikes.

import type { Attribute, StrikeSnapshot } from '../protocol/snapshot';
import { toArray, type ActorLike, type ItemLike, type StrikeLike } from '../types/pf2e';
import { baseValue } from './modifiers';

function traitsOf(item: ItemLike | undefined, strike: StrikeLike): string[] {
  const fromItem = item?.system?.traits?.value;
  if (Array.isArray(fromItem)) return fromItem.map(String);
  return (strike.traits ?? []).map((t) => t.name).filter((n): n is string => typeof n === 'string');
}

function traitNumber(traits: string[], prefix: string): number | null {
  for (const t of traits) {
    if (t.startsWith(prefix)) {
      const n = Number.parseInt(t.slice(prefix.length), 10);
      if (Number.isFinite(n)) return n;
    }
  }
  return null;
}

function abilityMod(actor: ActorLike, key: 'str' | 'dex'): number {
  return actor.abilities?.[key]?.mod ?? actor.system?.abilities?.[key]?.mod ?? 0;
}

export interface StrikeExtraction {
  strikes: StrikeSnapshot[];
  warnings: string[];
}

/** Conditions whose damage penalty PF2e already folds into strike formulas. */
const DAMAGE_CONDITIONS = new Set(['enfeebled']);

export async function extractStrikes(actor: ActorLike): Promise<StrikeExtraction> {
  const damageIncludesConditions = toArray(actor.conditions).some((c) => c.active !== false && !!c.slug && DAMAGE_CONDITIONS.has(c.slug));
  const out: StrikeSnapshot[] = [];
  const warnings: string[] = [];
  for (const s of actor.system?.actions ?? []) {
    if (s.type !== undefined && s.type !== 'strike') continue;
    if (s.ready === false || s.visible === false) continue;
    const item = s.item;
    const name = s.label ?? item?.name ?? 'Strike';
    if (typeof s.totalModifier !== 'number') continue;
    let formula: unknown = null;
    try {
      formula = await s.damage?.({ getFormula: true });
    } catch (e) {
      warnings.push(`${actor.name ?? 'Creature'}: could not read damage for ${name} (${String(e)})`);
    }
    if (typeof formula !== 'string' || !formula.trim()) {
      warnings.push(`${actor.name ?? 'Creature'}: ${name} has no damage formula; skipped`);
      continue;
    }
    const traits = traitsOf(item, s);
    const melee = item?.isMelee ?? (item?.isRanged !== undefined ? !item.isRanged : !traits.some((t) => t.startsWith('range')));
    let reach: number | null = null;
    if (melee) {
      try {
        reach = actor.getReach?.({ action: 'attack', weapon: item }) ?? null;
      } catch {
        reach = null;
      }
      reach ??= traitNumber(traits, 'reach-') ?? 5;
    }
    const increment = melee
      ? null
      : (item?.range?.increment ?? item?.system?.range?.increment ?? traitNumber(traits, 'range-increment-') ?? traitNumber(traits, 'range-') ?? 30);
    const finesse = traits.includes('finesse');
    // Which attribute the roll keys off (for clumsy vs enfeebled).
    const attribute: Attribute = !melee || (finesse && abilityMod(actor, 'dex') > abilityMod(actor, 'str')) ? 'dexterity' : 'strength';
    out.push({
      name,
      attack_bonus: baseValue(s.totalModifier, s.modifiers),
      damage_formula: formula,
      damage_type: item?.system?.damage?.damageType ?? null,
      traits,
      melee,
      reach_ft: reach,
      range_increment_ft: increment,
      attribute,
      damage_includes_conditions: damageIncludesConditions,
    });
  }
  return { strikes: out, warnings };
}
