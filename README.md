# Tabletop Combat Engine

A standalone tactical combat engine for tabletop RPGs — it runs encounters the way Stockfish runs
chess positions. It supports **multiple game systems**: **Pathfinder Second Edition** and
**Cyberpunk RED** today, with a documented contract for adding more ([docs/SYSTEMS.md](docs/SYSTEMS.md)).
Each rules core is pure and deterministic, *you* roll the dice, and a system-neutral analysis layer
evaluates the position (−10 … +10) and plays the hostile side to win.

An illustrative exchange:

```
"Roll 1d20+9 for Longsword attack (Kira Ironhand → Cave Brute, AC 17) — enter the natural d20"
> 14                       → 23 vs AC 17: success
"Roll 1d8+4 for Longsword slashing damage — enter the total"
> 9                        → Cave Brute takes 9 damage → 46 HP          eval +1.8 (slight player edge)
```

## Features

* **Multiple game systems** — every system plugs into the same store, API, analysis layer and
  combat screen through two traits (`GameSystem`, `TacticalSystem`); pick the system in the header.
* **Cyberpunk RED** — REF + 1d10 initiative, Move Action + Action, ROF, aimed (head) shots,
  autofire, melee vs Evasion, ranged DV by range band, dodging, armor SP with ablation and
  half-SP melee, headshot doubling, critical injuries on two sixes, seriously/mortally wounded,
  death saves, ammunition and reloading, destructible cover. Rules tables are data
  (`content/cpred/`). See [docs/CYBERPUNK_RED.md](docs/CYBERPUNK_RED.md) for the rules as
  implemented and the points to check against your rulebook.

Pathfinder 2e:

* **Character builder** — ancestries, backgrounds, classes, archetype dedications, feats with
  prerequisites, skills, equipment with runes, spells; exhaustive PF2e legality checks; derived stat
  block; JSON import/export.
* **Monster database** — store and edit monsters and hazards, import stat blocks, elite/weak templates.
* **Encounter builder** — place PCs, monsters and hazards on a painted grid; live XP budget and threat.
* **Combat engine** — initiative, three-action economy, MAP, 25 conditions with durations,
  persistent damage, dying/wounded/doomed, Reactive Strike (with spell disruption), Shield Block,
  cover, concealment, flanking, spells (attacks, basic and non-basic saves, areas, heightening),
  simple and complex hazards, GM overrides.
* **Human-controlled dice** — the engine prompts, you roll and type the result. Seeded auto-rolls are
  available only if you enable them for a combat, and are recorded like any other roll.
* **Reproducible** — every combat is `initial state + command log`; undo and replay verification are built in.
* **Tactical analysis** — evaluation bar with factor breakdown, best 3-action turn (expectiminimax)
  that always commits to a concrete turn, with an alternate plan, expected damage, expected status
  effects, priority target, chance to down it and resulting position,
  strongest target, best spell, monster strategy, Monte Carlo TPK probability.

## Foundry VTT module ("PF2e Stockfish")

The same engine runs inside **Foundry VTT v13 + PF2e** as a module (`foundry/`): an evaluation bar,
**Analyze Best Turn**, a GM **monster assistant**, a Monte Carlo **threat forecast** and
**explain position**, computed locally in a WebAssembly worker from the live combat (Foundry stays
the source of truth). Best turn ≈ 100 ms, monster plan ≈ 0.2 s, 200-run forecast < 1 s in the
browser. See [docs/FOUNDRY.md](docs/FOUNDRY.md).

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
scripts/build-wasm.sh
cd foundry && npm ci && npm run build   # → foundry/dist/pf2e-stockfish (install folder)
```

## Quick start

Requirements: Rust 1.80+, Node 18+.

```bash
cd frontend && npm install && npm run build && cd ..
```

```bash
cargo run --release -p pf2e-server
```

Open <http://127.0.0.1:8080>. An empty database is seeded with sample content for each system:

* **Pathfinder 2e**: four level-1 PCs, eight monsters/hazards and the *Crypt Ambush* encounter —
  **Encounters → Crypt Ambush → Save & start combat → Start combat**.
* **Cyberpunk RED** (pick it in the header): a crew of edgerunners, several NPCs and the
  *Warehouse Standoff* encounter — **Encounters → Warehouse Standoff → Save & start combat**.

For frontend development with hot reload run the server, then:

```bash
cd frontend && npm run dev
```

(Vite serves on :5173 and proxies `/api` to :8080.)

Configuration: `PF2E_DB` (SQLite path, default `pf2e.sqlite`, `:memory:` for ephemeral),
`PF2E_ADDR` (default `127.0.0.1:8080`), `PF2E_STATIC_DIR` (built UI, default `frontend/dist`),
`PF2E_NO_SEED`.

## Tests

```bash
cargo test --workspace
```

```bash
cd frontend && npm run typecheck && npm test
```

```bash
scripts/build-wasm.sh && cd foundry && npm run typecheck && npm test && npm run build && npm run smoke
```

The Rust suite (≈70 tests) covers dice, degrees of success, modifier stacking, IWR, conditions, grid
geometry (diagonals, reach, cover, flanking, bursts), the builder (sample PCs are legal; illegal
boosts/feats/spells are caught), the combat engine (MAP, crits with deadly, dying and recovery,
Reactive Strike, Shield Block, persistent damage, concealment, spells, hazards, slowed/stunned,
atomicity of illegal commands, exact replay), the AI (antisymmetric evaluation, the search finishes
wounded foes and closes distance, Monte Carlo reproducibility), storage, and the HTTP API end to end.

## Documentation

| Document | Contents |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System diagram, layering rules, command/event loop, determinism, design trade-offs |
| [docs/SYSTEMS.md](docs/SYSTEMS.md) | Multiple game systems: the `GameSystem` / `TacticalSystem` contracts, adding a system |
| [docs/CYBERPUNK_RED.md](docs/CYBERPUNK_RED.md) | Cyberpunk RED rules as implemented, assumptions to verify, content format |
| [docs/DATABASE.md](docs/DATABASE.md) | Schema, event-sourced combats, PostgreSQL notes |
| [docs/API.md](docs/API.md) | REST endpoints, commands, pending-input format, examples |
| [docs/ENGINE.md](docs/ENGINE.md) | Data models, resolution queue, rules algorithms, builder, example code, coverage |
| [docs/AI.md](docs/AI.md) | Evaluation model, expectiminimax search, queries, Monte Carlo |
| [docs/ROADMAP.md](docs/ROADMAP.md) | What comes next, suggested tests |
| [docs/FOUNDRY.md](docs/FOUNDRY.md) | Foundry VTT module: architecture, hooks, PF2e integration, WASM, search, UX, security, roadmap |

## Repository layout

```
crates/tactics-core  system-neutral primitives, GameSystem trait, neutral views
crates/tactics-ai    system-neutral analysis — evaluation, committed search, report, Monte Carlo (no rules)
crates/pf2e-core     PF2e rules engine — models, builder, combat state machine (no I/O, no RNG)
crates/pf2e-ai       PF2e adapter (GameSystem + TacticalSystem), EV helpers, best-spell query
crates/cpred-core    Cyberpunk RED rules engine — models, combat state machine (no I/O, no RNG)
crates/cpred-ai      Cyberpunk RED adapter (GameSystem + TacticalSystem), EV helpers
crates/pf2e-store    persistence — Store trait + SQLite (Postgres-compatible schema)
crates/pf2e-server   REST API (axum), per-system sessions, seeding, end-to-end tests
crates/pf2e-bridge   Foundry snapshot → CombatState, token-addressed reports (pure, JSON API)
crates/pf2e-wasm     WebAssembly build of bridge + AI + core (wasm-bindgen)
content/             rules data (JSON) and samples; content/cpred/ for Cyberpunk RED
frontend/            React + TypeScript UI (Vite)
foundry/             Foundry VTT module (TypeScript, Vite)
scripts/             build-wasm.sh
docs/                design documentation
```

## Content and licensing

Rules content in `content/` is a small sample of Pathfinder Second Edition (Remaster) game mechanics,
which Paizo publishes under the ORC licence; descriptions are paraphrased. The sample monsters and
hazards are original creations built with the creature-building guidelines. Class progressions were
entered by hand and should be verified against the published tables before relying on them; they are
plain JSON and easy to correct. This project is not affiliated with or endorsed by Paizo Inc.
