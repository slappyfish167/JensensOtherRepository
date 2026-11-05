import type { Battlefield, Combatant, CombatantId, Pos } from '../api/types';

const SIZE_SQUARES: Record<string, number> = { tiny: 1, small: 1, medium: 1, large: 2, huge: 3, gargantuan: 4 };
const TERRAIN_CLASS: Record<string, string> = { '.': 'open', '~': 'difficult', '^': 'greater', '#': 'wall', '%': 'obscured' };

export interface GridToken {
  id: CombatantId | string;
  name: string;
  side: string;
  position: Pos;
  size?: string;
  hp?: number;
  maxHp?: number;
  down?: boolean;
  hazard?: boolean;
}

export function tokensFromCombatants(cs: Combatant[]): GridToken[] {
  return cs
    .filter((c) => c.life !== 'dead' || c.kind !== 'hazard')
    .map((c) => ({
      id: c.id,
      name: c.name,
      side: c.side,
      position: c.position,
      size: c.statblock.size,
      hp: c.hp,
      maxHp: c.statblock.max_hp,
      down: c.life !== 'active' || c.conditions.some((x) => x.kind === 'unconscious'),
      hazard: c.kind === 'hazard',
    }));
}

interface Props {
  battlefield: Battlefield;
  tokens: GridToken[];
  activeId?: CombatantId | string | null;
  selectedId?: CombatantId | string | null;
  highlights?: Map<string, string>; // "x,y" → css class
  cell?: number;
  onCellClick?: (p: Pos) => void;
  onTokenClick?: (id: CombatantId | string) => void;
}

export function BattleGrid({ battlefield, tokens, activeId, selectedId, highlights, cell = 34, onCellClick, onTokenClick }: Props) {
  const { width, height, map } = battlefield;
  const cells = [];
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const glyph = map[y]?.[x] ?? '#';
      const hl = highlights?.get(`${x},${y}`);
      cells.push(
        <div
          key={`${x},${y}`}
          className={`cell ${TERRAIN_CLASS[glyph] ?? 'wall'} ${hl ?? ''}`}
          style={{ left: x * cell, top: y * cell, width: cell, height: cell }}
          onClick={() => onCellClick?.({ x, y })}
          title={`(${x}, ${y})`}
        />,
      );
    }
  }
  return (
    <div className="grid-wrap">
      <div className="grid" style={{ width: width * cell, height: height * cell }}>
        {cells}
        {tokens.map((t) => {
          const sq = SIZE_SQUARES[t.size ?? 'medium'] ?? 1;
          const hpPct = t.maxHp ? Math.max(0, (100 * (t.hp ?? 0)) / t.maxHp) : 100;
          return (
            <div
              key={String(t.id)}
              className={`token ${t.side} ${t.hazard ? 'hazard' : ''} ${t.down ? 'down' : ''} ${t.id === activeId ? 'active' : ''} ${t.id === selectedId ? 'selected' : ''}`}
              style={{ left: t.position.x * cell + 2, top: t.position.y * cell + 2, width: sq * cell - 4, height: sq * cell - 4 }}
              title={`${t.name}${t.hp != null ? ` — ${t.hp}/${t.maxHp} HP` : ''}`}
              onClick={(e) => {
                e.stopPropagation();
                onTokenClick ? onTokenClick(t.id) : onCellClick?.(t.position);
              }}
            >
              <span>{initials(t.name)}</span>
              {t.maxHp != null && !t.hazard && <i className="token-hp" style={{ width: `${hpPct}%` }} />}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function initials(name: string): string {
  const parts = name.split(/\s+/).filter(Boolean);
  const last = parts[parts.length - 1];
  if (parts.length > 1 && /^\d+$/.test(last)) return (parts[0][0] + last).toUpperCase();
  return parts.slice(0, 2).map((p) => p[0]).join('').toUpperCase();
}
