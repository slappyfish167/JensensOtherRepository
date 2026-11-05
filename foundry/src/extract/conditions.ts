// PF2e condition items → snapshot conditions and persistent damage.

import type { ConditionSnapshot, PersistentSnapshot } from '../protocol/snapshot';
import { toArray, type ActorLike } from '../types/pf2e';

export interface ExtractedConditions {
  conditions: ConditionSnapshot[];
  persistent: PersistentSnapshot[];
}

export function extractConditions(actor: ActorLike): ExtractedConditions {
  const best = new Map<string, number | null>();
  const persistent: PersistentSnapshot[] = [];
  for (const c of toArray(actor.conditions)) {
    if (!c.slug || c.active === false) continue;
    if (c.slug === 'persistent-damage') {
      const p = c.system?.persistent;
      if (p?.formula && p.damageType) persistent.push({ formula: String(p.formula), damage_type: String(p.damageType) });
      continue;
    }
    const value = typeof c.value === 'number' ? c.value : null;
    const prev = best.get(c.slug);
    // Valued conditions keep the highest instance (PF2e stacking rule is
    // applied by the system already; duplicates come from linked items).
    if (prev === undefined || (value ?? 0) > (prev ?? 0)) best.set(c.slug, value);
  }
  const conditions = [...best].map(([slug, value]) => (value === null ? { slug } : { slug, value }));
  conditions.sort((a, b) => a.slug.localeCompare(b.slug));
  return { conditions, persistent };
}
