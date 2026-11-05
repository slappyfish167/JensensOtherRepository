// Pure presentation helpers (unit-tested).

import type { StepKind } from '../protocol/report';

export function scoreText(score: number): string {
  if (Math.abs(score) >= 9.95) return score > 0 ? '+M' : '−M';
  const s = score.toFixed(1);
  return score > 0 ? `+${s}` : score < 0 ? `−${s.slice(1)}` : '0.0';
}

/** Party share of the bar, 0–100 (score −10 … +10). */
export function partyShare(score: number): number {
  return Math.max(0, Math.min(100, 50 + score * 5));
}

export function pct(p: number, digits = 0): string {
  return `${(Math.max(0, Math.min(1, p)) * 100).toFixed(digits)}%`;
}

export function signed(v: number, digits = 1): string {
  const s = Math.abs(v).toFixed(digits);
  return v > 0 ? `+${s}` : v < 0 ? `−${s}` : (0).toFixed(digits);
}

export const STEP_ICONS: Record<StepKind, string> = {
  strike: 'fa-solid fa-sword',
  stride: 'fa-solid fa-person-running',
  step: 'fa-solid fa-shoe-prints',
  stand: 'fa-solid fa-person',
  raise_shield: 'fa-solid fa-shield-halved',
  take_cover: 'fa-solid fa-person-shelter',
  demoralize: 'fa-solid fa-face-angry',
  trip: 'fa-solid fa-person-falling',
  cast_spell: 'fa-solid fa-wand-sparkles',
  disable_hazard: 'fa-solid fa-screwdriver-wrench',
};

/** CSS class for a signed impact value. */
export function impactClass(v: number): 'good' | 'bad' | 'neutral' {
  return v > 0.05 ? 'good' : v < -0.05 ? 'bad' : 'neutral';
}

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c] as string);
}
