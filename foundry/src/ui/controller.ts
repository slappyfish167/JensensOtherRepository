// User commands. Every entry point (scene controls, token HUD, combat
// tracker, context menus, hotkeys, public API) goes through here, so
// permission checks and error reporting live in one place.

import { EngineError } from '../engine/client';
import type { AnalysisService } from '../analysis/service';
import { canAnalyzeToken, canExplain, canForecast, canUseGmAssistant, type PermissionContext } from '../permissions';
import type { TurnReport } from '../protocol/report';
import { createReportWindow, type ReportWindow } from './apps/report-app';
import { clearOverlay, drawTurn } from './overlay';
import { explainContext, forecastContext, sidePlanContext, turnContext } from './view-models';

const T = (k: string) => game.i18n.localize(`PF2E_STOCKFISH.${k}`);

export class Controller {
  private windows: Partial<Record<'bestTurn' | 'gmAssistant' | 'forecast' | 'explain', ReportWindow>> = {};
  private lastTurn: TurnReport | null = null;
  private lastPlan = new Map<string, TurnReport>();
  private lastBestTurnToken: string | null = null;

  constructor(
    private readonly service: AnalysisService,
    private readonly perms: () => PermissionContext,
  ) {}

  private notifyError(e: unknown): void {
    if (e instanceof EngineError) {
      const key = `Errors.${e.code}`;
      const msg = game.i18n.localize(`PF2E_STOCKFISH.${key}`);
      ui.notifications.warn(msg === `PF2E_STOCKFISH.${key}` ? e.message : `${msg} (${e.message})`);
    } else {
      ui.notifications.error(`PF2e Stockfish: ${String(e)}`);
    }
  }

  private window(kind: 'bestTurn' | 'gmAssistant' | 'forecast' | 'explain'): ReportWindow {
    const existing = this.windows[kind];
    if (existing) return existing;
    const actions = {
      showOnMap: (target: HTMLElement) => {
        const tok = target.dataset.token;
        const turn = tok ? (this.lastPlan.get(tok) ?? (this.lastTurn?.token_id === tok ? this.lastTurn : null)) : this.lastTurn;
        if (turn) drawTurn(turn);
      },
      clearMap: () => clearOverlay(),
      rerun: () => {
        if (kind === 'bestTurn' && this.lastBestTurnToken) return this.analyzeBestTurn(this.lastBestTurnToken);
        if (kind === 'gmAssistant') return this.suggestMonsterTurn();
        if (kind === 'forecast') return this.forecast();
        return this.explain();
      },
      selectToken: (target: HTMLElement) => {
        const t = target.dataset.token ? canvas.tokens.get(target.dataset.token) : undefined;
        (t as unknown as { control?: (o: unknown) => void } | undefined)?.control?.({ releaseOthers: true });
      },
    };
    const w = createReportWindow(kind, T(`Windows.${kind}`), actions);
    this.windows[kind] = w;
    return w;
  }

  /** Token to analyse: explicit, else the single controlled token, else the current combatant. */
  resolveToken(tokenId?: string): FoundryTokenObject | null {
    if (tokenId) return canvas.tokens.get(tokenId) ?? null;
    const controlled = canvas.tokens.controlled;
    if (controlled.length === 1 && controlled[0]) return controlled[0];
    const cur = game.combat?.combatant?.tokenId;
    return cur ? (canvas.tokens.get(cur) ?? null) : null;
  }

  async analyzeBestTurn(tokenId?: string): Promise<TurnReport | null> {
    const token = this.resolveToken(tokenId);
    if (!token) {
      ui.notifications.info(T('Notify.selectToken'));
      return null;
    }
    if (!canAnalyzeToken(this.perms(), token.isOwner)) {
      ui.notifications.warn(T('Errors.not_permitted'));
      return null;
    }
    try {
      const report = await this.service.bestTurn(token.id);
      this.lastTurn = report;
      this.lastBestTurnToken = token.id;
      await this.window('bestTurn').show(turnContext(report));
      drawTurn(report);
      return report;
    } catch (e) {
      this.notifyError(e);
      return null;
    }
  }

  async suggestMonsterTurn(): Promise<void> {
    if (!canUseGmAssistant(this.perms())) {
      ui.notifications.warn(T('Errors.not_permitted'));
      return;
    }
    try {
      const plan = await this.service.sidePlan('enemy');
      this.lastPlan = new Map(plan.creatures.filter((c) => c.turn).map((c) => [c.token_id, c.turn as TurnReport]));
      await this.window('gmAssistant').show(sidePlanContext(plan));
      // Draw the plan of the monster whose turn it is, if any.
      const cur = game.combat?.combatant?.tokenId;
      const now = cur ? this.lastPlan.get(cur) : undefined;
      if (now) drawTurn(now);
    } catch (e) {
      this.notifyError(e);
    }
  }

  async forecast(): Promise<void> {
    if (!canForecast(this.perms())) {
      ui.notifications.warn(T('Errors.not_permitted'));
      return;
    }
    try {
      ui.notifications.info(T('Notify.forecasting'));
      const f = await this.service.forecast(Math.floor(Math.random() * 2 ** 31));
      await this.window('forecast').show(forecastContext(f));
    } catch (e) {
      this.notifyError(e);
    }
  }

  async explain(): Promise<void> {
    if (!canExplain(this.perms())) {
      ui.notifications.warn(T('Errors.not_permitted'));
      return;
    }
    try {
      await this.window('explain').show(explainContext(await this.service.explain(8)));
    } catch (e) {
      this.notifyError(e);
    }
  }
}
