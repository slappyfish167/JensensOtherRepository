// Draws a recommended turn on the canvas: the Stride path, the destination
// square, numbered steps and rings around targets. Cleared automatically.

import type { TurnReport } from '../protocol/report';

const PATH = 0x4fc3f7;
const TARGET = 0xef5350;
const STEP = 0xffd54f;

let current: { container: PixiDisplayObject; timer: ReturnType<typeof setTimeout> } | null = null;

export function clearOverlay(): void {
  if (!current) return;
  clearTimeout(current.timer);
  try {
    canvas.controls.removeChild(current.container);
    current.container.destroy({ children: true });
  } catch {
    /* canvas was torn down */
  }
  current = null;
}

export function drawTurn(report: TurnReport, ttlMs = 12_000): void {
  clearOverlay();
  if (!canvas?.ready) return;
  const size = canvas.grid.size;
  const container = new PIXI.Container();
  const g = new PIXI.Graphics();
  container.addChild(g);

  report.steps.forEach((step, i) => {
    if (step.path?.length) {
      g.lineStyle(6, PATH, 0.85);
      const [first, ...rest] = step.path;
      if (first) {
        g.moveTo(first.px[0] + size / 2, first.px[1] + size / 2);
        for (const p of rest) g.lineTo(p.px[0] + size / 2, p.px[1] + size / 2);
      }
    }
    if (step.to) {
      g.lineStyle(4, PATH, 1).beginFill(PATH, 0.18).drawRect(step.to.px[0] + 4, step.to.px[1] + 4, size - 8, size - 8).endFill();
      const label = new PIXI.Text(String(i + 1), { fontSize: Math.round(size / 3), fill: STEP, stroke: 0x000000, strokeThickness: 4, fontWeight: 'bold' });
      label.anchor.set(0.5);
      label.position.set(step.to.px[0] + size / 2, step.to.px[1] + size / 2);
      container.addChild(label);
    }
    for (const tokenId of step.target_tokens ?? []) {
      const t = canvas.tokens.get(tokenId);
      if (!t) continue;
      g.lineStyle(5, TARGET, 0.9).drawCircle(t.center.x, t.center.y, Math.max(t.w, t.h) * 0.62);
      const label = new PIXI.Text(String(i + 1), { fontSize: Math.round(size / 4), fill: STEP, stroke: 0x000000, strokeThickness: 4, fontWeight: 'bold' });
      label.anchor.set(0.5);
      label.position.set(t.center.x + t.w * 0.45, t.center.y - t.h * 0.45 + i * 6);
      container.addChild(label);
    }
  });
  canvas.controls.addChild(container);
  current = { container, timer: setTimeout(clearOverlay, ttlMs) };
}
