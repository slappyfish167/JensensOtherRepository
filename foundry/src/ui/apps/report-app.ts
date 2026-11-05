// One ApplicationV2 window class per report kind, built on Foundry's
// Handlebars mixin. Classes are created lazily (after `init`) because their
// base class only exists at runtime.

import { MODULE_ID, TEMPLATES } from '../../constants';

export type ReportKind = keyof typeof TEMPLATES;

export interface ReportWindow {
  /** Shows (or refreshes) the window with a new template context. */
  show(context: Record<string, unknown>): Promise<void>;
  close(): Promise<void>;
  isOpen(): boolean;
}

export type ActionHandler = (target: HTMLElement) => unknown;

const ICONS: Record<ReportKind, string> = {
  bestTurn: 'fa-solid fa-chess-knight',
  gmAssistant: 'fa-solid fa-dragon',
  forecast: 'fa-solid fa-chart-column',
  explain: 'fa-solid fa-magnifying-glass-chart',
};

const WIDTHS: Record<ReportKind, number> = { bestTurn: 420, gmAssistant: 520, forecast: 440, explain: 460 };

export function createReportWindow(kind: ReportKind, title: string, actions: Record<string, ActionHandler>): ReportWindow {
  const { ApplicationV2, HandlebarsApplicationMixin } = foundry.applications.api;
  const Base = HandlebarsApplicationMixin(ApplicationV2);
  let context: Record<string, unknown> = {};

  // ApplicationV2 calls action handlers with `this` bound to the app and
  // (event, target) as arguments; wrap ours to that shape.
  const wrapped: Record<string, (this: unknown, ev: Event, target: HTMLElement) => void> = {};
  for (const [name, fn] of Object.entries(actions)) {
    wrapped[name] = function (_ev, target) {
      void Promise.resolve(fn(target)).catch((e: unknown) => ui.notifications.error(String(e)));
    };
  }

  class ReportApp extends Base {
    static DEFAULT_OPTIONS = {
      id: `${MODULE_ID}-${kind}`,
      classes: [MODULE_ID, `${MODULE_ID}-${kind}`],
      tag: 'section',
      window: { title, icon: ICONS[kind], resizable: true },
      position: { width: WIDTHS[kind], height: 'auto' as const },
      actions: wrapped,
    };

    static PARTS = { body: { template: TEMPLATES[kind], scrollable: [''] } };

    async _prepareContext(): Promise<Record<string, unknown>> {
      return context;
    }
  }

  let app: ApplicationV2Instance | null = null;
  return {
    async show(ctx) {
      context = ctx;
      app ??= new ReportApp();
      await app.render({ force: true });
    },
    async close() {
      await app?.close();
    },
    isOpen() {
      return !!app?.rendered;
    },
  };
}
