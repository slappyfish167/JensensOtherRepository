// Orchestrates extraction of a full CombatSnapshot from Foundry documents.
//
// Who is included:
// * With a running combat on the scene: its combatants (defeated excluded).
// * Otherwise: every token on the scene whose actor is a creature or hazard.
// * For non-GM viewers, hidden tokens and tokens the viewer cannot see are
//   removed *before* analysis, so player-side results never encode
//   information the player does not have.

import { SNAPSHOT_VERSION, type CombatSnapshot, type CombatantSnapshot, type TerrainCell } from '../protocol/snapshot';
import type { TurnState } from '../tracking/turn-tracker';
import { toArray, type CombatLike, type SceneLike, type TokenDocLike } from '../types/pf2e';
import { extractCombatant } from './actor';
import { extractScene, geometry, toSquare } from './scene';

export interface Viewer {
  isGM: boolean;
  /** Whether this user may see the token (used for non-GM viewers). */
  canSee: (token: TokenDocLike) => boolean;
}

export interface SnapshotInput {
  scene: SceneLike;
  combat: CombatLike | null;
  turn: TurnState | null;
  viewer: Viewer;
  options: { flanking: boolean; autoCover: boolean; cropMargin: number };
  /** Terrain sampled from regions, if any (scene squares). */
  terrain?: TerrainCell[];
  strikeCache?: Map<string, import('./strikes').StrikeExtraction>;
}

export interface SnapshotResult {
  snapshot: CombatSnapshot;
  warnings: string[];
  /** Tokens left out for the viewer (hidden or not visible). */
  withheld: number;
}

export async function buildSnapshot(input: SnapshotInput): Promise<SnapshotResult> {
  const { scene, combat, turn, viewer } = input;
  const g = geometry(scene);
  const warnings: string[] = [];
  const onScene = combat && (!combat.scene || combat.scene.id === scene.id) ? combat : null;

  type Entry = { token: TokenDocLike; initiative: number | null };
  let entries: Entry[];
  if (onScene && (onScene.turns?.length ?? 0) > 0) {
    entries = (onScene.turns ?? [])
      .filter((c) => !c.defeated && c.token)
      .map((c) => ({ token: c.token as TokenDocLike, initiative: c.initiative ?? null }));
  } else {
    entries = toArray(scene.tokens).map((token) => ({ token, initiative: null }));
  }
  let withheld = 0;
  if (!viewer.isGM) {
    const before = entries.length;
    entries = entries.filter((e) => !e.token.hidden && viewer.canSee(e.token));
    withheld = before - entries.length;
  }

  const combatants: CombatantSnapshot[] = [];
  for (const { token, initiative } of entries) {
    const square = toSquare(g, token.x ?? 0, token.y ?? 0);
    const { combatant, warnings: w } = await extractCombatant(token, {
      square,
      initiative,
      reactionUsed: !!(token.id && turn?.reactionsUsed.includes(token.id)),
      strikeCache: input.strikeCache,
    });
    warnings.push(...w);
    if (combatant) combatants.push(combatant);
  }

  const order = onScene ? (onScene.turns ?? []).filter((c) => !c.defeated).map((c) => c.tokenId ?? c.token?.id).filter((t): t is string => !!t) : combatants.map((c) => c.token_id);
  const currentToken = onScene?.combatant?.tokenId ?? onScene?.combatant?.token?.id ?? null;
  const started = !!onScene?.started && !!currentToken;
  const tracked = started && turn && turn.combatantToken === currentToken && turn.round === onScene?.round;

  const snapshot: CombatSnapshot = {
    version: SNAPSHOT_VERSION,
    scene: extractScene(scene, input.terrain ?? []),
    combatants,
    turn: {
      started,
      round: onScene?.round ?? 0,
      current: started ? currentToken : null,
      order,
      actions_spent: tracked ? turn.actionsSpent : 0,
      attacks_made: tracked ? turn.attacksMade : 0,
    },
    options: { flanking: input.options.flanking, auto_cover: input.options.autoCover, crop_margin: input.options.cropMargin },
  };
  return { snapshot, warnings, withheld };
}
