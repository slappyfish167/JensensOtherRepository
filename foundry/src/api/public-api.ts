// Public API for macros and other modules:
//
//   const sf = game.modules.get('pf2e-stockfish').api;
//   const report = await sf.bestTurn();            // controlled/current token
//   await sf.bestTurn(token.id, { show: false });  // data only, no window
//   const ev = await sf.evaluate();
//   Hooks.on('pf2e-stockfish.evaluation', (ev) => console.log(ev?.score));
//
// Every call goes through the same permission checks as the UI.

import { canAnalyzeToken, canExplain, canForecast, canUseGmAssistant } from '../permissions';
import type { EvaluationReport, ExplainReport, ForecastReport, SidePlanReport, TurnReport } from '../protocol/report';
import type { CombatSnapshot } from '../protocol/snapshot';
import type { Runtime } from '../runtime';

export interface PublicApi {
  readonly version: string;
  evaluate(): Promise<EvaluationReport | null>;
  bestTurn(tokenId?: string, opts?: { show?: boolean }): Promise<TurnReport | null>;
  monsterPlan(opts?: { show?: boolean }): Promise<SidePlanReport | null>;
  forecast(opts?: { show?: boolean }): Promise<ForecastReport | null>;
  explain(opts?: { show?: boolean }): Promise<ExplainReport | null>;
  /** The snapshot the engine would receive now (debugging, bug reports). */
  snapshot(): Promise<CombatSnapshot | null>;
  toggleEvalBar(visible?: boolean): Promise<void>;
}

function denied(): never {
  throw new Error(game.i18n.localize('PF2E_STOCKFISH.Errors.not_permitted'));
}

export function createPublicApi(rt: Runtime): PublicApi {
  return {
    get version() {
      return game.modules.get('pf2e-stockfish')?.version ?? '0';
    },
    async evaluate() {
      if (!canExplain(rt.perms())) denied();
      await rt.service.refresh();
      return rt.service.lastEvaluation;
    },
    async bestTurn(tokenId, opts = {}) {
      if (opts.show !== false) return rt.controller.analyzeBestTurn(tokenId);
      const token = rt.controller.resolveToken(tokenId);
      if (!token) return null;
      if (!canAnalyzeToken(rt.perms(), token.isOwner)) denied();
      return rt.service.bestTurn(token.id);
    },
    async monsterPlan(opts = {}) {
      if (!canUseGmAssistant(rt.perms())) denied();
      if (opts.show !== false) await rt.controller.suggestMonsterTurn();
      return rt.service.sidePlan('enemy');
    },
    async forecast(opts = {}) {
      if (!canForecast(rt.perms())) denied();
      if (opts.show !== false) {
        await rt.controller.forecast();
        return null;
      }
      return rt.service.forecast();
    },
    async explain(opts = {}) {
      if (!canExplain(rt.perms())) denied();
      if (opts.show !== false) await rt.controller.explain();
      return rt.service.explain();
    },
    async snapshot() {
      if (!game.user.isGM) denied();
      const built = await rt.currentSnapshot();
      return built?.snapshot ?? null;
    },
    toggleEvalBar(visible) {
      return rt.toggleBar(visible);
    },
  };
}
