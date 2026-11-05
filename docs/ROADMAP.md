# Roadmap

Built so far (phase 1 — foundations):

- [x] Deterministic, serializable, replayable rules core with an event-sourced command log
- [x] Human-entered dice with validation; opt-in seeded auto-rolls recorded as commands
- [x] Character builder with exhaustive validation and stat-block derivation; JSON import/export
- [x] Monster library, stat-block import, elite/weak templates; simple and complex hazards
- [x] Encounter builder with XP budget/threat, map painting, placement
- [x] Combat: initiative, action economy, MAP, 25 conditions, durations, persistent damage, dying,
      Reactive Strike (with disruption), Shield Block, cover, concealment, flanking, spells
      (attack / basic save / save / automatic / heal / buff; single, multi-target, areas; heightening)
- [x] Evaluation (−10…+10 with factor breakdown), expectiminimax best turn, strongest target,
      spell ranking, monster strategy, Monte Carlo TPK odds
- [x] SQLite storage (Postgres-compatible schema), REST API, React UI, automated tests

## Phase 2 — rules breadth

1. **More actions**: Delay, Ready, Interact/Draw, Aid, Grapple/Shove/Escape/Reposition, Seek, Hide,
   Sneak, Point Out, Battle Medicine, Treat Wounds (out of combat), Escape.
2. **Activated feats and class features** as data-driven "activities": a small action DSL
   (sequence of Strikes with modifiers, movement + Strike, MAP rules) so Sudden Charge, Double Slice,
   Power Attack, Twin Feint, etc. can be added as content.
3. **Spell system**: Sustain, concentrate/manipulate flat checks (stupefied), counteract checks,
   persistent-damage heightening, emanation auras that follow the caster, true prepared slots,
   focus spells and focus points, spell attack MAP per target.
4. **Positioning**: initiative re-ordering when knocked out, squeezing, elevation/flying, reach
   through allies, light levels and vision (darkvision, low-light), cones by grid template.
5. **Content import**: an importer for Foundry-style or Archives-of-Nethys-style JSON, plus a
   content-pack format with source/licence metadata (ORC).

## Foundry VTT module

Built: see [FOUNDRY.md](FOUNDRY.md) (1.0 feature set, 1.5 and 2.0 roadmap). Thin walls between
squares (`Battlefield::edges`) were added to the core for Foundry walls.

## Phase 3 — engine & analysis

1. ~~Transposition table + iterative deepening in the turn search~~ (done, with an evaluation
   cache, deadline budgets and exact multi-worker Monte Carlo); parallel root search.
2. Evaluator trained on Monte Carlo outcomes (keep the heuristic one as baseline and fallback).
3. Multi-turn lookahead with opponent models (scripted → shallow search).
4. ~~"Explain this evaluation change" diffs between two positions~~ (done: `pf2e_ai::explain`);
   blunder check for a declared turn.
5. Snapshot every N commands to speed up loading very long combats.

## Phase 4 — product

1. PostgreSQL `Store` implementation (schema already portable) and multi-user campaigns.
2. Live table mode: WebSocket push of `CombatView` to player screens; player-side roll entry.
3. Desktop packaging (single binary serving the built UI already works: `PF2E_STATIC_DIR`).
4. Property-based tests (random legal command sequences must replay identically and never panic).

## Suggested additional automated tests

* Golden replays: store real play logs under `tests/golden/` and assert the final fingerprint.
* Property tests (proptest): for random encounters and random legal actions, `apply` never panics,
  illegal commands never change state, and replay is identical.
* Builder: table-driven tests of every class progression at levels 1, 5, 10, 15, 20 against the
  published tables once full content is in.
* Frontend: component tests for the roll prompt (input validation) and Playwright smoke tests of
  the encounter → combat → analysis flow (the manual script used during development is a template).
