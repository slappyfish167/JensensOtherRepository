// Pure presentation helpers (unit-tested in format.test.ts).
import type { CombatEvent, CombatState, Combatant, CombatantId, Condition } from '../api/types';

export function signed(n: number): string {
  return n >= 0 ? `+${n}` : `${n}`;
}

export function nameOf(state: CombatState, id: CombatantId | null | undefined): string {
  if (id == null) return '—';
  return state.combatants.find((c) => c.id === id)?.name ?? `#${id}`;
}

export function conditionLabel(c: Condition): string {
  const name = c.kind.replace(/_/g, '-');
  return c.value ? `${name} ${c.value}` : name;
}

export function hpPercent(c: Combatant): number {
  return Math.max(0, Math.min(100, (100 * c.hp) / Math.max(1, c.statblock.max_hp)));
}

/** Evaluation text like a chess engine: "+3.4" / "−10 (decided)". */
export function evalText(score: number): string {
  if (Math.abs(score) >= 9.995) return score > 0 ? '+10 (party wins)' : '−10 (party falls)';
  const s = score.toFixed(1);
  return score > 0 ? `+${s}` : score < 0 ? `−${s.slice(1)}` : '0.0';
}

/** Verbal band for a −10..+10 evaluation. */
export function evalBand(score: number): string {
  const a = Math.abs(score);
  const who = score > 0 ? 'player' : 'monster';
  if (a < 1) return 'balanced';
  if (a < 3) return `slight ${who} edge`;
  if (a < 5) return `clear ${who} advantage`;
  if (a < 8) return `strong ${who} advantage`;
  return score > 0 ? 'overwhelming player advantage' : 'near-certain player defeat';
}

export function describeEvent(e: CombatEvent, s: CombatState): string | null {
  const n = (id: CombatantId | null | undefined) => nameOf(s, id);
  switch (e.type) {
    case 'combat_started': return 'Combat begins — roll initiative.';
    case 'initiative_order': return `Initiative: ${e.order.map(([id, v]) => `${n(id)} ${v}`).join(', ')}`;
    case 'round_started': return `— Round ${e.round} —`;
    case 'turn_started': return `${n(e.combatant)}'s turn (${e.actions} action${e.actions === 1 ? '' : 's'})`;
    case 'turn_ended': return `${n(e.combatant)} ends their turn`;
    case 'action_declared': return `${n(e.actor)}: ${e.action}${e.cost > 1 ? ` (${e.cost} actions)` : ''}`;
    case 'moved': return null;
    case 'check_resolved':
      return e.dc > 0
        ? `${e.purpose}: ${e.natural} → ${e.total} vs ${e.dc} — ${e.degree.replace(/_/g, ' ')}`
        : `${n(e.roller)} ${e.purpose}: ${e.total}`;
    case 'flat_check': return `${n(e.combatant)} flat check (${e.purpose}) ${e.natural} vs DC ${e.dc}: ${e.success ? 'success' : 'failure'}`;
    case 'damage_rolled': return `${e.purpose}: rolled ${e.rolled}${e.counted !== e.rolled ? ` → ${e.counted}` : ''}`;
    case 'damage_taken': {
      const notes = e.lines.filter((l) => l.note).map((l) => `${l.damage_type} ${l.note}`).join('; ');
      return `${n(e.target)} takes ${e.amount} damage${notes ? ` (${notes})` : ''} → ${e.hp} HP`;
    }
    case 'shield_blocked': return `${n(e.target)} blocks ${e.prevented} with a shield${e.broken ? ' — the shield breaks!' : ''}`;
    case 'healed': return `${n(e.target)} heals ${e.amount} → ${e.hp} HP`;
    case 'condition_gained': return `${n(e.target)} is ${e.condition.replace(/_/g, '-')}${e.value ? ` ${e.value}` : ''}`;
    case 'condition_lost': return `${n(e.target)} is no longer ${e.condition.replace(/_/g, '-')}`;
    case 'effect_gained': return `${n(e.target)} gains ${e.name}`;
    case 'effect_ended': return `${e.name} ends on ${n(e.target)}`;
    case 'reaction_used': return `${n(e.reactor)} uses ${e.reaction.replace(/([a-z])([A-Z])/g, '$1 $2')}`;
    case 'action_disrupted': return `${n(e.actor)}'s action is disrupted!`;
    case 'knocked_out': return `${n(e.combatant)} falls unconscious (dying ${e.dying})`;
    case 'died': return `${n(e.combatant)} dies`;
    case 'hazard_triggered': return `${n(e.hazard)} triggers on ${n(e.victim)}!`;
    case 'hazard_disabled': return `${n(e.hazard)} is disabled`;
    case 'combat_ended': return e.winner ? `Combat over — ${e.winner === 'party' ? 'the party' : 'the enemies'} win.` : 'Combat over.';
    case 'note': return e.text;
  }
}
