# REST API

Base URL: `http://127.0.0.1:8080`. All bodies are JSON; field names are `snake_case` and match the
Rust models one-to-one (TypeScript mirrors: [`frontend/src/api/types.ts`](../frontend/src/api/types.ts)).

Errors: `{ "error": "message", "details": … }` with status 400 (bad input), 404, 409 (conflict /
wrong state) or 422 (rules violation: illegal action, impossible roll, wrong request id).

## Content & characters

| Method | Path | Body → Response |
|---|---|---|
| GET | `/api/health` | → `{status, version}` |
| GET | `/api/content` | → ancestries, backgrounds, classes, feats, weapons, armor, shields, spells |
| GET | `/api/characters` | → `CharacterSummary[]` (includes `is_legal`) |
| POST | `/api/characters` | `Character` → `{character, build}` (409 if id exists; empty id = new UUID) |
| POST | `/api/characters/import` | `Character` → `{character, build}` (never overwrites; new id on collision) |
| POST | `/api/characters/validate` | `Character` → `BuildResult {report, attributes, proficiencies, statblock}` |
| GET / PUT / DELETE | `/api/characters/{id}` | `Character` |
| GET | `/api/characters/{id}/statblock` | → `BuildResult` |
| GET | `/api/characters/{id}/export` | → `Character` JSON |

A build is saved even when illegal (so work is never lost); it cannot enter combat until legal.

```json
// POST /api/characters/validate → excerpt
{ "report": { "issues": [
    { "severity": "error", "code": "too_many_skills", "path": "trained_skills",
      "message": "Fighter may choose 3 additional trained skills; 4 chosen" },
    { "severity": "warning", "code": "empty_slot", "path": "feats",
      "message": "Skill feat slot at level 2 is empty" } ] },
  "statblock": null }
```

## Monsters & hazards

| Method | Path | Notes |
|---|---|---|
| GET | `/api/monsters` | `MonsterSummary[]` |
| POST | `/api/monsters` | create `Monster` |
| POST | `/api/monsters/import` | a `Monster`, an array of them, **or a bare `Statblock`** |
| GET / PUT / DELETE | `/api/monsters/{id}` | |
| POST | `/api/monsters/{id}/template` | `{template: "elite"\|"weak", save: bool}` → templated `Monster` (id `…-elite`) |

## Encounters

| Method | Path | Notes |
|---|---|---|
| GET / POST | `/api/encounters` | POST returns `{encounter, budget}` |
| GET / PUT / DELETE | `/api/encounters/{id}` | |
| GET | `/api/encounters/{id}/budget` | `EncounterBudget {party_size, total_xp, threat, budgets[5], warnings}` |
| POST | `/api/encounters/budget` | budget for an unsaved `Encounter` |

```json
{ "id": "", "name": "Bridge", "party_level": 3,
  "battlefield": { "width": 10, "height": 6, "map": ["..........", "..~~~.....", "...", "…"] },
  "participants": [
    { "source": {"kind": "character", "value": "sample-fighter"}, "side": "party", "position": {"x": 1, "y": 2} },
    { "source": {"kind": "monster", "value": "cave-brute"}, "side": "enemy", "position": {"x": 7, "y": 2}, "name": "Grog" }
  ] }
```

Map glyphs: `.` open, `~` difficult, `^` greater difficult, `#` wall, `%` obscured (concealment).

## Combats

| Method | Path | Notes |
|---|---|---|
| POST | `/api/combats` | `{encounter_id, name?, dice?: {mode:"manual"} \| {mode:"seeded", seed}}` → `CombatView` |
| GET | `/api/combats` | `CombatRecord[]` |
| GET / DELETE | `/api/combats/{id}` | `CombatView` |
| POST | `/api/combats/{id}/commands` | `Command` → `CombatView` (with this command's `events`) |
| POST | `/api/combats/{id}/auto-roll` | answer the pending roll with seeded dice (seeded combats only) |
| POST | `/api/combats/{id}/undo` | drop the last command, replay |
| GET | `/api/combats/{id}/log` | `CombatLog {initial, commands}` — everything needed to reproduce |
| GET | `/api/combats/{id}/replay` | `{identical, commands, live_fingerprint, replayed_fingerprint}` |
| GET | `/api/combats/{id}/legal-actions` | `[{action, description, cost}]` for the current creature (Strides omitted) |
| GET | `/api/combats/{id}/movement/{combatant}` | reachable squares `[{x, y, cost}]` |

`CombatView = { id, name, dice, status, state: CombatState, pending: PendingInput|null,
current_actor, command_count, fingerprint, events }`

### Commands

```json
{"type": "start_combat"}
{"type": "provide_roll", "request_id": 7, "value": 14, "extra_modifier": 0}
{"type": "decide", "request_id": 9, "accept": true}
{"type": "act", "actor": 3, "action": {"kind": "strike", "actor_strike": 0, "target": 5}}
{"type": "act", "actor": 3, "action": {"kind": "strike", "actor_strike": 1, "target": 5, "cover_override": "none"}}
{"type": "act", "actor": 3, "action": {"kind": "stride", "to": {"x": 4, "y": 6}}}
{"type": "act", "actor": 4, "action": {"kind": "cast_spell", "entry": 0, "spell_id": "fireball", "rank": 3, "targets": [], "aim": {"x": 9, "y": 4}}}
{"type": "act", "actor": 2, "action": {"kind": "raise_shield"}}
{"type": "end_turn", "actor": 3}
{"type": "gm", "op": {"kind": "add_condition", "target": 5, "condition": {"kind": "frightened", "value": 2}}}
```

Other actions: `step`, `stand`, `take_cover`, `demoralize {target}`, `trip {target}`,
`disable_hazard {hazard}`. GM ops: `adjust_hp`, `set_temp_hp`, `add_condition`, `remove_condition`,
`move`, `clear_pending`.

### Pending input

```json
{ "type": "roll", "request_id": 7, "roller": 3,
  "kind": {"kind": "check", "modifier": 14, "dc": 18},
  "expression": "1d20+14",
  "purpose": "Longsword attack (Kira Ironhand → Cave Brute, AC 18)",
  "prompt": "Roll 1d20+14 for Longsword attack (Kira Ironhand → Cave Brute, AC 18) — enter the natural d20",
  "breakdown": [{"kind": "untyped", "value": -5, "source": "multiple attack penalty"},
                {"kind": "circumstance", "value": -2, "source": "target AC: off-guard (flanked)"}] }

{ "type": "roll", "kind": {"kind": "total", "dice": "1d8+4"}, "prompt": "Roll 1d8+4 for Longsword slashing damage … — enter the total", … }
{ "type": "roll", "kind": {"kind": "flat", "dc": 15}, "prompt": "Roll a DC 15 flat check (1d20) for … to end persistent fire damage", … }
{ "type": "decision", "request_id": 9, "decider": 5, "reaction": "reactive_strike",
  "prompt": "Ezren Vale is moving out of a square within reach. Use Cave Brute's Reactive Strike?" }
```

## Analysis (optional AI layer)

| Method | Path | Answers |
|---|---|---|
| GET | `/api/combats/{id}/analysis/evaluation` | position score −10…+10, win estimate, factor breakdown |
| POST | `/api/combats/{id}/analysis/best-turn` | `{node_budget?, focus?}` → "What is the best 3-action turn?" |
| GET | `/api/combats/{id}/analysis/targets?attacker=ID` | "What is the strongest target?" |
| GET | `/api/combats/{id}/analysis/spells` | "Which spell maximizes expected value?" |
| POST | `/api/combats/{id}/analysis/tpk` | `{runs?, seed?, max_rounds?}` → "How likely is a TPK?" |
| GET | `/api/combats/{id}/analysis/strategy?side=enemy` | "What is the optimal monster strategy?" |

```json
// POST …/analysis/best-turn
{ "summary": "Bone Sentinel: Stride to (5, 7), then Stride to (2, 8), then Strike Ezren Vale with Rusted Blade (evaluation +6.5 → +6.3)",
  "steps": ["Stride to (5, 7)", "Stride to (2, 8)", "Strike Ezren Vale with Rusted Blade"],
  "plan": { "actor": 5, "actions": [ … ], "expected_score": 6.3, "baseline_score": 6.5, "gain": 0.2, "nodes": 18342, "truncated": false } }
```

Analysis never modifies the live combat; it works on copies.
