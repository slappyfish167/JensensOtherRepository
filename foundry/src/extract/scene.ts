// Scene geometry → snapshot scene (grid, walls, terrain).

import type { GridKind, SceneSnapshot, TerrainCell, TerrainKind, WallSnapshot } from '../protocol/snapshot';
import { toArray, type SceneLike, type WallDocLike } from '../types/pf2e';

/** CONST.GRID_TYPES: 0 gridless, 1 square, 2–5 hex variants. */
export function gridKind(type: number | undefined): GridKind {
  if (type === 1) return 'square';
  if (type === 0) return 'gridless';
  return 'hex';
}

export interface SceneGeometry {
  size: number;
  originX: number;
  originY: number;
  width: number;
  height: number;
}

export function geometry(scene: SceneLike): SceneGeometry {
  const size = scene.dimensions?.size ?? scene.grid?.size ?? 100;
  const d = scene.dimensions ?? {};
  return {
    size,
    originX: d.sceneX ?? 0,
    originY: d.sceneY ?? 0,
    width: Math.max(1, Math.ceil((d.sceneWidth ?? size) / size)),
    height: Math.max(1, Math.ceil((d.sceneHeight ?? size) / size)),
  };
}

/** Canvas pixels → scene square (top-left of a token). */
export function toSquare(g: SceneGeometry, x: number, y: number): [number, number] {
  return [Math.round((x - g.originX) / g.size), Math.round((y - g.originY) / g.size)];
}

/** CONST.WALL_MOVEMENT_TYPES.NONE / WALL_SENSE_TYPES.NONE = 0; door state OPEN = 1. */
export function wallSnapshot(w: WallDocLike, g: SceneGeometry): WallSnapshot | null {
  const c = w.c;
  if (!c || c.length < 4 || c.some((v) => typeof v !== 'number' || !Number.isFinite(v))) return null;
  const [x0, y0, x1, y1] = c as [number, number, number, number];
  return {
    a: [(x0 - g.originX) / g.size, (y0 - g.originY) / g.size],
    b: [(x1 - g.originX) / g.size, (y1 - g.originY) / g.size],
    blocks_movement: (w.move ?? 1) !== 0,
    blocks_sight: (w.sight ?? 1) !== 0,
    open_door: (w.door ?? 0) > 0 && w.ds === 1,
  };
}

/** Samples a terrain function at square centres inside a window. */
export function sampleTerrain(
  g: SceneGeometry,
  window: { x0: number; y0: number; x1: number; y1: number },
  terrainAt: (px: number, py: number) => TerrainKind | null,
): TerrainCell[] {
  const cells: TerrainCell[] = [];
  const x0 = Math.max(0, window.x0);
  const y0 = Math.max(0, window.y0);
  const x1 = Math.min(g.width, window.x1);
  const y1 = Math.min(g.height, window.y1);
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const kind = terrainAt(g.originX + (x + 0.5) * g.size, g.originY + (y + 0.5) * g.size);
      if (kind) cells.push({ x, y, kind });
    }
  }
  return cells;
}

export function extractScene(scene: SceneLike, terrain: TerrainCell[] = []): SceneSnapshot {
  const g = geometry(scene);
  const walls = toArray(scene.walls)
    .map((w) => wallSnapshot(w, g))
    .filter((w): w is WallSnapshot => w !== null && (w.blocks_movement || !!w.blocks_sight));
  return {
    id: scene.id ?? '',
    name: scene.name ?? '',
    grid: gridKind(scene.grid?.type),
    grid_size_px: g.size,
    grid_distance_ft: scene.grid?.distance ?? scene.dimensions?.distance ?? 5,
    origin_px: [g.originX, g.originY],
    width: g.width,
    height: g.height,
    walls,
    terrain,
  };
}
