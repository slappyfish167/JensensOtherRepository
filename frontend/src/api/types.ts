// TypeScript mirrors of the Rust serde models (pf2e-core / pf2e-store).
// Keep in sync with crates/pf2e-core/src/**. Field names are snake_case on
// the wire and are kept that way here to avoid a mapping layer.

export type CombatantId = number;
export type Side = 'party' | 'enemy' | 'neutral';
export type Attribute = 'strength' | 'dexterity' | 'constitution' | 'intelligence' | 'wisdom' | 'charisma';
export type Skill =
  | 'acrobatics' | 'arcana' | 'athletics' | 'crafting' | 'deception' | 'diplomacy' | 'intimidation' | 'medicine'
  | 'nature' | 'occultism' | 'performance' | 'religion' | 'society' | 'stealth' | 'survival' | 'thievery';
export type FeatType = 'ancestry' | 'class' | 'skill' | 'general' | 'archetype';
export type CoverLevel = 'none' | 'lesser' | 'standard' | 'greater';
export type ConditionKind =
  | 'blinded' | 'clumsy' | 'concealed' | 'dazzled' | 'doomed' | 'drained' | 'dying' | 'enfeebled' | 'fleeing'
  | 'frightened' | 'grabbed' | 'hidden' | 'immobilized' | 'invisible' | 'off_guard' | 'paralyzed' | 'prone'
  | 'quickened' | 'restrained' | 'sickened' | 'slowed' | 'stunned' | 'stupefied' | 'unconscious' | 'wounded';

export const ATTRIBUTES: Attribute[] = ['strength', 'dexterity', 'constitution', 'intelligence', 'wisdom', 'charisma'];
export const SKILLS: Skill[] = [
  'acrobatics', 'arcana', 'athletics', 'crafting', 'deception', 'diplomacy', 'intimidation', 'medicine',
  'nature', 'occultism', 'performance', 'religion', 'society', 'stealth', 'survival', 'thievery',
];
export const CONDITIONS: ConditionKind[] = [
  'blinded', 'clumsy', 'concealed', 'dazzled', 'doomed', 'drained', 'enfeebled', 'fleeing', 'frightened', 'grabbed',
  'hidden', 'immobilized', 'invisible', 'off_guard', 'paralyzed', 'prone', 'quickened', 'restrained', 'sickened',
  'slowed', 'stunned', 'stupefied', 'wounded',
];

export interface Pos { x: number; y: number }

export interface Modifier { kind: 'untyped' | 'circumstance' | 'status' | 'item'; value: number; source: string }

export interface Expiry { combatant: CombatantId; round: number; boundary: 'start' | 'end' }
export interface Condition { kind: ConditionKind; value?: number; source?: CombatantId; expires?: Expiry }

export interface DamageComponent { dice: string; damage_type: string; precision?: boolean }
export type StrikeRange = { kind: 'melee'; reach_ft: number } | { kind: 'ranged'; increment_ft: number };
export interface StrikeProfile {
  name: string;
  attack_bonus: number;
  damage: DamageComponent[];
  traits: string[];
  range: StrikeRange;
}
export interface KnownSpell { spell_id: string; rank: number }
export interface SpellcastingEntry {
  name: string;
  tradition: string;
  kind: string;
  attribute: Attribute;
  attack_bonus: number;
  dc: number;
  slots: Record<string, number>;
  spells: KnownSpell[];
  cantrips: string[];
}
export interface HazardProfile { complex: boolean; stealth_dc: number; disable: { skill: Skill; dc: number }[] }
export interface Statblock {
  name: string;
  level: number;
  traits: string[];
  size: string;
  attributes: Record<Attribute, number>;
  max_hp: number;
  ac: number;
  fortitude: number;
  reflex: number;
  will: number;
  perception: number;
  speed: number;
  skills: Partial<Record<Skill, number>>;
  strikes: StrikeProfile[];
  spellcasting: SpellcastingEntry[];
  iwr: { immunities: unknown[]; weaknesses: { applies_to: unknown; value: number }[]; resistances: { applies_to: unknown; value: number }[] };
  reactions: string[];
  shield?: { name: string; ac_bonus: number; hardness: number; max_hp: number; broken_threshold: number } | null;
  class_dc?: number | null;
  uses_dying_rules: boolean;
  hazard?: HazardProfile | null;
}

export interface Monster {
  id: string;
  source: string;
  tags: string[];
  description: string;
  applied_templates: string[];
  statblock: Statblock;
}
export interface MonsterSummary { id: string; name: string; level: number; is_hazard: boolean; tags: string[] }

export interface FeatChoice { feat_id: string; slot_type: FeatType; slot_level: number }
export interface WeaponItem { weapon_id: string; potency?: number; striking?: number }
export interface Character {
  id: string;
  name: string;
  level: number;
  ancestry_id: string;
  background_id: string;
  class_id: string;
  boosts: {
    ancestry_free: Attribute[];
    ancestry_alternate?: boolean;
    background: Attribute[];
    class_key: Attribute;
    level1_free: Attribute[];
    level_ups?: Record<string, Attribute[]>;
  };
  trained_skills: Skill[];
  skill_increases?: Record<string, Skill>;
  feats: FeatChoice[];
  equipment: { weapons: WeaponItem[]; armor?: { armor_id: string; potency?: number; resilient?: number } | null; shield_id?: string | null };
  spells: { cantrips: string[]; spells: KnownSpell[] };
  notes?: string;
}
export interface CharacterSummary { id: string; name: string; level: number; ancestry_id: string; class_id: string; is_legal: boolean }
export interface ValidationIssue { severity: 'error' | 'warning'; code: string; path: string; message: string }
export interface BuildResult {
  report: { issues: ValidationIssue[] };
  attributes: Record<Attribute, number>;
  statblock: Statblock | null;
}

export interface AncestryDef { id: string; name: string; hp: number; speed: number; boosts: ('free' | { fixed: Attribute })[]; flaws: Attribute[] }
export interface BackgroundDef { id: string; name: string; boost_options: Attribute[]; trained_skills: Skill[] }
export interface ClassDef { id: string; name: string; key_attributes: Attribute[]; additional_skills: number; spellcasting?: { tradition: string; cantrips: number } | null }
export interface FeatDef { id: string; name: string; level: number; feat_type: FeatType; traits: string[]; archetype?: string | null }
export interface WeaponDef { id: string; name: string; category: string; damage_die: number; damage_type: string; traits: string[] }
export interface ArmorDef { id: string; name: string; category: string; ac_bonus: number }
export interface ShieldDef { id: string; name: string }
export interface SpellDef {
  id: string;
  name: string;
  rank: number;
  cantrip: boolean;
  traditions: string[];
  actions: number;
  range_ft?: number | null;
  targeting: { kind: 'creatures'; max: number; allies?: boolean } | { kind: 'area'; area: { shape: string; size_ft: number } } | { kind: 'self_only' };
  description: string;
}
export interface Content {
  ancestries: AncestryDef[];
  backgrounds: BackgroundDef[];
  classes: ClassDef[];
  feats: FeatDef[];
  weapons: WeaponDef[];
  armor: ArmorDef[];
  shields: ShieldDef[];
  spells: SpellDef[];
}

export interface Battlefield { width: number; height: number; map: string[]; notes?: string }
export type ParticipantSource = { kind: 'character'; value: string } | { kind: 'monster'; value: string } | { kind: 'inline'; value: Statblock };
export interface EncounterParticipant { source: ParticipantSource; side: Side; position: Pos; name?: string | null; current_hp?: number | null }
export interface Encounter { id: string; name: string; party_level: number; participants: EncounterParticipant[]; battlefield: Battlefield; notes?: string }
export interface EncounterSummary { id: string; name: string; party_level: number }
export interface EncounterBudget { party_size: number; total_xp: number; threat: string; budgets: number[]; warnings: string[] }

// ----- combat -----

export interface Combatant {
  id: CombatantId;
  name: string;
  side: Side;
  kind: 'character' | 'monster' | 'hazard';
  statblock: Statblock;
  hp: number;
  temp_hp: number;
  position: Pos;
  conditions: Condition[];
  persistent: { dice: string; damage_type: string }[];
  effects: { id: number; name: string }[];
  actions_remaining: number;
  reaction_available: boolean;
  attacks_this_turn: number;
  shield_hp: number;
  spell_slots: Record<string, number>[];
  life: 'active' | 'dead' | 'inactive';
  in_initiative: boolean;
}

export type Phase = { kind: 'setup' } | { kind: 'rolling_initiative' } | { kind: 'active' } | { kind: 'ended'; winner: Side | null };

export interface CombatState {
  battlefield: Battlefield;
  combatants: Combatant[];
  spells: Record<string, SpellDef>;
  phase: Phase;
  round: number;
  turn: number;
  initiative: { combatant: CombatantId; value: number; natural: number }[];
  input_seq: number;
}

export type RollKind = { kind: 'check'; modifier: number; dc: number } | { kind: 'flat'; dc: number } | { kind: 'total'; dice: string };
export interface RollRequest {
  request_id: number;
  roller: CombatantId | null;
  kind: RollKind;
  expression: string;
  purpose: string;
  prompt: string;
  breakdown: Modifier[];
}
export interface DecisionRequest { request_id: number; decider: CombatantId; reaction: string; prompt: string }
export type PendingInput = ({ type: 'roll' } & RollRequest) | ({ type: 'decision' } & DecisionRequest);

export type Action =
  | { kind: 'strike'; actor_strike: number; target: CombatantId; cover_override?: CoverLevel }
  | { kind: 'stride'; to: Pos }
  | { kind: 'step'; to: Pos }
  | { kind: 'stand' }
  | { kind: 'raise_shield' }
  | { kind: 'take_cover' }
  | { kind: 'demoralize'; target: CombatantId }
  | { kind: 'trip'; target: CombatantId }
  | { kind: 'cast_spell'; entry: number; spell_id: string; rank: number; targets: CombatantId[]; aim?: Pos }
  | { kind: 'disable_hazard'; hazard: CombatantId };

export type GmOp =
  | { kind: 'adjust_hp'; target: CombatantId; delta: number }
  | { kind: 'set_temp_hp'; target: CombatantId; value: number }
  | { kind: 'add_condition'; target: CombatantId; condition: Condition }
  | { kind: 'remove_condition'; target: CombatantId; condition: ConditionKind }
  | { kind: 'move'; target: CombatantId; to: Pos }
  | { kind: 'clear_pending' };

export type Command =
  | { type: 'start_combat' }
  | { type: 'provide_roll'; request_id: number; value: number; extra_modifier?: number }
  | { type: 'decide'; request_id: number; accept: boolean }
  | { type: 'act'; actor: CombatantId; action: Action }
  | { type: 'end_turn'; actor: CombatantId }
  | { type: 'gm'; op: GmOp };

export interface DamageLine { damage_type: string; rolled: number; applied: number; note: string }
export type CombatEvent =
  | { type: 'combat_started' }
  | { type: 'initiative_order'; order: [CombatantId, number][] }
  | { type: 'round_started'; round: number }
  | { type: 'turn_started'; combatant: CombatantId; actions: number }
  | { type: 'turn_ended'; combatant: CombatantId }
  | { type: 'action_declared'; actor: CombatantId; action: string; cost: number }
  | { type: 'moved'; combatant: CombatantId; from: Pos; to: Pos }
  | { type: 'check_resolved'; roller: CombatantId | null; purpose: string; natural: number; total: number; dc: number; degree: string }
  | { type: 'flat_check'; combatant: CombatantId; purpose: string; natural: number; dc: number; success: boolean }
  | { type: 'damage_rolled'; purpose: string; expression: string; rolled: number; counted: number }
  | { type: 'damage_taken'; target: CombatantId; amount: number; lines: DamageLine[]; hp: number; temp_hp: number }
  | { type: 'shield_blocked'; target: CombatantId; prevented: number; shield_hp: number; broken: boolean }
  | { type: 'healed'; target: CombatantId; amount: number; hp: number }
  | { type: 'condition_gained'; target: CombatantId; condition: ConditionKind; value: number }
  | { type: 'condition_lost'; target: CombatantId; condition: ConditionKind }
  | { type: 'effect_gained'; target: CombatantId; name: string }
  | { type: 'effect_ended'; target: CombatantId; name: string }
  | { type: 'reaction_used'; reactor: CombatantId; reaction: string }
  | { type: 'action_disrupted'; actor: CombatantId }
  | { type: 'knocked_out'; combatant: CombatantId; dying: number }
  | { type: 'died'; combatant: CombatantId }
  | { type: 'hazard_triggered'; hazard: CombatantId; victim: CombatantId }
  | { type: 'hazard_disabled'; hazard: CombatantId }
  | { type: 'combat_ended'; winner: Side | null }
  | { type: 'note'; text: string };

export type DiceMode = { mode: 'manual' } | { mode: 'seeded'; seed: number };

export interface CombatView {
  id: string;
  name: string;
  encounter_id: string | null;
  dice: DiceMode;
  status: string;
  state: CombatState;
  pending: PendingInput | null;
  current_actor: CombatantId | null;
  command_count: number;
  fingerprint: string;
  events: CombatEvent[];
}
export interface CombatRecord { id: string; name: string; status: string; dice: DiceMode; created_at: number }

export interface LegalAction { action: Action; description: string; cost: number }

// ----- analysis -----

export interface EvalFactor { name: string; value: number; detail: string }
export interface Evaluation {
  score: number;
  win_probability: number;
  terminal: boolean;
  factors: EvalFactor[];
  party: { hit_points: number; damage_per_round: number; standing: number; down: number };
  enemies: { hit_points: number; damage_per_round: number; standing: number; down: number };
}
export interface TurnRecommendation {
  plan: { actor: CombatantId; actions: Action[]; expected_score: number; baseline_score: number; gain: number; nodes: number; truncated: boolean };
  steps: string[];
  summary: string;
}
export interface TargetAssessment { target: CombatantId; name: string; hp: number; expected_damage: number; kill_probability: number; threat: number; priority: number; rationale: string }
export interface SpellAssessment { action: Action; description: string; eval_gain: number; expected_enemy_damage: number; expected_ally_healing: number; kill_probability: number }
export interface SimulationReport {
  runs: number;
  party_win_rate: number;
  enemy_win_rate: number;
  timeout_rate: number;
  tpk_probability: number;
  any_pc_death_probability: number;
  average_rounds: number;
  combatants: { id: CombatantId; name: string; side: Side; death_rate: number; down_rate: number }[];
}
export interface Strategy {
  side: Side;
  focus_target: CombatantId | null;
  creatures: { combatant: CombatantId; name: string; recommendation: TurnRecommendation | null; preferred_targets: TargetAssessment[] }[];
  notes: string[];
}
