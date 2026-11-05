// Every Foundry hook the module uses, in one place.
//
// Lifecycle
//   init            settings, keybindings, template preload
//   setup           public API on game.modules.get(id).api
//   ready           engine worker start, eval bar, socket, first analysis
// Re-analysis triggers (debounced in AnalysisService)
//   canvasReady, create/update/deleteCombat, combatStart/Turn/Round,
//   create/update/deleteCombatant, create/update/deleteToken, updateActor,
//   create/update/deleteItem (conditions, effects and gear are items),
//   create/update/deleteActiveEffect, create/update/deleteWall,
//   create/update/deleteRegion
// Turn tracker (active GM)
//   updateCombat (turn/round change), createChatMessage, updateToken (moves)
// UI
//   renderCombatTracker, renderTokenHUD, getSceneControlButtons,
//   getCombatantContextOptions (v13) / getCombatTrackerEntryContext (v12),
//   getActorSheetHeaderButtons (AppV1 sheets),
//   getHeaderControlsActorSheetV2 (AppV2 sheets)

import { MODULE_ID, TEMPLATES } from '../constants';
import { createPublicApi } from '../api/public-api';
import { actorUuidOf, Runtime } from '../runtime';
import { registerSettings } from '../settings';

type AnyDoc = { id?: string; documentName?: string; uuid?: string; parent?: { documentName?: string; uuid?: string } | null };

export function registerHooks(): Runtime {
  const rt = new Runtime();
  const invalidate = () => rt.invalidate();
  const on = (name: string, fn: (...args: never[]) => unknown) => Hooks.on(name, fn);

  Hooks.once('init', () => {
    registerSettings(() => rt.onSettingsChanged());
    rt.init();
    const load = foundry.applications.handlebars?.loadTemplates;
    if (load) void load(Object.values(TEMPLATES));
  });

  Hooks.once('setup', () => {
    const mod = game.modules.get(MODULE_ID);
    if (mod) mod.api = createPublicApi(rt);
  });

  Hooks.once('ready', () => {
    if (game.system.id !== 'pf2e') {
      ui.notifications.error(game.i18n.localize('PF2E_STOCKFISH.Errors.wrongSystem'));
      return;
    }
    void rt.ready();
  });

  // ---- re-analysis -------------------------------------------------------
  for (const h of ['canvasReady', 'createCombat', 'deleteCombat', 'combatStart', 'combatTurn', 'combatRound', 'createCombatant', 'updateCombatant', 'deleteCombatant', 'createToken', 'deleteToken', 'createWall', 'updateWall', 'deleteWall', 'createRegion', 'updateRegion', 'deleteRegion']) {
    on(h, invalidate);
  }
  on('updateActor', (actor: AnyDoc) => {
    rt.forgetActor(actor.uuid);
    invalidate();
  });
  for (const h of ['createItem', 'updateItem', 'deleteItem', 'createActiveEffect', 'updateActiveEffect', 'deleteActiveEffect']) {
    on(h, (doc: AnyDoc) => {
      const uuid = actorUuidOf(doc);
      if (!uuid) return;
      rt.forgetActor(uuid);
      invalidate();
    });
  }

  // ---- turn tracker ------------------------------------------------------
  on('updateCombat', (combat: FoundryCombat, changes: Record<string, unknown>) => {
    if ('turn' in changes || 'round' in changes) void rt.onTurnChange(combat);
    invalidate();
  });
  on('createChatMessage', (msg: Parameters<Runtime['onChatMessage']>[0]) => rt.onChatMessage(msg));
  on('updateToken', (doc: FoundryTokenDocument, changes: Record<string, unknown>) => {
    if ('x' in changes || 'y' in changes) rt.onTokenMoved(doc.id);
    invalidate();
  });

  // ---- UI ----------------------------------------------------------------
  on('renderCombatTracker', (_app: unknown, html: unknown) => rt.renderCombatTracker(html));
  on('renderTokenHUD', (hud: { object?: FoundryTokenObject }, html: unknown) => rt.renderTokenHUD(hud, html));
  on('getSceneControlButtons', (controls: unknown) => rt.sceneControls(controls));
  on('getCombatantContextOptions', (_app: unknown, entries: unknown[]) => rt.combatantContext(entries));
  on('getCombatTrackerEntryContext', (_html: unknown, entries: unknown[]) => rt.combatantContext(entries));
  on('getActorSheetHeaderButtons', (sheet: unknown, buttons: unknown[]) => rt.actorSheetButtons(sheet, buttons));
  on('getHeaderControlsActorSheetV2', (sheet: unknown, controls: unknown[]) => rt.actorSheetButtons(sheet, controls));

  return rt;
}
