# PF2e Stockfish for Foundry VTT

The engine in `crates/` running inside Foundry VTT as a module: an evaluation bar, best-turn search,
a GM monster assistant, a Monte Carlo threat forecast and position explanations — computed locally
in a WebAssembly worker from the live Foundry combat.

Status: implemented and tested outside Foundry (Rust unit tests, TypeScript tests against the real
WASM engine, and a headless-Chromium test of the built module bundle). **Not yet exercised inside a
live Foundry v13 + PF2e 7.x world** — see [§12 Migration](#12-migration-steps) for the verification
checklist; the PF2e document paths in `foundry/src/extract/` are the part most likely to need
adjustment.

---

## 1. Architecture

```
 Foundry VTT client (browser / Electron)
 ┌──────────────────────────────────────────────────────────────────────────────┐
 │ Foundry documents (source of truth)                                          │
 │   Scene · Token · Actor (PF2e) · Item (conditions, spells, gear) · Combat    │
 │        │ hooks (debounced)                                                   │
 │        ▼                                                                     │
 │ extract/  ── CombatSnapshot (JSON, viewer-filtered) ──┐                      │
 │ tracking/ ── actions spent / MAP / reactions (combat flag, GM-written)       │
 │                                                       │ postMessage          │
 │ analysis/service ── engine/client ────────────────────┼────────────┐         │
 │        ▲                                              ▼            ▼         │
 │ ui/ (eval bar, windows, overlay,          ┌───────────────┐ ┌───────────────┐│
 │      tracker panel, HUD, controls)        │ primary worker│ │forecast worker││
 │                                           │  pf2e-wasm    │ │  pf2e-wasm ×N ││
 │ socket: GM → players (score only)         │   └ bridge    │ │ (Monte Carlo  ││
 │                                           │     └ ai/core │ │  slices)      ││
 │                                           └───────────────┘ └───────────────┘│
 └──────────────────────────────────────────────────────────────────────────────┘
```

| Layer | Location | Knows rules? | Runs where |
|---|---|---|---|
| Rules | `crates/pf2e-core` | **yes — the only place** | WASM worker (and native server) |
| Analysis | `crates/pf2e-ai` | no (asks core) | WASM worker |
| Bridge | `crates/pf2e-bridge` | no (data mapping) | WASM worker; tested natively |
| WASM shell | `crates/pf2e-wasm` | no | WASM worker |
| Module | `foundry/` | no | Foundry main thread |

### Architecture decisions

| Decision | Why | Trade-off / mitigation |
|---|---|---|
| **Foundry is the source of truth**; a snapshot is rebuilt from documents on every change | Rule #1: no parallel database; the module can never disagree with the sheet | Rebuilding costs ~ms per creature; per-actor strike-formula cache and JSON dedupe (`analysis/service.ts`) skip unchanged positions |
| **PF2e computes the numbers, the engine computes the combat** | PF2e's totals include feats, runes, effects, rule elements the engine cannot model; the engine models conditions, MAP, cover, flanking, reactions | Condition-derived modifiers are *stripped* from PF2e totals (`extract/modifiers.ts`) and conditions sent separately, so nothing counts twice |
| **Conversion in Rust, not TypeScript** (`pf2e-bridge`) | Formula parsing, wall rasterization, spell translation and turn state are testable natively and identical in every client | TS only reads documents; the snapshot is the single schema |
| **JSON strings across the WASM boundary** | One schema (the TS interfaces), no generated glue types; parsing a 30 KB snapshot costs < 1 ms | Slightly more CPU than `serde-wasm-bindgen`; irrelevant next to search time |
| **Engine in a Web Worker** | Search never blocks the canvas | A trapped (panicked) WASM instance cannot be reset in place → the worker is replaced and the snapshot replayed |
| **Time budgets, not depth limits**, for interactive search | Users feel milliseconds, not plies; iterative deepening makes any stop point return a complete answer | Slow machines get shallower (flagged) answers rather than late ones |
| **Thin walls added to the core battlefield** (`Battlefield::edges`) | Foundry walls lie *between* squares; snapping them onto squares closes 1-square corridors | Core change; serde-default and skipped when empty, so stored combats and fingerprints are unchanged |
| **Shared immutable data via `Arc`** (stat blocks, spells, map) | Search clones the state per node; this was 44% of search time | `Arc` serializes exactly like the inner value; mutation sites use `Arc::make_mut` |
| **GM-authoritative evaluation for players** | The evaluation encodes hidden information (unseen monsters, exact HP) | Players see only the GM's score, never factors; GM-only features stay GM-only |

---

## 2. Module structure

Source (`foundry/`):

```
module.json                 manifest: requires system pf2e ≥ 7.0, Foundry 13, socket: true
package.json, vite.config.ts, tsconfig.json, test/tsconfig.json
src/
  main.ts                   entry: registers hooks
  constants.ts              module id, socket name, template paths, public hook name
  settings.ts               world policy + client preferences
  permissions.ts            pure permission policy (unit-tested)
  runtime.ts                owns engine, service, UI, tracker; hook handlers
  hooks/index.ts            every Foundry hook the module uses
  api/public-api.ts         game.modules.get('pf2e-stockfish').api
  protocol/snapshot.ts      CombatSnapshot (mirror of pf2e-bridge/src/snapshot.rs)
  protocol/report.ts        reports (mirror of pf2e-bridge/src/report.rs)
  protocol/rpc.ts           main thread ⇄ worker messages
  extract/                  Foundry/PF2e documents → snapshot
    modifiers.ts            strip engine-applied modifiers from PF2e totals
    actor.ts strikes.ts spells.ts conditions.ts scene.ts snapshot.ts
  tracking/turn-tracker.ts  actions spent, attacks made, reactions used (pure reducer)
  engine/worker.ts          the worker: loads WASM, serves requests, self-destructs on trap
  engine/dispatch.ts        request → Engine method (shared with tests)
  engine/client.ts          promise RPC, timeouts, restart + snapshot replay, forecast split
  analysis/service.ts       debounce, dedupe, coalesced refresh, on-demand analyses
  ui/                       eval-bar, overlay (canvas), controller, view-models, format,
                            integrations (tracker, HUD, controls, menus, sheets, keys),
                            apps/report-app.ts (ApplicationV2 windows)
  types/foundry.d.ts        narrow ambient declarations of the Foundry globals used
  types/pf2e.ts             the PF2e document fields the extractor reads
templates/                  best-turn, gm-assistant, forecast, explain (.hbs)
styles/stockfish.css
lang/en.json
scripts/assemble.mjs        copies static files, validates the output folder
scripts/package.mjs         release zip + manifest
scripts/browser-smoke.mjs   built bundle in headless Chromium
test/                       vitest suites + Rust-generated fixture
wasm-pkg/                   (generated) wasm-bindgen output
```

Installed module (`foundry/dist/pf2e-stockfish/`, produced by `npm run build`):

| Folder | Contents |
|---|---|
| `module.json` | manifest |
| `scripts/` | `main.js` — the ES module Foundry loads (44 KB) |
| `workers/` | `engine-worker.js` — the analysis worker (6 KB + WASM glue) |
| `wasm/` | `pf2e_stockfish_bg.wasm` — engine + analysis + bundled content (1.6 MB) |
| `templates/` | Handlebars templates for the windows |
| `styles/` | `stockfish.css` |
| `lang/` | localization |

There is no `api/`, `hooks/` or `analysis/` folder at runtime: they are bundled into `main.js`.

---

## 3. Foundry hooks

All registrations are in `foundry/src/hooks/index.ts`.

| Hook | Purpose |
|---|---|
| `init` | register settings and keybindings, preload templates |
| `setup` | publish the public API |
| `ready` | refuse non-PF2e worlds; start the worker; check engine/snapshot version; mount the eval bar; socket listener; first analysis |
| `canvasReady` | scene switched → re-analyse |
| `createCombat`, `deleteCombat`, `combatStart`, `combatTurn`, `combatRound` | re-analyse |
| `updateCombat` | on `turn`/`round` change the active GM resets the turn tracker; re-analyse |
| `createCombatant`, `updateCombatant`, `deleteCombatant` | re-analyse |
| `createToken`, `deleteToken` | re-analyse |
| `updateToken` | moves of the current combatant count as a Stride (setting); re-analyse |
| `updateActor` | drop the actor's cached strike formulas; re-analyse |
| `createItem`, `updateItem`, `deleteItem`, `create/update/deleteActiveEffect` | conditions, effects and gear are embedded items: drop cache, re-analyse |
| `createWall`, `updateWall`, `deleteWall`, `create/update/deleteRegion` | geometry changed → re-analyse |
| `createChatMessage` | active GM classifies Strike/spell/action/reaction messages for the tracker |
| `renderCombatTracker` | mini eval bar, buttons, action pips with GM ± controls |
| `renderTokenHUD` | "Analyze Best Turn" button on the token right-click HUD |
| `getSceneControlButtons` | "Tactical analysis" control group (v13 record form) |
| `getCombatantContextOptions` (v13), `getCombatTrackerEntryContext` (v12 name) | combatant context-menu entries |
| `getActorSheetHeaderButtons` (AppV1 sheets), `getHeaderControlsActorSheetV2` (AppV2) | sheet header button |
| `pf2e-stockfish.evaluation` (*emitted*) | other modules can react to every new evaluation |

`controlToken` is deliberately unused: the controlled token is read when a command runs
(`Controller.resolveToken`), which avoids re-analysing on every click.

Re-analysis is debounced (150 ms) because one Strike fires chat, actor and item hooks within
milliseconds; identical snapshots never reach the engine (`AnalysisService.refreshOnce`).

---

## 4. PF2e system integration

### Data extraction (`foundry/src/extract/`)

| Snapshot field | PF2e source |
|---|---|
| level, size, traits | `actor.level`, `actor.size` (`sm/med/lg/…`), `actor.traits` |
| HP | `actor.hitPoints {value,max,temp}` (fallback `system.attributes.hp`) |
| AC | `actor.getStatistic('ac').dc` (fallback `actor.armorClass`) |
| saves, perception, skills | `actor.saves[k]`, `actor.perception`, `actor.skills[k]` (`Statistic.check.mod`) |
| speed | `system.movement.speeds.land.value` → `system.attributes.speed.total/value` |
| strikes | `actor.system.actions` (type `strike`, ready, visible): `totalModifier`, item traits, `actor.getReach()`, range increment, `await strike.damage({ getFormula: true })` |
| spellcasting | `actor.spellcasting.regular`: category, tradition, `statistic.check.mod`, `statistic.dc.value`; slots from `system.slots.slotN` (spontaneous `value`, prepared unexpended `prepared[]`, innate per-spell `uses`, focus points); spells' `system.time/range/area/target/defense/damage/heightening` |
| IWR | `actor.attributes.immunities/weaknesses/resistances` |
| reactions | items with slug `reactive-strike` / `attack-of-opportunity`, `shield-block` |
| shield | `actor.attributes.shield` (ac, hardness, hp, brokenThreshold, raised) |
| conditions | condition items (`slug`, `value`, `active`); `persistent-damage` → `system.persistent {formula, damageType}` |
| side | player-owned or `character` → party; token disposition otherwise |
| position | token `x/y` minus `scene.dimensions.sceneX/Y`, divided by grid size |
| walls | `scene.walls`: `c`, `move`, `sight`, `door`, `ds` |
| terrain | regions with `modifyMovementCost` behaviors, or the region flag `flags.pf2e-stockfish.terrain` |

### Avoiding double counting

PF2e totals already contain every active modifier. The engine re-applies conditions (frightened,
sickened, clumsy, enfeebled, drained, stupefied, off-guard, prone, unconscious, blinded), cover and
raised shields itself. `ENGINE_APPLIED_SLUGS` lists those modifier slugs; they are subtracted from
PF2e totals. Spell effects, auras and feats the engine cannot model stay in the numbers.
Strike damage *formulas* cannot be un-summed, so when an actor is enfeebled the snapshot marks
`damage_includes_conditions` and the engine skips its own enfeebled damage adjustment.

### Synchronisation

Hooks → debounced `invalidate()` → `buildSnapshot()` → if the JSON changed, `engine.load()` →
`evaluate()` → eval bar / tracker / `pf2e-stockfish.evaluation`. On-demand analyses first force a
refresh, so they always see the current position.

Turn state the PF2e system does not track — actions spent, attacks made (MAP), reactions used —
comes from the turn tracker (`tracking/turn-tracker.ts`): the active GM classifies chat messages
(attack rolls, spell cards by cast time, action items by action type, skill actions such as Trip)
and token moves, and stores the result in a combat flag every client reads. The GM can correct it
with ± buttons in the tracker panel.

### Validation (`pf2e-bridge/src/convert.rs`)

* Schema version, square grid, 5-ft squares; limits (64 combatants, 20 000 walls, 16 strikes and
  200 spells per creature, 8 MiB snapshot) against hostile or corrupt input.
* Duplicate tokens, implausible statistics → `invalid_snapshot`.
* Defeated NPCs are dropped; stacked or walled tokens are moved to the nearest free square with a
  warning; unknown conditions, IWR types, strike riders and spells are reported as warnings and
  ignored, never guessed.
* Every report carries token ids; squares are mapped back to scene coordinates and canvas pixels.

---

## 5. Combat analysis features

| Feature | Who | Entry points | Engine call |
|---|---|---|---|
| **Evaluation bar** (−10 … +10, win %) | GM; players per setting | always on; click → Explain | `evaluate` |
| **Best Turn** (steps, expected value, reasoning, map overlay) | GM; players for owned tokens | Token HUD, tracker, scene control, context menu, sheet header, `Shift+B`, API | `bestTurn(token, 75 ms)` |
| **Monster GM assistant** (per-monster plan, targets, focus fire) | GM | tracker, scene control, context menu, `Shift+M`, API | `sidePlan('enemy', 400 ms/creature)` |
| **Threat forecast** (victory, defeat, TPK, any-PC-death, per-creature odds, margin of error) | GM | tracker, scene control, `Shift+F`, API | `forecastSlice` × workers + `mergeForecast` |
| **Explain position** (ranked reasons with impact, factor deltas) | whoever sees the bar | eval-bar click, tracker, scene control, `Shift+X`, API | `explain` |

Actual Best Turn output for the rogue on the bundled *Crypt Ambush* fixture (WASM engine, Node):

```
Pip Quickfoot 1: Stride to square (7, 7), then Strike Goblin Cutter 2 with Shortbow,
                 then Strike Goblin Cutter 2 with Shortbow — expected +0.4 for its side
Reasoning:
  Goblin Cutter 2 (top priority target): 6.3 expected damage (91% of its 7 HP), 37% to drop it now,
                                         it threatens 5.5 damage/round
  Goblin Cutter 2 lost 4 HP (3 / 7 left) (+0.4)
```

"Explain" measures each reason **counterfactually**: undo exactly one difference (one creature's
HP, its conditions, its square, a spent reaction, flanking), re-evaluate, report the score change
(`crates/pf2e-ai/src/explain.rs`). Reason kinds: hit points, downed/recovered, conditions
gained/lost, flanked, reaction used, actions left, spell slots spent, persistent damage, position —
each with its measured impact in evaluation units.

---

## 6. WebAssembly

```
crates/pf2e-core ─┐
crates/pf2e-ai  ──┼─ crates/pf2e-bridge (pure Rust, JSON API, native tests)
                  └─ crates/pf2e-wasm   (cdylib, wasm-bindgen 0.2.130)
                        │ scripts/build-wasm.sh
                        ▼
                  foundry/wasm-pkg/pf2e_stockfish{.js,_bg.wasm,.d.ts}
                        │ imported by foundry/src/engine/worker.ts, bundled by Vite
                        ▼
                  dist/pf2e-stockfish/{workers/engine-worker.js, wasm/pf2e_stockfish_bg.wasm}
```

* **Cargo**: `crates/pf2e-wasm/Cargo.toml` (`crate-type = ["cdylib","rlib"]`, `wasm-bindgen = "=0.2.130"`);
  workspace `[profile.wasm]` (release + LTO, 1 codegen unit, `opt-level = 3` — latency over size).
* **Build**: `scripts/build-wasm.sh` checks the wasm-bindgen CLI version against the pinned crate,
  builds, runs `wasm-bindgen --target web`, and `wasm-opt -O3` when available.
* **Bindings**: `Engine` class — `load`, `evaluate`, `bestTurn(token, options, deadlineMs)`,
  `sidePlan(side, options, deadlineMs)`, `forecastSlice`, `mergeForecast`, `explain`; free
  functions `version()` and `lastPanic()`. TypeScript wraps them in `engine/client.ts`.
* **Serialization**: JSON strings both ways. Requests ≤ 8 MiB.
* **Errors**: engine errors are thrown as `{"code","message"}` JSON (`ErrorCode` in
  `pf2e-bridge/src/api.rs`); the client turns them into `EngineError` with the same code; the UI
  localizes by code. Host-side codes (`timeout`, `worker_unavailable`, `cancelled`,
  `not_permitted`) share the type.
* **Panics**: a panic hook records the message; the trap surfaces as `WebAssembly.RuntimeError`;
  the worker reports `lastPanic()`, flags `restarted` and closes; the client starts a fresh worker
  and replays the last snapshot transparently.
* **Memory**: one `Engine` per worker holds the converted position (the snapshot is not re-sent per
  analysis). Search caches are per call and freed afterwards; caches are bounded
  (`EvalCache::MAX_ENTRIES`, `MoveGenCache::MAX_ENTRIES`). Linear memory never shrinks but is reused.
* **Time**: deadlines use `performance.now()` through the pure `Budget` trait; the Rust analysis
  code itself stays clock-free and deterministic. A request that exceeds the hard timeout (15 s) is
  answered with `timeout` and the worker is killed — WASM cannot be interrupted otherwise.

---

## 7. State conversion

```
Foundry documents ─ extract/*.ts ─▶ CombatSnapshot ─ convert.rs ─▶ CombatState ─ pf2e-ai ─▶ report.rs ─▶ UI
```

* TypeScript interface: `foundry/src/protocol/snapshot.ts` (`CombatSnapshot`, `SceneSnapshot`,
  `WallSnapshot`, `TerrainCell`, `TurnSnapshot`, `CombatantSnapshot`, `StrikeSnapshot`,
  `SpellcastingSnapshot`, `SpellSnapshot`, `IwrSnapshot`, `ShieldSnapshot`, `ConditionSnapshot`,
  `PersistentSnapshot`, `HazardSnapshot`, `SnapshotOptions`).
* Rust: `crates/pf2e-bridge/src/snapshot.rs`, field-for-field.
* Contract tests in both directions: Rust generates `foundry/test/fixtures/crypt-ambush.snapshot.json`
  from an engine state (`fixtures::snapshot_from_state`) and the TypeScript engine tests consume it;
  the TypeScript extractor's output from PF2e-shaped fake actors is loaded by the real WASM engine
  (`foundry/test/extract.test.ts`). A round trip state → snapshot → state keeps every position,
  HP, action count and spell slot, and the evaluation within 0.05.
* Conversion steps (`snapshot_to_state`): validate → crop the scene to the combatants + margin
  (≤ 200 squares a side) → rasterize walls to blocked edges (an edge is blocked when the segment
  between the two square centres crosses a wall) and terrain → stat blocks (Foundry damage formulas
  parsed by `formula.rs`; spells by library slug, else structured translation, else skipped) →
  `CombatSetup::build` → live state (HP, temp HP, conditions, persistent damage, raised shield via
  the core's own `expiry_for`, spent reactions, remaining slots) → initiative order, round, current
  creature; its start-of-turn processing comes from `CombatState::hypothetical_turn`, then actions
  spent and attacks made are deducted.
* Reports: `EvaluationReport`, `TurnReport` (steps with target tokens, destination and Stride path
  as scene squares + canvas pixels), `SidePlanReport`, `ForecastReport`, `ExplainReport`.

---

## 8. Search improvements

### Implemented (all in `crates/pf2e-ai`)

Measured on *Crypt Ambush*, worst creature, native release build (`cargo run --release -p pf2e-bridge --example bench`):

| Step | Worst best turn | Exact? |
|---|---|---|
| Baseline (expectiminimax, beam 8, 40k-node budget) | 1841 ms | — |
| + pair-damage evaluation cache, transposition table, iterative deepening with history ordering | 363 ms | cache and TT: yes; history ordering changes only which actions survive the beam |
| + legality/movement cache in move generation | 345 ms | yes |
| + O(1) terrain lookup, allocation-free pathfinder | 214 ms | yes (core, same algorithm) |
| + `Arc`-shared stat blocks / spells / map | 112 ms | yes (core, same serialization) |
| + single Dijkstra per Stride, fast grid hasher | **75 ms** | yes |

The per-creature plan values were identical at every step. With a wide beam, plain search,
transpositions and iterative deepening agree to 1e-9
(`search_accelerations_agree_when_the_beam_does_not_bind`).

| Analysis | Target | Native | WASM (Node/V8) | Built bundle, headless Chromium |
|---|---|---|---|---|
| Best turn (worst creature, unlimited) | < 100 ms | 75 ms | 92 ms | — |
| Best turn (75 ms deadline default, end to end) | < 100 ms | — | — | ≈ 100 ms (126 ms with a 100 ms deadline) |
| Monster plan (3 monsters) | < 2 s | 156 ms | 238 ms | 212 ms |
| TPK forecast, 200 runs, 1 worker | < 3 s | 754 ms | 909 ms | 793 ms |

* **Transposition table** — `keys::state_key` hashes everything that determines the future and
  skips request/effect-id counters, so transposed move orders and chance branches that resolve to
  the same state (critical failure vs failure) are searched once. Cost: O(nodes) memory, bounded.
* **Evaluation cache** — expected damage per (attacker, target) keyed by both creatures' combat
  profiles and the *attacker's side* positions (flanking), since hit points and per-turn counters
  never enter those formulas. Most leaves after a Strike hit the cache.
* **Iterative deepening** — depth 1, 2, 3 actions. Overhead ≈ depth-(n−1) cost, small next to
  depth n (branching ≈ 8–20 per action). Benefit: anytime answers under a deadline
  (`best_turn_with_budget`), and a history table that orders the beam by gains seen in shallower
  iterations.
* **Action pruning** — unchanged policy from `movegen` (tactically distinct Strides only, best
  approach square per foe, retreat square for ranged creatures) plus beam truncation below the
  root; now cached per legality key.
* **Split Monte Carlo** — `simulate_range` + `SimulationTally::merge`; run seeds depend only on the
  run index, so N workers produce exactly the single-thread result.

### Proposed next (in order)

| # | Technique | Complexity | Benefit | Notes |
|---|---|---|---|---|
| 1 | **Star1/Star2 pruning** for chance nodes | O(b^d) worst case, typically 30–60% fewer nodes | deeper search in the same budget | evaluation is bounded (−10…+10), the prerequisite for Star pruning in expectimax |
| 2 | **Opponent-reply ply** (one scripted enemy turn after ours) | × (enemy count × cheap policy) | catches "don't end next to the ogre" | reuse `simulate::scripted_action`; budget-split by iterative deepening |
| 3 | **Beam search across creatures** for side plans | O(k · creatures · turn search) | coordinated monster plans (flank, then strike) instead of independent plans | keep the k best joint states after each creature's turn |
| 4 | **Monte Carlo rollouts at leaves** (truncated, 1–2 rounds, seeded) | runs × rounds × creatures per leaf — too slow for every leaf | better evaluation of positions the heuristic misjudges (persistent damage, dying) | use only at the root's top-3 children, in the forecast workers |
| 5 | **MCTS (UCT with progressive widening)** for multi-round plans | anytime; quality grows with simulations | plans over 2–3 rounds for the GM assistant | deterministic seeds keep it reproducible |
| 6 | **Learned evaluator** trained on forecast outcomes | offline training; O(features) per leaf | sharper evaluation, honest win % | `Evaluator` trait already pluggable; keep heuristic as fallback |

---

## 9. User experience

### Views

* **GM**: evaluation bar (left of the canvas, configurable side), combat tracker panel (mini bar,
  Best Turn, Monster Turn, Forecast, Explain, action pips with ± correction), all scene-control
  tools, all hotkeys, context-menu entries, windows with "Show on map" overlays.
* **Player**: Best Turn for owned tokens (from Token HUD, tracker, scene control, `Shift+B`) built
  only from tokens they can see; the evaluation bar only if the GM allows it, fed by the GM's score.

### Permissions (`foundry/src/permissions.ts`)

| Capability | GM | Player |
|---|---|---|
| Evaluation bar | ✓ (unless "Nobody") | only if visibility = Everyone; GM's score when GM-authoritative |
| Explain | ✓ | only when computing locally (visibility = Everyone, not authoritative) |
| Best Turn | any token | owned tokens, if "Players may analyze their own turns" |
| Monster assistant, forecast | ✓ | ✗ |
| Turn tracker corrections | ✓ | ✗ (read-only pips) |

### Settings

World: evaluation bar visibility (GM / Everyone / Nobody), players may analyze their own turns,
GM-authoritative evaluation, forecast simulations (300), count token moves as Stride, flanking,
automatic cover, analysis margin. Client: Best Turn time limit (75 ms), GM assistant time per
creature (400 ms), forecast workers (cores − 1, ≤ 4), analyze automatically, bar side.

### Scene controls, HUD, tracker, menus, hotkeys

* Scene controls → **Tactical analysis**: Evaluation bar (toggle), Analyze Best Turn,
  Suggest Monster Turn (GM), Threat forecast (GM), Explain position.
* Token right-click HUD → knight button: Analyze Best Turn for that token.
* Combat tracker → panel under the header; combatant context menu → Analyze Best Turn,
  Suggest Monster Turn (GM).
* Actor sheet header → Best Turn (first active token of the actor).
* Hotkeys (rebindable): `Shift+B` best turn, `Shift+M` monster turn, `Shift+F` forecast,
  `Shift+X` explain, `Shift+E` toggle bar.

### Public API

```js
const sf = game.modules.get('pf2e-stockfish').api;
await sf.bestTurn();                       // controlled or current token, opens the window
const plan = await sf.bestTurn(id, { show: false });
const ev = await sf.evaluate();
Hooks.on('pf2e-stockfish.evaluation', (ev) => console.log(ev?.score));
```

---

## 10. Roadmap

### 1.0 — evaluation, best turn, GM suggestions *(implemented; needs live-world verification)*

* Evaluation bar, Best Turn, Monster assistant, Threat forecast, Explain position.
* Turn tracker, permissions, settings, localization, public API.
* CI: Rust, WASM build, TypeScript tests against the WASM engine, browser smoke test, release zip.
* Before release: the [§12](#12-migration-steps) live-world checklist; a second language;
  screenshots for the package listing.

### 1.5

* **Encounter balancing**: run the forecast on a prepared (not started) encounter from the
  Encounter Builder / combat tracker, with "add/remove monster" what-ifs; report XP budget
  (`pf2e_core::models::encounter::encounter_budget`) next to simulated difficulty.
* **Monster tactics learning**: record GM-accepted vs ignored suggestions and simulated outcomes
  per creature type (world setting, local only); tune move-generation weights per creature.
* **Replay viewer**: persist the per-turn snapshots of a combat (compressed, in a combat flag or a
  journal page) and step through them with the evaluation graph ("where the fight turned").
* Opponent-reply ply and Star pruning (§8 #1–2); coordinated monster beam (§8 #3).
* More engine actions through PF2e data (Grapple/Shove/Reposition, Aid, Seek/Hide, activities
  such as Sudden Charge) — `docs/ROADMAP.md` Phase 2.

### 2.0

* **Campaign analytics**: per-character damage dealt/taken, accuracy vs expectation, "luck index"
  (actual vs expected rolls), across sessions.
* **Combat database**: snapshots + command logs from Foundry combats stored in `pf2e-store`
  (already event-sourced and PostgreSQL-ready), opt-in export.
* **Pattern recognition**: tag recurring tactical motifs (focus fire, flanking pincer, kiting) in
  the database; surface them in Explain.
* **Position library**: save/load positions as tactical puzzles ("best turn?") with known answers.
* **Opening book**: precomputed first-round plans for common encounter templates (party level ×
  creature roles × map archetype), so round 1 suggestions are instant and deeper than real-time
  search allows.
* Learned evaluator (§8 #6).

---

## 11. Security

| Risk | Mitigation |
|---|---|
| Information leakage to players (hidden monsters, exact HP, monster numbers) | Bar GM-only by default; GM-authoritative mode sends only `{score, band, win_probability}`; player snapshots exclude hidden and unseen tokens (`extract/snapshot.ts`); GM assistant and forecast GM-only |
| Untrusted socket messages | Only the `evaluation` message type is handled; fields are type-checked and clamped before rendering; scene id must match |
| Hostile or corrupt snapshot (module conflict, crafted world data) | Size and count limits, plausibility checks, formula parser rejects anything but dice arithmetic (no `@` references, no functions), formula length ≤ 512 |
| XSS through actor or item names | Templates use escaped Handlebars `{{…}}`; DOM built in code uses `textContent` or `escapeHtml` |
| Denial of service (pathological search) | Per-request deadlines, node budget, bounded caches, 15 s hard timeout that kills and replaces the worker |
| Supply chain | One Rust dependency added (`wasm-bindgen`, pinned exactly); the module has no runtime npm dependencies; release zip built in CI |
| Tracker integrity | Only the active GM writes the turn flag; players cannot update Combat documents |

---

## 12. Migration steps

1. **Build**: `rustup target add wasm32-unknown-unknown`;
   `cargo install wasm-bindgen-cli --version 0.2.130 --locked`; `scripts/build-wasm.sh`;
   `cd foundry && npm ci && npm run build`.
2. **Install for development**: symlink `foundry/dist/pf2e-stockfish` into `Data/modules/`.
3. **Verify in a live Foundry v13 + PF2e 7.x world** (not yet done):
   * `game.modules.get('pf2e-stockfish').api.snapshot()` on a populated scene: compare AC, saves,
     strike bonuses and damage formulas against the sheets with and without frightened/clumsy;
     confirm stripped modifier slugs match `ENGINE_APPLIED_SLUGS`.
   * Spellcasting: prepared, spontaneous, innate and focus entries report the expected remaining
     slots; check `spell.system.damage` / `defense` / `heightening` shapes.
   * Walls: Best Turn never paths or strikes through walls; doors open/closed.
   * Turn tracker: Strike, spell (1/2/3 actions), Trip, reaction and token moves update the pips.
   * UI: scene-control group renders in v13's record form; combatant context-menu hook name;
     ApplicationV2 windows; Token HUD button placement.
4. **Release**: tag `v0.1.0`; CI builds `module.json` + `pf2e-stockfish.zip` and attaches them to
   the GitHub release; the manifest URL in `module.json` points at the latest release.

The existing server and React UI are unchanged and keep working; both share the improved core
and AI crates.

---

## 13. Testing strategy

| Layer | Tests | Command |
|---|---|---|
| Core | 58 tests (3 new: thin walls block movement/LoE, wall corners block diagonals, empty edges keep the serialized form) | `cargo test -p pf2e-core` |
| AI | 13 tests (6 new: cached evaluation exact, accelerations agree, budget gives complete shallower answers, split simulation merges exactly, explain change, explain flanking) | `cargo test -p pf2e-ai` |
| Bridge | 19 tests: formula parser, wall rasterization (no leaks through diagonal walls), crop, spell translation, round trip, every API call, error codes, live state, stacked/dead tokens, walls block strikes | `cargo test -p pf2e-bridge` |
| Module | 20 vitest tests: extraction from PF2e-shaped actors, modifier stripping, viewer filtering, TS→Rust contract, engine client on the real WASM (all analyses, split forecast identical, error codes, timeout + restart), service debounce/dedupe, tracker, permissions, view models | `cd foundry && npm test` |
| Bundle | headless Chromium loads the built worker + WASM from `/modules/pf2e-stockfish/` and runs every analysis | `cd foundry && npm run smoke` |
| Performance | latency benchmark | `cargo run --release -p pf2e-bridge --example bench` |

Next: a Foundry integration harness (Quench or Playwright against a scripted Foundry server) to
cover the hook wiring and the PF2e document paths in CI.

---

## 14. Performance considerations

* Main thread: snapshot extraction only (no rules); strike formulas cached per actor; refresh
  debounced; unchanged snapshots skipped. Search and simulation always run in workers.
* Forecasts use dedicated workers so the evaluation bar never waits behind a simulation; results
  merge exactly.
* Board cropping keeps the engine map small (combatant bounding box + 12 squares, ≤ 200²).
* Deadlines: Best Turn 75 ms search (≈ 100 ms end to end), 400 ms per monster; both configurable.
* WASM size 1.6 MB (includes the bundled rules content); loaded once per worker.
