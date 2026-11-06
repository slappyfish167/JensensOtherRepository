import { useEffect, useState } from 'react';
import { api } from './api/client';
import type { Content, SystemId } from './api/types';
import { CharactersPage } from './pages/CharactersPage';
import { CombatPage } from './pages/CombatPage';
import { EncountersPage } from './pages/EncountersPage';
import { MonstersPage } from './pages/MonstersPage';
import { SystemActorsPage } from './pages/SystemActorsPage';
import { SystemEncountersPage } from './pages/SystemEncountersPage';
import { CPRED } from './systems';

type Page = 'characters' | 'monsters' | 'actors' | 'encounters' | 'combat';

const NAV: Record<SystemId, [Page, string][]> = {
  pf2e: [['characters', 'Characters'], ['monsters', 'Monsters'], ['encounters', 'Encounters'], ['combat', 'Combat']],
  cpred: [['actors', 'Roster'], ['encounters', 'Encounters'], ['combat', 'Combat']],
};

function storedSystem(): SystemId {
  try {
    const v = localStorage.getItem('system');
    return v === 'cpred' ? 'cpred' : 'pf2e';
  } catch {
    return 'pf2e';
  }
}

export function App() {
  const [system, setSystem] = useState<SystemId>(storedSystem);
  const [page, setPage] = useState<Page>('encounters');
  const [combatId, setCombatId] = useState<string | null>(null);
  const [content, setContent] = useState<Content | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.content().then(setContent).catch((e) => setError(`Cannot reach the engine API: ${e.message}`));
  }, []);

  function pickSystem(s: SystemId) {
    setSystem(s);
    try {
      localStorage.setItem('system', s);
    } catch {
      // storage unavailable: the choice just isn't remembered
    }
    if (!NAV[s].some(([p]) => p === page)) setPage('encounters');
  }

  const startCombat = (id: string) => { setCombatId(id); setPage('combat'); };
  return (
    <div className="app">
      <header>
        <h1>Tabletop Combat Engine</h1>
        <div className="system-pick">
          <select value={system} onChange={(e) => pickSystem(e.target.value as SystemId)} aria-label="Game system">
            <option value="pf2e">Pathfinder 2e</option>
            <option value="cpred">Cyberpunk RED</option>
          </select>
        </div>
        <nav>
          {NAV[system].map(([p, label]) => (
            <button key={p} className={page === p ? 'on' : ''} onClick={() => { setPage(p); if (p === 'combat') setCombatId(null); }}>{label}</button>
          ))}
        </nav>
      </header>
      {error && <div className="error">{error}</div>}
      <main>
        {system === 'pf2e' && page === 'characters' && content && <CharactersPage content={content} />}
        {system === 'pf2e' && page === 'monsters' && <MonstersPage />}
        {system === 'pf2e' && page === 'encounters' && <EncountersPage onStartCombat={startCombat} />}
        {system === 'cpred' && page === 'actors' && <SystemActorsPage system="cpred" systemName={CPRED.name} template={CPRED.actorTemplate} summarize={CPRED.summarizeActor} />}
        {system === 'cpred' && page === 'encounters' && <SystemEncountersPage system="cpred" systemName={CPRED.name} palette={CPRED.palette} onStartCombat={startCombat} />}
        {page === 'combat' && <CombatPage combatId={combatId} onOpen={setCombatId} />}
      </main>
    </div>
  );
}
