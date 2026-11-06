import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api/client';
import type { Action, AnyCombatView, CombatRecord, CombatView, CombatantId, Command, ConditionKind, Evaluation, LegalAction, Pos } from '../api/types';
import { CONDITIONS } from '../api/types';
import { AnalysisPanel } from '../components/AnalysisPanel';
import { BattleGrid, tokensFromCombatants } from '../components/BattleGrid';
import { EvalBar } from '../components/EvalBar';
import { InitiativeTracker } from '../components/InitiativeTracker';
import { PendingPanel } from '../components/PendingPanel';
import { StatblockView } from '../components/StatblockView';
import { describeEvent, nameOf } from '../lib/format';
import { TableCombatPage } from './TableCombatPage';

type Mode = null | { kind: 'stride' | 'step' } | { kind: 'aim'; entry: number; spell_id: string; rank: number };

export function CombatPage({ combatId, onOpen }: { combatId: string | null; onOpen: (id: string) => void }) {
  const [combats, setCombats] = useState<CombatRecord[]>([]);
  const [view, setView] = useState<CombatView | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [legal, setLegal] = useState<LegalAction[]>([]);
  const [evaluation, setEvaluation] = useState<Evaluation | null>(null);
  const [mode, setMode] = useState<Mode>(null);
  const [moveSquares, setMoveSquares] = useState<Map<string, string>>(new Map());
  const [selected, setSelected] = useState<CombatantId | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const logEnd = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.listCombats().then(setCombats).catch((e) => setError(e.message));
  }, [combatId]);

  useEffect(() => {
    setView(null);
    setLog([]);
    if (combatId) api.getCombat(combatId).then(setView).catch((e) => setError(e.message));
  }, [combatId]);

  // Refresh derived data whenever the state changes.
  useEffect(() => {
    if (!view || view.system !== 'pf2e') return;
    setMode(null);
    setMoveSquares(new Map());
    if (view.state.phase.kind === 'active' && !view.pending) {
      api.legalActions(view.id).then(setLegal).catch(() => setLegal([]));
    } else {
      setLegal([]);
    }
    if (view.state.phase.kind !== 'setup' && view.state.phase.kind !== 'rolling_initiative') {
      api.evaluation(view.id).then(setEvaluation).catch(() => setEvaluation(null));
    }
  }, [view?.fingerprint]); // eslint-disable-line react-hooks/exhaustive-deps

  // Block body on purpose: scrollIntoView() returns a Promise in recent
  // browsers, and an effect must not return anything but a cleanup function.
  useEffect(() => {
    logEnd.current?.scrollIntoView({ block: 'nearest' });
  }, [log.length]);

  const run = useCallback(async (f: () => Promise<CombatView>) => {
    setBusy(true);
    setError(null);
    try {
      const v = await f();
      setView(v);
      const lines = v.events.map((e) => describeEvent(e, v.state)).filter((x): x is string => !!x);
      setLog((l) => [...l, ...lines].slice(-400));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }, []);

  const send = useCallback((cmd: Command) => view && run(() => api.command(view.id, cmd)), [view, run]);
  const act = useCallback((action: Action) => view?.current_actor != null && send({ type: 'act', actor: view.current_actor, action }), [view, send]);

  const actor = view?.state.combatants.find((c) => c.id === view.current_actor) ?? null;
  const inspected = view?.state.combatants.find((c) => c.id === (selected ?? view.current_actor)) ?? null;

  const grouped = useMemo(() => {
    const g: Record<string, LegalAction[]> = { Strikes: [], Spells: [], Maneuvers: [], Other: [] };
    for (const a of legal) {
      if (a.action.kind === 'step') continue;
      const k = a.action.kind === 'strike' ? 'Strikes' : a.action.kind === 'cast_spell' ? 'Spells' : ['trip', 'demoralize', 'disable_hazard'].includes(a.action.kind) ? 'Maneuvers' : 'Other';
      g[k].push(a);
    }
    return g;
  }, [legal]);

  const areaSpells = useMemo(() => {
    const seen = new Map<string, { entry: number; spell_id: string; rank: number }>();
    for (const a of legal) {
      if (a.action.kind === 'cast_spell' && a.action.aim) seen.set(`${a.action.spell_id}@${a.action.rank}`, a.action);
    }
    return [...seen.values()];
  }, [legal]);

  async function startMove(kind: 'stride' | 'step') {
    if (!view || !actor) return;
    setMode({ kind });
    const map = new Map<string, string>();
    if (kind === 'stride') {
      for (const sq of await api.movement(view.id, actor.id)) map.set(`${sq.x},${sq.y}`, 'reach');
    } else {
      for (const a of legal) if (a.action.kind === 'step') map.set(`${a.action.to.x},${a.action.to.y}`, 'reach');
    }
    setMoveSquares(map);
  }

  function onCellClick(p: Pos) {
    if (!mode) return;
    if (mode.kind === 'stride' || mode.kind === 'step') {
      if (moveSquares.has(`${p.x},${p.y}`)) act({ kind: mode.kind, to: p });
    } else if (mode.kind === 'aim') {
      act({ kind: 'cast_spell', entry: mode.entry, spell_id: mode.spell_id, rank: mode.rank, targets: [], aim: p });
    }
  }

  if (!combatId || !view) {
    return (
      <div className="page">
        <h2>Combats</h2>
        {error && <div className="error">{error}</div>}
        <p className="muted">Start a combat from an Encounters page, or resume one (any game system):</p>
        <ul className="list">
          {combats.map((c) => (
            <li key={c.id} onClick={() => onOpen(c.id)}>
              <strong>{c.name}</strong> <span className="badge">{c.system === 'cpred' ? 'Cyberpunk RED' : 'Pathfinder 2e'}</span>{' '}
              <span className="muted">{c.status} · {c.dice.mode} dice · {new Date(c.created_at * 1000).toLocaleString()}</span>
            </li>
          ))}
        </ul>
      </div>
    );
  }

  // Systems without a dedicated screen run on the system-neutral one.
  if (view.system !== 'pf2e') return <TableCombatPage key={view.id} initial={view as unknown as AnyCombatView} />;

  const s = view.state;
  const phase = s.phase;
  return (
    <div className="combat">
      <div className="combat-left">
        <EvalBar score={evaluation?.score ?? null} winProbability={evaluation?.win_probability} />
        <InitiativeTracker state={s} selected={selected} onSelect={setSelected} />
      </div>

      <div className="combat-center">
        <div className="combat-header">
          <h2>{view.name}</h2>
          <span className="muted">
            {phase.kind === 'ended' ? `Ended — ${phase.winner ?? 'no'} victory` : phase.kind.replace('_', ' ')}
            {phase.kind === 'active' && ` · Round ${s.round} · ${nameOf(s, view.current_actor)}'s turn`}
            {' · '}{view.command_count} commands · {view.dice.mode} dice
          </span>
          <div className="spacer" />
          <button disabled={busy || view.command_count === 0} onClick={() => run(() => api.undo(view.id))}>↶ Undo</button>
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
          battlefield={s.battlefield}
          tokens={tokensFromCombatants(s.combatants)}
          activeId={view.current_actor}
          selectedId={selected}
          highlights={moveSquares}
          onCellClick={onCellClick}
          onTokenClick={(id) => setSelected(id as CombatantId)}
        />
        <div className="panel log">
          {log.map((l, i) => <div key={i}>{l}</div>)}
          <div ref={logEnd} />
        </div>
      </div>

      <div className="combat-right">
        {phase.kind === 'setup' && (
          <div className="panel">
            <p>{s.combatants.length} combatants placed.</p>
            <button className="primary" disabled={busy} onClick={() => send({ type: 'start_combat' })}>Start combat (roll initiative)</button>
          </div>
        )}

        {view.pending && (
          <PendingPanel
            pending={view.pending}
            state={s}
            seeded={view.dice.mode === 'seeded'}
            busy={busy}
            onRoll={(value, extra) => send({ type: 'provide_roll', request_id: view.pending!.request_id, value, extra_modifier: extra })}
            onAutoRoll={() => run(() => api.autoRoll(view.id))}
            onDecide={(accept) => send({ type: 'decide', request_id: view.pending!.request_id, accept })}
          />
        )}

        {phase.kind === 'active' && !view.pending && actor && (
          <div className="panel actions">
            <h3>{actor.name} <span className="pips">{'◆'.repeat(actor.actions_remaining)}{'◇'.repeat(Math.max(0, 3 - actor.actions_remaining))}</span></h3>
            {mode && (
              <div className="mode-hint">
                {mode.kind === 'aim' ? `Click a square to aim ${mode.spell_id}` : `Click a highlighted square to ${mode.kind}`}
                <button onClick={() => { setMode(null); setMoveSquares(new Map()); }}>Cancel</button>
              </div>
            )}
            <div className="row wrap">
              <button disabled={busy} onClick={() => startMove('stride')}>Stride</button>
              <button disabled={busy || !legal.some((a) => a.action.kind === 'step')} onClick={() => startMove('step')}>Step</button>
              {areaSpells.map((sp) => (
                <button key={`${sp.spell_id}${sp.rank}`} disabled={busy} onClick={() => setMode({ kind: 'aim', ...sp })}>Aim {s.spells[sp.spell_id]?.name ?? sp.spell_id}…</button>
              ))}
              <button className="primary" disabled={busy} onClick={() => send({ type: 'end_turn', actor: actor.id })}>End turn</button>
            </div>
            {Object.entries(grouped).map(([group, items]) =>
              items.length ? (
                <div key={group} className="action-group">
                  <h4>{group}</h4>
                  {items.map((a, i) => (
                    <button key={i} className="action" disabled={busy} onClick={() => act(a.action)}>
                      {a.description}{a.cost > 1 ? ` (${a.cost}◆)` : ''}
                    </button>
                  ))}
                </div>
              ) : null,
            )}
          </div>
        )}

        <AnalysisPanel
          combatId={view.id}
          ready={phase.kind === 'active' && !view.pending}
          active={phase.kind === 'active'}
          currentActor={view.current_actor}
          actorName={nameOf(s, view.current_actor)}
          evaluation={evaluation}
          onPlay={act}
        />

        {inspected && (
          <div className="panel">
            <GmTools key={inspected.id} id={inspected.id} name={inspected.name} busy={busy} send={send} conditions={inspected.conditions.map((c) => c.kind)} />
            <StatblockView sb={inspected.statblock} />
          </div>
        )}
      </div>
    </div>
  );
}

function GmTools({ id, name, busy, send, conditions }: { id: CombatantId; name: string; busy: boolean; send: (c: Command) => void; conditions: ConditionKind[] }) {
  const [delta, setDelta] = useState('-5');
  const [cond, setCond] = useState<ConditionKind>('frightened');
  const [value, setValue] = useState('1');
  return (
    <details className="gm">
      <summary>GM tools — {name}</summary>
      <div className="row">
        <input className="narrow" type="number" value={delta} onChange={(e) => setDelta(e.target.value)} />
        <button disabled={busy} onClick={() => send({ type: 'gm', op: { kind: 'adjust_hp', target: id, delta: Number(delta) } })}>Adjust HP</button>
      </div>
      <div className="row">
        <select value={cond} onChange={(e) => setCond(e.target.value as ConditionKind)}>
          {CONDITIONS.map((c) => <option key={c}>{c}</option>)}
        </select>
        <input className="narrow" type="number" value={value} onChange={(e) => setValue(e.target.value)} />
        <button disabled={busy} onClick={() => send({ type: 'gm', op: { kind: 'add_condition', target: id, condition: { kind: cond, value: Number(value) } } })}>Add</button>
      </div>
      <div className="row wrap">
        {conditions.map((c) => (
          <button key={c} disabled={busy} onClick={() => send({ type: 'gm', op: { kind: 'remove_condition', target: id, condition: c } })}>✕ {c}</button>
        ))}
        <button disabled={busy} onClick={() => send({ type: 'gm', op: { kind: 'clear_pending' } })} title="Abandon the action being resolved">Clear pending</button>
      </div>
    </details>
  );
}
