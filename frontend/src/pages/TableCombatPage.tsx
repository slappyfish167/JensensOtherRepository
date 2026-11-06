import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api/client';
import type { Action, ActionView, AnyCombatView, CombatantId, Evaluation, Pos } from '../api/types';
import { AnalysisPanel } from '../components/AnalysisPanel';
import { BattleGrid, type GridToken } from '../components/BattleGrid';
import { EvalBar } from '../components/EvalBar';
import { NeutralPrompt } from '../components/NeutralPrompt';

const kindOf = (a: ActionView) => (a.action as { kind?: string } | null)?.kind ?? 'move';
const label = (kind: string) => kind.replace(/_/g, ' ').replace(/^./, (c) => c.toUpperCase());

/**
 * A combat screen for any game system, driven only by the engine's
 * system-neutral views (table, prompt, actions, log). Systems with a
 * dedicated screen (PF2e) can still be opened here.
 */
export function TableCombatPage({ initial }: { initial: AnyCombatView }) {
  const [view, setView] = useState<AnyCombatView>(initial);
  const [log, setLog] = useState<string[]>(initial.log);
  const [actions, setActions] = useState<ActionView[]>([]);
  const [evaluation, setEvaluation] = useState<Evaluation | null>(null);
  const [moveKind, setMoveKind] = useState<string | null>(null);
  const [selected, setSelected] = useState<CombatantId | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hpDelta, setHpDelta] = useState('-5');
  const logEnd = useRef<HTMLDivElement>(null);
  const t = view.table;
  const phase = t.phase.kind;

  useEffect(() => {
    setMoveKind(null);
    if (phase === 'active' && !view.prompt) api.actions(view.id).then(setActions).catch(() => setActions([]));
    else setActions([]);
    if (phase === 'active' || phase === 'ended') api.evaluation(view.id).then(setEvaluation).catch(() => setEvaluation(null));
  }, [view.fingerprint]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    logEnd.current?.scrollIntoView({ block: 'nearest' });
  }, [log.length]);

  const run = useCallback(async (f: () => Promise<AnyCombatView>) => {
    setBusy(true);
    setError(null);
    try {
      const v = await f();
      setView(v);
      setLog((l) => [...l, ...v.log].slice(-400));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }, []);
  const send = useCallback((cmd: unknown) => run(() => api.commandAny(view.id, cmd)), [view.id, run]);
  const act = useCallback((action: unknown) => view.current_actor != null && send({ type: 'act', actor: view.current_actor, action }), [view.current_actor, send]);

  const unit = (id: CombatantId | null | undefined) => t.combatants.find((c) => c.id === id);
  const actor = unit(view.current_actor);
  const inspected = unit(selected ?? view.current_actor);

  const moveKinds = useMemo(() => [...new Set(actions.filter((a) => a.move_to).map(kindOf))], [actions]);
  const groups = useMemo(() => {
    const g = new Map<string, ActionView[]>();
    for (const a of actions.filter((x) => !x.move_to)) g.set(a.group, [...(g.get(a.group) ?? []), a]);
    return [...g.entries()];
  }, [actions]);
  const highlights = useMemo(() => {
    const m = new Map<string, string>();
    if (moveKind) for (const a of actions) if (a.move_to && kindOf(a) === moveKind) m.set(`${a.move_to.x},${a.move_to.y}`, 'reach');
    return m;
  }, [actions, moveKind]);
  const terrain = useMemo(() => Object.fromEntries(t.map.legend.map((l) => [l.glyph, l.class])), [t.map.legend]);
  const tokens: GridToken[] = t.combatants
    .filter((c) => !(c.dead && c.kind === 'hazard'))
    .map((c) => ({ id: c.id, name: c.name, side: c.side, position: c.position, squares: c.squares, hp: c.hp, maxHp: c.max_hp, down: c.down, hazard: c.kind === 'hazard' }));

  function onCell(p: Pos) {
    if (!moveKind) return;
    const a = actions.find((x) => x.move_to && kindOf(x) === moveKind && x.move_to.x === p.x && x.move_to.y === p.y);
    if (a) act(a.action);
  }

  const order = t.initiative.length ? t.initiative.map((e) => ({ value: e.value as number | null, c: unit(e.combatant)! })).filter((x) => x.c) : t.combatants.map((c) => ({ value: null, c }));

  return (
    <div className="combat">
      <div className="combat-left">
        <EvalBar score={evaluation?.score ?? null} winProbability={evaluation?.win_probability} />
        <div className="panel initiative">
          <h3>Initiative{t.round ? ` · Round ${t.round}` : ''}</h3>
          <ol>
            {order.map(({ value, c }) => (
              <li key={c.id} className={`${c.side} ${c.id === t.current_actor ? 'current' : ''} ${c.id === selected ? 'selected' : ''} ${c.dead ? 'dead' : ''}`} onClick={() => setSelected(c.id)}>
                <div className="init-head">
                  <span className="init-val">{value ?? '–'}</span>
                  <span className="init-name">{c.name}</span>
                  <span className="init-hp">{c.hp}/{c.max_hp}</span>
                </div>
                <div className="hpbar"><i style={{ width: `${Math.max(0, Math.min(100, (100 * c.hp) / Math.max(1, c.max_hp)))}%` }} /></div>
                <div className="init-meta">
                  {c.id === t.current_actor && t.turn && <span className="pips">{t.turn}</span>}
                  {c.statuses.map((x) => <span key={x} className="tag">{x}</span>)}
                  {c.dead && <span className="tag bad">dead</span>}
                </div>
              </li>
            ))}
          </ol>
        </div>
      </div>

      <div className="combat-center">
        <div className="combat-header">
          <h2>{view.name} <span className="badge">{view.system_name}</span></h2>
          <span className="muted">
            {phase === 'ended' ? `Ended — ${t.phase.winner ?? 'no'} victory` : phase.replace('_', ' ')}
            {phase === 'active' && ` · Round ${t.round} · ${actor?.name ?? '—'}'s turn`}
            {' · '}{view.command_count} commands · {view.dice.mode} dice · squares are {t.map.square}
          </span>
          <div className="spacer" />
          <button disabled={busy || view.command_count === 0} onClick={() => run(() => api.undoAny(view.id))}>↶ Undo</button>
          <button
            disabled={busy}
            onClick={async () => {
              const r = await api.replay(view.id);
              setLog((l) => [...l, r.identical ? `Replay verified: ${r.commands} commands reproduce state ${r.live_fingerprint}` : 'Replay MISMATCH!']);
            }}
          >Verify replay</button>
        </div>
        {error && <div className="error" onClick={() => setError(null)}>{error}</div>}
        <BattleGrid
          battlefield={{ width: t.map.width, height: t.map.height, map: t.map.rows }}
          terrain={terrain}
          tokens={tokens}
          activeId={t.current_actor}
          selectedId={selected}
          highlights={highlights}
          onCellClick={onCell}
          onTokenClick={(id) => setSelected(id as CombatantId)}
        />
        <div className="muted small">{t.map.legend.map((l) => `${l.glyph} ${l.label}`).join(' · ')}</div>
        <div className="panel log">
          {log.map((l, i) => <div key={i}>{l}</div>)}
          <div ref={logEnd} />
        </div>
      </div>

      <div className="combat-right">
        {phase === 'setup' && (
          <div className="panel">
            <p>{t.combatants.length} combatants placed.</p>
            <button className="primary" disabled={busy} onClick={() => send({ type: 'start_combat' })}>Start combat (roll initiative)</button>
          </div>
        )}

        {view.prompt && (
          <NeutralPrompt
            prompt={view.prompt}
            actorName={unit(view.prompt.actor)?.name ?? ''}
            seeded={view.dice.mode === 'seeded'}
            busy={busy}
            onRoll={(value, faces, extra) => send({ type: 'provide_roll', request_id: view.prompt!.request_id, value, faces, extra_modifier: extra })}
            onAutoRoll={() => run(() => api.autoRollAny(view.id))}
            onDecide={(accept) => send({ type: 'decide', request_id: view.prompt!.request_id, accept })}
          />
        )}

        {phase === 'active' && !view.prompt && actor && (
          <div className="panel actions">
            <h3>{actor.name} {t.turn && <span className="pips">{t.turn}</span>}</h3>
            {moveKind && (
              <div className="mode-hint">
                Click a highlighted square to {moveKind.replace(/_/g, ' ')}
                <button onClick={() => setMoveKind(null)}>Cancel</button>
              </div>
            )}
            <div className="row wrap">
              {moveKinds.map((k) => (
                <button key={k} disabled={busy} className={moveKind === k ? 'on' : ''} onClick={() => setMoveKind(k)}>{label(k)}</button>
              ))}
              <button className="primary" disabled={busy} onClick={() => send({ type: 'end_turn', actor: actor.id })}>End turn</button>
            </div>
            {groups.map(([group, items]) => (
              <div key={group} className="action-group">
                <h4>{group}</h4>
                {items.map((a, i) => (
                  <button key={i} className="action" disabled={busy} onClick={() => act(a.action)}>{a.description}</button>
                ))}
              </div>
            ))}
          </div>
        )}

        <AnalysisPanel
          combatId={view.id}
          ready={phase === 'active' && !view.prompt}
          active={phase === 'active'}
          currentActor={view.current_actor}
          actorName={actor?.name ?? ''}
          spells={view.system === 'pf2e'}
          evaluation={evaluation}
          onPlay={(a: Action) => act(a)}
        />

        {inspected && (
          <div className="panel">
            <h3>{inspected.name} <span className="badge">{inspected.kind}</span></h3>
            <div className="muted small">{inspected.hp}/{inspected.max_hp} HP{inspected.temp_hp ? ` (+${inspected.temp_hp} temp)` : ''} · {inspected.side}</div>
            <dl className="statlines">
              {inspected.stats.map((l, i) => (
                <div key={i} style={{ display: 'contents' }}>
                  <dt>{l.label}</dt>
                  <dd>{l.value}</dd>
                </div>
              ))}
            </dl>
            <details className="gm">
              <summary>GM tools — {inspected.name}</summary>
              <div className="row">
                <input className="narrow" type="number" value={hpDelta} onChange={(e) => setHpDelta(e.target.value)} />
                <button disabled={busy} onClick={() => send({ type: 'gm', op: { kind: 'adjust_hp', target: inspected.id, delta: Number(hpDelta) } })}>Adjust HP</button>
                <button disabled={busy} onClick={() => send({ type: 'gm', op: { kind: 'clear_pending' } })} title="Abandon the action being resolved">Clear pending</button>
              </div>
            </details>
          </div>
        )}
      </div>
    </div>
  );
}
