// Analysis results — mirror of crates/pf2e-bridge/src/report.rs and api.rs.

import type { Side } from './snapshot';

export type ErrorCode =
  | 'invalid_json' | 'unsupported_version' | 'unsupported_scene' | 'invalid_snapshot' | 'nothing_to_analyse'
  | 'no_snapshot' | 'unknown_token' | 'cannot_act' | 'internal'
  // Host-side codes (TypeScript only).
  | 'worker_unavailable' | 'timeout' | 'cancelled' | 'not_permitted';

export interface EngineErrorShape { code: ErrorCode; message: string }

export interface LoadSummary {
  combatants: number;
  crop: { x: number; y: number; width: number; height: number };
  wall_edges: number;
  turn_active: boolean;
  current_token: string | null;
  warnings: string[];
  fingerprint: string;
}

export interface FactorReport { name: string; value: number; detail: string }

export interface EvaluationReport {
  score: number;
  band: string;
  win_probability: number;
  terminal: boolean;
  factors: FactorReport[];
  party_hp: number;
  enemy_hp: number;
  party_damage_per_round: number;
  enemy_damage_per_round: number;
  current_token: string | null;
}

export interface ScenePoint { square: [number, number]; px: [number, number] }

export type StepKind =
  | 'strike' | 'stride' | 'step' | 'stand' | 'raise_shield' | 'take_cover' | 'demoralize' | 'trip' | 'cast_spell' | 'disable_hazard';

export interface StepReport {
  kind: StepKind;
  label: string;
  target_tokens?: string[];
  to?: ScenePoint;
  path?: ScenePoint[];
  action: unknown;
}

export interface TurnReport {
  token_id: string;
  name: string;
  steps: StepReport[];
  baseline_score: number;
  expected_score: number;
  gain: number;
  reasoning: string[];
  depth: number;
  nodes: number;
  truncated: boolean;
  hypothetical: boolean;
  summary: string;
}

export interface TargetReport {
  token_id: string;
  name: string;
  expected_damage: number;
  kill_probability: number;
  threat: number;
  priority: number;
  rationale: string;
}

export interface CreaturePlanReport { token_id: string; name: string; turn: TurnReport | null; targets: TargetReport[] }

export interface SidePlanReport { side: Side; focus_token: string | null; notes: string[]; creatures: CreaturePlanReport[] }

/** Raw Monte Carlo counts; merged across workers before reporting. */
export interface SimulationTally {
  runs: number;
  party_wins: number;
  enemy_wins: number;
  timeouts: number;
  tpks: number;
  any_pc_deaths: number;
  total_rounds: number;
  combatants: { id: number; name: string; side: Side; deaths: number; downs: number }[];
}

export interface ForecastReport {
  runs: number;
  victory: number;
  defeat: number;
  unresolved: number;
  tpk: number;
  any_pc_death: number;
  average_rounds: number;
  combatants: { token_id: string; name: string; side: Side; death_rate: number; down_rate: number }[];
  margin_of_error: number;
}

export type ReasonKind =
  | 'hit_points' | 'downed' | 'recovered' | 'condition_gained' | 'condition_lost' | 'flanked' | 'reaction'
  | 'action_economy' | 'resources' | 'position' | 'persistent_damage';

export interface ExplainReport {
  score: number;
  previous_score: number | null;
  delta: number;
  reasons: { kind: ReasonKind; token_id: string | null; text: string; impact: number }[];
  factors: { name: string; before: number; after: number }[];
}
