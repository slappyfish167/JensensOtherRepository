import { useEffect, useRef, useState } from 'react';
import type { PromptView } from '../api/types';

interface Props {
  prompt: PromptView;
  actorName: string;
  seeded: boolean;
  busy: boolean;
  onRoll: (value: number, faces: number[], extra: number) => void;
  onAutoRoll: () => void;
  onDecide: (accept: boolean) => void;
}

/** Parses "4 6 6" / "4,6,6" into faces. */
export function parseFaces(text: string): number[] {
  return text
    .split(/[\s,]+/)
    .filter(Boolean)
    .map(Number);
}

/**
 * The human-dice prompt for any game system. The engine says how the roll
 * is entered — one natural die, a total, or every die face — and validates it.
 */
export function NeutralPrompt({ prompt, actorName, seeded, busy, onRoll, onAutoRoll, onDecide }: Props) {
  const [value, setValue] = useState('');
  const [extra, setExtra] = useState('0');
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    setValue('');
    setExtra('0');
    input.current?.focus();
  }, [prompt.request_id]);

  if (prompt.kind === 'decision') {
    return (
      <div className="panel pending decision">
        <h3>Decision — {actorName}</h3>
        <p>{prompt.prompt}</p>
        <div className="row">
          <button className="primary" disabled={busy} onClick={() => onDecide(true)}>{prompt.accept_label ?? 'Yes'}</button>
          <button disabled={busy} onClick={() => onDecide(false)}>{prompt.decline_label ?? 'No'}</button>
        </div>
      </div>
    );
  }

  const spec = prompt.input ?? { kind: 'total' as const, min: -999, max: 999 };
  let valid = false;
  let label = 'Total rolled';
  let faces: number[] = [];
  const n = Number(value);
  if (spec.kind === 'die') {
    label = `Natural d${spec.sides}`;
    valid = value !== '' && Number.isInteger(n) && n >= 1 && n <= spec.sides;
  } else if (spec.kind === 'total') {
    valid = value !== '' && Number.isInteger(n) && n >= spec.min && n <= spec.max;
  } else {
    label = `Each die (${spec.count}×d${spec.sides})`;
    faces = parseFaces(value);
    valid = faces.length === spec.count && faces.every((f) => Number.isInteger(f) && f >= 1 && f <= spec.sides);
  }
  const submit = () => {
    if (!valid) return;
    const total = spec.kind === 'faces' ? faces.reduce((a, b) => a + b, 0) : n;
    onRoll(total, faces, Number(extra || 0));
  };

  return (
    <div className="panel pending roll">
      <h3>{prompt.actor != null ? `${actorName} rolls` : 'Roll'}</h3>
      <p className="prompt">{prompt.prompt}</p>
      {prompt.expression && <div className="dice-expr">{prompt.expression}</div>}
      {prompt.breakdown.length > 0 && (
        <ul className="breakdown">
          {prompt.breakdown.map((m, i) => (
            <li key={i}>
              <span>{m.label}</span> <strong>{m.value}</strong>
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
          {label}
          <input ref={input} type={spec.kind === 'faces' ? 'text' : 'number'} value={value} placeholder={spec.kind === 'faces' ? 'e.g. 4 6 2' : undefined} onChange={(e) => setValue(e.target.value)} />
        </label>
        <label title="GM's situational modifier (recorded in the log)">
          Extra
          <input type="number" value={extra} onChange={(e) => setExtra(e.target.value)} className="narrow" />
        </label>
        <button className="primary" type="submit" disabled={!valid || busy}>Submit</button>
        {seeded && <button type="button" disabled={busy} onClick={onAutoRoll} title="Roll with the combat's seeded dice">🎲 Auto</button>}
      </form>
    </div>
  );
}
