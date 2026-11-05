import type { Statblock } from '../api/types';
import { signed } from '../lib/format';

export function StatblockView({ sb }: { sb: Statblock }) {
  return (
    <div className="statblock">
      <div className="sb-title">
        <strong>{sb.name}</strong>
        <span>{sb.hazard ? 'Hazard' : 'Creature'} {sb.level}</span>
      </div>
      {sb.traits.length > 0 && <div className="sb-traits">{sb.traits.map((t) => <span key={t} className="tag">{t}</span>)}</div>}
      <p>
        <b>Perception</b> {signed(sb.perception)}; <b>Speed</b> {sb.speed} ft; <b>Size</b> {sb.size}
      </p>
      {Object.keys(sb.skills ?? {}).length > 0 && (
        <p><b>Skills</b> {Object.entries(sb.skills).map(([k, v]) => `${k} ${signed(v ?? 0)}`).join(', ')}</p>
      )}
      <hr />
      <p>
        <b>AC</b> {sb.ac}; <b>Fort</b> {signed(sb.fortitude)}, <b>Ref</b> {signed(sb.reflex)}, <b>Will</b> {signed(sb.will)}
        {sb.class_dc ? <>; <b>Class DC</b> {sb.class_dc}</> : null}
      </p>
      <p><b>HP</b> {sb.max_hp}
        {sb.iwr?.weaknesses?.length ? <>; <b>Weaknesses</b> {sb.iwr.weaknesses.map((w) => `${iwrName(w.applies_to)} ${w.value}`).join(', ')}</> : null}
        {sb.iwr?.resistances?.length ? <>; <b>Resistances</b> {sb.iwr.resistances.map((w) => `${iwrName(w.applies_to)} ${w.value}`).join(', ')}</> : null}
        {sb.iwr?.immunities?.length ? <>; <b>Immunities</b> {sb.iwr.immunities.map(iwrName).join(', ')}</> : null}
      </p>
      {sb.shield && <p><b>Shield</b> {sb.shield.name} (+{sb.shield.ac_bonus} AC, Hardness {sb.shield.hardness}, HP {sb.shield.max_hp})</p>}
      {sb.reactions.length > 0 && <p><b>Reactions</b> {sb.reactions.map((r) => r.replace(/_/g, ' ')).join(', ')}</p>}
      <hr />
      {sb.strikes.map((s, i) => (
        <p key={i}>
          <b>{s.range.kind === 'melee' ? 'Melee' : 'Ranged'}</b> ◆ {s.name} {signed(s.attack_bonus)}
          {s.traits.length ? <span className="muted"> ({s.traits.join(', ')}{s.range.kind === 'ranged' ? `, range ${s.range.increment_ft} ft` : ''})</span> : s.range.kind === 'ranged' ? <span className="muted"> (range {s.range.increment_ft} ft)</span> : null}
          , <b>Damage</b> {s.damage.map((d) => `${d.dice} ${d.damage_type}${d.precision ? ' precision' : ''}`).join(' plus ')}
        </p>
      ))}
      {sb.spellcasting.map((e, i) => (
        <p key={i}>
          <b>{e.name}</b> DC {e.dc}, attack {signed(e.attack_bonus)};{' '}
          {Object.entries(e.slots).map(([r, n]) => `rank ${r} (${n} slots)`).join(', ')}
          {e.spells.length ? `: ${e.spells.map((s) => `${s.spell_id} (${s.rank})`).join(', ')}` : ''}
          {e.cantrips.length ? `; cantrips: ${e.cantrips.join(', ')}` : ''}
        </p>
      ))}
      {sb.hazard && (
        <p>
          <b>Stealth DC</b> {sb.hazard.stealth_dc}; <b>Disable</b> {sb.hazard.disable.map((d) => `${d.skill} DC ${d.dc}`).join(' or ') || '—'};{' '}
          {sb.hazard.complex ? 'complex (acts in initiative)' : 'simple'}
        </p>
      )}
    </div>
  );
}

function iwrName(sel: unknown): string {
  if (typeof sel === 'string') return sel;
  if (sel && typeof sel === 'object' && 'type' in sel) return String((sel as { type: string }).type);
  return JSON.stringify(sel);
}
