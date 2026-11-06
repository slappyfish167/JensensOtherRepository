# Cyberpunk RED

`crates/cpred-core` is a pure, deterministic Cyberpunk RED combat engine built on the same
principles as the PF2e engine: commands in, events out, a serializable resolution queue, humans
enter every die, and `initial state + command log` replays exactly. `crates/cpred-ai` adapts it to
the system-neutral analysis layer (see [SYSTEMS.md](SYSTEMS.md)).

It implements game **mechanics** only. No rulebook text or critical-injury tables are reproduced,
and the sample characters and NPCs are original. Weapons and armor use generic class names.

## Rules as implemented

The authoritative list, with every interpretation, is the module documentation of
`crates/cpred-core/src/rules.rs`. In short:

| Area | Implementation |
|---|---|
| Grid | 1 square = 2 m, Chebyshev distance (diagonal = 1). `.` open, `~` difficult (2 squares), `#` wall (blocks movement and sight), `%` cover object (blocks movement, has HP). |
| Checks | STAT + skill + 1d10. Natural 10: roll another d10 and add. Natural 1: roll another d10 and subtract. The follow-up never explodes. The roller must **beat** the number (ties go to the defender). |
| Initiative | REF + 1d10 (no explosion). Ties go to higher REF, then lower id. |
| Turn | One Move Action (MOVE squares) and one Action. Run = a second move using the Action. |
| Ranged attacks | REF + weapon skill vs the range DV for the weapon class and range band (0–6, 7–12, 13–25, 26–50, 51–100, 101–200, 201–400, 401–800 m). Needs line of sight and ammunition. ROF 2 weapons get two attacks per Action. |
| Aimed shot | −8, hits the head, takes the whole Action. |
| Autofire | Autofire skill vs the autofire DV, 10 rounds, the whole Action. Damage = 2d6 × min(margin, weapon's max multiplier). |
| Dodge | A target with REF ≥ 8 (not mortally wounded) may roll DEX + Evasion; that total replaces the DV. |
| Melee | Adjacent only. The defender always rolls DEX + Evasion, then the attacker must beat it. Every combatant also has Brawling (damage by BODY). |
| Armor | Body and head SP. Melee weapons (not Brawling) halve SP, rounding up. Damage that beats SP: the excess goes to HP (doubled on head hits) and SP drops by 1. Armor penalties reduce REF, DEX and MOVE. |
| Critical injury | Two or more 6s on the damage dice: 5 damage straight to HP, the injury is recorded at the hit location, and the death save penalty goes up by 1. The GM resolves the specific injury on their own table. |
| Wounds | Max HP = 10 + 5 × ceil((BODY + WILL) / 2), unless an NPC gives `max_hp`. Seriously wounded (below half): −2 to all checks. Mortally wounded (0 HP): −4 to checks, MOVE −6, death saves. |
| Death saves | At the start of each turn while mortally wounded, and when taking damage while mortally wounded: d10, success if the face isn't 10 and face + penalty < BODY. Each success raises the penalty by 1. Failure = death. |
| Cover | A ranged shot whose line passes through a cover square next to the target hits the cover instead (default 20 HP; at 0 HP the square becomes open floor). Melee ignores cover. |
| End of combat | A side whose every member is dead or mortally wounded has lost. |

### Check these against your rulebook

These are the points marked **VERIFY** in `rules.rs`. They come from memory of the published
rules or are simplifications. Most of them are data (`content/cpred/rules.json`, the weapon and
armor files) or `RulesOptions` flags, so a GM can correct them without code changes:

* Range and autofire DV tables, autofire multipliers, weapon damage/ROF/magazines and armor
  SP/penalties (all in `content/cpred/*.json`).
* The HP formula and the seriously/mortally wounded thresholds and penalties.
* Dodging replaces the DV (rather than, for example, using the higher of the two).
* Death save comparison: `<` vs `≤` BODY (`RulesOptions::death_save_inclusive`), and whether
  taking damage while mortally wounded forces a save (`death_save_on_damage`).
* Cover as destructible objects that absorb blocked shots (a simplification of the cover rules).

Not modelled: Netrunning, cyberware and drugs, grappling and martial-arts special moves,
suppressive fire, explosives' areas, healing and stabilization (use the GM's *Adjust HP*), and
critical-injury effects (recorded for the GM to apply).

## Entering dice

| Prompt | What you type |
|---|---|
| Checks, initiative, death saves, critical follow-ups | the natural d10 |
| Damage | **every die face**, e.g. `4 6 6 2 1` for 5d6, because the number of sixes decides a critical injury |

As in PF2e, *Extra* adds a logged situational modifier, and seeded auto-rolls are opt-in per combat.

## How the AI plays Cyberpunk RED

The analysis layer is shared with PF2e: the same committed expectiminimax search, tactical report,
strategy and Monte Carlo. `cpred-ai` supplies the system-specific parts:

* **Expected damage** comes from the core's exact math: an exploding-d10 hit chance against the DV
  or the dodge roll (the defender is assumed to dodge when that helps it), the full damage
  distribution against SP, head doubling, the critical-injury bonus and the autofire multiplier.
  A target behind cover counts at 40% of an open shot, since the attacker has to reposition or
  shoot the cover down first.
* **Movement candidates** are picked for what they do: the best cover square (fewest enemies with
  a clear shot), the best firing square (most targets), the square closest to the nearest enemy
  (melee, or no shot anywhere) and a retreat square.
* **The Action is always used.** Once it is spent, the rest of the turn (the Move Action, a second
  ROF shot) is optional, so the search may choose to stay put behind cover. This is a real choice,
  not "no move improves the position".
* **Factor breakdown:** Cover, Armor (ablation), Ammunition and Wounds, in addition to the generic
  Hit points / Offense / Position.

## Content format

`content/cpred/` holds `weapons.json`, `armor.json`, `rules.json`, `actors.json` and
`encounters.json`. In the bundled actor files a statblock may reference catalogue entries
(`weapon_ids`, `armor_ids`). Actors stored through the API (`/api/systems/cpred/actors`) are
fully resolved statblocks; the easiest way to make one is **Roster → New actor**, which starts
from a copy of an existing actor. An encounter lists participants by `actor_id` (or with an
inline `statblock`), side and position on its battlefield map.
