// The subset of Foundry VTT (v13) and PF2e system (7.x) document shapes the
// extractor reads. Everything is optional: PF2e data moves between
// releases, and the extractor degrades gracefully (with warnings) instead
// of throwing. Tests build plain objects of these shapes.

export interface ModifierLike {
  slug?: string;
  label?: string;
  modifier?: number;
  type?: string;
  enabled?: boolean;
  ignored?: boolean;
}

/** PF2e `Statistic` / `StatisticCheck` / `StatisticDifficultyClass`. */
export interface StatisticLike {
  mod?: number;
  totalModifier?: number;
  value?: number;
  modifiers?: ModifierLike[];
  check?: { mod?: number; modifiers?: ModifierLike[] };
  dc?: { value?: number; modifiers?: ModifierLike[] };
}

export interface ItemLike {
  id?: string;
  slug?: string | null;
  name?: string;
  type?: string;
  isMelee?: boolean;
  isRanged?: boolean;
  range?: { increment?: number | null; max?: number | null } | null;
  system?: {
    traits?: { value?: string[] };
    damage?: { damageType?: string; dice?: number; die?: string };
    range?: { increment?: number | null; value?: string | null } | null;
    slug?: string | null;
    [k: string]: unknown;
  };
}

export interface StrikeLike {
  slug?: string | null;
  label?: string;
  type?: string;
  ready?: boolean;
  visible?: boolean;
  totalModifier?: number;
  modifiers?: ModifierLike[];
  item?: ItemLike;
  traits?: { name?: string }[];
  damage?: (params: { getFormula: true }) => Promise<string | unknown>;
}

export interface SpellLike {
  id?: string;
  slug?: string | null;
  name?: string;
  rank?: number;
  baseRank?: number;
  isCantrip?: boolean;
  traits?: Set<string> | string[];
  system?: {
    time?: { value?: string };
    range?: { value?: string };
    area?: { type?: string; value?: number } | null;
    target?: { value?: string };
    defense?: { passive?: { statistic?: string } | null; save?: { statistic?: string; basic?: boolean } | null } | null;
    damage?: Record<string, { formula?: string; type?: string; kinds?: string[] | Set<string> }>;
    heightening?: { type?: string; interval?: number; damage?: Record<string, string> } | null;
    location?: { uses?: { value?: number; max?: number } };
    traits?: { value?: string[] };
  };
}

export interface SpellcastingEntryLike {
  id?: string;
  name?: string;
  tradition?: string | null;
  category?: string;
  attribute?: string;
  statistic?: StatisticLike | null;
  spells?: Iterable<SpellLike> | { contents?: SpellLike[] } | null;
  system?: {
    slots?: Record<string, { value?: number; max?: number; prepared?: { id?: string | null; expended?: boolean }[] }>;
  };
}

export interface ConditionItemLike {
  slug?: string;
  value?: number | null;
  active?: boolean;
  system?: { persistent?: { formula?: string; damageType?: string } | null };
}

export interface ShieldDataLike {
  name?: string;
  ac?: number;
  hardness?: number;
  hp?: { value?: number; max?: number };
  brokenThreshold?: number;
  raised?: boolean;
  broken?: boolean;
  destroyed?: boolean;
  itemId?: string | null;
}

export interface ActorLike {
  id?: string;
  uuid?: string;
  name?: string;
  type?: string;
  level?: number;
  size?: string;
  hasPlayerOwner?: boolean;
  /** Hazards: complex hazards act in initiative. */
  isComplex?: boolean;
  traits?: Set<string>;
  hitPoints?: { value?: number; max?: number; temp?: number };
  armorClass?: StatisticLike | null;
  saves?: Partial<Record<'fortitude' | 'reflex' | 'will', StatisticLike>>;
  perception?: StatisticLike | null;
  skills?: Record<string, StatisticLike | undefined>;
  abilities?: Record<string, { mod?: number }> | null;
  classDC?: StatisticLike | null;
  getStatistic?: (slug: string) => StatisticLike | null;
  getReach?: (options: { action?: string; weapon?: ItemLike }) => number;
  conditions?: Iterable<ConditionItemLike> | { contents?: ConditionItemLike[] };
  items?: Iterable<ItemLike> | { contents?: ItemLike[] };
  spellcasting?: { regular?: SpellcastingEntryLike[] } | null;
  attributes?: {
    hp?: { value?: number; max?: number; temp?: number };
    ac?: { value?: number };
    speed?: { value?: number; total?: number };
    shield?: ShieldDataLike | null;
    immunities?: { type?: string }[];
    weaknesses?: { type?: string; value?: number }[];
    resistances?: { type?: string; value?: number }[];
  };
  system?: {
    attributes?: ActorLike['attributes'] & { speed?: { value?: number; total?: number } };
    movement?: { speeds?: { land?: { value?: number } } };
    abilities?: Record<string, { mod?: number }>;
    resources?: { focus?: { value?: number; max?: number } };
    details?: { level?: { value?: number } };
    actions?: StrikeLike[];
    traits?: { value?: string[]; size?: { value?: string } };
  };
}

export interface TokenDocLike {
  id?: string;
  name?: string;
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  hidden?: boolean;
  disposition?: number;
  actor?: ActorLike | null;
}

export interface WallDocLike {
  c?: number[];
  move?: number;
  sight?: number;
  door?: number;
  ds?: number;
}

export interface CombatantLike {
  id?: string;
  tokenId?: string | null;
  token?: TokenDocLike | null;
  initiative?: number | null;
  defeated?: boolean;
  hidden?: boolean;
}

export interface CombatLike {
  id?: string;
  started?: boolean;
  round?: number;
  turn?: number | null;
  combatant?: CombatantLike | null;
  turns?: CombatantLike[];
  scene?: { id?: string } | null;
  getFlag?: (scope: string, key: string) => unknown;
}

export interface SceneLike {
  id?: string;
  name?: string;
  grid?: { type?: number; size?: number; distance?: number };
  dimensions?: { sceneX?: number; sceneY?: number; sceneWidth?: number; sceneHeight?: number; size?: number; distance?: number };
  walls?: Iterable<WallDocLike> | { contents?: WallDocLike[] };
  tokens?: Iterable<TokenDocLike> | { contents?: TokenDocLike[] };
}

/** Normalises Foundry collections, arrays and iterables to arrays. */
export function toArray<T>(c: Iterable<T> | { contents?: T[] } | null | undefined): T[] {
  if (!c) return [];
  if (Array.isArray(c)) return c;
  if (typeof (c as Iterable<T>)[Symbol.iterator] === 'function') return Array.from(c as Iterable<T>);
  return (c as { contents?: T[] }).contents ?? [];
}
