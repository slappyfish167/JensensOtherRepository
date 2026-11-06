# Architecture

The Tabletop Combat Engine runs tabletop RPG encounters the way a chess engine runs positions: a
**deterministic rules core** per game system holds the position, humans (or an opt-in seeded
roller) supply dice, and an **optional, system-neutral analysis layer** evaluates the position and
searches for strong moves.

Two game systems ship today — Pathfinder 2e and Cyberpunk RED. They plug into the same store,
server, analysis and UI through the `GameSystem` and `TacticalSystem` traits; see
[SYSTEMS.md](SYSTEMS.md) for the contracts and for adding a system. The diagram below shows the
PF2e path; Cyberpunk RED follows the same shape (`cpred-core` rules, `cpred-ai` adapter).

## 1. System diagram

```mermaid
flowchart LR
    subgraph Browser["Frontend — React + TypeScript (frontend/)"]
        UI_Char[Character builder]
        UI_Mon[Monster library]
        UI_Enc[Encounter builder]
        UI_Combat[Combat screen<br/>roll prompts · grid · eval bar]
    end

    subgraph Server["pf2e-server — axum REST API"]
        Routes[routes.rs<br/>thin HTTP handlers]
        AppSvc[app.rs<br/>sessions · encounter→combat · auto-roll]
    end

    subgraph Core["pf2e-core — pure rules (no I/O, no RNG)"]
        Builder[builder<br/>validate + derive stat block]
        Models[models<br/>Character · Monster · Spell · Feat · Weapon · Encounter · Battlefield]
        Engine[combat<br/>CombatState.apply(Command) → Events]
        Rules[rules primitives<br/>dice · checks · modifiers · conditions · damage · grid]
    end

    subgraph AI["pf2e-ai — optional"]
        Eval[evaluation<br/>−10…+10]
        Search[search<br/>expectiminimax]
        MC[simulate<br/>Monte Carlo]
        Q[queries<br/>targets · spells · strategy]
    end

    subgraph Store["pf2e-store"]
        Repo[Store trait]
        SQLite[(SQLite<br/>Postgres-compatible schema)]
    end

    Content[(content/*.json<br/>rules data)]

    Browser -- JSON over HTTP --> Routes
    Routes --> AppSvc
    AppSvc --> Engine
    AppSvc --> Builder
    AppSvc --> Repo --> SQLite
    Routes --> AI
    AI -- "legal_actions() · apply() on clones" --> Engine
    Engine --> Rules
    Builder --> Models
    Content --> Builder
```

ASCII version of the dependency rule:

```
  frontend ──HTTP──▶ pf2e-server ──▶ pf2e-ai  ──▶ pf2e-core  ──┐
                          │      └─▶ cpred-ai ──▶ cpred-core ──┼──▶ tactics-core
                          │          (both implement            │
                          │           tactics-ai's trait) ──────┘
                          └──▶ pf2e-store (system-tagged JSON combat logs + documents)
```

* `tactics-core` holds the shared primitives (ids, sides, positions, dice, seeded RNG), the
  `GameSystem` trait and the neutral view models. It depends only on `serde`/`thiserror`.
* Each rules core (`pf2e-core`, `cpred-core`) depends only on `tactics-core`. It cannot do I/O,
  read a clock, or roll dice.
* `tactics-ai` depends only on `tactics-core`. It contains **no rules**: it asks the engine for
  legal actions through `TacticalSystem` and simulates by applying commands to cloned states.
* The adapters (`pf2e-ai`, `cpred-ai`) implement `GameSystem` and `TacticalSystem` for their core.
  Deleting them leaves fully working rules engines.
* `pf2e-store` stores PF2e aggregates in dedicated tables, everything else as system-tagged JSON.
* `pf2e-server` wires everything together and is the only crate with async, HTTP or environment access.

## 2. The central loop: commands in, events out

```
           ┌─────────────────────────── CombatState ─────────────────────────────┐
Command ──▶│ validate (pure) ─▶ mutate ─▶ push Steps ─▶ run automatic Steps ──┐   │──▶ Vec<Event>
           │                                       ▲                         │   │
           │                                       └── stop at a Step that ──┘   │
           │                                           needs a die / decision    │
           └─────────────────────────────────────────────────────────────────────┘
                                         │
                                 pending() ─▶ "Roll 1d20+14 for Longsword attack (Kira → Ogre, AC 18)"
```

* **Commands** (`StartCombat`, `Act`, `ProvideRoll`, `Decide`, `EndTurn`, `Gm`) are the only inputs.
* Each rules procedure is a sequence of **Steps** in a resolution queue inside the state. A Strike is
  `[ConcealmentCheck?] → AttackRoll → DamageRolls → (Shield Block decision?) → DeliverDamage`.
* Steps that need a human stop the queue. `pending()` turns the front step into a `RollRequest`
  (with expression, DC and modifier breakdown) or a `DecisionRequest`.
* **Events** are derived output for UIs and logs; they are never stored.
* `apply` is atomic (it mutates a clone and swaps on success), so an illegal command never corrupts state.

## 3. Determinism, serialization and replay

Every combat is reproducible from **initial state + ordered command log**:

* The `CombatState` contains everything the rules need, including copies of the spell
  definitions in play, so later content edits cannot change an old combat.
* No hash-map iteration order leaks into results (ordered collections, explicit tie-breaks, a
  deterministic Dijkstra).
* Dice are inputs. Seeded auto-rolls are produced *outside* the engine (`pf2e_core::rng`, used only by
  the server's `/auto-roll` and by the Monte Carlo simulator) and enter the log as ordinary
  `ProvideRoll` commands.
* The database stores the initial state once and appends commands. Undo = drop the last command and
  replay. `GET /api/combats/{id}/replay` re-derives the state and compares fingerprints.

## 4. Human-controlled dice

The engine never rolls. For d20 checks the user enters the **natural die** (so natural 1/20 rules
work); for damage the **total**. The engine validates the value (1–20; or within the dice's min–max),
computes the total and degree of success, doubles crits itself, applies IWR and everything else.
An `extra_modifier` lets the GM add an unmodelled situational bonus; it is logged, so replays stay exact.

Seeded dice must be explicitly enabled per combat (`dice: {mode: "seeded", seed}`) and are only used
when the user presses **Auto** for a specific roll.

## 5. Key design decisions and trade-offs

| Decision | Why | Cost / mitigation |
|---|---|---|
| **Rust** backend (vs C#) | Single static binary, strong enum/ADT modelling of rules, fast enough to search ~10⁵ engine nodes/s in release. | Smaller ecosystem for web admin tooling; mitigated by keeping the API plain JSON/REST. |
| **Event sourcing** (initial state + commands) | Replay, undo, audit, and bug reproduction are free. | Loading a long combat replays every command (sub-millisecond per command); an in-memory session cache avoids repeated replays. Snapshots can be added later without schema changes. |
| **Resolution queue of `Step`s** inside the state (vs callbacks/coroutines) | Serializable mid-action: a combat can stop waiting for a die, be saved, restarted, and resumed. Reactions and disruption are just queue edits. | Multi-step rules are written as small state transitions rather than straight-line code. |
| **Unified `Statblock`** for PCs, monsters and hazards | The combat engine has one creature model; characters are *derived* by the builder. | Builder must faithfully compute derived numbers (covered by tests); GM can still import raw stat blocks. |
| **Rules content as data** (`content/*.json`) | Content can be corrected or extended without code changes; JSON import/export everywhere. | Only behaviours the engine understands (feat effects, spell outcomes) can be expressed; new mechanics need code. |
| **JSON documents + indexed columns** in SQL | Aggregates evolve quickly; schema stays small and portable to PostgreSQL (`TEXT`→`JSONB`). | Cross-document queries need the scalar columns we chose to index. |
| **AI uses the real engine** for simulation | The search can never disagree with the rules; no duplicated logic. | Each node clones a state (a few KB). A node budget and beam keep searches bounded. |
| **Heuristic evaluation** (Lanchester square law) rather than learned | Explainable factors; works for any creature without training data. | Approximate; the `Evaluator` trait allows a learned evaluator to be plugged in. |

## 6. Repository layout

```
Cargo.toml                 workspace
crates/tactics-core/       shared primitives, GameSystem trait, neutral views
crates/tactics-ai/         system-neutral evaluation, search, report, simulation
crates/pf2e-core/          PF2e rules engine (models, builder, combat)
crates/pf2e-ai/            PF2e adapter, EV helpers, best-spell query
crates/cpred-core/         Cyberpunk RED rules engine (models, combat)
crates/cpred-ai/           Cyberpunk RED adapter, EV helpers
crates/pf2e-store/         Store trait, SQLite impl, migrations/
crates/pf2e-server/        axum API, seeding, integration tests
content/                   PF2e rules data and samples; content/cpred/ for Cyberpunk RED
frontend/                  React + TypeScript (Vite)
docs/                      this documentation
```
