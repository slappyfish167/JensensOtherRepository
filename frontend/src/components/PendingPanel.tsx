import { useEffect, useRef, useState } from 'react';
import type { CombatState, PendingInput } from '../api/types';
import { nameOf, signed } from '../lib/format';

interface Props {
  pending: PendingInput;
  state: CombatState;
  seeded: boolean;
  busy: boolean;
  onRoll: (value: number, extra: number) => void;
  onAutoRoll: () => void;
  onDecide: (accept: boolean) => void;
}

/**
 * The human-dice prompt: "Roll 1d20+14 for attack". The user rolls real dice
 * and types the result; the engine computes the outcome.
 */
export function PendingPanel({ pending, state, seeded, busy, onRoll, onAutoRoll, onDecide }: Props) {
  const [value, setValue] = useState('');
  const [extra, setExtra] = useState('0');
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    setValue('');
    setExtra('0');
    input.current?.focus();
  }, [pending.request_id]);

  if (pending.type === 'decision') {
    return (
      <div className="panel pending decision">
        <h3>Reaction — {nameOf(state, pending.decider)}</h3>
        <p>{pending.prompt}</p>
        <div className="row">
          <button className="primary" disabled={busy} onClick={() => onDecide(true)}>Use reaction</button>
          <button disabled={busy} onClick={() => onDecide(false)}>Decline</button>
        </div>
      </div>
    );
  }

  const k = pending.kind;
  const isD20 = k.kind === 'check' || k.kind === 'flat';
  const n = Number(value);
  const valid = value !== '' && Number.isInteger(n) && (isD20 ? n >= 1 && n <= 20 : true);
  const preview =
    k.kind === 'check' && valid ? `total ${n + k.modifier + Number(extra || 0)}${k.dc ? ` vs DC ${k.dc}` : ''}` : null;
  const submit = () => valid && onRoll(n, Number(extra || 0));

  return (
    <div className="panel pending roll">
      <h3>{pending.roller != null ? `${nameOf(state, pending.roller)} rolls` : 'Roll'}</h3>
      <p className="prompt">{pending.prompt}</p>
      <div className="dice-expr">{pending.expression}{k.kind === 'flat' ? ` (DC ${k.dc})` : ''}</div>
      {pending.breakdown.length > 0 && (
        <ul className="breakdown">
          {pending.breakdown.map((m, i) => (
            <li key={i}>
              <span>{m.source}</span> <span className="muted">{m.kind}</span> <strong>{signed(m.value)}</strong>
            </li>
          ))}
        </ul>
      )}
      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <label>
          {isD20 ? 'Natural d20' : 'Total rolled'}
          <input ref={input} type="number" value={value} onChange={(e) => setValue(e.target.value)} min={isD20 ? 1 : undefined} max={isD20 ? 20 : undefined} />
        </label>
        <label title="GM's situational modifier (recorded in the log)">
          Extra
          <input type="number" value={extra} onChange={(e) => setExtra(e.target.value)} className="narrow" />
        </label>
        <button className="primary" type="submit" disabled={!valid || busy}>Submit</button>
        {seeded && (
          <button type="button" disabled={busy} onClick={onAutoRoll} title="Roll with the combat's seeded dice">🎲 Auto</button>
        )}
      </form>
      {preview && <div className="muted">{preview}</div>}
    </div>
  );
}
