// Who may see and run what. Pure functions over a small context so the
// policy is unit-tested; Foundry glue supplies the context.
//
// Threat model: analysis encodes information. A player who can run an
// unrestricted analysis learns monster numbers and hidden creatures. So:
// * the GM assistant and the forecast are GM-only;
// * the evaluation bar is GM-only by default; when shown to players and
//   "GM authoritative" is on, players receive only the GM's score;
// * player-side best-turn analysis is built only from tokens the player can
//   see (extract/snapshot.ts) and only for tokens they own.

import type { Settings } from './settings';

export interface PermissionContext {
  isGM: boolean;
  settings: Pick<Settings, 'evalBarVisibility' | 'playerBestTurn' | 'gmAuthoritative'>;
}

export function canSeeEvalBar(c: PermissionContext): boolean {
  if (c.settings.evalBarVisibility === 'none') return false;
  return c.isGM || c.settings.evalBarVisibility === 'all';
}

/** Players get the GM's evaluation over the socket instead of computing it. */
export function receivesBroadcastEvaluation(c: PermissionContext): boolean {
  return !c.isGM && canSeeEvalBar(c) && c.settings.gmAuthoritative;
}

export function computesEvaluationLocally(c: PermissionContext): boolean {
  return canSeeEvalBar(c) && (c.isGM || !c.settings.gmAuthoritative);
}

export function canAnalyzeToken(c: PermissionContext, ownsToken: boolean): boolean {
  return c.isGM || (c.settings.playerBestTurn && ownsToken);
}

export function canUseGmAssistant(c: PermissionContext): boolean {
  return c.isGM;
}

export function canForecast(c: PermissionContext): boolean {
  return c.isGM;
}

export function canExplain(c: PermissionContext): boolean {
  return computesEvaluationLocally(c);
}
