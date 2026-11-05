import { useEffect, useState } from 'react';
import { api } from '../api/client';
import type { Monster, MonsterSummary } from '../api/types';
import { StatblockView } from '../components/StatblockView';

export function MonstersPage() {
  const [list, setList] = useState<MonsterSummary[]>([]);
  const [m, setM] = useState<Monster | null>(null);
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState('');
  const [importText, setImportText] = useState('');
  const [msg, setMsg] = useState<string | null>(null);
  const [filter, setFilter] = useState('');

  const refresh = () => api.listMonsters().then(setList);
  useEffect(() => { refresh(); }, []);

  const open = (id: string) => api.getMonster(id).then((x) => { setM(x); setEditing(false); });
  const guard = async (f: () => Promise<void>) => {
    try { await f(); } catch (e) { setMsg((e as Error).message); }
  };

  return (
    <div className="page split">
      <aside>
        <input placeholder="Filter…" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <ul className="list">
          {list.filter((x) => x.name.toLowerCase().includes(filter.toLowerCase())).map((x) => (
            <li key={x.id} className={m?.id === x.id ? 'on' : ''} onClick={() => open(x.id)}>
              <span className="lvl">{x.level}</span> <strong>{x.name}</strong> {x.is_hazard && <span className="tag warn">hazard</span>}
            </li>
          ))}
        </ul>
        <details>
          <summary>Import stat block(s)</summary>
          <p className="muted small">Paste a monster, an array of monsters, or a bare stat block (JSON).</p>
          <textarea className="code" rows={8} value={importText} onChange={(e) => setImportText(e.target.value)} />
          <button onClick={() => guard(async () => {
            const saved = await api.importMonsters(JSON.parse(importText));
            setMsg(`Imported ${saved.map((s) => s.statblock.name).join(', ')}`);
            setImportText('');
            refresh();
          })}>Import</button>
        </details>
      </aside>
      {m && (
        <section className="editor">
          <div className="row">
            <h2>{m.statblock.name}</h2>
            <div className="spacer" />
            <button onClick={() => guard(async () => { const t = await api.templateMonster(m.id, 'elite', true); setM(t); refresh(); })}>Make Elite</button>
            <button onClick={() => guard(async () => { const t = await api.templateMonster(m.id, 'weak', true); setM(t); refresh(); })}>Make Weak</button>
            <button onClick={() => { setEditing(!editing); setText(JSON.stringify(m, null, 2)); }}>{editing ? 'Cancel' : 'Edit JSON'}</button>
            <button onClick={() => guard(async () => { await api.deleteMonster(m.id); setM(null); refresh(); })}>Delete</button>
          </div>
          {msg && <div className="note" onClick={() => setMsg(null)}>{msg}</div>}
          {m.description && <p className="muted">{m.description}</p>}
          {m.applied_templates.length > 0 && <p>Templates: {m.applied_templates.join(', ')}</p>}
          {editing ? (
            <div>
              <textarea className="code" rows={30} value={text} onChange={(e) => setText(e.target.value)} />
              <button className="primary" onClick={() => guard(async () => { const saved = await api.saveMonster(JSON.parse(text)); setM(saved); setEditing(false); refresh(); })}>Save</button>
            </div>
          ) : (
            <StatblockView sb={m.statblock} />
          )}
        </section>
      )}
    </div>
  );
}
