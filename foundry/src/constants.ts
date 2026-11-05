export const MODULE_ID = 'pf2e-stockfish';
export const SOCKET = `module.${MODULE_ID}`;
export const FLAG_TURN = 'turn';
export const TEMPLATE_ROOT = `modules/${MODULE_ID}/templates`;

export const TEMPLATES = {
  bestTurn: `${TEMPLATE_ROOT}/best-turn.hbs`,
  gmAssistant: `${TEMPLATE_ROOT}/gm-assistant.hbs`,
  forecast: `${TEMPLATE_ROOT}/forecast.hbs`,
  explain: `${TEMPLATE_ROOT}/explain.hbs`,
} as const;

/** Hook other modules can listen to: `Hooks.on('pf2e-stockfish.evaluation', (report) => …)`. */
export const HOOK_EVALUATION = `${MODULE_ID}.evaluation`;
