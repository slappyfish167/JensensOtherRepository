// The evaluation bar: a vertical chess-engine style bar beside the canvas.
// Party advantage fills from the bottom, monster advantage from the top.

import type { EvaluationReport } from '../protocol/report';
import { partyShare, pct, scoreText, signed } from './format';

/** What players receive in GM-authoritative mode (no factor details). */
export interface BroadcastEvaluation {
  score: number;
  band: string;
  win_probability: number;
}

export class EvalBar {
  private el: HTMLElement | null = null;
  private visible = true;

  constructor(private readonly onClick: () => void) {}

  mount(side: 'left' | 'right'): void {
    if (this.el) return;
    const el = document.createElement('aside');
    el.id = 'pf2e-stockfish-evalbar';
    el.className = `pf2e-stockfish-evalbar side-${side}`;
    el.setAttribute('role', 'meter');
    el.setAttribute('aria-valuemin', '-10');
    el.setAttribute('aria-valuemax', '10');
    el.innerHTML = `
      <div class="track"><div class="enemy"></div><div class="party"></div><div class="midline"></div></div>
      <div class="label"><strong class="score">…</strong><span class="band"></span><span class="win"></span></div>`;
    el.addEventListener('click', () => this.onClick());
    document.body.append(el);
    this.el = el;
    this.render(null);
  }

  setSide(side: 'left' | 'right'): void {
    this.el?.classList.remove('side-left', 'side-right');
    this.el?.classList.add(`side-${side}`);
  }

  setVisible(v: boolean): void {
    this.visible = v;
    if (this.el) this.el.hidden = !v;
  }

  isVisible(): boolean {
    return this.visible;
  }

  render(r: EvaluationReport | BroadcastEvaluation | null, note?: string): void {
    const el = this.el;
    if (!el) return;
    const party = el.querySelector<HTMLElement>('.party');
    const enemy = el.querySelector<HTMLElement>('.enemy');
    const score = el.querySelector<HTMLElement>('.score');
    const band = el.querySelector<HTMLElement>('.band');
    const win = el.querySelector<HTMLElement>('.win');
    if (!party || !enemy || !score || !band || !win) return;
    if (!r) {
      party.style.height = '50%';
      enemy.style.height = '50%';
      score.textContent = '…';
      band.textContent = note ?? '';
      win.textContent = '';
      el.title = note ?? '';
      el.classList.add('idle');
      return;
    }
    el.classList.remove('idle');
    const share = partyShare(r.score);
    party.style.height = `${share}%`;
    enemy.style.height = `${100 - share}%`;
    score.textContent = scoreText(r.score);
    band.textContent = r.band;
    win.textContent = pct(r.win_probability);
    el.setAttribute('aria-valuenow', r.score.toFixed(1));
    const factors = 'factors' in r ? r.factors.map((f) => `${f.name}: ${signed(f.value)}`).join('\n') : '';
    el.title = `${scoreText(r.score)} — ${r.band}\nwin ≈ ${pct(r.win_probability)}${factors ? `\n\n${factors}` : ''}`;
  }

  destroy(): void {
    this.el?.remove();
    this.el = null;
  }
}
