import type { TacticalReport } from '../api/types';

/** The engine's committed turn: primary and alternate plans plus expectations. */
export function TacticalReportView({ report }: { report: TacticalReport }) {
  const pct = (p: number) => `${(100 * p).toFixed(0)}%`;
  return (
    <div className="tactics">
      <h4>Recommended turn — {report.actor_name}</h4>
      {report.primary.length === 0 ? (
        <p className="muted">No legal actions. {report.reason}</p>
      ) : (
        <ol>{report.primary.map((s, i) => <li key={i}>{s}</li>)}</ol>
      )}
      <h4>Alternate turn</h4>
      {report.alternate.length === 0 ? (
        <p className="muted">None — no other legal opening.</p>
      ) : (
        <>
          <ol>{report.alternate.map((s, i) => <li key={i}>{s}</li>)}</ol>
          {report.alternate_note && <p className="muted small">{report.alternate_note}</p>}
        </>
      )}
      <dl>
        <dt>Expected damage</dt>
        <dd>
          {report.expected_damage.toFixed(1)}
          {report.expected_damage_taken >= 0.05 && <span className="muted small"> (takes {report.expected_damage_taken.toFixed(1)})</span>}
        </dd>
        <dt>Expected status effects</dt>
        <dd>
          {report.expected_status_effects.length === 0
            ? 'None'
            : report.expected_status_effects.slice(0, 6).map((e, i) => <div key={i}>{e.name}: {e.effect} ({pct(e.probability)})</div>)}
        </dd>
        <dt>Priority target</dt>
        <dd>{report.priority_target_name ?? 'None in reach'}</dd>
        <dt>Chance to down target</dt>
        <dd>{pct(report.down_chance)}</dd>
        <dt>Position evaluation</dt>
        <dd className={report.position_score >= 0 ? 'good' : 'bad'}>{report.position}</dd>
        <dt>Reason</dt>
        <dd>{report.reason}</dd>
      </dl>
    </div>
  );
}
