// CombatSnapshot — mirror of crates/pf2e-bridge/src/snapshot.rs.
// Field names are snake_case on the wire. Keep in sync: the Rust-generated
// fixture test/fixtures/crypt-ambush.snapshot.json is type-checked against
// these interfaces (test/protocol.test.ts) and converted by Rust in CI.

export const SNAPSHOT_VERSION = 1;

export type Side = 'party' | 'enemy' | 'neutral';
export type CombatantKind = 'character' | 'monster' | 'hazard';
export type Size = 'tiny' | 'small' | 'medium' | 'large' | 'huge' | 'gargantuan';
export type Attribute = 'strength' | 'dexterity' | 'constitution' | 'intelligence' | 'wisdom' | 'charisma';
export type Save = 'fortitude' | 'reflex' | 'will';
export type Skill =
  | 'acrobatics' | 'arcana' | 'athletics' | 'crafting' | 'deception' | 'diplomacy' | 'intimidation' | 'medicine'
  | 'nature' | 'occultism' | 'performance' | 'religion' | 'society' | 'stealth' | 'survival' | 'thievery';
export type GridKind = 'square' | 'gridless' | 'hex';
export type TerrainKind = 'difficult' | 'greater_difficult' | 'obscured' | 'wall';

export interface AttributeSet {
  strength?: number;
  dexterity?: number;
  constitution?: number;
  intelligence?: number;
  wisdom?: number;
  charisma?: number;
}

export interface SnapshotOptions {
  flanking?: boolean;
  auto_cover?: boolean;
  crop_margin?: number;
}

export interface WallSnapshot {
  /** Endpoints in fractional squares, scene-relative. */
  a: [number, number];
  b: [number, number];
  blocks_movement: boolean;
  blocks_sight?: boolean;
  open_door?: boolean;
}

export interface TerrainCell { x: number; y: number; kind: TerrainKind }

export interface SceneSnapshot {
  id: string;
  name?: string;
  grid: GridKind;
  grid_size_px: number;
  grid_distance_ft: number;
  origin_px?: [number, number];
  width: number;
  height: number;
  walls?: WallSnapshot[];
  terrain?: TerrainCell[];
}

export interface TurnSnapshot {
  started: boolean;
  round?: number;
  current?: string | null;
  order?: string[];
  actions_spent?: number;
  attacks_made?: number;
}

export interface StrikeSnapshot {
  name: string;
  attack_bonus: number;
  damage_formula: string;
  damage_type?: string | null;
  traits?: string[];
  melee: boolean;
  reach_ft?: number | null;
  range_increment_ft?: number | null;
  attribute?: Attribute;
  /** Formula already includes condition damage penalties (enfeebled). */
  damage_includes_conditions?: boolean;
}

export interface SpellAreaSnapshot { kind: 'burst' | 'cone' | 'emanation' | 'line' | string; size_ft: number }
export type SpellDefenseSnapshot = { kind: 'attack' } | { kind: 'save'; save: Save; basic: boolean };
export interface FormulaSnapshot { formula: string; damage_type?: string | null }
export interface HeighteningSnapshot { interval: number; damage?: string[] }

export interface SpellSnapshot {
  slug: string;
  name: string;
  rank: number;
  base_rank?: number | null;
  cantrip?: boolean;
  actions?: number | null;
  range_ft?: number | null;
  area?: SpellAreaSnapshot | null;
  target_count?: number | null;
  targets_allies?: boolean;
  targets_self?: boolean;
  defense?: SpellDefenseSnapshot | null;
  damage?: FormulaSnapshot[];
  healing?: string | null;
  heightening?: HeighteningSnapshot | null;
  traits?: string[];
}

export interface SpellcastingSnapshot {
  name: string;
  tradition?: string | null;
  kind: 'prepared' | 'spontaneous' | 'innate' | 'focus' | string;
  attribute?: Attribute;
  attack_bonus: number;
  dc: number;
  /** Remaining slots by rank (JSON object keys are rank numbers). */
  slots?: Record<string, number>;
  spells?: SpellSnapshot[];
}

export interface IwrValueSnapshot { type: string; value: number }
export interface IwrSnapshot { immunities?: string[]; weaknesses?: IwrValueSnapshot[]; resistances?: IwrValueSnapshot[] }
export interface ReactionsSnapshot { reactive_strike?: boolean; shield_block?: boolean }
export interface ShieldSnapshot {
  name: string;
  ac_bonus: number;
  hardness: number;
  hp: number;
  max_hp: number;
  broken_threshold: number;
  raised?: boolean;
}
export interface ConditionSnapshot { slug: string; value?: number | null }
export interface PersistentSnapshot { formula: string; damage_type: string }
export interface HazardSnapshot { complex?: boolean; stealth_dc?: number; disable?: { skill: Skill; dc: number }[] }

export interface CombatantSnapshot {
  token_id: string;
  actor_id?: string;
  name: string;
  kind: CombatantKind;
  side: Side;
  x: number;
  y: number;
  level: number;
  size?: Size;
  traits?: string[];
  hp: number;
  max_hp: number;
  temp_hp?: number;
  ac: number;
  fortitude: number;
  reflex: number;
  will: number;
  perception: number;
  speed?: number;
  attributes?: AttributeSet;
  skills?: Partial<Record<Skill, number>>;
  strikes?: StrikeSnapshot[];
  spellcasting?: SpellcastingSnapshot[];
  iwr?: IwrSnapshot;
  reactions?: ReactionsSnapshot;
  shield?: ShieldSnapshot | null;
  conditions?: ConditionSnapshot[];
  persistent?: PersistentSnapshot[];
  class_dc?: number | null;
  sneak_attack?: string | null;
  initiative?: number | null;
  hazard?: HazardSnapshot | null;
  reaction_used?: boolean;
  uses_dying_rules?: boolean;
}

export interface CombatSnapshot {
  version: number;
  scene: SceneSnapshot;
  combatants: CombatantSnapshot[];
  turn: TurnSnapshot;
  options?: SnapshotOptions;
}
