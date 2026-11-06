import { useState } from 'react';
import { api } from '../api/client';
import type { Action, CombatantId, Evaluation, SimulationReport, SpellAssessment, Strategy, TargetAssessment, TurnRecommendation } from '../api/types';
import { evalText } from '../lib/format';
import { TacticalReportView } from './TacticalReportView';

type Tab = 'eval' | 'best' | 'targets' | 'spells' | 'tpk' | 'strategy';

interface Props {
  combatId: string;
  /** Combat running and no input pending. */
  ready: boolean;
  /** Combat running (input may be pending). */
  active: boolean;
  currentActor: CombatantId | null;
  actorName: string;
  /** The system has spellcasting (shows the Spells tab). */
  spells?: boolean;
  evaluation: Evaluation | null;
  onPlay: (action: Action) => void;
}

/** Engine analysis for any game system: the questions from the brief, one tab each. */
export function AnalysisPanel({ combatId, ready, active, currentActor, actorName, spells: hasSpells = true, evaluation, onPlay }: Props) {
  const [tab, setTab] = useState<Tab>('eval');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [best, setBest] = useState<TurnRecommendation | null>(null);
  const [targets, setTargets] = useState<TargetAssessment[] | null>(null);
  const [spells, setSpells] = useState<SpellAssessment[] | null>(null);
  const [tpk, setTpk] = useState<SimulationReport | null>(null);
  const [strategy, setStrategy] = useState<Strategy | null>(null);
  const [runs, setRuns] = useState(200);
  const id = combatId;

  async function go<T>(f: () => Promise<T>, set: (v: T) => void) {
    setBusy(true);
    setError(null);
    try {
      set(await f());
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const tabs = ([
    ['eval', 'Evaluation'], ['best', 'Best turn'], ['targets', 'Targets'], ['spells', 'Spells'], ['tpk', 'TPK odds'], ['strategy', 'Strategy'],
  ] as [Tab, string][]).filter(([t]) => t !== 'spells' || hasSpells);
  return (
    <div className="panel analysis">
      <div className="tabs">
        {tabs.map(([t, label]) => (
          <button key={t} className={t === tab ? 'on' : ''} onClick={() => setTab(t)}>{label}</button>
        ))}
      </div>
      {error && <div className="error">{error}</div>}

      {tab === 'eval' && evaluation && (
        <div>
          <p className="big">{evalText(evaluation.score)} <span className="muted">win ≈ {(100 * evaluation.win_probability).toFixed(0)}%</span></p>
          <p className="muted small">
            Party {evaluation.party.hit_points.toFixed(0)} HP · {evaluation.party.damage_per_round.toFixed(1)} dmg/round — Enemies {evaluation.enemies.hit_points.toFixed(0)} HP · {evaluation.enemies.damage_per_round.toFixed(1)} dmg/round
          </p>
          <table className="factors">
            <tbody>
              {evaluation.factors.map((f) => (
                <tr key={f.name} title={f.detail}>
                  <td>{f.name}</td>
                  <td className={f.value > 0.05 ? 'good' : f.value < -0.05 ? 'bad' : 'muted'}>{f.value >= 0 ? '+' : ''}{f.value.toFixed(2)}</td>
                  <td className="muted small">{f.detail}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {tab === 'best' && (
        <div>
          <button disabled={busy || !ready} onClick={() => go(() => api.bestTurn(id), setBest)}>
            {busy ? 'Searching…' : 'What is the best turn?'}
          </button>
          {best && (
            <div>
              <TacticalReportView report={best.report} />
              <p className="muted small">{best.plan.nodes.toLocaleString()} nodes{best.plan.truncated ? ' (budget reached)' : ''} · gain {best.plan.gain >= 0 ? '+' : ''}{best.plan.gain.toFixed(2)}</p>
              {best.plan.actions[0] && ready && currentActor === best.plan.actor && (
                <button className="primary" onClick={() => { onPlay(best.plan.actions[0]); setBest(null); }}>Play first action</button>
              )}
            </div>
          )}
        </div>
      )}

      {tab === 'targets' && (
        <div>
          <button disabled={busy || !active} onClick={() => go(() => api.targets(id), setTargets)}>
            What is the strongest target?
          </button>
          {targets && (
            <table>
              <thead><tr><th>Target</th><th>Priority</th><th>Exp. dmg</th><th>Drop now</th><th>Threat</th></tr></thead>
              <tbody>
                {targets.map((t) => (
                  <tr key={t.target} title={t.rationale}>
                    <td>{t.name} <span className="muted">({t.hp} HP)</span></td>
                    <td>{t.priority.toFixed(2)}</td>
                    <td>{t.expected_damage.toFixed(1)}</td>
                    <td>{(100 * t.kill_probability).toFixed(0)}%</td>
                    <td>{t.threat.toFixed(1)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}

      {tab === 'spells' && (
        <div>
          <button disabled={busy || !ready} onClick={() => go(() => api.spells(id), setSpells)}>Which spell maximizes expected value?</button>
          {spells && spells.length === 0 && <p className="muted">No castable spells for {actorName}.</p>}
          {spells && spells.length > 0 && (
            <table>
              <thead><tr><th>Spell</th><th>Δ eval</th><th>Exp. dmg</th><th>Healing</th><th>Kill</th><th /></tr></thead>
              <tbody>
                {spells.map((s, i) => (
                  <tr key={i}>
                    <td>{s.description}</td>
                    <td className={s.eval_gain > 0 ? 'good' : 'bad'}>{s.eval_gain >= 0 ? '+' : ''}{s.eval_gain.toFixed(2)}</td>
                    <td>{s.expected_enemy_damage.toFixed(1)}</td>
                    <td>{s.expected_ally_healing.toFixed(1)}</td>
                    <td>{(100 * s.kill_probability).toFixed(0)}%</td>
                    <td><button onClick={() => onPlay(s.action)}>Cast</button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}

      {tab === 'tpk' && (
        <div>
          <div className="row">
            <label>Simulations <input type="number" className="narrow" value={runs} min={10} max={2000} onChange={(e) => setRuns(Number(e.target.value))} /></label>
            <button disabled={busy} onClick={() => go(() => api.tpk(id, runs, 1), setTpk)}>{busy ? 'Simulating…' : 'How likely is a TPK?'}</button>
          </div>
          <p className="muted small">Monte Carlo with seeded dice and a scripted policy; your live combat is not affected.</p>
          {tpk && (
            <div>
              <p className="big">TPK {(100 * tpk.tpk_probability).toFixed(1)}%</p>
              <p>Party wins {(100 * tpk.party_win_rate).toFixed(0)}% · enemies win {(100 * tpk.enemy_win_rate).toFixed(0)}% · timeouts {(100 * tpk.timeout_rate).toFixed(0)}% · ≥1 PC death {(100 * tpk.any_pc_death_probability).toFixed(0)}% · avg {tpk.average_rounds.toFixed(1)} rounds</p>
              <table>
                <thead><tr><th>Combatant</th><th>Dies</th><th>Goes down</th></tr></thead>
                <tbody>{tpk.combatants.map((c) => (
                  <tr key={c.id} className={c.side}><td>{c.name}</td><td>{(100 * c.death_rate).toFixed(0)}%</td><td>{(100 * c.down_rate).toFixed(0)}%</td></tr>
                ))}</tbody>
              </table>
            </div>
          )}
        </div>
      )}

      {tab === 'strategy' && (
        <div>
          <button disabled={busy || !active} onClick={() => go(() => api.strategy(id, 'enemy'), setStrategy)}>
            {busy ? 'Thinking…' : 'What is the optimal monster strategy?'}
          </button>
          {strategy && (
            <div>
              {strategy.notes.map((n, i) => <p key={i}>{n}</p>)}
              {strategy.creatures.map((c) => (
                <div key={c.combatant} className="strategy-creature">
                  <strong>{c.name}</strong>
                  {c.recommendation ? <TacticalReportView report={c.recommendation.report} /> : <div className="muted">cannot act this round</div>}
                  {c.preferred_targets[0] && <div className="muted small">target order: {c.preferred_targets.map((t) => t.name).join(' > ')}</div>}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
