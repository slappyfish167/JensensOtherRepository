// Report → template context. Pure (unit-tested); templates contain no logic
// beyond iteration and conditionals.

import type { ExplainReport, ForecastReport, SidePlanReport, TurnReport } from '../protocol/report';
import { impactClass, pct, scoreText, signed, STEP_ICONS } from './format';

export function turnContext(r: TurnReport, showReasoning = true) {
  return {
    name: r.name,
    tokenId: r.token_id,
    hypothetical: r.hypothetical,
    empty: r.steps.length === 0,
    steps: r.steps.map((s, i) => ({ n: i + 1, label: s.label, icon: STEP_ICONS[s.kind] ?? 'fa-solid fa-circle' })),
    gain: signed(r.gain),
    gainClass: impactClass(r.gain),
    before: scoreText(r.baseline_score),
    after: scoreText(r.expected_score),
    reasoning: showReasoning ? r.reasoning : [],
    searchNote: `${r.truncated ? 'time limit reached — ' : ''}depth ${r.depth}, ${r.nodes.toLocaleString()} positions`,
  };
}

export function sidePlanContext(p: SidePlanReport) {
  const focus = p.creatures.flatMap((c) => c.targets).find((t) => t.token_id === p.focus_token);
  return {
    focusName: focus?.name ?? null,
    notes: p.notes,
    creatures: p.creatures.map((c) => ({
      name: c.name,
      tokenId: c.token_id,
      plan: c.turn ? turnContext(c.turn, true) : null,
      targets: c.targets.slice(0, 3).map((t) => ({
        name: t.name,
        kill: pct(t.kill_probability),
        damage: t.expected_damage.toFixed(1),
        rationale: t.rationale,
      })),
    })),
  };
}

export function forecastContext(f: ForecastReport) {
  const bar = (label: string, v: number, cls: string) => ({ label, value: pct(v), width: `${(v * 100).toFixed(1)}%`, cls });
  return {
    runs: f.runs,
    margin: `±${(f.margin_of_error * 100).toFixed(1)} pts`,
    rounds: f.average_rounds.toFixed(1),
    bars: [bar('Victory', f.victory, 'good'), bar('Defeat', f.defeat, 'bad'), bar('TPK', f.tpk, 'bad'), bar('Any PC death', f.any_pc_death, 'bad'), bar('Unresolved', f.unresolved, 'neutral')],
    party: f.combatants.filter((c) => c.side === 'party').map((c) => ({ name: c.name, down: pct(c.down_rate), death: pct(c.death_rate) })),
    enemies: f.combatants.filter((c) => c.side !== 'party').map((c) => ({ name: c.name, down: pct(c.down_rate), death: pct(c.death_rate) })),
  };
}

export function explainContext(e: ExplainReport) {
  return {
    score: scoreText(e.score),
    change: e.previous_score === null ? null : { from: scoreText(e.previous_score), delta: signed(e.delta), cls: impactClass(e.delta) },
    reasons: e.reasons.map((r) => ({ text: r.text, impact: signed(r.impact), cls: impactClass(r.impact), tokenId: r.token_id })),
    factors: e.factors.map((f) => ({ name: f.name, value: signed(f.after), cls: impactClass(f.after), moved: Math.abs(f.after - f.before) > 0.05 ? signed(f.after - f.before) : '' })),
  };
}
