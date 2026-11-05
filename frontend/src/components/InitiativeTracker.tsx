import type { CombatState, CombatantId } from '../api/types';
import { conditionLabel, hpPercent } from '../lib/format';

export function InitiativeTracker({ state, selected, onSelect }: { state: CombatState; selected: CombatantId | null; onSelect: (id: CombatantId) => void }) {
  const current = state.phase.kind === 'active' ? state.initiative[state.turn]?.combatant : null;
  const ordered = state.initiative.length
    ? state.initiative.map((e) => ({ entry: e, c: state.combatants.find((c) => c.id === e.combatant)! }))
    : state.combatants.map((c) => ({ entry: null, c }));
  const offTrack = state.combatants.filter((c) => !c.in_initiative);
  return (
    <div className="panel initiative">
      <h3>Initiative{state.round ? ` · Round ${state.round}` : ''}</h3>
      <ol>
        {ordered.map(({ entry, c }) => (
          <li
            key={c.id}
            className={`${c.side} ${c.id === current ? 'current' : ''} ${c.id === selected ? 'selected' : ''} ${c.life !== 'active' ? 'dead' : ''}`}
            onClick={() => onSelect(c.id)}
          >
            <div className="init-head">
              <span className="init-val">{entry?.value ?? '–'}</span>
              <span className="init-name">{c.name}</span>
              <span className="init-hp">{c.hp}/{c.statblock.max_hp}{c.temp_hp ? ` (+${c.temp_hp})` : ''}</span>
            </div>
            <div className="hpbar"><i style={{ width: `${hpPercent(c)}%` }} /></div>
            <div className="init-meta">
              {c.id === current && <span className="pips">{'◆'.repeat(c.actions_remaining)}{c.reaction_available ? ' ↺' : ''}</span>}
              {c.conditions.map((x) => <span key={x.kind} className="tag">{conditionLabel(x)}</span>)}
              {c.persistent.map((p) => <span key={p.damage_type} className="tag warn">{p.dice} {p.damage_type}</span>)}
              {c.effects.map((e) => <span key={e.id} className="tag good">{e.name}</span>)}
              {c.life === 'dead' && <span className="tag bad">dead</span>}
            </div>
          </li>
        ))}
      </ol>
      {offTrack.length > 0 && (
        <div className="muted small">Not in initiative: {offTrack.map((c) => `${c.name}${c.life !== 'active' ? ' (inactive)' : ''}`).join(', ')}</div>
      )}
    </div>
  );
}
