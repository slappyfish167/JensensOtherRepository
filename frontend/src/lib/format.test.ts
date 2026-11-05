import { describe, expect, it } from 'vitest';
import { describeEvent, evalBand, evalText, signed } from './format';
import type { CombatState } from '../api/types';

const state = {
  combatants: [
    { id: 1, name: 'Kira' },
    { id: 2, name: 'Goblin' },
  ],
} as unknown as CombatState;

describe('format helpers', () => {
  it('signs numbers', () => {
    expect(signed(3)).toBe('+3');
    expect(signed(-2)).toBe('-2');
    expect(signed(0)).toBe('+0');
  });

  it('renders evaluations like a chess engine', () => {
    expect(evalText(3.42)).toBe('+3.4');
    expect(evalText(-1.25)).toBe('−1.3');
    expect(evalText(10)).toBe('+10 (party wins)');
    expect(evalBand(0.3)).toBe('balanced');
    expect(evalBand(6)).toBe('strong player advantage');
    expect(evalBand(-9)).toBe('near-certain player defeat');
  });

  it('describes events with names', () => {
    expect(describeEvent({ type: 'damage_taken', target: 2, amount: 7, lines: [], hp: 3, temp_hp: 0 }, state)).toBe(
      'Goblin takes 7 damage → 3 HP',
    );
    expect(
      describeEvent({ type: 'check_resolved', roller: 1, purpose: 'attack', natural: 15, total: 24, dc: 16, degree: 'success' }, state),
    ).toBe('attack: 15 → 24 vs 16 — success');
    expect(describeEvent({ type: 'moved', combatant: 1, from: { x: 0, y: 0 }, to: { x: 1, y: 0 } }, state)).toBeNull();
  });
});
