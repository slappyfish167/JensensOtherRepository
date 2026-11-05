import { useEffect, useState } from 'react';
import { api } from '../api/client';
import type { CharacterSummary, Encounter, EncounterBudget, EncounterSummary, MonsterSummary, Side } from '../api/types';
import { BattleGrid } from '../components/BattleGrid';

const blank = (): Encounter => ({
  id: '',
  name: 'New Encounter',
  party_level: 1,
  participants: [],
  battlefield: { width: 12, height: 10, map: Array.from({ length: 10 }, () => '.'.repeat(12)), notes: '' },
});

export function EncountersPage({ onStartCombat }: { onStartCombat: (combatId: string) => void }) {
  const [list, setList] = useState<EncounterSummary[]>([]);
  const [chars, setChars] = useState<CharacterSummary[]>([]);
  const [monsters, setMonsters] = useState<MonsterSummary[]>([]);
  const [enc, setEnc] = useState<Encounter | null>(null);
  const [budget, setBudget] = useState<EncounterBudget | null>(null);
  const [placing, setPlacing] = useState<number | null>(null);
  const [paint, setPaint] = useState<string | null>(null);
  const [dice, setDice] = useState<'manual' | 'seeded'>('manual');
  const [seed, setSeed] = useState(1);
  const [msg, setMsg] = useState<string | null>(null);

  const refresh = () => api.listEncounters().then(setList);
  useEffect(() => {
    refresh();
    api.listCharacters().then(setChars);
    api.listMonsters().then(setMonsters);
  }, []);

  useEffect(() => {
    if (!enc) return;
    const t = setTimeout(() => api.budget(enc).then(setBudget).catch(() => setBudget(null)), 250);
    return () => clearTimeout(t);
  }, [enc]);

  const update = (f: (e: Encounter) => void) => setEnc((x) => { if (!x) return x; const n = structuredClone(x); f(n); return n; });
  const nameFor = (p: Encounter['participants'][number]) =>
    p.name || (p.source.kind === 'character' ? chars.find((c) => c.id === p.source.value)?.name : p.source.kind === 'monster' ? monsters.find((m) => m.id === p.source.value)?.name : 'Inline') || String(p.source.value);

  function resize(w: number, h: number) {
    update((e) => {
      const rows = Array.from({ length: h }, (_, y) => (e.battlefield.map[y] ?? '').padEnd(w, '.').slice(0, w));
      e.battlefield = { ...e.battlefield, width: w, height: h, map: rows };
    });
  }

  function onCell(x: number, y: number) {
    if (placing != null) {
      update((e) => { e.participants[placing].position = { x, y }; });
      setPlacing(null);
    } else if (paint) {
      update((e) => {
        const row = e.battlefield.map[y].split('');
        row[x] = paint;
        e.battlefield.map[y] = row.join('');
      });
    }
  }

  async function save(): Promise<Encounter | null> {
    if (!enc) return null;
    try {
      const r = await api.saveEncounter(enc);
      setEnc(r.encounter);
      setMsg('Saved');
      refresh();
      return r.encounter;
    } catch (e) {
      setMsg((e as Error).message);
      return null;
    }
  }

  async function start() {
    const saved = await save();
    if (!saved) return;
    try {
      const v = await api.createCombat(saved.id, dice === 'manual' ? { mode: 'manual' } : { mode: 'seeded', seed });
      onStartCombat(v.id);
    } catch (e) {
      setMsg((e as Error).message);
    }
  }

  return (
    <div className="page split">
      <aside>
        <button className="primary" onClick={() => setEnc(blank())}>New encounter</button>
        <ul className="list">
          {list.map((e) => (
            <li key={e.id} className={enc?.id === e.id ? 'on' : ''} onClick={() => api.getEncounter(e.id).then(setEnc)}>
              <strong>{e.name}</strong> <span className="muted">party level {e.party_level}</span>
            </li>
          ))}
        </ul>
      </aside>
      {enc && (
        <section className="editor">
          <div className="row">
            <input className="title" value={enc.name} onChange={(e) => update((x) => { x.name = e.target.value; })} />
            <label>Party level <input type="number" className="narrow" min={1} max={20} value={enc.party_level} onChange={(e) => update((x) => { x.party_level = Number(e.target.value); })} /></label>
            <div className="spacer" />
            <button onClick={save}>Save</button>
            {enc.id && <button onClick={async () => { await api.deleteEncounter(enc.id); setEnc(null); refresh(); }}>Delete</button>}
          </div>
          {msg && <div className="note" onClick={() => setMsg(null)}>{msg}</div>}
          {budget && (
            <div className={`budget threat-${budget.threat}`}>
              <strong>{budget.threat.replace('_', ' ')}</strong> threat — {budget.total_xp} XP for {budget.party_size} PCs
              <span className="muted"> (trivial ≤{budget.budgets[0]}, low ≤{budget.budgets[1]}, moderate ≤{budget.budgets[2]}, severe ≤{budget.budgets[3]}, extreme ≤{budget.budgets[4]})</span>
              {budget.warnings.map((w, i) => <div key={i} className="bad">{w}</div>)}
            </div>
          )}

          <div className="encounter-layout">
            <div>
              <table className="participants">
                <thead><tr><th>Who</th><th>Side</th><th>Pos</th><th>Name override</th><th /></tr></thead>
                <tbody>
                  {enc.participants.map((p, i) => (
                    <tr key={i} className={placing === i ? 'placing' : ''}>
                      <td>{nameFor(p)}</td>
                      <td>
                        <select value={p.side} onChange={(e) => update((x) => { x.participants[i].side = e.target.value as Side; })}>
                          <option>party</option><option>enemy</option><option>neutral</option>
                        </select>
                      </td>
                      <td><button onClick={() => setPlacing(i)} title="Click, then click the map">({p.position.x},{p.position.y})</button></td>
                      <td><input value={p.name ?? ''} onChange={(e) => update((x) => { x.participants[i].name = e.target.value || null; })} /></td>
                      <td><button onClick={() => update((x) => { x.participants.splice(i, 1); })}>✕</button></td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className="row wrap">
                <select value="" onChange={(e) => { const id = e.target.value; if (id) update((x) => { x.participants.push({ source: { kind: 'character', value: id }, side: 'party', position: { x: 1, y: 1 + x.participants.length % 8 } }); }); }}>
                  <option value="">+ Add character…</option>
                  {chars.map((c) => <option key={c.id} value={c.id} disabled={!c.is_legal}>{c.name} (L{c.level}){c.is_legal ? '' : ' — illegal'}</option>)}
                </select>
                <select value="" onChange={(e) => { const id = e.target.value; if (id) update((x) => { x.participants.push({ source: { kind: 'monster', value: id }, side: 'enemy', position: { x: x.battlefield.width - 2, y: 1 + x.participants.length % 8 } }); }); }}>
                  <option value="">+ Add monster / hazard…</option>
                  {monsters.map((m) => <option key={m.id} value={m.id}>{m.name} ({m.level}){m.is_hazard ? ' hazard' : ''}</option>)}
                </select>
              </div>
              <fieldset>
                <legend>Battlefield</legend>
                <div className="row">
                  <label>W <input type="number" className="narrow" min={1} max={60} value={enc.battlefield.width} onChange={(e) => resize(Number(e.target.value), enc.battlefield.height)} /></label>
                  <label>H <input type="number" className="narrow" min={1} max={60} value={enc.battlefield.height} onChange={(e) => resize(enc.battlefield.width, Number(e.target.value))} /></label>
                </div>
                <div className="row wrap">
                  Paint:
                  {[['.', 'open'], ['~', 'difficult'], ['^', 'greater difficult'], ['#', 'wall'], ['%', 'concealment']].map(([g, label]) => (
                    <button key={g} className={paint === g ? 'on' : ''} onClick={() => setPaint(paint === g ? null : g)}>{label}</button>
                  ))}
                </div>
              </fieldset>
              <fieldset>
                <legend>Run it</legend>
                <div className="row">
                  <label className="inline"><input type="radio" checked={dice === 'manual'} onChange={() => setDice('manual')} /> Manual dice (enter every roll)</label>
                </div>
                <div className="row">
                  <label className="inline"><input type="radio" checked={dice === 'seeded'} onChange={() => setDice('seeded')} /> Allow seeded auto-rolls, seed</label>
                  <input type="number" className="narrow" value={seed} onChange={(e) => setSeed(Number(e.target.value))} disabled={dice !== 'seeded'} />
                </div>
                <button className="primary" onClick={start} disabled={enc.participants.length === 0}>Save &amp; start combat</button>
              </fieldset>
            </div>
            <BattleGrid
              battlefield={enc.battlefield}
              cell={28}
              tokens={enc.participants.map((p, i) => ({ id: `p${i}`, name: nameFor(p), side: p.side, position: p.position }))}
              selectedId={placing != null ? `p${placing}` : null}
              onCellClick={(p) => onCell(p.x, p.y)}
              onTokenClick={(id) => setPlacing(Number(String(id).slice(1)))}
            />
          </div>
        </section>
      )}
    </div>
  );
}
