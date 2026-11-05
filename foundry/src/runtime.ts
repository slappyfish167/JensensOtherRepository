// Module runtime: owns the engine, the analysis service, the UI and the
// turn tracker, and connects them to Foundry hooks.

import { AnalysisService } from './analysis/service';
import { FLAG_TURN, HOOK_EVALUATION, MODULE_ID, SOCKET } from './constants';
import { EngineClient } from './engine/client';
import { buildSnapshot } from './extract/snapshot';
import { geometry, sampleTerrain, toSquare } from './extract/scene';
import type { StrikeExtraction } from './extract/strikes';
import { computesEvaluationLocally, receivesBroadcastEvaluation, canSeeEvalBar, canAnalyzeToken, type PermissionContext } from './permissions';
import type { EvaluationReport } from './protocol/report';
import { SNAPSHOT_VERSION, type TerrainKind } from './protocol/snapshot';
import { getSettings } from './settings';
import { adjust, classifyMessage, emptyTurn, record, startTurn, type MessageLike, type OriginItemLike, type TrackedEvent, type TurnState } from './tracking/turn-tracker';
import type { CombatLike, SceneLike } from './types/pf2e';
import { Controller } from './ui/controller';
import { EvalBar, type BroadcastEvaluation } from './ui/eval-bar';
import { actorSheetButton, addSceneControls, addTokenHudButton, asElement, combatantContextEntries, registerKeybindings, renderTrackerPanel } from './ui/integrations';

type SocketMessage = { type: 'evaluation'; sceneId: string; evaluation: BroadcastEvaluation | null };

function isTurnState(v: unknown): v is TurnState {
  const t = v as TurnState | null;
  return !!t && typeof t.round === 'number' && typeof t.actionsSpent === 'number' && Array.isArray(t.reactionsUsed);
}

export class Runtime {
  engine!: EngineClient;
  service!: AnalysisService;
  controller!: Controller;
  readonly bar: EvalBar;
  private broadcast: BroadcastEvaluation | null = null;
  private strikeCache = new Map<string, StrikeExtraction>();
  private started = false;

  constructor() {
    this.bar = new EvalBar(() => void this.controller?.explain());
  }

  perms(): PermissionContext {
    return { isGM: game.user.isGM, settings: getSettings() };
  }

  private get isActiveGM(): boolean {
    return !!game.users.activeGM && game.users.activeGM.id === game.user.id;
  }

  // ---- lifecycle -----------------------------------------------------------

  /** `init`: things that must exist before the world is ready. */
  init(): void {
    // Hotkeys may fire before `ready` completes; the proxy reports that.
    registerKeybindings(this.controllerProxy(), () => void this.toggleBar());
  }

  /** `ready`: start the engine and the UI. */
  async ready(): Promise<void> {
    const s = getSettings();
    const workerUrl = foundry.utils.getRoute(`modules/${MODULE_ID}/workers/engine-worker.js`);
    this.engine = new EngineClient(() => new Worker(workerUrl, { type: 'module', name: `${MODULE_ID}-engine` }) as unknown as import('./engine/client').WorkerLike, {
      forecastWorkers: s.forecastWorkers,
      timeoutMs: 15_000,
    });
    this.service = new AnalysisService(this.engine, {
      snapshot: () => this.currentSnapshot(),
      evaluatesLocally: () => computesEvaluationLocally(this.perms()),
      searchMs: () => getSettings().searchMs,
      monsterSearchMs: () => getSettings().monsterSearchMs,
      forecastRuns: () => getSettings().forecastRuns,
    });
    this.controller = new Controller(this.service, () => this.perms());
    this.service.onEvaluation((report) => this.onEvaluation(report));

    try {
      const version = await this.engine.start();
      if (!version.endsWith(`snapshot.${SNAPSHOT_VERSION}`)) {
        ui.notifications.warn(game.i18n.format('PF2E_STOCKFISH.Errors.versionMismatch', { version }));
      }
    } catch (e) {
      ui.notifications.error(game.i18n.format('PF2E_STOCKFISH.Errors.workerFailed', { error: String(e) }));
      return;
    }

    game.socket.on(SOCKET, (data) => this.onSocket(data as SocketMessage));
    this.bar.mount(s.evalBarSide);
    this.applyBarVisibility();
    this.started = true;
    this.invalidate();
  }

  // ---- analysis ------------------------------------------------------------

  private autoRefresh(): boolean {
    const p = this.perms();
    return this.started && getSettings().autoAnalyze && computesEvaluationLocally(p);
  }

  invalidate(): void {
    if (this.autoRefresh()) this.service.invalidate();
  }

  /** Drops cached strike formulas for an actor (its items or data changed). */
  forgetActor(uuid: string | undefined): void {
    if (uuid) this.strikeCache.delete(uuid);
  }

  /** The snapshot this client would analyse now (viewer-filtered). */
  async currentSnapshot() {
    const scene = canvas.scene;
    if (!canvas.ready || !scene) return null;
    const s = getSettings();
    const combat = game.combat && (!game.combat.scene || game.combat.scene.id === scene.id) ? game.combat : null;
    const isGM = game.user.isGM;
    const result = await buildSnapshot({
      scene: scene as unknown as SceneLike,
      combat: combat as unknown as CombatLike | null,
      turn: this.turnState(combat),
      viewer: { isGM, canSee: (t) => (t as { object?: { visible?: boolean } }).object?.visible ?? !t.hidden },
      options: { flanking: s.flanking, autoCover: s.autoCover, cropMargin: s.cropMargin },
      terrain: this.regionTerrain(scene, s.cropMargin),
      strikeCache: this.strikeCache,
    });
    if (result.snapshot.combatants.length === 0) return null;
    return { snapshot: result.snapshot, warnings: result.warnings };
  }

  /**
   * Terrain from scene regions: a `modifyMovementCost` behavior with cost ≥ 2
   * is difficult (≥ 3 greater difficult); a region flag
   * `flags.pf2e-stockfish.terrain` ("difficult" | "greater_difficult" |
   * "obscured" | "wall") overrides it. Sampled only around the tokens.
   */
  private regionTerrain(scene: FoundryScene, margin: number) {
    const regions = canvas.regions?.placeables ?? [];
    if (regions.length === 0) return [];
    const g = geometry(scene as unknown as SceneLike);
    const squares = [...scene.tokens].map((t) => toSquare(g, t.x, t.y));
    if (squares.length === 0) return [];
    const xs = squares.map((s) => s[0]);
    const ys = squares.map((s) => s[1]);
    const window = { x0: Math.min(...xs) - margin, y0: Math.min(...ys) - margin, x1: Math.max(...xs) + margin + 4, y1: Math.max(...ys) + margin + 4 };
    const classified = regions
      .map((r) => {
        const doc = r.document as unknown as { getFlag(s: string, k: string): unknown; behaviors: Iterable<{ type: string; disabled?: boolean; system?: { difficulties?: Record<string, number> } }>; testPoint?(p: { x: number; y: number; elevation: number }): boolean };
        const flag = doc.getFlag(MODULE_ID, 'terrain') as TerrainKind | undefined;
        let kind: TerrainKind | null = flag ?? null;
        if (!kind) {
          for (const b of doc.behaviors) {
            if (b.disabled || b.type !== 'modifyMovementCost') continue;
            const cost = Math.max(...Object.values(b.system?.difficulties ?? { walk: 1 }));
            kind = cost >= 3 ? 'greater_difficult' : cost >= 2 ? 'difficult' : kind;
          }
        }
        return kind && doc.testPoint ? { kind, test: (x: number, y: number) => doc.testPoint!({ x, y, elevation: 0 }) } : null;
      })
      .filter((r): r is { kind: TerrainKind; test: (x: number, y: number) => boolean } => r !== null);
    if (classified.length === 0) return [];
    return sampleTerrain(g, window, (px, py) => classified.find((r) => r.test(px, py))?.kind ?? null);
  }

  private onEvaluation(report: EvaluationReport | null): void {
    const p = this.perms();
    if (computesEvaluationLocally(p)) {
      const err = this.service.lastError;
      this.bar.render(report, err ? err.message : game.i18n.localize('PF2E_STOCKFISH.Bar.idle'));
    }
    Hooks.callAll(HOOK_EVALUATION, report);
    if (p.isGM && p.settings.gmAuthoritative && p.settings.evalBarVisibility === 'all' && canvas.scene) {
      const evaluation = report ? { score: report.score, band: report.band, win_probability: report.win_probability } : null;
      game.socket.emit(SOCKET, { type: 'evaluation', sceneId: canvas.scene.id, evaluation } satisfies SocketMessage);
    }
    ui.combat?.render(false);
  }

  private onSocket(msg: SocketMessage): void {
    if (msg?.type !== 'evaluation' || !receivesBroadcastEvaluation(this.perms())) return;
    if (msg.sceneId !== canvas.scene?.id) return;
    const e = msg.evaluation;
    // Validate untrusted socket data before rendering.
    if (e && (typeof e.score !== 'number' || !Number.isFinite(e.score) || typeof e.band !== 'string' || typeof e.win_probability !== 'number')) return;
    this.broadcast = e ? { score: Math.max(-10, Math.min(10, e.score)), band: e.band.slice(0, 64), win_probability: Math.max(0, Math.min(1, e.win_probability)) } : null;
    this.bar.render(this.broadcast, game.i18n.localize('PF2E_STOCKFISH.Bar.idle'));
    ui.combat?.render(false);
  }

  // ---- eval bar visibility -------------------------------------------------

  applyBarVisibility(): void {
    const s = getSettings();
    this.bar.setSide(s.evalBarSide);
    this.bar.setVisible(s.showEvalBar && canSeeEvalBar(this.perms()));
  }

  async toggleBar(force?: boolean): Promise<void> {
    const next = force ?? !getSettings().showEvalBar;
    await game.settings.set(MODULE_ID, 'showEvalBar', next);
    this.applyBarVisibility();
  }

  onSettingsChanged(): void {
    if (!this.started) return;
    this.applyBarVisibility();
    this.invalidate();
  }

  // ---- turn tracker (state written by the active GM only) -------------------

  turnState(combat: FoundryCombat | null): TurnState | null {
    if (!combat) return null;
    const raw = combat.getFlag(MODULE_ID, FLAG_TURN);
    return isTurnState(raw) ? raw : null;
  }

  private async writeTurn(combat: FoundryCombat, next: TurnState): Promise<void> {
    await combat.setFlag(MODULE_ID, FLAG_TURN, next);
  }

  async onTurnChange(combat: FoundryCombat): Promise<void> {
    if (!this.isActiveGM || !combat.started) return;
    const prev = this.turnState(combat) ?? emptyTurn(combat.id);
    await this.writeTurn(combat, startTurn(prev, combat.round, combat.combatant?.tokenId ?? null));
  }

  private async recordEvent(e: TrackedEvent | null): Promise<void> {
    const combat = game.combat;
    if (!e || !combat?.started || !this.isActiveGM) return;
    const prev = this.turnState(combat) ?? startTurn(emptyTurn(combat.id), combat.round, combat.combatant?.tokenId ?? null);
    const next = record(prev, e);
    if (next !== prev) await this.writeTurn(combat, next);
  }

  onChatMessage(msg: MessageLike): void {
    void this.recordEvent(classifyMessage(msg, (uuid) => (fromUuidSync(uuid) as OriginItemLike | null) ?? null));
  }

  onTokenMoved(tokenId: string): void {
    if (!getSettings().countMovesAsStride) return;
    if (game.combat?.combatant?.tokenId !== tokenId) return;
    void this.recordEvent({ token: tokenId, kind: 'move', actions: 1, attack: false });
  }

  async adjustTurn(field: 'actionsSpent' | 'attacksMade', delta: number): Promise<void> {
    const combat = game.combat;
    if (!combat || !game.user.isGM) return;
    const prev = this.turnState(combat) ?? startTurn(emptyTurn(combat.id), combat.round, combat.combatant?.tokenId ?? null);
    await this.writeTurn(combat, adjust(prev, field, delta));
  }

  // ---- UI hook handlers ----------------------------------------------------

  renderCombatTracker(html: unknown): void {
    const root = asElement(html);
    if (!root || !this.started) return;
    const p = this.perms();
    renderTrackerPanel(
      root,
      {
        evaluation: this.service.lastEvaluation ?? (this.broadcast as EvaluationReport | null),
        turn: game.combat?.started ? this.turnState(game.combat) : null,
        isGM: p.isGM,
        showBar: canSeeEvalBar(p),
      },
      this.controller,
      (field, d) => void this.adjustTurn(field, d),
    );
  }

  renderTokenHUD(hud: { object?: FoundryTokenObject }, html: unknown): void {
    const root = asElement(html);
    const token = hud.object;
    if (!root || !token || !this.started) return;
    addTokenHudButton(root, token.id, canAnalyzeToken(this.perms(), token.isOwner), this.controller);
  }

  sceneControls(controls: unknown): void {
    addSceneControls(controls, this.controllerProxy(), {
      isGM: game.user.isGM,
      barVisible: getSettings().showEvalBar,
      toggleBar: (v) => void this.toggleBar(v),
    });
  }

  combatantContext(entries: unknown[]): void {
    entries.push(...combatantContextEntries(this.controllerProxy(), game.user.isGM));
  }

  actorSheetButtons(sheet: unknown, buttons: unknown[]): void {
    buttons.unshift(actorSheetButton(sheet as never, this.controllerProxy()));
  }

  /** Controller access that tolerates calls before `ready` finished. */
  private controllerProxy(): Controller {
    return new Proxy({} as Controller, {
      get: (_t, prop) => (...args: unknown[]) => {
        if (!this.controller) return ui.notifications.info(game.i18n.localize('PF2E_STOCKFISH.Notify.starting'));
        return (this.controller[prop as keyof Controller] as (...a: unknown[]) => unknown).apply(this.controller, args);
      },
    });
  }
}

// Narrowing helper used by hooks: is this embedded document on an actor?
export function actorUuidOf(doc: { parent?: { documentName?: string; uuid?: string } | null; documentName?: string; uuid?: string }): string | undefined {
  if (doc.documentName === 'Actor') return doc.uuid;
  return doc.parent?.documentName === 'Actor' ? doc.parent.uuid : undefined;
}
