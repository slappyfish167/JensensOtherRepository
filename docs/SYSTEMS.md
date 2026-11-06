# Game systems

The engine runs more than one tabletop RPG. Each **game system** is a pure rules engine plus a
thin adapter; everything else (storage, HTTP API, analysis, the combat UI) is written once against
two traits and works for every system.

| System | Id | Rules engine | Adapter | UI |
|---|---|---|---|---|
| Pathfinder Second Edition | `pf2e` | `crates/pf2e-core` | `crates/pf2e-ai` (`Pf2e`) | dedicated pages (builder, monsters, encounters, combat) |
| Cyberpunk RED | `cpred` | `crates/cpred-core` | `crates/cpred-ai` (`CyberpunkRed`) | roster, encounters, system-neutral combat page |

## Layers

```
                      ┌──────────── tactics-core ────────────┐
                      │ CombatantId · Side · Pos · DiceExpr  │
                      │ SeededDice · GameSystem · views      │
                      └──────────────────────────────────────┘
                           ▲                ▲            ▲
          ┌────────────────┘                │            └──────────────┐
   pf2e-core (rules)                cpred-core (rules)            tactics-ai (analysis)
          ▲                                 ▲              evaluation · search · report
   pf2e-ai (adapter) ──── implements ──── cpred-ai (adapter) ──▶ TacticalSystem
          ▲                                 ▲
          └──────────── pf2e-server (axum) ─┘   one Session<S> per combat, dispatched by system id
                               ▲
                        frontend (React)        PF2e pages · neutral combat page · document pages
```

* **Rules engines** (`*-core`) depend only on `tactics-core`. They are pure and deterministic:
  commands in, events out, humans supply dice, `initial state + command log` replays exactly.
* **`tactics-ai`** contains no rules. It reads rules-derived quantities through
  `TacticalSystem` and simulates only through `GameSystem::apply` on cloned states.
* **Adapters** (`pf2e-ai`, `cpred-ai`) implement both traits for a marker type. They hold the
  system's expected-value helpers, move generation and Monte Carlo policy — all built on the
  core's own rule functions.

## The two contracts

### `tactics_core::GameSystem` — for drivers and UIs

| Method | Purpose |
|---|---|
| `apply` / `apply_in_place` | atomic command application / fast path for search |
| `pending`, `current_actor`, `phase`, `round`, `legal_actions` | what the engine waits for |
| `start_combat`, `act`, `end_turn`, `answer_roll`, `decide` | build commands without knowing their JSON |
| `auto_answer` | answer the pending roll with seeded dice (opt-in auto-roll, Monte Carlo) |
| `table`, `prompt`, `action_views`, `describe_action`, `describe_event` | system-neutral read models |
| `fingerprint`, `replay` | determinism checks (defaults provided) |

The neutral views are what make one UI work for every system:

* `TableView` — map (rows + legend + square size), units (HP, statuses, stat lines), initiative,
  what the actor has left this turn (`◆◆◇`, `Move ✓ · Action ✓`).
* `PromptView` — the roll or decision the engine waits for, and **how to enter it**:
  `die` (a natural face), `total`, or `faces` (every die, when individual faces matter — Cyberpunk
  RED's critical injuries need to know how many sixes were rolled).
* `ActionView` — each legal action as opaque JSON plus a description, a group and, for movement,
  its destination square.

### `tactics_ai::TacticalSystem` — for the analysis layer

| Method | Used by |
|---|---|
| `units`, `hp`, `is_down`, `status_labels` | evaluation, target ranking, the tactical report's outcome metrics |
| `node` | search: is the engine waiting for a chance roll (weighted representative answers), a decision (with its decider), or the actor? |
| `turn_open`, `may_stop` | search: when the turn is over, and whether what remains is optional |
| `candidates`, `action_target`, `action_kind`, `plan_notes` | search: tactically distinct actions, and how plans are described |
| `effective_hp`, `round_damage`, `engagement`, `action_factor`, `tempo_fraction`, `winner`, `counterfactuals` | the Lanchester evaluator and its factor breakdown |
| `kill_probability`, `hypothetical_turn`, `scripted_action`, `turn_budget` | target ranking, side strategy, Monte Carlo |

Everything in `tactics-ai` — the −10…+10 evaluation, the committed expectiminimax turn search
(never "no move improves the position"), the tactical report (primary and alternate plans,
expected damage, expected status effects, priority target, chance to down it, position), target
ranking, side strategy and TPK simulation — is generic over this trait.

## Storage and API

* Every combat row records its `system`; its initial state and commands are that system's JSON.
* PF2e characters, monsters and encounters keep their dedicated tables. Other systems store actors
  and encounters in the generic `documents` table (`system, kind, id`), so a new system needs no
  migration.
* All `/api/combats/...` endpoints (commands, auto-roll, undo, replay, analysis) work for any
  system. `GET /api/systems` lists them; `/api/systems/{system}/{actors|encounters}` is the
  document CRUD for document-backed systems. See [API.md](API.md).

## Adding a system

1. Write `crates/<id>-core`: models, a command/event combat state machine with a resolution queue
   (see `pf2e-core` or `cpred-core`), setup from an encounter, replay. Depend only on `tactics-core`.
2. Write `crates/<id>-ai`: a marker type implementing `GameSystem` and `TacticalSystem`. Most
   methods are one-liners onto the core; the real work is `round_damage` (expected damage per
   round), `node` (representative weighted roll answers) and `candidates` (pruned actions).
3. Server: add a variant to `AnySession` and `Snapshot` (and their macros), an encounter loader,
   and an entry in `GET /api/systems`.
4. Frontend: add an entry to `src/systems.ts` (map palette, actor template). The roster, encounter
   builder and combat screen are already system-neutral.
5. Tests: rules tests in the core; in the adapter, check that the committed search plays sensible
   turns and that every representative roll answer is accepted by the engine.
