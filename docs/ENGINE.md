# Rules engine (`pf2e-core`)

## 1. Data models

All models are plain serde structs/enums; JSON is the interchange and storage format.

| Model | Type | Purpose |
|---|---|---|
| **Character** | `models::character::Character` | A *build*: ancestry/background/class ids, attribute boost choices, trained skills, skill increases, feat choices (with slot type + level), equipment (runes), spells. No derived numbers. |
| **Monster** | `models::monster::Monster` | Reusable asset: `Statblock` + source/tags/description + applied templates (`elite`/`weak`). Hazards are monsters whose stat block has a `HazardProfile`. |
| **Statblock** | `models::statblock::Statblock` | The one creature model the combat engine sees: level, size, attributes, HP, AC, saves, Perception, Speed, skills, `StrikeProfile`s, `SpellcastingEntry`s, IWR, reactions, shield, damage riders (sneak attack), dying rules flag, hazard profile. |
| **Weapon / Armor / Shield** | `models::weapon` | Item definitions; weapon traits are a typed enum (`agile`, `deadly-d10`, `reach-10`, `thrown-20`, `fatal-d10`, `forceful`, `sweep`, …) serialized as compact strings. |
| **Spell** | `models::spell::Spell` | Rank, traditions, traits, actions, range, targeting (creatures / area / self), defense (none / attack / save + basic), damage & healing with heightening, per-degree outcomes (conditions with durations, persistent damage, effects). |
| **Feat** | `models::feat::Feat` | Type (ancestry/class/skill/general/archetype), level, traits, archetype, machine-checkable prerequisites, passive effects (HP per level, speed, reactions, save/perception ranks, initiative, dedication). |
| **Encounter** | `models::encounter::Encounter` | Party level, participants (character / monster / inline stat block, side, position, name, HP override), battlefield. Plus the XP budget calculator. |
| **Battlefield** | `models::battlefield::Battlefield` | Width × height grid; rows of glyphs (`.` `~` `^` `#` `%`). |
| **Combatant** | `combat::state::Combatant` | Runtime creature: stat block, HP/temp HP, position, `ConditionSet`, persistent damage, effects, actions remaining, reaction, attacks this turn (MAP), shield HP, remaining spell slots, life state. |
| **Condition** | `conditions::Condition` | Kind (25 conditions), value, source, optional expiry. `ConditionSet` keeps the highest value and resolves implications transitively (dying ⇒ unconscious ⇒ prone ⇒ off-guard). |
| **Effect** | `effects::Effect` | Named bundle of `ModifierGrant`s (selector + typed modifier) and tags (shield raised, taking cover, immunity markers) with an expiry. |
| **InitiativeEntry** | `combat::state::InitiativeEntry` | `{combatant, value, natural}`. |
| **CombatState** | `combat::state::CombatState` | Battlefield, combatants, spell definitions in play, phase, round, turn index, initiative order, resolution queue, input sequence number, rules options. |

```rust
pub struct CombatState {
    pub schema_version: u32,
    pub battlefield: Battlefield,
    pub combatants: Vec<Combatant>,
    pub spells: BTreeMap<String, Spell>,   // copies → old combats never change
    pub phase: Phase,                       // Setup | RollingInitiative | Active | Ended{winner}
    pub round: u32,
    pub turn: usize,                        // index into `initiative`
    pub initiative: Vec<InitiativeEntry>,
    pub queue: VecDeque<Step>,              // resolution queue (front = next)
    pub input_seq: u32,                     // id of the next roll/decision request
    pub next_effect_id: u32,
    pub next_tag: u32,                      // groups the Steps of one action (for disruption)
    pub options: RulesOptions,              // flanking, auto-cover
}

impl CombatState {
    pub fn apply(&mut self, cmd: &Command) -> Result<Vec<Event>, CombatError>; // atomic
    pub fn pending(&self) -> Option<PendingInput>;
    pub fn legal_actions(&self) -> Vec<Action>;
    pub fn check_action(&self, actor: CombatantId, a: &Action) -> Result<u8, CombatError>; // cost
    pub fn movement(&self, id: CombatantId) -> MoveMap;
    pub fn hypothetical_turn(&self, id: CombatantId) -> Option<CombatState>;
    pub fn fingerprint(&self) -> String;
}
```

## 2. The resolution queue

```rust
pub enum Step {
    // turn structure
    RollInitiative{combatant}, FinalizeInitiative, BeginTurn, RecoveryCheck{combatant},
    EndTurn{combatant}, PersistentDamage{..}, PersistentRecovery{..}, EndTurnTail{..}, AdvanceTurn,
    // action resolution
    ConcealmentCheck{tag, ctx, dc}, AttackRoll{tag, ctx},
    DamageRolls{tag, source, shares, specs, rolled, critical, from_strike, purpose},
    DeliverDamage{tag, source, target, parts, critical, from_strike, shield_offered, shield_blocked},
    SaveSequence{tag, source, save, pending, results},
    ResolveSpell{..}, SpellOutcomes{..}, HealRoll{..}, SkillCheck{..},
    ContinueMove{tag, mover, kind, remaining, offered}, HazardTrigger{hazard, victim},
    Decision{tag, decider, reaction, prompt, on_accept: Vec<Step>, on_decline: Vec<Step>},
}
```

`Step::needs_input()` marks the steps that wait for a human. Everything else runs in `run_automatic`.
Because the queue is part of the serialized state, a combat can be saved in the middle of a Strike
and resumed later.

**Example — a Strike that triggers Shield Block**

```
Act(Strike)        → [AttackRoll]                           pending: "Roll 1d20+9 …"
ProvideRoll(15)    → hit → [DamageRolls{specs:[1d8+4]}]     pending: "Roll 1d8+4 …"
ProvideRoll(9)     → [DeliverDamage] → eligible for Shield Block →
                     [Decision{ShieldBlock, on_accept:[DeliverDamage{blocked}], on_decline:[…]}]
Decide(true)       → reaction spent → DeliverDamage{blocked}: IWR, hardness, shield HP, HP, dying …
```

**Reactive Strike disrupting a spell**: casting a manipulate spell pushes Reactive Strike decisions
*in front of* the spell's `ResolveSpell` step. The reaction's attack carries `disrupts: Some(tag)`;
on a critical hit the engine removes every queued step carrying that tag.

## 3. Algorithms

### Degree of success
```
base = total ≥ DC+10 ? CritSuccess : total ≥ DC ? Success : total ≤ DC−10 ? CritFailure : Failure
natural 20 → one step better; natural 1 → one step worse
```
`degree_distribution(modifier, dc)` enumerates all 20 faces (exact probabilities).

### Modifier stacking
Circumstance, status and item: only the best bonus and the worst penalty of each type count;
untyped penalties all stack. Conditions and effects are *sources* of typed modifiers selected by
`CheckType` (attack with attribute, spell attack, save, Perception, skill, AC, spell DC), e.g.
frightened → status −X to everything; clumsy → status −X to Dex-based checks and AC; off-guard →
circumstance −2 AC; cover and a raised shield are circumstance AC bonuses and therefore don't stack.

### Multiple attack penalty
`0 / −5 / −10` (agile `0 / −4 / −8`) based on `attacks_this_turn`, captured when the attack is declared.
Trip and spell attack rolls count; Reactive Strike neither counts nor suffers it.

### Strike damage
Base weapon dice (fatal die size on a crit) + flat modifiers (Str, specialization, forceful,
enfeebled, effects) → doubled on a crit; deadly/fatal extra dice are added undoubled; rune riders and
sneak attack (when the target is off-guard to the attacker, including flanking) are separate typed
parts. Minimum 1 damage. IWR per part: immunity → highest weakness → highest resistance.

### Movement (5-10-5 diagonals)
Dijkstra over `(square, diagonal-parity)` so the second diagonal costs 10 ft; difficult +5,
greater difficult +10, walls impassable, no corner cutting, enemies impassable, allies passable but
not valid end squares. Multi-square creatures check their whole footprint. The binary heap is
ordered by `(cost, y, x, parity)` so equal-cost paths are chosen deterministically.

### Line of effect and cover
Centre-to-centre line between the two spaces: through a wall ⇒ standard cover (+2); through another
creature ⇒ lesser cover (+1). No line of effect only if the centre line *and* all four corner lines
are blocked. Take Cover upgrades standard → greater (+4). The GM can override per Strike.

### Flanking
A Liang–Barsky segment/rectangle test: the line between the attacker's and an ally's centres must
pass through the interior of the target's space, and both must be able to act and threaten the target
with melee reach.

### Concealment
Concealed (or the target in obscuring terrain, or the attacker dazzled) ⇒ DC 5 flat check;
hidden/invisible (or the attacker blinded) ⇒ DC 11. A failed check still counts toward MAP.

### Areas
Bursts are centred on a grid intersection; a square is included when its distance (5-10-5) from the
intersection is within the radius (a 10-ft burst covers 12 squares). Emanations measure from the
caster's space; cones are a 90° wedge toward the aim point; lines follow the aim segment.

### Turn structure
* **Initiative**: Perception (+ bonuses); ties go to the PCs' adversaries, then lower id.
* **Start of turn**: expire "until start of turn" effects, reset MAP and reaction, actions =
  3 (+1 quickened) − stunned (consumed) or − slowed; dying creatures attempt a recovery check
  (flat DC 10 + dying) and their turn ends.
* **End of turn**: each persistent damage (roll → apply → DC 15 flat check to end), frightened −1,
  expire "until end of turn" effects.
* **Damage to 0 HP**: monsters die; creatures with dying rules gain dying 1 (2 on a crit) + wounded
  and fall unconscious; dying ≥ 4 − doomed kills; damage ≥ 2 × max HP kills outright. Healing above 0
  removes dying (wounded +1) and unconscious.

### Durations
Rules-text durations become concrete expiries: *N rounds* → start of the source's turn in
`round + N`; *until the end of the target's next turn*; *until the start of the source's next turn*.

### Encounter budget
XP by level difference (−4 → 10 … +4 → 160; simple hazards one fifth), compared with
40/60/80/120/160 (±10/15/20/30/40 per PC beyond four) → trivial … extreme.

## 4. Character builder

`builder::build(&Character, &Content) -> BuildResult { report, attributes, proficiencies, statblock }`

Validation is exhaustive (it never stops at the first error) and covers: ancestry fixed/free
boosts, flaws, the two-free-boosts alternative, background boost options, class key attribute, four
distinct free boosts, +4 cap at level 1, level 5/10/15/20 boosts with partial boosts above +4,
trained-skill count (class + Int), skill increase levels and rank gates (master 7+, legendary 15+),
feat slot existence/type/level, prerequisites (feats taken at an earlier slot, skill ranks,
attributes, class, ancestry), duplicates, archetype dedication order and the
"two feats before another dedication" rule, rune limits and item levels, spell tradition, cantrip
count, slot counts per rank.

Derivation computes HP, AC (Dex cap, armor proficiency, potency), Speed (armor Strength), saves
(resilient), Perception, every skill (armor check penalty), Strikes for each weapon (finesse, ranged
Dex, thrown and propulsive Strength, striking dice, weapon specialization) plus a fist, class DC,
spellcasting (attack, DC, slots), reactions, shield and sneak attack.

## 5. Example: driving a combat from Rust

```rust
use pf2e_core::builder::content::Content;
use pf2e_core::combat::*;

let content = Content::bundled()?;
let mut combat = CombatSetup::default().build(battlefield, participants, &content)?;
let mut log = Vec::new();

let mut run = |s: &mut CombatState, cmd: Command| -> Result<(), CombatError> {
    for e in s.apply(&cmd)? { println!("{e:?}"); }
    log.push(cmd);
    Ok(())
};

run(&mut combat, Command::StartCombat)?;
while let Some(PendingInput::Roll(r)) = combat.pending() {
    println!("{}", r.prompt);                       // "Roll 1d20+6 for initiative (Kira) …"
    let natural = read_die_from_user();
    run(&mut combat, Command::ProvideRoll { request_id: r.request_id, value: natural, extra_modifier: 0 })?;
}
let actor = combat.current_actor().unwrap();
run(&mut combat, Command::Act { actor, action: Action::Stride { to: Pos::new(4, 6) } })?;

// Reproducibility
let (replayed, _) = replay(&initial_state, &log)?;
assert_eq!(replayed.fingerprint(), combat.fingerprint());
```

## 6. Rules coverage and known simplifications

Implemented: Strike, Stride, Step, Stand, Raise a Shield, Take Cover, Demoralize, Trip,
Cast a Spell (attack / basic save / non-basic save / automatic / healing / buff, single target,
multi-target and areas, heightening, cantrip auto-heightening), Disable a Device; Reactive Strike
(move, Stand, manipulate with disruption), Shield Block; 25 conditions; persistent damage; dying,
wounded, doomed; MAP; cover and concealment; flanking; IWR including precision; simple and complex
hazards; elite/weak templates; encounter budgets.

Deliberate simplifications (each is a roadmap item):

* Content is a **sample** (5 ancestries, 6 backgrounds, 4 classes, ~25 feats, 15 weapons,
  11 spells). Class progressions were entered from the Player Core tables and should be verified
  before relying on them; they live in `content/classes.json`.
* Activated feats (Sudden Charge, Double Slice, …) validate in the builder but have no combat action yet.
* Prepared casters are modelled as a slot pool per rank (any listed spell of that rank can use a slot).
* Not yet modelled: Delay/Ready, Interact/draw, Aid, Grapple/Shove/Escape, Seek/Hide/Sneak (set
  hidden/invisible via GM tools), the stupefied casting flat check, initiative reordering when
  knocked out, squeezing, lighting, elevation, persistent-damage heightening.
* Cones are approximated by a 90° wedge.
