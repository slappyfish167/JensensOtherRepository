# PF2e Combat Engine

A standalone tactical combat engine for **Pathfinder Second Edition** — it runs encounters the way
Stockfish runs chess positions. The rules core is pure and deterministic, *you* roll the dice, and an
optional analysis layer evaluates the position (−10 … +10) and searches for the best moves.

An illustrative exchange:

```
"Roll 1d20+9 for Longsword attack (Kira Ironhand → Cave Brute, AC 17) — enter the natural d20"
> 14                       → 23 vs AC 17: success
"Roll 1d8+4 for Longsword slashing damage — enter the total"
> 9                        → Cave Brute takes 9 damage → 46 HP          eval +1.8 (slight player edge)
```

## Features

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
* **Tactical analysis** — evaluation bar with factor breakdown, best 3-action turn (expectiminimax),
  strongest target, best spell, monster strategy, Monte Carlo TPK probability.

## Quick start

Requirements: Rust 1.80+, Node 18+.

```bash
cd frontend && npm install && npm run build && cd ..
```

```bash
cargo run --release -p pf2e-server
```

Open <http://127.0.0.1:8080>. An empty database is seeded with four sample level-1 PCs, eight sample
monsters/hazards and the *Crypt Ambush* encounter: **Encounters → Crypt Ambush → Save & start combat →
Start combat**.

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
| [docs/DATABASE.md](docs/DATABASE.md) | Schema, event-sourced combats, PostgreSQL notes |
| [docs/API.md](docs/API.md) | REST endpoints, commands, pending-input format, examples |
| [docs/ENGINE.md](docs/ENGINE.md) | Data models, resolution queue, rules algorithms, builder, example code, coverage |
| [docs/AI.md](docs/AI.md) | Evaluation model, expectiminimax search, queries, Monte Carlo |
| [docs/ROADMAP.md](docs/ROADMAP.md) | What comes next, suggested tests |

## Repository layout

```
crates/pf2e-core     rules engine — models, builder, combat state machine (no I/O, no RNG)
crates/pf2e-ai       optional analysis — evaluation, search, simulation (no rules)
crates/pf2e-store    persistence — Store trait + SQLite (Postgres-compatible schema)
crates/pf2e-server   REST API (axum), seeding, end-to-end tests
content/             rules data (JSON), sample characters and encounter
frontend/            React + TypeScript UI (Vite)
docs/                design documentation
```

## Content and licensing

Rules content in `content/` is a small sample of Pathfinder Second Edition (Remaster) game mechanics,
which Paizo publishes under the ORC licence; descriptions are paraphrased. The sample monsters and
hazards are original creations built with the creature-building guidelines. Class progressions were
entered by hand and should be verified against the published tables before relying on them; they are
plain JSON and easy to correct. This project is not affiliated with or endorsed by Paizo Inc.
