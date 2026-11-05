# Tactical analysis (`pf2e-ai`)

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
  standing enemy (`round_damage`: best Strike at each MAP step, or a damaging spell — only if a slot
  remains — plus the remaining Strikes), scaled by an **engagement factor** (rounds lost closing
  distance) and an **action factor** (slowed, stunned, quickened, unconscious), averaged between the
  mean and the best target.
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
when that aspect is neutralised: Hit points, Offense vs defense, Conditions (clear all non-dying
conditions/effects/persistent damage), Action economy (remove tempo and slowed/stunned/quickened),
Position (assume everyone engaged), Resources (empty all spell slots), Battlefield control
(remove obscuring terrain and auto-cover).

## 2. "What is the best 3-action turn?" — expectiminimax

```
decide(state):                       # the acting creature's choice
    best = leaf(state)               # stopping here (end turn) is always an option
    for action in candidates(state):
        child = state.clone(); child.apply(Act(action))
        best = max(best, resolve(child, decide))
    return best

resolve(state, then):                # nature and opponents
    match state.pending():
        None              → then(state)
        Roll(check|flat)  → Σ_p  p · resolve(apply(ProvideRoll(face_p)))   # one face per degree
        Roll(total)       → resolve(apply(ProvideRoll(round(avg))))
        Decision(react)   → max over {accept, decline} if the reactor is on our side, else min
```

* **Chance nodes** use `representative_faces(modifier, dc)`: one natural roll per achievable degree,
  weighted by its exact probability (at most 4 branches instead of 20).
* **Move generation** prunes, never invents: from the engine's legal list it keeps all attacks,
  spells and maneuvers, one escaping Step, and a few Stride destinations — the cheapest square
  threatening each foe (bonus for flanking squares), an approach square, and a retreat square for
  ranged creatures.
* **Budget**: `node_budget` (default 40 000); past half the budget chance nodes collapse to their
  most likely outcome and the plan is flagged `truncated`. Below the root a beam keeps the 8 best
  ordered candidates.
* The reported plan is the principal variation along the most likely outcomes.

Measured on the sample Crypt Ambush (release build, one core): a best-turn query takes ~0.05 s,
the full three-monster strategy query ~2 s, and a 200-run TPK simulation ~2.5 s. Turns with many
multi-target saves grow fastest (4ⁿ save outcomes); the budget keeps them bounded.

## 3. Other queries

| Question | Function | Method |
|---|---|---|
| Strongest target | `queries::rank_targets` | expected damage this turn (`round_damage` × engagement × actions), kill probability (enumerates hit/crit/miss for up to three Strikes), target threat (its own damage/round vs our side). `priority = threat × (P(kill) + E[dmg]/HP)` — threat removed per point of damage. |
| Best spell | `queries::rank_spells` | for every castable spell option: one-ply expectiminimax gain in evaluation, plus exact expectations over the chance tree of enemy HP lost, ally HP healed and P(an enemy drops). |
| TPK odds | `simulate::simulate` | Monte Carlo: seeded dice + a fast scripted policy for every creature (best immediate expected value, else close distance, else end turn). Reports win/TPK/death rates per side and per creature. Deterministic for a given seed. |
| Monster strategy | `queries::side_strategy` | for every creature on the side: `hypothetical_turn` (the core's start-of-turn on a copy) + best-turn search; a focus target chosen by summed target priorities; the biggest evaluation swing factor. |

## 4. Extending

* Implement `Evaluator` for a new evaluator (e.g. one fitted to Monte Carlo outcomes) and pass it to
  `best_turn`.
* Swap the scripted Monte Carlo policy for `best_turn` with a small budget for stronger (slower)
  simulations.
* The same search works for PCs: ask for the best turn on a PC's turn to get advice for players.
