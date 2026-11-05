import { evalBand, evalText } from '../lib/format';

/** Vertical evaluation bar in the style of a chess engine. Party at the bottom. */
export function EvalBar({ score, winProbability }: { score: number | null; winProbability?: number }) {
  const s = score ?? 0;
  const partyShare = 50 + s * 5; // −10..+10 → 0..100 %
  return (
    <div className="evalbar" title={score == null ? 'not evaluated' : evalBand(s)}>
      <div className="evalbar-track">
        <div className="evalbar-enemy" style={{ height: `${100 - partyShare}%` }} />
        <div className="evalbar-party" style={{ height: `${partyShare}%` }} />
      </div>
      <div className="evalbar-label">
        <strong>{score == null ? '…' : evalText(s)}</strong>
        <span>{score == null ? '' : evalBand(s)}</span>
        {winProbability != null && <span className="muted">win ≈ {(100 * winProbability).toFixed(0)}%</span>}
      </div>
    </div>
  );
}
