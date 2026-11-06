// Per-system UI configuration. A system without a dedicated UI needs only
// an entry here: its combats run on the system-neutral combat page, and its
// actors and encounters on the document-backed pages.
import type { SystemId } from './api/types';

export interface SystemUi {
  id: SystemId;
  name: string;
  /** Map glyphs: [glyph, css class, label]. */
  palette: [string, string, string][];
  actorTemplate: Record<string, unknown>;
  summarizeActor?: (doc: Record<string, unknown>) => string;
}

export const CPRED: SystemUi = {
  id: 'cpred',
  name: 'Cyberpunk RED',
  palette: [
    ['.', 'open', 'open'],
    ['~', 'difficult', 'difficult terrain'],
    ['#', 'wall', 'wall'],
    ['%', 'cover', 'cover object'],
  ],
  // Empty: "New actor" starts from a copy of an existing actor, so the
  // template always matches the engine's model.
  actorTemplate: {},
  summarizeActor: (doc) => {
    const sb = (doc.statblock ?? {}) as { stats?: Record<string, number>; max_hp?: number | null; weapons?: { name: string }[] };
    const st = sb.stats ?? {};
    const hp = sb.max_hp ?? 10 + 5 * Math.ceil(((st.body ?? 0) + (st.will ?? 0)) / 2);
    const weapons = (sb.weapons ?? []).map((w) => w.name).join(', ');
    return `${String(doc.kind ?? 'npc').toUpperCase()} · REF ${st.ref ?? '?'} · DEX ${st.dex ?? '?'} · BODY ${st.body ?? '?'} · MOVE ${st.move ?? '?'} · ${hp} HP${weapons ? ` · ${weapons}` : ''}`;
  },
};
