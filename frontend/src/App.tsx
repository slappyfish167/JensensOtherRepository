import { useEffect, useState } from 'react';
import { api } from './api/client';
import type { Content } from './api/types';
import { CharactersPage } from './pages/CharactersPage';
import { CombatPage } from './pages/CombatPage';
import { EncountersPage } from './pages/EncountersPage';
import { MonstersPage } from './pages/MonstersPage';

type Page = 'characters' | 'monsters' | 'encounters' | 'combat';

export function App() {
  const [page, setPage] = useState<Page>('encounters');
  const [combatId, setCombatId] = useState<string | null>(null);
  const [content, setContent] = useState<Content | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.content().then(setContent).catch((e) => setError(`Cannot reach the engine API: ${e.message}`));
  }, []);

  const nav: [Page, string][] = [['characters', 'Characters'], ['monsters', 'Monsters'], ['encounters', 'Encounters'], ['combat', 'Combat']];
  return (
    <div className="app">
      <header>
        <h1>PF2e Combat Engine</h1>
        <nav>
          {nav.map(([p, label]) => (
            <button key={p} className={page === p ? 'on' : ''} onClick={() => { setPage(p); if (p === 'combat') setCombatId(null); }}>{label}</button>
          ))}
        </nav>
      </header>
      {error && <div className="error">{error}</div>}
      <main>
        {page === 'characters' && content && <CharactersPage content={content} />}
        {page === 'monsters' && <MonstersPage />}
        {page === 'encounters' && <EncountersPage onStartCombat={(id) => { setCombatId(id); setPage('combat'); }} />}
        {page === 'combat' && <CombatPage combatId={combatId} onOpen={setCombatId} />}
      </main>
    </div>
  );
}
