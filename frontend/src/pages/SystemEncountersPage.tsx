import { useEffect, useState } from 'react';
import { api } from '../api/client';
import type { DocumentSummary, Pos, Side, SystemId } from '../api/types';
import { BattleGrid } from '../components/BattleGrid';

export interface DocParticipant { actor_id?: string | null; name?: string | null; side: Side; position: Pos; statblock?: unknown }
export interface DocEncounter {
  id: string;
  name: string;
  battlefield: { width: number; height: number; map: string[]; notes?: string };
  participants: DocParticipant[];
}

interface Props {
  system: SystemId;
  systemName: string;
  /** Map glyphs this system understands: [glyph, css class, label]. */
  palette: [string, string, string][];
  onStartCombat: (combatId: string) => void;
}

const blank = (): DocEncounter => ({
  id: '',
  name: 'New Encounter',
  participants: [],
  battlefield: { width: 16, height: 12, map: Array.from({ length: 12 }, () => '.'.repeat(16)), notes: '' },
});

/** Encounter builder for a document-backed game system. */
export function SystemEncountersPage({ system, systemName, palette, onStartCombat }: Props) {
  const [list, setList] = useState<DocumentSummary[]>([]);
  const [actors, setActors] = useState<DocumentSummary[]>([]);
  const [enc, setEnc] = useState<DocEncounter | null>(null);
  const [placing, setPlacing] = useState<number | null>(null);
  const [paint, setPaint] = useState<string | null>(null);
  const [dice, setDice] = useState<'manual' | 'seeded'>('manual');
  const [seed, setSeed] = useState(1);
  const [msg, setMsg] = useState<string | null>(null);

  const refresh = () => api.listDocs(system, 'encounters').then(setList).catch((e) => setMsg(e.message));
  useEffect(() => {
    refresh();
    api.listDocs(system, 'actors').then(setActors).catch(() => setActors([]));
    setEnc(null);
  }, [system]); // eslint-disable-line react-hooks/exhaustive-deps

  const update = (f: (e: DocEncounter) => void) => setEnc((x) => { if (!x) return x; const n = structuredClone(x); f(n); return n; });
  const nameFor = (p: DocParticipant) => p.name || actors.find((a) => a.id === p.actor_id)?.name || p.actor_id || 'Inline';
  const terrain = Object.fromEntries(palette.map(([g, c]) => [g, c]));

  function resize(w: number, h: number) {
    update((e) => {
      e.battlefield.map = Array.from({ length: h }, (_, y) => (e.battlefield.map[y] ?? '').padEnd(w, '.').slice(0, w));
      e.battlefield.width = w;
      e.battlefield.height = h;
    });
  }

  function onCell(p: Pos) {
    if (placing != null) {
      update((e) => { e.participants[placing].position = p; });
      setPlacing(null);
    } else if (paint) {
      update((e) => {
        const row = e.battlefield.map[p.y].split('');
        row[p.x] = paint;
        e.battlefield.map[p.y] = row.join('');
      });
    }
  }

  async function save(): Promise<DocEncounter | null> {
    if (!enc) return null;
    try {
      const saved = await api.saveDoc(system, 'encounters', enc);
      setEnc(saved);
      setMsg('Saved');
      refresh();
      return saved;
    } catch (e) {
      setMsg((e as Error).message);
      return null;
    }
  }

  async function start() {
    const saved = await save();
    if (!saved) return;
    try {
      const v = await api.createCombat(saved.id, dice === 'manual' ? { mode: 'manual' } : { mode: 'seeded', seed }, system);
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
            <li key={e.id} className={enc?.id === e.id ? 'on' : ''} onClick={() => api.getDoc<DocEncounter>(system, 'encounters', e.id).then(setEnc)}>
              <strong>{e.name}</strong>
            </li>
          ))}
        </ul>
      </aside>
      {enc && (
        <section className="editor">
          <div className="row">
            <input className="title" value={enc.name} onChange={(e) => update((x) => { x.name = e.target.value; })} />
            <span className="badge">{systemName}</span>
            <div className="spacer" />
            <button onClick={save}>Save</button>
            {enc.id && <button onClick={async () => { await api.deleteDoc(system, 'encounters', enc.id); setEnc(null); refresh(); }}>Delete</button>}
          </div>
          {msg && <div className="note" onClick={() => setMsg(null)}>{msg}</div>}
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
                <select value="" onChange={(e) => { const id = e.target.value; if (id) update((x) => { x.participants.push({ actor_id: id, side: 'party', position: { x: 1, y: 1 + (x.participants.length % 8) } }); }); }}>
                  <option value="">+ Add to the party…</option>
                  {actors.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}
                </select>
                <select value="" onChange={(e) => { const id = e.target.value; if (id) update((x) => { x.participants.push({ actor_id: id, side: 'enemy', position: { x: x.battlefield.width - 2, y: 1 + (x.participants.length % 8) } }); }); }}>
                  <option value="">+ Add an opponent…</option>
                  {actors.map((a) => <option key={a.id} value={a.id}>{a.name}</option>)}
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
                  {palette.map(([g, , l]) => (
                    <button key={g} className={paint === g ? 'on' : ''} onClick={() => setPaint(paint === g ? null : g)}>{l}</button>
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
              terrain={terrain}
              cell={28}
              tokens={enc.participants.map((p, i) => ({ id: `p${i}`, name: nameFor(p), side: p.side, position: p.position }))}
              selectedId={placing != null ? `p${placing}` : null}
              onCellClick={onCell}
              onTokenClick={(id) => setPlacing(Number(String(id).slice(1)))}
            />
          </div>
        </section>
      )}
    </div>
  );
}
