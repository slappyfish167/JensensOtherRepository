# Architecture

The PF2e Combat Engine runs Pathfinder Second Edition encounters the way a chess
engine runs positions: a **deterministic rules core** holds the position, humans
(or an opt-in seeded roller) supply dice, and an **optional analysis layer**
evaluates the position and searches for strong moves.

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
  frontend ──HTTP──▶ pf2e-server ──▶ pf2e-ai ──▶ pf2e-core ◀── pf2e-store
                          │                         ▲
                          └─────────────────────────┘
```

* `pf2e-core` depends on nothing but `serde`/`thiserror`. It cannot do I/O, read a clock, or roll dice.
* `pf2e-ai` depends only on `pf2e-core`. It contains **no rules**: it asks the engine for legal
  actions and simulates by applying commands to cloned states. Deleting the crate leaves a fully
  working rules engine and API (minus `/analysis/*`).
* `pf2e-store` depends on `pf2e-core` for the types it persists.
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
crates/pf2e-core/          rules engine (models, builder, combat)
crates/pf2e-ai/            evaluation, search, simulation, queries
crates/pf2e-store/         Store trait, SQLite impl, migrations/
crates/pf2e-server/        axum API, seeding, integration tests
content/                   ancestries, classes, feats, items, spells, monsters, sample PCs & encounter
frontend/                  React + TypeScript (Vite)
docs/                      this documentation
```
