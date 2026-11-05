// Minimal ambient declarations for the Foundry VTT v13 globals this module
// uses. Deliberately narrow: only members the code touches are declared,
// so the compiler checks our usage without pulling in a full type package
// whose release cadence differs from Foundry's.

interface FoundryUser {
  id: string;
  name: string;
  isGM: boolean;
  isSelf?: boolean;
}

interface FoundryCollection<T> extends Iterable<T> {
  get(id: string): T | undefined;
  contents: T[];
  size: number;
}

interface FoundryDocument {
  id: string;
  uuid?: string;
  documentName?: string;
  parent?: FoundryDocument | null;
  isOwner?: boolean;
  getFlag(scope: string, key: string): unknown;
  setFlag(scope: string, key: string, value: unknown): Promise<unknown>;
  unsetFlag?(scope: string, key: string): Promise<unknown>;
}

interface FoundryTokenDocument extends FoundryDocument {
  name: string;
  x: number;
  y: number;
  hidden: boolean;
  disposition: number;
  actor: (FoundryDocument & Record<string, unknown>) | null;
  object?: FoundryTokenObject | null;
}

interface FoundryTokenObject {
  id: string;
  document: FoundryTokenDocument;
  visible: boolean;
  isOwner: boolean;
  controlled: boolean;
  center: { x: number; y: number };
  w: number;
  h: number;
}

interface FoundryCombatant extends FoundryDocument {
  tokenId: string | null;
  token: FoundryTokenDocument | null;
  initiative: number | null;
  defeated: boolean;
  hidden: boolean;
}

interface FoundryCombat extends FoundryDocument {
  started: boolean;
  round: number;
  turn: number | null;
  combatant: FoundryCombatant | null;
  turns: FoundryCombatant[];
  scene: (FoundryDocument & { id: string }) | null;
  combatants: FoundryCollection<FoundryCombatant>;
}

interface FoundryScene extends FoundryDocument {
  name: string;
  grid: { type: number; size: number; distance: number; units?: string };
  dimensions: { sceneX: number; sceneY: number; sceneWidth: number; sceneHeight: number; size: number; distance: number };
  walls: FoundryCollection<FoundryDocument & { c: number[]; move: number; sight: number; door: number; ds: number }>;
  tokens: FoundryCollection<FoundryTokenDocument>;
}

interface FoundrySettingConfig<T> {
  name: string;
  hint?: string;
  scope: 'world' | 'client' | 'user';
  config: boolean;
  type: unknown;
  default: T;
  choices?: Record<string, string>;
  range?: { min: number; max: number; step: number };
  requiresReload?: boolean;
  onChange?: (value: T) => void;
}

interface FoundryKeybinding {
  name: string;
  hint?: string;
  editable: { key: string; modifiers?: string[] }[];
  onDown: () => boolean | void;
  restricted?: boolean;
  precedence?: number;
}

interface FoundryGame {
  ready: boolean;
  user: FoundryUser;
  users: FoundryCollection<FoundryUser> & { activeGM?: FoundryUser | null };
  combat: FoundryCombat | null;
  combats: FoundryCollection<FoundryCombat>;
  scenes: FoundryCollection<FoundryScene> & { viewed?: FoundryScene | null };
  system: { id: string; version: string };
  modules: FoundryCollection<{ id: string; active: boolean; version: string; api?: unknown }>;
  settings: {
    register<T>(module: string, key: string, config: FoundrySettingConfig<T>): void;
    get(module: string, key: string): unknown;
    set(module: string, key: string, value: unknown): Promise<unknown>;
  };
  keybindings: { register(module: string, key: string, config: FoundryKeybinding): void };
  socket: { on(event: string, cb: (data: unknown) => void): void; emit(event: string, data: unknown): void };
  i18n: { localize(key: string): string; format(key: string, data: Record<string, unknown>): string };
}

interface FoundryNotifications {
  info(msg: string, opts?: unknown): void;
  warn(msg: string, opts?: unknown): void;
  error(msg: string, opts?: unknown): void;
}

interface ApplicationV2Instance {
  rendered: boolean;
  element: HTMLElement;
  render(options?: boolean | { force?: boolean }): Promise<unknown>;
  close(options?: unknown): Promise<unknown>;
}

type ApplicationV2Ctor = new (options?: Record<string, unknown>) => ApplicationV2Instance;

interface FoundryNamespace {
  utils: {
    getRoute(path: string): string;
    debounce<F extends (...args: never[]) => unknown>(fn: F, ms: number): F;
    mergeObject<T>(a: T, b: unknown, opts?: unknown): T;
  };
  applications: {
    api: {
      ApplicationV2: ApplicationV2Ctor;
      HandlebarsApplicationMixin: (base: ApplicationV2Ctor) => ApplicationV2Ctor;
    };
    handlebars?: { loadTemplates(paths: string[]): Promise<unknown>; renderTemplate(path: string, data: unknown): Promise<string> };
  };
}

declare const game: FoundryGame;
declare const ui: { notifications: FoundryNotifications; combat?: { render(force?: boolean): unknown } };
declare const foundry: FoundryNamespace;
declare const Hooks: {
  on(hook: string, fn: (...args: never[]) => unknown): number;
  once(hook: string, fn: (...args: never[]) => unknown): number;
  callAll(hook: string, ...args: unknown[]): boolean;
};
declare const canvas: {
  ready: boolean;
  scene: FoundryScene | null;
  grid: { size: number };
  tokens: { controlled: FoundryTokenObject[]; get(id: string): FoundryTokenObject | undefined; placeables: FoundryTokenObject[] };
  controls: { addChild<T>(child: T): T; removeChild<T>(child: T): T };
  regions?: { placeables: { document: { behaviors: Iterable<{ type: string; disabled?: boolean; system?: Record<string, unknown> }>; getFlag(scope: string, key: string): unknown }; testPoint?(p: { x: number; y: number; elevation?: number }): boolean; document_testPoint?: unknown }[] };
};
declare function fromUuidSync(uuid: string): unknown;
declare const PIXI: {
  Graphics: new () => PixiGraphics;
  Text: new (text: string, style?: Record<string, unknown>) => PixiDisplayObject & { anchor: { set(x: number, y?: number): void } };
  Container: new () => PixiDisplayObject & { addChild<T>(c: T): T };
};
interface PixiDisplayObject {
  position: { set(x: number, y: number): void };
  destroy(opts?: unknown): void;
  zIndex?: number;
}
interface PixiGraphics extends PixiDisplayObject {
  lineStyle(width: number, color: number, alpha?: number): PixiGraphics;
  beginFill(color: number, alpha?: number): PixiGraphics;
  endFill(): PixiGraphics;
  moveTo(x: number, y: number): PixiGraphics;
  lineTo(x: number, y: number): PixiGraphics;
  drawCircle(x: number, y: number, r: number): PixiGraphics;
  drawRect(x: number, y: number, w: number, h: number): PixiGraphics;
}
