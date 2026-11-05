// Module settings. World settings are GM policy; client settings are
// per-machine preferences (search time depends on the machine).

import { MODULE_ID } from './constants';

export type EvalBarVisibility = 'gm' | 'all' | 'none';

export interface Settings {
  evalBarVisibility: EvalBarVisibility;
  playerBestTurn: boolean;
  gmAuthoritative: boolean;
  forecastRuns: number;
  countMovesAsStride: boolean;
  flanking: boolean;
  autoCover: boolean;
  cropMargin: number;
  searchMs: number;
  monsterSearchMs: number;
  forecastWorkers: number;
  autoAnalyze: boolean;
  evalBarSide: 'left' | 'right';
  showEvalBar: boolean;
}

const L = (k: string) => `PF2E_STOCKFISH.Settings.${k}`;

export function registerSettings(onChange: () => void): void {
  const reg = <T>(key: keyof Settings, cfg: Omit<FoundrySettingConfig<T>, 'name' | 'hint'>) =>
    game.settings.register<T>(MODULE_ID, key, { name: L(`${key}.Name`), hint: L(`${key}.Hint`), ...cfg });

  // World policy (GM).
  reg<EvalBarVisibility>('evalBarVisibility', {
    scope: 'world', config: true, type: String, default: 'gm', onChange,
    choices: { gm: L('evalBarVisibility.gm'), all: L('evalBarVisibility.all'), none: L('evalBarVisibility.none') },
  });
  reg<boolean>('playerBestTurn', { scope: 'world', config: true, type: Boolean, default: true });
  reg<boolean>('gmAuthoritative', { scope: 'world', config: true, type: Boolean, default: true, onChange });
  reg<number>('forecastRuns', { scope: 'world', config: true, type: Number, default: 300, range: { min: 50, max: 5000, step: 50 } });
  reg<boolean>('countMovesAsStride', { scope: 'world', config: true, type: Boolean, default: true });
  reg<boolean>('flanking', { scope: 'world', config: true, type: Boolean, default: true, onChange });
  reg<boolean>('autoCover', { scope: 'world', config: true, type: Boolean, default: true, onChange });
  reg<number>('cropMargin', { scope: 'world', config: true, type: Number, default: 12, range: { min: 4, max: 60, step: 1 }, onChange });

  // Client preferences.
  reg<number>('searchMs', { scope: 'client', config: true, type: Number, default: 75, range: { min: 30, max: 2000, step: 10 } });
  reg<number>('monsterSearchMs', { scope: 'client', config: true, type: Number, default: 400, range: { min: 50, max: 5000, step: 50 } });
  const cores = typeof navigator !== 'undefined' ? navigator.hardwareConcurrency || 2 : 2;
  reg<number>('forecastWorkers', { scope: 'client', config: true, type: Number, default: Math.max(1, Math.min(4, cores - 1)), range: { min: 0, max: 8, step: 1 }, requiresReload: true });
  reg<boolean>('autoAnalyze', { scope: 'client', config: true, type: Boolean, default: true, onChange });
  reg<'left' | 'right'>('evalBarSide', { scope: 'client', config: true, type: String, default: 'left', choices: { left: L('evalBarSide.left'), right: L('evalBarSide.right') }, onChange });
  reg<boolean>('showEvalBar', { scope: 'client', config: false, type: Boolean, default: true, onChange });
}

export function getSettings(): Settings {
  const g = <T>(k: keyof Settings) => game.settings.get(MODULE_ID, k) as T;
  return {
    evalBarVisibility: g('evalBarVisibility'),
    playerBestTurn: g('playerBestTurn'),
    gmAuthoritative: g('gmAuthoritative'),
    forecastRuns: g('forecastRuns'),
    countMovesAsStride: g('countMovesAsStride'),
    flanking: g('flanking'),
    autoCover: g('autoCover'),
    cropMargin: g('cropMargin'),
    searchMs: g('searchMs'),
    monsterSearchMs: g('monsterSearchMs'),
    forecastWorkers: g('forecastWorkers'),
    autoAnalyze: g('autoAnalyze'),
    evalBarSide: g('evalBarSide'),
    showEvalBar: g('showEvalBar'),
  };
}
