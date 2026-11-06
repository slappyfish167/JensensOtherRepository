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
  actorTemplate: {}, // filled in below once the model is known
};
