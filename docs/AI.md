# Tactical analysis (`tactics-ai`)

The analysis layer is **system-neutral**. `crates/tactics-ai` holds the evaluator, the committed
search, the tactical report, target ranking, side strategy and Monte Carlo, all generic over the
`TacticalSystem` trait. `crates/pf2e-ai` and `crates/cpred-ai` implement that trait for Pathfinder
2e and Cyberpunk RED (see [SYSTEMS.md](SYSTEMS.md) and [CYBERPUNK_RED.md](CYBERPUNK_RED.md)).
The PF2e specifics below (Strikes, MAP, spell slots) are what `pf2e-ai` feeds into the generic
model; Cyberpunk RED feeds ROF attacks, armor SP, cover and ammunition into the same slots.

The analysis layer is optional and **contains no rules**. It reads state, asks the engine for
`legal_actions()`, and simulates by calling `apply_in_place` on cloned states. Expected-value helpers
build `AttackCtx` values and call the core's `attack_numbers`, `strike_damage_specs`,
`degree_distribution`, `apply_iwr` and `concealment_dc`, so a rule change in the core is
automatically reflected here.

## 1. Evaluation (−10 … +10)

Model: **Lanchester's square law**. A side's fighting strength is `H × O`:

* `H` — effective hit points: HP + temp HP of standing members, minus expected persistent damage
  (average × 1/0.3 ticks, the DC 15 flat-check expectation).
* `O` — expected damage per round: for every standing member, the best full-round plan against each
  member of the opposing roster (`round_damage`: best Strike at each MAP step, or a damaging spell —
  only if a slot remains — plus the remaining Strikes), scaled by an **engagement factor** (rounds
  lost closing distance) and an **action factor** (slowed, stunned, quickened, unconscious),
  averaged between the mean and the best target. The opposing roster **includes downed members**:
  offense is a property of the attackers, so dropping the softest enemy never makes your own side
  look weaker just because harder targets remain. (Averaging over standing enemies only made
  "finish the wounded PC" score worse than doing nothing whenever the survivors were armored.)
* **Tempo**: the creature whose turn it is gets half its remaining-action damage credited against
  the other side's `H`.

```
logit = ln(H_party · O_party) − ln(H_enemies · O_enemies)
score = 10 · tanh(logit / 2)            # +10 overwhelming party … −10 near-certain defeat
win   ≈ 1 / (1 + e^(−1.6 · logit))
```

Terminal positions are exactly ±10. The evaluation is antisymmetric (swapping the sides negates it;
tested) and conditions need no special cases: frightened lowers the frightened creature's attack
and AC inside the core's modifier computation, which changes `O` on both sides.

**Factor breakdown** (`evaluate_detailed`) is counterfactual — each factor is the change in score
when that aspect is neutralised. Generic: Hit points, Offense vs defense, Position (assume everyone
engaged). Each system adds its own through `TacticalSystem::counterfactuals`. PF2e adds Conditions
(clear all non-dying conditions/effects/persistent damage), Action economy (remove tempo and
slowed/stunned/quickened), Resources (empty all spell slots) and Battlefield control (remove
obscuring terrain and auto-cover). Cyberpunk RED adds Cover, Armor, Ammunition and Wounds.

## 2. "What is the best 3-action turn?" — expectiminimax

The engine plays to win. When it controls a creature it always returns the **best legal combat
turn**. Ending the turn early is not a candidate while any legal action remains, so the answer is
never "no move improves the position". A turn comes back empty only when the creature literally
has no legal action. If every line loses ground, the engine still picks the one that loses least.

```
decide(state):                       # the acting creature's choice
    if no actions left or cannot act: return leaf(state)
    best = None                      # committed: stopping is not an option
    for action in candidates(state):
        child = state.clone(); child.apply(Act(action))
        best = max(best, resolve(child, decide))
    return best ?? leaf(state)       # only when nothing could legally be taken

resolve(state, then):                # nature and opponents
    match state.pending():
        None              → then(state)
        Roll(check|flat)  → Σ_p  p · resolve(apply(ProvideRoll(face_p)))   # one face per degree
        Roll(total)       → resolve(apply(ProvideRoll(round(avg))))
        Decision(react)   → max over {accept, decline} if the reactor is on our side, else min
```

`SearchConfig.allow_end_turn = true` (API: `allow_end_turn`) restores advisory mode, where
`best` starts as `leaf(state)` and stopping competes with acting.

* **Chance nodes** use `representative_faces(modifier, dc)`: one natural roll per achievable degree,
  weighted by its exact probability (at most 4 branches instead of 20).
* **Move generation** prunes, never invents: from the engine's legal list it keeps all attacks,
  spells and maneuvers, one escaping Step, and a few Stride destinations — the cheapest square
  threatening each foe (bonus for flanking squares), an approach square, and a retreat square for
  ranged creatures.
* **Committed move generation** also keeps Take Cover. If pruning leaves nothing, it falls back to raw
  legal actions, so a creature with any legal action always has a candidate.
* **Budget**: `node_budget` (default 40 000). Each opening (root) action searches inside a budget
  window: it may use what is left but must leave a reserve of `budget / 4n` for every opening
  still to come. Past half of its window, chance nodes collapse to their most likely outcome.
  Past the whole window, the rest of the turn is filled greedily (one-ply lookahead per action)
  rather than abandoned. Either way the plan is flagged `truncated`. The windows matter: without
  them, late openings were judged on collapsed trees, which always assume the likeliest result
  (usually a hit), so they looked better than they were. Below the root, a beam keeps the 8
  best-ordered candidates.
* The reported plan is the principal variation along the most likely outcomes.
* **Outcome metrics** travel with every node and are aggregated with the same probabilities and
  choices as the score: damage dealt to each hostile creature, damage the actor takes, whether
  each foe is down, and every condition, persistent damage or effect newly applied. The death
  track (dying/unconscious/wounded/doomed) is reported only as the chance to down the target.
  The expectations therefore describe the adaptive policy the search found. Because that policy
  can switch targets in branches where the first Strike misses, they are not limited to the
  principal line.
* **Alternate plan**: the best other opening whose idea differs from the primary (another kind of
  first action, or another target). If none differs, the next-best opening is used.

### Tactical report (`queries::recommend_turn`)

Every recommendation carries a `TacticalReport`:

| Field | Meaning |
|---|---|
| `primary` / `alternate` | the two action plans, described step by step (Strides note the foes they flank) |
| `expected_damage` | expected HP removed from hostile creatures this turn |
| `expected_status_effects` | `{name, effect, probability}`, most likely first |
| `priority_target` | the creature the primary plan is aimed at: most actions in the plan, then most expected damage and chance to drop it |
| `down_chance` | P(priority target is down at the end of the turn) |
| `position` | expected evaluation after the turn from the actor's side, e.g. `+0.3 Monster Advantage` (always signed toward one side, never "equal") |
| `reason` | why: lowest effective HP / biggest threat / best threat-per-damage, drop chance and the action-economy value of removing a PC, flanking, likely conditions, reaction damage accepted |
| `text` | the whole read-out as a printable block |

```
RECOMMENDED TURN — Cave Brute

Action 1: Strike Kira with Blade.
Action 2: Strike Kira with Blade.
Action 3: Demoralize Valeros.

ALTERNATE TURN
…
Expected Damage:
14.0

Chance to Down Target:
58%

Position Evaluation:
+1.9 Monster Advantage

Reason:
Kira has the lowest effective HP (14); 58% chance to drop Kira this turn — removing a player
grants an action economy advantage.
```

Measured on the sample Crypt Ambush (release build, one core): a best-turn query takes ~0.05 s,
the full three-monster strategy query ~2 s, and a 200-run TPK simulation ~2.5 s. Turns with many
multi-target saves grow fastest (4ⁿ save outcomes); the budget keeps them bounded.

## 3. Other queries

| Question | Function | Method |
|---|---|---|
| Strongest target | `queries::rank_targets` | expected damage this turn (`round_damage` × engagement × actions), kill probability (enumerates hit/crit/miss for up to three Strikes), target threat (its own damage/round vs our side). `priority = threat × (P(kill) + E[dmg]/HP)` — threat removed per point of damage. |
| Best spell | `queries::rank_spells` | for every castable spell option: one-ply expectiminimax gain in evaluation, plus exact expectations over the chance tree of enemy HP lost, ally HP healed and P(an enemy drops). |
| TPK odds | `simulate::simulate` | Monte Carlo (system-neutral driver, system-specific policy): seeded dice + a fast scripted policy for every creature (best immediate expected value, else close distance; hostile creatures then fall back to their best remaining attack, a defensive action, or any legal action instead of ending the turn). Reports win/TPK/death rates per side and per creature. Deterministic for a given seed. |
| Monster strategy | `queries::side_strategy` | for every creature on the side: `hypothetical_turn` (the core's start-of-turn on a copy) + best-turn search; a focus target chosen by summed target priorities; the biggest evaluation swing factor. |

## 4. Extending

* Implement `Evaluator` for a new evaluator (e.g. one fitted to Monte Carlo outcomes) and pass it to
  `best_turn`.
* Swap the scripted Monte Carlo policy for `best_turn` with a small budget for stronger (slower)
  simulations.
* The same search works for PCs: ask for the best turn on a PC's turn to get advice for players.
* A new game system gets all of the above by implementing `TacticalSystem` (see SYSTEMS.md).
  `TacticalSystem::may_stop` lets a system mark the rest of a turn as optional (Cyberpunk RED: once
  the Action is spent), without ever allowing an empty turn.
