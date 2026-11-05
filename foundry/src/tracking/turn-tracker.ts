// Turn tracker: actions spent, attacks made (MAP) and reactions used.
//
// State lives in a combat flag written only by the active GM, so every
// client builds the same snapshot. The reducer and the chat classifier are
// pure; Foundry glue (hooks.ts) feeds them documents.

export interface TurnState {
  combatId: string;
  round: number;
  combatantToken: string | null;
  actionsSpent: number;
  attacksMade: number;
  /** Tokens whose reaction is spent (refreshed at the start of their turn). */
  reactionsUsed: string[];
}

export function emptyTurn(combatId: string): TurnState {
  return { combatId, round: 0, combatantToken: null, actionsSpent: 0, attacksMade: 0, reactionsUsed: [] };
}

/** A new turn starts: reset the counters and refresh that creature's reaction. */
export function startTurn(s: TurnState, round: number, token: string | null): TurnState {
  return {
    ...s,
    round,
    combatantToken: token,
    actionsSpent: 0,
    attacksMade: 0,
    reactionsUsed: s.reactionsUsed.filter((t) => t !== token),
  };
}

export type TrackedKind = 'strike' | 'spell' | 'action' | 'reaction' | 'free' | 'move';

export interface TrackedEvent {
  token: string;
  kind: TrackedKind;
  /** Actions consumed (0 for free actions and reactions). */
  actions: number;
  /** Counts toward the multiple attack penalty. */
  attack: boolean;
}

export function record(s: TurnState, e: TrackedEvent): TurnState {
  const isCurrent = e.token === s.combatantToken;
  // Off-turn Strikes and explicit reactions spend the reaction.
  if (e.kind === 'reaction' || (!isCurrent && e.kind === 'strike')) {
    return s.reactionsUsed.includes(e.token) ? s : { ...s, reactionsUsed: [...s.reactionsUsed, e.token] };
  }
  if (!isCurrent) return s;
  return {
    ...s,
    actionsSpent: Math.min(4, s.actionsSpent + Math.max(0, e.actions)),
    attacksMade: Math.min(10, s.attacksMade + (e.attack ? 1 : 0)),
  };
}

/** Manual correction from the UI (± one action or attack). */
export function adjust(s: TurnState, field: 'actionsSpent' | 'attacksMade', delta: number): TurnState {
  const max = field === 'actionsSpent' ? 4 : 10;
  return { ...s, [field]: Math.max(0, Math.min(max, s[field] + delta)) };
}

// ---------------------------------------------------------------------------
// PF2e chat message classification
// ---------------------------------------------------------------------------

export interface MessageLike {
  speaker?: { token?: string | null; actor?: string | null };
  flags?: {
    pf2e?: {
      context?: { type?: string; options?: string[]; traits?: (string | { name?: string })[] } | null;
      origin?: { type?: string; uuid?: string | null } | null;
      casting?: unknown;
    };
  };
}

/** Action cost data of the item behind a message (resolved by the caller). */
export interface OriginItemLike {
  type?: string;
  system?: { actionType?: { value?: string }; actions?: { value?: number | null }; time?: { value?: string } };
}

function traitNames(t: (string | { name?: string })[] | undefined): string[] {
  return (t ?? []).map((x) => (typeof x === 'string' ? x : (x.name ?? ''))).filter(Boolean);
}

/**
 * Turns a chat message into a tracked event, or null if it does not spend
 * actions (damage rolls, saves, flavor text, ...).
 */
export function classifyMessage(msg: MessageLike, resolveOrigin: (uuid: string) => OriginItemLike | null): TrackedEvent | null {
  const token = msg.speaker?.token;
  const pf2e = msg.flags?.pf2e;
  if (!token || !pf2e) return null;
  const ctx = pf2e.context;
  switch (ctx?.type) {
    case 'attack-roll':
      return { token, kind: 'strike', actions: 1, attack: true };
    case 'spell-attack-roll':
      // The cast itself is counted from the spell card; only MAP here.
      return { token, kind: 'free', actions: 0, attack: true };
    case 'skill-check': {
      const options = ctx.options ?? [];
      if (!options.some((o) => o.startsWith('action:'))) return null;
      return { token, kind: 'action', actions: 1, attack: traitNames(ctx.traits).includes('attack') };
    }
    case undefined:
      break;
    default:
      return null; // damage-roll, saving-throw, flat-check, initiative, ...
  }
  const origin = pf2e.origin;
  if (!origin?.uuid) return null;
  const item = resolveOrigin(origin.uuid);
  if (!item) return null;
  if (item.type === 'spell' || origin.type === 'spell') {
    const t = item.system?.time?.value ?? '';
    if (t === 'reaction') return { token, kind: 'reaction', actions: 0, attack: false };
    if (t === 'free') return { token, kind: 'free', actions: 0, attack: false };
    const n = Number.parseInt(t, 10);
    return Number.isFinite(n) && n >= 1 && n <= 3 ? { token, kind: 'spell', actions: n, attack: false } : null;
  }
  const actionType = item.system?.actionType?.value;
  if (actionType === 'reaction') return { token, kind: 'reaction', actions: 0, attack: false };
  if (actionType === 'free') return { token, kind: 'free', actions: 0, attack: false };
  if (actionType === 'action') {
    const n = item.system?.actions?.value ?? 1;
    return { token, kind: 'action', actions: Math.max(1, Math.min(3, n)), attack: false };
  }
  return null;
}
