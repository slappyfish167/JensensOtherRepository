import { describe, expect, it } from 'vitest';
import { canAnalyzeToken, canForecast, canSeeEvalBar, computesEvaluationLocally, receivesBroadcastEvaluation } from '../src/permissions';
import { adjust, classifyMessage, emptyTurn, record, startTurn } from '../src/tracking/turn-tracker';
import { partyShare, scoreText, signed } from '../src/ui/format';
import { explainContext, turnContext } from '../src/ui/view-models';

describe('turn tracker', () => {
  const t0 = startTurn(emptyTurn('c'), 1, 'hero');

  it('counts actions and attacks of the current creature only', () => {
    let s = record(t0, { token: 'hero', kind: 'strike', actions: 1, attack: true });
    s = record(s, { token: 'hero', kind: 'spell', actions: 2, attack: false });
    s = record(s, { token: 'orc', kind: 'action', actions: 1, attack: false });
    expect([s.actionsSpent, s.attacksMade]).toEqual([3, 1]);
  });

  it('treats off-turn strikes as reactions and refreshes them on the next turn', () => {
    let s = record(t0, { token: 'orc', kind: 'strike', actions: 1, attack: true });
    expect(s.reactionsUsed).toEqual(['orc']);
    expect(s.actionsSpent).toBe(0);
    s = startTurn(s, 1, 'orc');
    expect(s.reactionsUsed).toEqual([]);
    expect(adjust(s, 'actionsSpent', -1).actionsSpent).toBe(0);
    expect(adjust(s, 'actionsSpent', 9).actionsSpent).toBe(4);
  });

  it('classifies PF2e chat messages', () => {
    const none = () => null;
    expect(classifyMessage({ speaker: { token: 't' }, flags: { pf2e: { context: { type: 'attack-roll' } } } }, none)).toMatchObject({ kind: 'strike', actions: 1, attack: true });
    expect(classifyMessage({ speaker: { token: 't' }, flags: { pf2e: { context: { type: 'damage-roll' } } } }, none)).toBeNull();
    expect(classifyMessage({ speaker: { token: 't' }, flags: { pf2e: { context: { type: 'skill-check', options: ['action:trip'], traits: ['attack'] } } } }, none)).toMatchObject({ actions: 1, attack: true });
    const spell = () => ({ type: 'spell', system: { time: { value: '2' } } });
    expect(classifyMessage({ speaker: { token: 't' }, flags: { pf2e: { origin: { type: 'spell', uuid: 'x' } } } }, spell)).toMatchObject({ kind: 'spell', actions: 2 });
    const reaction = () => ({ type: 'action', system: { actionType: { value: 'reaction' } } });
    expect(classifyMessage({ speaker: { token: 't' }, flags: { pf2e: { origin: { type: 'action', uuid: 'x' } } } }, reaction)).toMatchObject({ kind: 'reaction' });
    expect(classifyMessage({ flags: { pf2e: { context: { type: 'attack-roll' } } } }, none)).toBeNull();
  });
});

describe('permissions', () => {
  const ctx = (isGM: boolean, vis: 'gm' | 'all' | 'none', authoritative = true) => ({ isGM, settings: { evalBarVisibility: vis, playerBestTurn: true, gmAuthoritative: authoritative } });

  it('keeps analysis that leaks monster information GM-only by default', () => {
    expect(canSeeEvalBar(ctx(false, 'gm'))).toBe(false);
    expect(canSeeEvalBar(ctx(true, 'gm'))).toBe(true);
    expect(canForecast(ctx(false, 'all'))).toBe(false);
    expect(canAnalyzeToken(ctx(false, 'gm'), false)).toBe(false);
    expect(canAnalyzeToken(ctx(false, 'gm'), true)).toBe(true);
  });

  it('players get the GM score in authoritative mode', () => {
    expect(receivesBroadcastEvaluation(ctx(false, 'all'))).toBe(true);
    expect(computesEvaluationLocally(ctx(false, 'all'))).toBe(false);
    expect(computesEvaluationLocally(ctx(false, 'all', false))).toBe(true);
    expect(canSeeEvalBar(ctx(true, 'none'))).toBe(false);
  });
});

describe('presentation', () => {
  it('formats scores like a chess engine', () => {
    expect(scoreText(2.44)).toBe('+2.4');
    expect(scoreText(-0.96)).toBe('−1.0');
    expect(scoreText(10)).toBe('+M');
    expect(partyShare(-12)).toBe(0);
    expect(signed(-0.04)).toBe('−0.0');
  });

  it('builds template contexts', () => {
    const c = turnContext({
      token_id: 't', name: 'Kira', steps: [{ kind: 'strike', label: 'Strike Orc', action: {} }], baseline_score: 1, expected_score: 2.4,
      gain: 1.4, reasoning: ['r'], depth: 3, nodes: 1200, truncated: true, hypothetical: false, summary: '',
    });
    expect(c.steps[0]).toEqual({ n: 1, label: 'Strike Orc', icon: 'fa-solid fa-sword' });
    expect(c.gain).toBe('+1.4');
    expect(c.searchNote).toContain('time limit');
    const e = explainContext({ score: 1, previous_score: 0, delta: 1, reasons: [{ kind: 'flanked', token_id: 'o', text: 'Orc is flanked', impact: 0.8 }], factors: [] });
    expect(e.change?.cls).toBe('good');
    expect(e.reasons[0]?.cls).toBe('good');
  });
});
