//! The Pathfinder 2e adapter: `pf2e-core` presented through the
//! system-neutral contracts ([`GameSystem`] for drivers and UIs,
//! [`TacticalSystem`] for the analysis layer). No rules live here — every
//! number comes from `pf2e-core` or the expected-value helpers in
//! [`crate::analysis`], which are themselves built on core rule functions.

use crate::analysis::{action_factor, engagement_factor, round_damage};
use crate::movegen::{candidates, MoveGenConfig};
use crate::queries::{describe_action, flanked_from, kill_probability};
use pf2e_core::checks::representative_faces;
use pf2e_core::combat::{Action, CombatState, Command, Event, PendingInput, Phase, RollKind};
use pf2e_core::conditions::ConditionKind as CK;
use pf2e_core::rng::AnswerRoll;
use tactics_ai::{CandidateConfig, Counterfactual, EvalOptions, Node, TacticalSystem, Unit};
use tactics_core::view::*;
use tactics_core::{CombatantId, GameSystem, Pos, SeededDice, Side};

/// Marker type for Pathfinder Second Edition.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pf2e;

/// Answer for a PF2e roll request, weighted (one face per degree of success).
pub fn roll_outcomes(r: &pf2e_core::combat::RollRequest) -> Vec<(i32, f64)> {
    match &r.kind {
        RollKind::Check { modifier, dc } => representative_faces(*modifier, *dc).into_iter().map(|(f, _, p)| (f as i32, p)).collect(),
        RollKind::Flat { dc } => representative_faces(0, *dc).into_iter().map(|(f, _, p)| (f as i32, p)).collect(),
        RollKind::Total { dice } => vec![((dice.average().round() as i32).clamp(dice.min(), dice.max()), 1.0)],
    }
}

fn label(kind: CK) -> String {
    kind.label().replace("offguard", "off-guard")
}

impl GameSystem for Pf2e {
    const ID: &'static str = "pf2e";
    const NAME: &'static str = "Pathfinder 2e";
    type State = CombatState;
    type Command = Command;
    type Action = Action;
    type Event = Event;
    type Pending = PendingInput;

    fn apply(state: &mut CombatState, cmd: &Command) -> Result<Vec<Event>, String> {
        state.apply(cmd).map_err(|e| e.to_string())
    }

    fn apply_in_place(state: &mut CombatState, cmd: &Command) -> Result<(), String> {
        state.apply_in_place(cmd).map(|_| ()).map_err(|e| e.to_string())
    }

    fn pending(state: &CombatState) -> Option<PendingInput> {
        state.pending()
    }

    fn current_actor(state: &CombatState) -> Option<CombatantId> {
        state.current_actor()
    }

    fn phase(state: &CombatState) -> PhaseView {
        match state.phase {
            Phase::Setup => PhaseView { kind: "setup".into(), winner: None },
            Phase::RollingInitiative => PhaseView { kind: "rolling_initiative".into(), winner: None },
            Phase::Active => PhaseView { kind: "active".into(), winner: None },
            Phase::Ended { winner } => PhaseView { kind: "ended".into(), winner },
        }
    }

    fn round(state: &CombatState) -> u32 {
        state.round
    }

    fn is_over(state: &CombatState) -> bool {
        state.is_over()
    }

    fn legal_actions(state: &CombatState) -> Vec<Action> {
        state.legal_actions()
    }

    fn start_combat() -> Command {
        Command::StartCombat
    }

    fn act(actor: CombatantId, action: Action) -> Command {
        Command::Act { actor, action }
    }

    fn end_turn(actor: CombatantId) -> Command {
        Command::EndTurn { actor }
    }

    fn answer_roll(request_id: u32, value: i32, _faces: Vec<i32>, extra_modifier: i32) -> Command {
        Command::ProvideRoll { request_id, value, extra_modifier }
    }

    fn decide(request_id: u32, accept: bool) -> Command {
        Command::Decide { request_id, accept }
    }

    fn auto_answer(state: &CombatState, dice: &mut SeededDice) -> Option<Command> {
        match state.pending()? {
            PendingInput::Roll(r) => Some(Command::ProvideRoll { request_id: r.request_id, value: dice.answer(&r), extra_modifier: 0 }),
            PendingInput::Decision(_) => None,
        }
    }

    fn fingerprint(state: &CombatState) -> String {
        state.fingerprint()
    }

    fn table(s: &CombatState) -> TableView {
        let legend = [('.', "open", "open"), ('~', "difficult", "difficult terrain"), ('^', "greater", "greater difficult terrain"), ('#', "wall", "wall"), ('%', "obscured", "obscured (concealment)")]
            .into_iter()
            .map(|(g, c, l)| LegendEntry { glyph: g.to_string(), class: c.into(), label: l.into() })
            .collect();
        let combatants = s
            .combatants
            .iter()
            .map(|c| {
                let sb = &c.statblock;
                let mut statuses: Vec<String> = c.conditions.0.iter().map(|k| if k.kind.is_valued() { format!("{} {}", label(k.kind), k.value) } else { label(k.kind) }).collect();
                statuses.extend(c.effects.iter().map(|e| e.name.clone()));
                statuses.extend(c.persistent.iter().map(|p| format!("persistent {} {}", format!("{:?}", p.damage_type).to_lowercase(), p.dice)));
                let mut stats = vec![
                    StatLine::new("Level", sb.level),
                    StatLine::new("AC", sb.ac),
                    StatLine::new("Saves", format!("Fort {:+} · Ref {:+} · Will {:+}", sb.fortitude, sb.reflex, sb.will)),
                    StatLine::new("Speed", format!("{} ft", sb.speed)),
                ];
                for st in &sb.strikes {
                    stats.push(StatLine::new(st.name.clone(), format!("{:+} · {}", st.attack_bonus, st.damage.iter().map(|d| d.dice.to_string()).collect::<Vec<_>>().join("+"))));
                }
                UnitView {
                    id: c.id,
                    name: c.name.clone(),
                    side: c.side,
                    kind: match c.kind {
                        pf2e_core::combat::CombatantKind::Character => "pc",
                        pf2e_core::combat::CombatantKind::Monster => "npc",
                        pf2e_core::combat::CombatantKind::Hazard => "hazard",
                    }
                    .into(),
                    position: c.position,
                    squares: sb.size.squares() as u32,
                    hp: c.hp,
                    max_hp: c.effective_max_hp(),
                    temp_hp: c.temp_hp,
                    down: c.is_down(),
                    dead: c.is_dead(),
                    statuses,
                    stats,
                }
            })
            .collect();
        let turn = s.current_actor().and_then(|id| s.combatant(id)).map(|c| {
            format!("{}{}{}", "◆".repeat(c.actions_remaining as usize), "◇".repeat(3usize.saturating_sub(c.actions_remaining as usize)), if c.reaction_available { " · reaction ready" } else { "" })
        });
        TableView {
            system: Self::ID.into(),
            system_name: Self::NAME.into(),
            phase: Self::phase(s),
            round: s.round,
            current_actor: s.current_actor(),
            map: MapView { width: s.battlefield.width, height: s.battlefield.height, rows: s.battlefield.map.clone(), legend, square: "5 ft".into() },
            combatants,
            initiative: s.initiative.iter().map(|e| InitiativeView { combatant: e.combatant, value: e.value }).collect(),
            turn,
        }
    }

    fn prompt(s: &CombatState) -> Option<PromptView> {
        Some(match s.pending()? {
            PendingInput::Roll(r) => PromptView {
                kind: "roll".into(),
                request_id: r.request_id,
                actor: r.roller,
                prompt: r.prompt.clone(),
                expression: r.expression.clone(),
                input: Some(match &r.kind {
                    RollKind::Check { .. } | RollKind::Flat { .. } => InputSpec::Die { sides: 20 },
                    RollKind::Total { dice } => InputSpec::Total { min: dice.min(), max: dice.max() },
                }),
                breakdown: r.breakdown.iter().map(|m| StatLine::new(m.source.clone(), format!("{:+}", m.value))).collect(),
                accept_label: None,
                decline_label: None,
            },
            PendingInput::Decision(d) => PromptView {
                kind: "decision".into(),
                request_id: d.request_id,
                actor: Some(d.decider),
                prompt: d.prompt.clone(),
                expression: String::new(),
                input: None,
                breakdown: vec![],
                accept_label: Some("Use reaction".into()),
                decline_label: Some("Decline".into()),
            },
        })
    }

    fn describe_action(state: &CombatState, actor: CombatantId, action: &Action) -> String {
        describe_action(state, actor, action)
    }

    fn action_group(action: &Action) -> &'static str {
        match action {
            Action::Strike { .. } => "Attacks",
            Action::CastSpell { .. } => "Spells",
            Action::Trip { .. } | Action::Demoralize { .. } | Action::DisableHazard { .. } => "Maneuvers",
            Action::Stride { .. } | Action::Step { .. } => "Movement",
            _ => "Other",
        }
    }

    fn action_move_to(action: &Action) -> Option<Pos> {
        match action {
            Action::Stride { to } | Action::Step { to } => Some(*to),
            _ => None,
        }
    }

    fn describe_event(s: &CombatState, e: &Event) -> Option<String> {
        let n = |id: &CombatantId| s.combatant(*id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_string());
        Some(match e {
            Event::CombatStarted => "Combat begins — roll initiative.".into(),
            Event::InitiativeOrder { order } => format!("Initiative: {}", order.iter().map(|(id, v)| format!("{} {v}", n(id))).collect::<Vec<_>>().join(", ")),
            Event::RoundStarted { round } => format!("— Round {round} —"),
            Event::TurnStarted { combatant, actions } => format!("{}'s turn ({actions} action{})", n(combatant), if *actions == 1 { "" } else { "s" }),
            Event::TurnEnded { combatant } => format!("{} ends their turn", n(combatant)),
            Event::ActionDeclared { actor, action, cost } => format!("{}: {action}{}", n(actor), if *cost > 1 { format!(" ({cost} actions)") } else { String::new() }),
            Event::Moved { .. } => return None,
            Event::CheckResolved { roller, purpose, natural, total, dc, degree } => {
                if *dc > 0 {
                    format!("{purpose}: {natural} → {total} vs {dc} — {}", format!("{degree:?}").to_lowercase())
                } else {
                    format!("{} {purpose}: {total}", roller.as_ref().map(n).unwrap_or_default())
                }
            }
            Event::FlatCheck { combatant, purpose, natural, dc, success } => format!("{} flat check ({purpose}) {natural} vs DC {dc}: {}", n(combatant), if *success { "success" } else { "failure" }),
            Event::DamageRolled { purpose, rolled, counted, .. } => format!("{purpose}: rolled {rolled}{}", if counted != rolled { format!(" → {counted}") } else { String::new() }),
            Event::DamageTaken { target, amount, hp, .. } => format!("{} takes {amount} damage → {hp} HP", n(target)),
            Event::ShieldBlocked { target, prevented, broken, .. } => format!("{} blocks {prevented} with a shield{}", n(target), if *broken { " — the shield breaks!" } else { "" }),
            Event::Healed { target, amount, hp } => format!("{} heals {amount} → {hp} HP", n(target)),
            Event::ConditionGained { target, condition, value } => format!("{} is {}{}", n(target), label(*condition), if *value > 0 { format!(" {value}") } else { String::new() }),
            Event::ConditionLost { target, condition } => format!("{} is no longer {}", n(target), label(*condition)),
            Event::EffectGained { target, name } => format!("{} gains {name}", n(target)),
            Event::EffectEnded { target, name } => format!("{name} ends on {}", n(target)),
            Event::ReactionUsed { reactor, reaction } => format!("{} uses {reaction}", n(reactor)),
            Event::ActionDisrupted { actor } => format!("{}'s action is disrupted!", n(actor)),
            Event::KnockedOut { combatant, dying } => format!("{} falls unconscious (dying {dying})", n(combatant)),
            Event::Died { combatant } => format!("{} dies", n(combatant)),
            Event::HazardTriggered { hazard, victim } => format!("{} triggers on {}!", n(hazard), n(victim)),
            Event::HazardDisabled { hazard } => format!("{} is disabled", n(hazard)),
            Event::CombatEnded { winner } => match winner {
                Some(Side::Party) => "Combat over — the party wins.".into(),
                Some(_) => "Combat over — the enemies win.".into(),
                None => "Combat over.".into(),
            },
            Event::Note { text } => text.clone(),
        })
    }
}

impl TacticalSystem for Pf2e {
    fn units(s: &CombatState) -> Vec<Unit> {
        s.combatants
            .iter()
            .map(|c| Unit {
                id: c.id,
                name: c.name.clone(),
                side: c.side,
                is_pc: c.kind == pf2e_core::combat::CombatantKind::Character,
                ignored: c.is_hazard(),
                down: c.is_down(),
                dead: c.is_dead(),
                hp: c.hp + c.temp_hp,
                max_hp: c.effective_max_hp(),
            })
            .collect()
    }

    fn hp(s: &CombatState, id: CombatantId) -> i32 {
        s.combatant(id).map(|c| c.hp.max(0) + c.temp_hp.max(0)).unwrap_or(0)
    }

    fn is_down(s: &CombatState, id: CombatantId) -> bool {
        s.combatant(id).map(|c| c.is_down()).unwrap_or(true)
    }

    fn status_labels(s: &CombatState, id: CombatantId) -> Vec<String> {
        let Some(c) = s.combatant(id) else { return vec![] };
        let down = c.is_down();
        let mut out: Vec<String> = c
            .conditions
            .0
            .iter()
            .filter(|k| !matches!(k.kind, CK::Dying | CK::Unconscious | CK::Wounded | CK::Doomed) && !(down && matches!(k.kind, CK::Prone | CK::OffGuard)))
            .map(|k| if k.kind.is_valued() { format!("{} {}", label(k.kind), k.value) } else { label(k.kind) })
            .collect();
        out.extend(c.persistent.iter().map(|p| format!("persistent {} {}", format!("{:?}", p.damage_type).to_lowercase(), p.dice)));
        out.extend(c.effects.iter().map(|e| e.name.clone()));
        out
    }

    fn node(s: &CombatState) -> Node<Command> {
        match s.pending() {
            None => Node::Ready,
            Some(PendingInput::Roll(r)) => {
                Node::Chance(roll_outcomes(&r).into_iter().map(|(value, p)| (Command::ProvideRoll { request_id: r.request_id, value, extra_modifier: 0 }, p)).collect())
            }
            Some(PendingInput::Decision(d)) => Node::Decision {
                decider: d.decider,
                accept: Command::Decide { request_id: d.request_id, accept: true },
                decline: Command::Decide { request_id: d.request_id, accept: false },
            },
        }
    }

    fn turn_open(s: &CombatState, actor: CombatantId) -> bool {
        s.combatant(actor).map(|a| a.actions_remaining > 0 && a.can_act()).unwrap_or(false)
    }

    fn candidates(s: &CombatState, cfg: &CandidateConfig) -> Vec<Action> {
        let mut c = candidates(s, &MoveGenConfig { max_moves: cfg.max_moves, focus: cfg.focus, committed: cfg.committed });
        order(&mut c);
        c
    }

    fn action_target(a: &Action) -> Option<CombatantId> {
        match a {
            Action::Strike { target, .. } | Action::Demoralize { target } | Action::Trip { target } => Some(*target),
            Action::CastSpell { targets, .. } => targets.first().copied(),
            _ => None,
        }
    }

    fn action_kind(a: &Action) -> &'static str {
        a.name()
    }

    fn plan_notes(s: &CombatState, actor: CombatantId, actions: &[Action]) -> Vec<String> {
        let flanks = actions.iter().any(|a| match a {
            Action::Stride { to } | Action::Step { to } => !flanked_from(s, actor, *to).is_empty(),
            _ => false,
        });
        if flanks {
            vec!["the move sets up a flank (target off-guard, −2 AC)".into()]
        } else {
            vec![]
        }
    }

    fn effective_hp(s: &CombatState, id: CombatantId) -> f64 {
        /// Expected ticks of persistent damage with a DC 15 flat check (1 / 0.3).
        const PERSISTENT_TICKS: f64 = 1.0 / 0.3;
        let Some(c) = s.combatant(id) else { return 0.0 };
        let persistent: f64 = c.persistent.iter().map(|p| p.dice.average() * PERSISTENT_TICKS).sum();
        (c.hp + c.temp_hp) as f64 - persistent
    }

    fn round_damage(s: &CombatState, attacker: CombatantId, target: CombatantId, include_resources: bool) -> f64 {
        round_damage(s, attacker, target, include_resources)
    }

    fn engagement(s: &CombatState, attacker: CombatantId, target: CombatantId) -> f64 {
        engagement_factor(s, attacker, target)
    }

    fn action_factor(s: &CombatState, id: CombatantId) -> f64 {
        s.combatant(id).map(action_factor).unwrap_or(0.0)
    }

    fn tempo_fraction(s: &CombatState, actor: CombatantId) -> f64 {
        s.combatant(actor).map(|c| c.actions_remaining as f64 / 3.0).unwrap_or(0.0)
    }

    fn winner(s: &CombatState) -> Option<Option<Side>> {
        match s.phase {
            Phase::Ended { winner } => Some(winner),
            _ => None,
        }
    }

    fn counterfactuals() -> Vec<Counterfactual<Self>> {
        vec![
            Counterfactual {
                name: "Conditions",
                detail: "effect of current conditions and effects (frightened, off-guard, bless, ...)",
                apply: |s, _| {
                    for c in &mut s.combatants {
                        c.conditions.0.retain(|x| matches!(x.kind, CK::Dying | CK::Unconscious | CK::Wounded | CK::Slowed | CK::Stunned | CK::Quickened));
                        c.effects.clear();
                        c.persistent.clear();
                    }
                },
            },
            Counterfactual {
                name: "Action economy",
                detail: "slowed / stunned / quickened and whose turn it is",
                apply: |s, o: &mut EvalOptions| {
                    o.include_tempo = false;
                    for c in &mut s.combatants {
                        c.conditions.0.retain(|x| !matches!(x.kind, CK::Slowed | CK::Stunned | CK::Quickened));
                    }
                },
            },
            Counterfactual {
                name: "Resources",
                detail: "remaining spell slots",
                apply: |s, _| {
                    for c in &mut s.combatants {
                        for slots in &mut c.spell_slots {
                            slots.values_mut().for_each(|v| *v = 0);
                        }
                    }
                },
            },
            Counterfactual {
                name: "Battlefield control",
                detail: "cover and concealment from terrain",
                apply: |s, _| {
                    s.options.auto_cover = false;
                    for row in &mut s.battlefield.map {
                        *row = row.replace('%', ".");
                    }
                },
            },
        ]
    }

    fn kill_probability(s: &CombatState, attacker: CombatantId, target: CombatantId) -> f64 {
        let Some(a) = s.combatant(attacker) else { return 0.0 };
        let actions = if s.current_actor() == Some(attacker) { a.actions_remaining } else { 3 };
        let actions = if engagement_factor(s, attacker, target) < 1.0 { actions.saturating_sub(1) } else { actions };
        kill_probability(s, attacker, target, actions)
    }

    fn hypothetical_turn(s: &CombatState, id: CombatantId) -> Option<CombatState> {
        s.hypothetical_turn(id)
    }

    fn scripted_action(s: &CombatState) -> Option<Action> {
        crate::simulate::scripted_action(s)
    }

    fn turn_budget(s: &CombatState, actor: CombatantId) -> Option<String> {
        s.combatant(actor).map(|c| format!("{} action{}", c.actions_remaining, if c.actions_remaining == 1 { "" } else { "s" }))
    }
}

/// Cheap move ordering so the beam keeps the likeliest-good actions.
pub fn order(c: &mut [Action]) {
    let rank = |a: &Action| match a {
        Action::Strike { .. } => 0,
        Action::CastSpell { .. } => 1,
        Action::Trip { .. } | Action::Demoralize { .. } => 2,
        Action::Stride { .. } => 3,
        Action::Step { .. } => 4,
        Action::RaiseShield | Action::Stand => 5,
        Action::TakeCover => 6,
        _ => 7,
    };
    c.sort_by_key(rank);
}
