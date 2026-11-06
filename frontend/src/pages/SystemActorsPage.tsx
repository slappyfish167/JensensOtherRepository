import { useEffect, useState } from 'react';
import { api } from '../api/client';
import type { DocumentSummary, SystemId } from '../api/types';

interface Props {
  system: SystemId;
  systemName: string;
  /** Starting point for a new actor. */
  template: Record<string, unknown>;
  /** One-line summary of an actor document. */
  summarize?: (doc: Record<string, unknown>) => string;
}

/**
 * Roster of characters and NPCs for a document-backed game system. Actors
 * are edited as JSON; the server validates them against the system's model.
 */
export function SystemActorsPage({ system, systemName, template, summarize }: Props) {
  const [list, setList] = useState<DocumentSummary[]>([]);
  const [doc, setDoc] = useState<Record<string, unknown> | null>(null);
  const [text, setText] = useState('');
  const [msg, setMsg] = useState<string | null>(null);

  const refresh = () => api.listDocs(system, 'actors').then(setList).catch((e) => setMsg(e.message));
  useEffect(() => {
    refresh();
    setDoc(null);
  }, [system]); // eslint-disable-line react-hooks/exhaustive-deps

  function open(d: Record<string, unknown>) {
    setDoc(d);
    setText(JSON.stringify(d, null, 2));
    setMsg(null);
  }

  async function save() {
    try {
      const parsed = JSON.parse(text) as Record<string, unknown>;
      const saved = await api.saveDoc(system, 'actors', parsed);
      open(saved);
      setMsg('Saved');
      refresh();
    } catch (e) {
      setMsg((e as Error).message);
    }
  }

  return (
    <div className="page split">
      <aside>
        <button className="primary" onClick={() => open({ ...structuredClone(template), id: '' })}>New actor</button>
        <ul className="list">
          {list.map((a) => (
            <li key={a.id} className={doc?.id === a.id ? 'on' : ''} onClick={() => api.getDoc(system, 'actors', a.id).then(open)}>
              <strong>{a.name}</strong>
            </li>
          ))}
        </ul>
      </aside>
      {doc ? (
        <section className="editor">
          <div className="row">
            <h2>{String(doc.name ?? 'New actor')}</h2>
            <span className="badge">{systemName}</span>
            <div className="spacer" />
            <button className="primary" onClick={save}>Save</button>
            {doc.id ? <button onClick={async () => { await api.deleteDoc(system, 'actors', String(doc.id)); setDoc(null); refresh(); }}>Delete</button> : null}
          </div>
          {summarize && <p className="muted">{summarize(doc)}</p>}
          {msg && <div className="note" onClick={() => setMsg(null)}>{msg}</div>}
          <textarea className="code" rows={32} value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} />
        </section>
      ) : (
        <section className="editor muted">Pick an actor or create one. Actors are stored as {systemName} JSON and validated by the rules engine on save.</section>
      )}
    </div>
  );
}
