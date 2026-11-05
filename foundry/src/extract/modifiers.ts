// Removing modifiers the engine applies itself.
//
// PF2e statistics already include every active modifier. Conditions are
// sent to the engine separately (and cover / raised shields are computed by
// it), so their modifiers must be taken back out of the totals or they
// would count twice. Effects the engine cannot model (spell effects, auras,
// feats) stay in the numbers: they are part of the creature's real totals.

import type { ModifierLike, StatisticLike } from '../types/pf2e';

/** Modifier slugs produced by conditions/situations that `pf2e-core` models. */
export const ENGINE_APPLIED_SLUGS: ReadonlySet<string> = new Set([
  'frightened',
  'sickened',
  'clumsy',
  'enfeebled',
  'drained',
  'stupefied',
  'off-guard',
  'flat-footed',
  'prone',
  'unconscious',
  'blinded',
  'raised-shield',
  'cover',
  'lesser-cover',
  'standard-cover',
  'greater-cover',
  'multiple-attack-penalty',
]);

export function isEngineApplied(m: ModifierLike): boolean {
  return !!m.slug && ENGINE_APPLIED_SLUGS.has(m.slug) && m.enabled !== false && m.ignored !== true;
}

/** `total` minus the active modifiers the engine will re-apply. */
export function baseValue(total: number, modifiers: readonly ModifierLike[] | undefined): number {
  if (!modifiers) return total;
  let v = total;
  for (const m of modifiers) if (isEngineApplied(m)) v -= m.modifier ?? 0;
  return v;
}

/** Check modifier of a PF2e Statistic (saves, perception, skills, spell attack). */
export function checkBase(stat: StatisticLike | null | undefined): number | null {
  if (!stat) return null;
  const total = stat.check?.mod ?? stat.mod ?? stat.totalModifier;
  if (typeof total !== 'number' || !Number.isFinite(total)) return null;
  return baseValue(total, stat.check?.modifiers ?? stat.modifiers);
}

/** DC of a PF2e Statistic (AC, spell DC, class DC). */
export function dcBase(stat: StatisticLike | null | undefined): number | null {
  if (!stat) return null;
  const total = stat.dc?.value ?? stat.value;
  if (typeof total !== 'number' || !Number.isFinite(total)) return null;
  return baseValue(total, stat.dc?.modifiers ?? stat.modifiers);
}
