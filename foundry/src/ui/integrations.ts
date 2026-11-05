// Foundry UI integration points: combat tracker, token HUD, scene controls,
// context menus, actor sheet header, hotkeys.

import { MODULE_ID } from '../constants';
import type { EvaluationReport } from '../protocol/report';
import type { TurnState } from '../tracking/turn-tracker';
import type { Controller } from './controller';
import { escapeHtml, partyShare, scoreText } from './format';

const T = (k: string) => game.i18n.localize(`PF2E_STOCKFISH.${k}`);

/** Foundry passes HTMLElement (AppV2) or jQuery (AppV1) to render hooks. */
export function asElement(html: unknown): HTMLElement | null {
  if (html instanceof HTMLElement) return html;
  const first = (html as { [0]?: unknown } | null)?.[0];
  return first instanceof HTMLElement ? first : null;
}

// ---------------------------------------------------------------------------
// Combat tracker
// ---------------------------------------------------------------------------

export interface TrackerPanelState {
  evaluation: EvaluationReport | null;
  turn: TurnState | null;
  isGM: boolean;
  showBar: boolean;
}

export function renderTrackerPanel(root: HTMLElement, s: TrackerPanelState, controller: Controller, adjust: (field: 'actionsSpent' | 'attacksMade', d: number) => void): void {
  root.querySelector(`.${MODULE_ID}-tracker`)?.remove();
  const panel = document.createElement('div');
  panel.className = `${MODULE_ID}-tracker`;
  const share = s.evaluation ? partyShare(s.evaluation.score) : 50;
  const pips = (n: number) => [0, 1, 2].map((i) => `<i class="pip ${i < n ? 'spent' : ''}"></i>`).join('');
  panel.innerHTML = `
    ${s.showBar ? `<div class="mini-bar" title="${escapeHtml(s.evaluation?.band ?? '')}">
      <div class="party" style="width:${share}%"></div><span>${s.evaluation ? scoreText(s.evaluation.score) : '…'}</span></div>` : ''}
    <div class="buttons">
      <button type="button" data-sf="best"><i class="fa-solid fa-chess-knight"></i> ${T('Buttons.bestTurn')}</button>
      ${s.isGM ? `<button type="button" data-sf="monster"><i class="fa-solid fa-dragon"></i> ${T('Buttons.monsterTurn')}</button>
      <button type="button" data-sf="forecast"><i class="fa-solid fa-chart-column"></i></button>` : ''}
      ${s.showBar ? `<button type="button" data-sf="explain"><i class="fa-solid fa-magnifying-glass-chart"></i></button>` : ''}
    </div>
    ${s.turn ? `<div class="turn-state">
      <span title="${T('Tracker.actions')}">${pips(s.turn.actionsSpent)}</span>
      <span class="map" title="${T('Tracker.attacks')}">MAP ${s.turn.attacksMade}</span>
      ${s.isGM ? `<button type="button" data-sf="act-">−</button><button type="button" data-sf="act+">+</button>
      <button type="button" data-sf="atk-" title="${T('Tracker.attacks')}">⚔−</button><button type="button" data-sf="atk+" title="${T('Tracker.attacks')}">⚔+</button>` : ''}
    </div>` : ''}`;
  panel.addEventListener('click', (ev) => {
    const b = (ev.target as HTMLElement).closest<HTMLElement>('[data-sf]');
    switch (b?.dataset.sf) {
      case 'best':
        void controller.analyzeBestTurn();
        break;
      case 'monster':
        void controller.suggestMonsterTurn();
        break;
      case 'forecast':
        void controller.forecast();
        break;
      case 'explain':
        void controller.explain();
        break;
      case 'act-':
        adjust('actionsSpent', -1);
        break;
      case 'act+':
        adjust('actionsSpent', 1);
        break;
      case 'atk-':
        adjust('attacksMade', -1);
        break;
      case 'atk+':
        adjust('attacksMade', 1);
        break;
    }
  });
  const header = root.querySelector('.combat-tracker-header, header, .encounter-controls');
  if (header) header.after(panel);
  else root.prepend(panel);
}

// ---------------------------------------------------------------------------
// Token HUD (right-click on a token)
// ---------------------------------------------------------------------------

export function addTokenHudButton(html: HTMLElement, tokenId: string, enabled: boolean, controller: Controller): void {
  if (!enabled) return;
  const col = html.querySelector('.col.right') ?? html;
  const button = document.createElement('button');
  button.type = 'button';
  button.className = `control-icon ${MODULE_ID}-hud`;
  button.dataset.tooltip = T('Buttons.bestTurn');
  button.innerHTML = '<i class="fa-solid fa-chess-knight"></i>';
  button.addEventListener('click', (ev) => {
    ev.preventDefault();
    void controller.analyzeBestTurn(tokenId);
  });
  col.append(button);
}

// ---------------------------------------------------------------------------
// Scene controls (v13: record of controls with record of tools)
// ---------------------------------------------------------------------------

interface ToolV13 {
  name: string;
  title: string;
  icon: string;
  order: number;
  button?: boolean;
  toggle?: boolean;
  active?: boolean;
  visible?: boolean;
  onChange?: (event: Event, active: boolean) => void;
}

export function addSceneControls(controls: unknown, controller: Controller, opts: { isGM: boolean; barVisible: boolean; toggleBar: (v: boolean) => void }): void {
  const tools: Record<string, ToolV13> = {
    evalbar: { name: 'evalbar', title: T('Controls.evalBar'), icon: 'fa-solid fa-chess-board', order: 1, toggle: true, active: opts.barVisible, onChange: (_e, active) => opts.toggleBar(active) },
    bestturn: { name: 'bestturn', title: T('Controls.bestTurn'), icon: 'fa-solid fa-chess-knight', order: 2, button: true, onChange: () => void controller.analyzeBestTurn() },
    monster: { name: 'monster', title: T('Controls.monsterTurn'), icon: 'fa-solid fa-dragon', order: 3, button: true, visible: opts.isGM, onChange: () => void controller.suggestMonsterTurn() },
    forecast: { name: 'forecast', title: T('Controls.forecast'), icon: 'fa-solid fa-chart-column', order: 4, button: true, visible: opts.isGM, onChange: () => void controller.forecast() },
    explain: { name: 'explain', title: T('Controls.explain'), icon: 'fa-solid fa-magnifying-glass-chart', order: 5, button: true, onChange: () => void controller.explain() },
  };
  const group = { name: MODULE_ID, title: T('Controls.group'), icon: 'fa-solid fa-chess', order: 90, layer: 'tokens', activeTool: 'evalbar', tools };
  if (controls && typeof controls === 'object' && !Array.isArray(controls)) {
    (controls as Record<string, unknown>)[MODULE_ID] = group;
  }
}

// ---------------------------------------------------------------------------
// Context menus
// ---------------------------------------------------------------------------

interface ContextEntry {
  name: string;
  icon: string;
  condition?: (li: unknown) => boolean;
  callback: (li: unknown) => void;
}

function combatantIdOf(li: unknown): string | null {
  const el = asElement(li) ?? (li instanceof HTMLElement ? li : null);
  return el?.dataset.combatantId ?? null;
}

export function combatantContextEntries(controller: Controller, isGM: boolean): ContextEntry[] {
  const tokenOf = (li: unknown) => {
    const id = combatantIdOf(li);
    return id ? (game.combat?.combatants.get(id)?.tokenId ?? null) : null;
  };
  const entries: ContextEntry[] = [
    {
      name: T('Buttons.bestTurn'),
      icon: '<i class="fa-solid fa-chess-knight"></i>',
      condition: (li) => !!tokenOf(li),
      callback: (li) => void controller.analyzeBestTurn(tokenOf(li) ?? undefined),
    },
  ];
  if (isGM) entries.push({ name: T('Buttons.monsterTurn'), icon: '<i class="fa-solid fa-dragon"></i>', callback: () => void controller.suggestMonsterTurn() });
  return entries;
}

// ---------------------------------------------------------------------------
// Actor sheet header (AppV1 PF2e sheets)
// ---------------------------------------------------------------------------

export function actorSheetButton(sheet: { actor?: { getActiveTokens?: () => FoundryTokenObject[] } }, controller: Controller) {
  return {
    label: T('Buttons.bestTurnShort'),
    class: `${MODULE_ID}-sheet`,
    icon: 'fa-solid fa-chess-knight',
    onclick: () => {
      const token = sheet.actor?.getActiveTokens?.()[0];
      if (token) void controller.analyzeBestTurn(token.id);
      else ui.notifications.info(T('Notify.noTokenOnScene'));
    },
  };
}

// ---------------------------------------------------------------------------
// Hotkeys
// ---------------------------------------------------------------------------

export function registerKeybindings(controller: Controller, toggleBar: () => void): void {
  const bind = (key: string, code: string, onDown: () => void, restricted = false) =>
    game.keybindings.register(MODULE_ID, key, {
      name: `PF2E_STOCKFISH.Keys.${key}`,
      editable: [{ key: code, modifiers: ['Shift'] }],
      restricted,
      onDown: () => {
        onDown();
        return true;
      },
    });
  bind('bestTurn', 'KeyB', () => void controller.analyzeBestTurn());
  bind('monsterTurn', 'KeyM', () => void controller.suggestMonsterTurn(), true);
  bind('forecast', 'KeyF', () => void controller.forecast(), true);
  bind('explain', 'KeyX', () => void controller.explain());
  bind('toggleBar', 'KeyE', toggleBar);
}
