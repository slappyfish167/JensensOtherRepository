// Thin typed client for the REST API (see docs/API.md).
import type {
  ActionView, AnyCombatView, BuildResult, Character, CharacterSummary, CombatRecord, CombatView, Command, Content, DiceMode, DocumentSummary, Encounter, SystemId, SystemInfo,
  EncounterBudget, EncounterSummary, Evaluation, LegalAction, Monster, MonsterSummary, SimulationReport,
  SpellAssessment, Strategy, TargetAssessment, TurnRecommendation, Side,
} from './types';

export class ApiError extends Error {
  constructor(public status: number, message: string, public details?: unknown) {
    super(message);
  }
}

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  const data = text ? JSON.parse(text) : null;
  if (!res.ok) {
    throw new ApiError(res.status, data?.error ?? `${res.status} ${res.statusText}`, data?.details);
  }
  return data as T;
}

const get = <T>(url: string) => request<T>('GET', url);
const post = <T>(url: string, body?: unknown) => request<T>('POST', url, body);
const put = <T>(url: string, body: unknown) => request<T>('PUT', url, body);
const del = <T>(url: string) => request<T>('DELETE', url);

export const api = {
  content: () => get<Content>('/api/content'),

  listCharacters: () => get<CharacterSummary[]>('/api/characters'),
  getCharacter: (id: string) => get<Character>(`/api/characters/${id}`),
  createCharacter: (c: Character) => post<{ character: Character; build: BuildResult }>('/api/characters', c),
  saveCharacter: (c: Character) => put<{ character: Character; build: BuildResult }>(`/api/characters/${c.id}`, c),
  importCharacter: (c: Character) => post<{ character: Character; build: BuildResult }>('/api/characters/import', c),
  deleteCharacter: (id: string) => del<{ deleted: boolean }>(`/api/characters/${id}`),
  validateCharacter: (c: Character) => post<BuildResult>('/api/characters/validate', c),

  listMonsters: () => get<MonsterSummary[]>('/api/monsters'),
  getMonster: (id: string) => get<Monster>(`/api/monsters/${id}`),
  saveMonster: (m: Monster) => put<Monster>(`/api/monsters/${m.id}`, m),
  importMonsters: (json: unknown) => post<Monster[]>('/api/monsters/import', json),
  deleteMonster: (id: string) => del<{ deleted: boolean }>(`/api/monsters/${id}`),
  templateMonster: (id: string, template: 'elite' | 'weak', save: boolean) =>
    post<Monster>(`/api/monsters/${id}/template`, { template, save }),

  listEncounters: () => get<EncounterSummary[]>('/api/encounters'),
  getEncounter: (id: string) => get<Encounter>(`/api/encounters/${id}`),
  saveEncounter: (e: Encounter) =>
    e.id
      ? put<{ encounter: Encounter; budget: EncounterBudget }>(`/api/encounters/${e.id}`, e)
      : post<{ encounter: Encounter; budget: EncounterBudget }>('/api/encounters', e),
  deleteEncounter: (id: string) => del<{ deleted: boolean }>(`/api/encounters/${id}`),
  budget: (e: Encounter) => post<EncounterBudget>('/api/encounters/budget', e),

  systems: () => get<SystemInfo[]>('/api/systems'),
  systemContent: (system: SystemId) => get<Record<string, unknown>>(`/api/systems/${system}/content`),
  listDocs: (system: SystemId, kind: 'actors' | 'encounters') => get<DocumentSummary[]>(`/api/systems/${system}/${kind}`),
  getDoc: <T = Record<string, unknown>>(system: SystemId, kind: 'actors' | 'encounters', id: string) => get<T>(`/api/systems/${system}/${kind}/${id}`),
  saveDoc: <T extends { id?: string }>(system: SystemId, kind: 'actors' | 'encounters', doc: T) =>
    doc.id ? put<T>(`/api/systems/${system}/${kind}/${doc.id}`, doc) : post<T>(`/api/systems/${system}/${kind}`, doc),
  deleteDoc: (system: SystemId, kind: 'actors' | 'encounters', id: string) => del<{ deleted: boolean }>(`/api/systems/${system}/${kind}/${id}`),

  listCombats: () => get<CombatRecord[]>('/api/combats'),
  createCombat: (encounter_id: string, dice: DiceMode, system: SystemId = 'pf2e') => post<AnyCombatView>('/api/combats', { system, encounter_id, dice }),
  getCombat: (id: string) => get<CombatView>(`/api/combats/${id}`),
  getAnyCombat: (id: string) => get<AnyCombatView>(`/api/combats/${id}`),
  commandAny: (id: string, cmd: unknown) => post<AnyCombatView>(`/api/combats/${id}/commands`, cmd),
  autoRollAny: (id: string) => post<AnyCombatView>(`/api/combats/${id}/auto-roll`),
  undoAny: (id: string) => post<AnyCombatView>(`/api/combats/${id}/undo`),
  actions: (id: string) => get<ActionView[]>(`/api/combats/${id}/actions`),
  deleteCombat: (id: string) => del<{ deleted: boolean }>(`/api/combats/${id}`),
  command: (id: string, cmd: Command) => post<CombatView>(`/api/combats/${id}/commands`, cmd),
  autoRoll: (id: string) => post<CombatView>(`/api/combats/${id}/auto-roll`),
  undo: (id: string) => post<CombatView>(`/api/combats/${id}/undo`),
  replay: (id: string) => get<{ identical: boolean; commands: number; live_fingerprint: string }>(`/api/combats/${id}/replay`),
  legalActions: (id: string) => get<LegalAction[]>(`/api/combats/${id}/legal-actions`),
  movement: (id: string, combatant: number) => get<{ x: number; y: number; cost: number }[]>(`/api/combats/${id}/movement/${combatant}`),

  evaluation: (id: string) => get<Evaluation>(`/api/combats/${id}/analysis/evaluation`),
  bestTurn: (id: string, node_budget = 40000) => post<TurnRecommendation>(`/api/combats/${id}/analysis/best-turn`, { node_budget }),
  targets: (id: string, attacker?: number) =>
    get<TargetAssessment[]>(`/api/combats/${id}/analysis/targets${attacker ? `?attacker=${attacker}` : ''}`),
  spells: (id: string) => get<SpellAssessment[]>(`/api/combats/${id}/analysis/spells`),
  tpk: (id: string, runs: number, seed: number) => post<SimulationReport>(`/api/combats/${id}/analysis/tpk`, { runs, seed }),
  strategy: (id: string, side: Side) => get<Strategy>(`/api/combats/${id}/analysis/strategy?side=${side}`),
};
