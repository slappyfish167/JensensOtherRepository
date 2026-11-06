//! The Cyberpunk RED adapter: `cpred-core` presented through the
//! system-neutral contracts ([`GameSystem`] for drivers and UIs,
//! [`TacticalSystem`] for the analysis layer). No rules live here — every
//! number comes from `cpred-core`'s rule and analysis functions.

use crate::ev;
use crate::movegen::{self, candidates, exposure, foes};
use cpred_core::analysis::representative_answers;
use cpred_core::combat::{Action, CombatState, Combatant, Command, Event, PendingInput, Phase, RollKind};
use cpred_core::model::{ActorKind, HitLocation, Stat};
use cpred_core::rules::METRES_PER_SQUARE;
use tactics_ai::{CandidateConfig, Counterfactual, Node, TacticalSystem, Unit};
use tactics_core::view::*;
use tactics_core::{CombatantId, GameSystem, Pos, SeededDice, Side};

/// Marker type for Cyberpunk RED.
#[derive(Debug, Clone, Copy, Default)]
pub struct CyberpunkRed;

fn location(l: HitLocation) -> &'static str {
    match l {
        HitLocation::Body => "body",
        HitLocation::Head => "head",
    }
}

fn tick(b: bool) -> &'static str {
    if b {
        "✓"
    } else {
        "✗"
    }
}

/// Full SP of a location from the statblock's armor (before ablation).
fn full_sp(c: &Combatant, l: HitLocation) -> i32 {
    match l {
        HitLocation::Body => c.statblock.armor.body.as_ref().map(|a| a.sp).unwrap_or(0),
        HitLocation::Head => c.statblock.armor.head.as_ref().map(|a| a.sp).unwrap_or(0),
    }
}

/// Status labels, death track included or not.
fn statuses(c: &Combatant, death_track: bool) -> Vec<String> {
    let mut out = Vec::new();
    if death_track {
        if c.is_dead() {
            out.push("dead".into());
        } else if c.is_mortally_wounded() {
            out.push(format!("mortally wounded (death save +{})", c.death_save_penalty));
        }
    }
    if c.alive && !c.is_mortally_wounded() && c.is_seriously_wounded() {
        out.push("seriously wounded".into());
    }
    for l in [HitLocation::Body, HitLocation::Head] {
        let n = c.injuries.iter().filter(|i| i.location == l).count();
        if n == 1 {
            out.push(format!("critical injury ({})", location(l)));
        } else if n > 1 {
            out.push(format!("{n} critical injuries ({})", location(l)));
        }
        let (now, full) = (c.armor_sp.get(l), full_sp(c, l));
        if now < full {
            out.push(format!("{} armor ablated to SP {now}", location(l)));
        }
    }
    out
}

fn turn_text(c: &Combatant) -> String {
    let e = c.economy;
    let mut t = format!("Move {} · Action {}", tick(e.move_available), tick(e.action_available));
    if e.shots_left > 0 {
        t.push_str(&format!(" · {} more shot{}", e.shots_left, if e.shots_left == 1 { "" } else { "s" }));
    }
    t
}

impl GameSystem for CyberpunkRed {
    const ID: &'static str = "cpred";
    const NAME: &'static str = "Cyberpunk RED";
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

    fn answer_roll(request_id: u32, value: i32, faces: Vec<i32>, extra_modifier: i32) -> Command {
        Command::ProvideRoll { request_id, value, faces, extra_modifier }
    }

    fn decide(request_id: u32, accept: bool) -> Command {
        Command::Decide { request_id, accept }
    }

    fn auto_answer(state: &CombatState, dice: &mut SeededDice) -> Option<Command> {
        match state.pending()? {
            PendingInput::Roll(r) => Some(match &r.kind {
                RollKind::Damage { dice: d } => Command::faces(r.request_id, dice.faces(d)),
                _ => Command::roll(r.request_id, dice.die(10) as i32),
            }),
            PendingInput::Decision(_) => None,
        }
    }

    fn fingerprint(state: &CombatState) -> String {
        state.fingerprint()
    }

    fn table(s: &CombatState) -> TableView {
        let legend = [('.', "open", "open"), ('~', "difficult", "difficult terrain (2 squares)"), ('#', "wall", "wall (blocks movement and sight)"), ('%', "cover", "cover object (blocks shots, has HP)")]
            .into_iter()
            .map(|(g, c, l)| LegendEntry { glyph: g.to_string(), class: c.into(), label: l.into() })
            .collect();
        let combatants = s
            .combatants
            .iter()
            .map(|c| {
                let sb = &c.statblock;
                let stat = |st: Stat| c.effective_stat(st);
                let mut stats = vec![
                    StatLine::new("REF · DEX · BODY", format!("{} · {} · {}", stat(Stat::Ref), stat(Stat::Dex), sb.stats.body)),
                    StatLine::new("MOVE", format!("{} squares", stat(Stat::Move))),
                    StatLine::new("SP body / head", format!("{}/{} · {}/{}", c.armor_sp.body, full_sp(c, HitLocation::Body), c.armor_sp.head, full_sp(c, HitLocation::Head))),
                    StatLine::new("Evasion", format!("{:+}", s.evasion_modifier(c.id).0)),
                ];
                for (i, w) in sb.weapons.iter().enumerate() {
                    let ammo = if w.magazine > 0 { format!(" · {}/{} rds", c.ammo.get(i).copied().unwrap_or(0), w.magazine) } else { String::new() };
                    let auto = w.autofire.map(|m| format!(" · autofire ×{m}")).unwrap_or_default();
                    stats.push(StatLine::new(w.name.clone(), format!("{:+} · {} · ROF {}{ammo}{auto}", s.attack_modifier(c.id, i, cpred_core::rules::AttackMode::Single).0, w.damage, w.rof)));
                }
                UnitView {
                    id: c.id,
                    name: c.name.clone(),
                    side: c.side,
                    kind: match c.kind {
                        ActorKind::Pc => "pc",
                        ActorKind::Npc => "npc",
                    }
                    .into(),
                    position: c.position,
                    squares: 1,
                    hp: c.hp,
                    max_hp: c.max_hp(),
                    temp_hp: 0,
                    down: c.is_down(),
                    dead: c.is_dead(),
                    statuses: statuses(c, true),
                    stats,
                }
            })
            .collect();
        let mut rows = s.battlefield.map.clone();
        // Destroyed cover is already open floor in the engine's map.
        rows.truncate(s.battlefield.height as usize);
        TableView {
            system: Self::ID.into(),
            system_name: Self::NAME.into(),
            phase: Self::phase(s),
            round: s.round,
            current_actor: s.current_actor(),
            map: MapView { width: s.battlefield.width, height: s.battlefield.height, rows, legend, square: format!("{METRES_PER_SQUARE} m") },
            combatants,
            initiative: s.initiative.iter().map(|e| InitiativeView { combatant: e.combatant, value: e.value }).collect(),
            turn: s.current_actor().and_then(|id| s.combatant(id)).map(turn_text),
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
                    RollKind::Damage { dice } => InputSpec::Faces { count: dice.dice_count(), sides: dice.terms.first().map(|t| t.sides).unwrap_or(6) },
                    _ => InputSpec::Die { sides: 10 },
                }),
                breakdown: r.breakdown.iter().map(|(label, v)| StatLine::new(label.clone(), format!("{v:+}"))).collect(),
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
                accept_label: Some("Dodge".into()),
                decline_label: Some("Take the DV".into()),
            },
        })
    }

    fn describe_action(state: &CombatState, actor: CombatantId, action: &Action) -> String {
        let base = state.describe_action(actor, action);
        match action {
            Action::Move { to } | Action::Run { to } => {
                let Some(a) = state.combatant(actor) else { return base };
                let f = foes(state, a, None);
                if f.is_empty() {
                    return base;
                }
                let shooters = f.iter().filter(|x| x.statblock.weapons.iter().any(|w| !w.melee)).count();
                match exposure(state, &f, *to) {
                    _ if shooters == 0 => base,
                    0 => format!("{base}, into full cover"),
                    n if n < shooters => format!("{base}, covered from {} of {shooters} shooters", shooters - n),
                    _ => base,
                }
            }
            _ => base,
        }
    }

    fn action_group(action: &Action) -> &'static str {
        match action {
            Action::Attack { .. } | Action::AimedShot { .. } | Action::Autofire { .. } => "Attacks",
            Action::Move { .. } | Action::Run { .. } => "Movement",
            Action::Reload { .. } => "Other",
        }
    }

    fn action_move_to(action: &Action) -> Option<Pos> {
        action.move_to()
    }

    fn describe_event(s: &CombatState, e: &Event) -> Option<String> {
        let n = |id: &CombatantId| s.name_of(*id);
        Some(match e {
            Event::CombatStarted => "Combat begins — roll initiative (REF + 1d10).".into(),
            Event::InitiativeOrder { order } => format!("Initiative: {}", order.iter().map(|(id, v)| format!("{} {v}", n(id))).collect::<Vec<_>>().join(", ")),
            Event::RoundStarted { round } => format!("— Round {round} —"),
            Event::RoundEnded { .. } => return None,
            Event::TurnStarted { combatant } => format!("{}'s turn", n(combatant)),
            Event::TurnEnded { combatant } => format!("{} ends their turn", n(combatant)),
            Event::ActionDeclared { actor, action } => format!("{}: {action}", n(actor)),
            Event::Moved { .. } => return None,
            Event::CheckResolved { roller, purpose, natural, total, beat, success } => match (beat, success) {
                (Some(b), Some(ok)) => format!("{purpose}: {natural} → {total} vs {b} — {}", if *ok { "success" } else { "failure" }),
                _ => format!("{} {purpose}: {natural} → {total}", roller.as_ref().map(n).unwrap_or_default()),
            },
            Event::DodgeDeclared { combatant, attacker } => format!("{} dodges {}'s attack", n(combatant), n(attacker)),
            Event::Missed { attacker, target } => format!("{} misses {}", n(attacker), n(target)),
            Event::DamageRolled { purpose, faces, total } => format!("{purpose}: {} = {total}", faces.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(" ")),
            Event::HitLocation { target, location: l } => format!("hit {}'s {}", n(target), location(*l)),
            Event::ArmorStopped { target, location: l, damage, sp } => format!("{}'s {} armor (SP {sp}) stops {damage}", n(target), location(*l)),
            Event::ArmorPenetrated { target, location: l, sp_before, sp_after, .. } => format!("{}'s {} armor penetrated — SP {sp_before} → {sp_after}", n(target), location(*l)),
            Event::HpLost { target, amount, hp } => format!("{} loses {amount} HP → {hp}", n(target)),
            Event::CriticalInjury { target, location: l, note } => format!("CRITICAL INJURY: {} ({}) — {note}", n(target), location(*l)),
            Event::CoverHit { pos, damage, hp } => format!("the cover at ({}, {}) absorbs {damage} → {hp} HP", pos.x, pos.y),
            Event::CoverDestroyed { pos } => format!("the cover at ({}, {}) is destroyed", pos.x, pos.y),
            Event::Reloaded { combatant, ammo, .. } => format!("{} reloads ({ammo} rounds)", n(combatant)),
            Event::DeathSave { combatant, face, threshold, success } => format!("{} death save: {face} (needs ≤ {threshold}) — {}", n(combatant), if *success { "survives" } else { "fails" }),
            Event::MortallyWounded { combatant } => format!("{} is mortally wounded!", n(combatant)),
            Event::Died { combatant } => format!("{} dies", n(combatant)),
            Event::CombatEnded { winner } => match winner {
                Some(Side::Party) => "Combat over — the crew wins.".into(),
                Some(_) => "Combat over — the opposition wins.".into(),
                None => "Combat over.".into(),
            },
            Event::Note { text } => text.clone(),
        })
    }
}

impl TacticalSystem for CyberpunkRed {
    fn units(s: &CombatState) -> Vec<Unit> {
        s.combatants
            .iter()
            .map(|c| Unit {
                id: c.id,
                name: c.name.clone(),
                side: c.side,
                is_pc: c.kind == ActorKind::Pc,
                ignored: false,
                down: c.is_down(),
                dead: c.is_dead(),
                hp: c.hp,
                max_hp: c.max_hp(),
            })
            .collect()
    }

    fn hp(s: &CombatState, id: CombatantId) -> i32 {
        s.combatant(id).map(|c| c.hp.max(0)).unwrap_or(0)
    }

    fn is_down(s: &CombatState, id: CombatantId) -> bool {
        s.combatant(id).map(|c| c.is_down()).unwrap_or(true)
    }

    fn status_labels(s: &CombatState, id: CombatantId) -> Vec<String> {
        s.combatant(id).map(|c| statuses(c, false)).unwrap_or_default()
    }

    fn node(s: &CombatState) -> Node<Command> {
        match s.pending() {
            None => Node::Ready,
            Some(PendingInput::Roll(_)) => {
                let outs = representative_answers(s);
                if outs.is_empty() {
                    Node::Stuck
                } else {
                    Node::Chance(outs)
                }
            }
            Some(PendingInput::Decision(d)) => Node::Decision {
                decider: d.decider,
                accept: Command::Decide { request_id: d.request_id, accept: true },
                decline: Command::Decide { request_id: d.request_id, accept: false },
            },
        }
    }

    fn turn_open(s: &CombatState, actor: CombatantId) -> bool {
        s.combatant(actor).map(|c| c.alive && (c.economy.move_available || c.economy.action_available || c.economy.shots_left > 0)).unwrap_or(false)
    }

    /// Once the Action is spent, the rest (the Move Action, a second ROF
    /// shot) is optional: standing still behind cover is a real choice.
    fn may_stop(s: &CombatState, actor: CombatantId) -> bool {
        s.combatant(actor).map(|c| !c.economy.action_available).unwrap_or(true)
    }

    fn candidates(s: &CombatState, cfg: &CandidateConfig) -> Vec<Action> {
        candidates(s, cfg)
    }

    fn action_target(a: &Action) -> Option<CombatantId> {
        a.attack().map(|(_, t, _)| t)
    }

    fn action_kind(a: &Action) -> &'static str {
        a.name()
    }

    fn plan_notes(s: &CombatState, actor: CombatantId, actions: &[Action]) -> Vec<String> {
        let Some(a) = s.combatant(actor) else { return vec![] };
        let f = foes(s, a, None);
        let shooters = f.iter().filter(|x| x.statblock.weapons.iter().any(|w| !w.melee)).count();
        let mut notes = Vec::new();
        if let Some(to) = actions.iter().rev().find_map(|x| x.move_to()) {
            let exposed = exposure(s, &f, to);
            if shooters > 0 && exposed == 0 {
                notes.push("ends the turn in full cover".into());
            } else if exposed < exposure(s, &f, a.position) {
                notes.push(format!("ends the turn exposed to {exposed} of {shooters} shooters instead of {}", exposure(s, &f, a.position)));
            }
        }
        if actions.iter().any(|x| matches!(x, Action::AimedShot { .. })) {
            notes.push("an aimed shot doubles damage past head armor".into());
        }
        if actions.iter().any(|x| matches!(x, Action::Reload { .. })) {
            notes.push("reloads so the next turn can fire".into());
        }
        notes
    }

    fn effective_hp(s: &CombatState, id: CombatantId) -> f64 {
        s.combatant(id).map(|c| c.hp as f64).unwrap_or(0.0)
    }

    fn round_damage(s: &CombatState, attacker: CombatantId, target: CombatantId, include_resources: bool) -> f64 {
        ev::round_damage(s, attacker, target, include_resources)
    }

    fn engagement(s: &CombatState, attacker: CombatantId, target: CombatantId) -> f64 {
        ev::engagement(s, attacker, target)
    }

    fn action_factor(s: &CombatState, id: CombatantId) -> f64 {
        s.combatant(id).map(|c| if c.can_act() { 1.0 } else { 0.0 }).unwrap_or(0.0)
    }

    fn tempo_fraction(s: &CombatState, actor: CombatantId) -> f64 {
        s.combatant(actor)
            .map(|c| {
                let e = c.economy;
                if e.action_available {
                    1.0
                } else if e.shots_left > 0 {
                    let rof = e.rof_weapon.and_then(|w| c.statblock.weapons.get(w)).map(|w| w.rof.max(1)).unwrap_or(1) as f64;
                    e.shots_left as f64 / rof
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0)
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
                name: "Cover",
                detail: "cover objects blocking shots",
                apply: |s, _| {
                    for c in s.cover.clone() {
                        s.battlefield.set_terrain(c.pos, cpred_core::model::Terrain::Open);
                    }
                    s.cover.clear();
                },
            },
            Counterfactual {
                name: "Armor",
                detail: "armor ablated below its full SP",
                apply: |s, _| {
                    for c in &mut s.combatants {
                        c.armor_sp.body = full_sp(c, HitLocation::Body);
                        c.armor_sp.head = full_sp(c, HitLocation::Head);
                    }
                },
            },
            Counterfactual {
                name: "Ammunition",
                detail: "empty magazines that cost an Action to reload",
                apply: |s, _| {
                    for c in &mut s.combatants {
                        for (i, w) in c.statblock.weapons.iter().enumerate() {
                            if let Some(a) = c.ammo.get_mut(i) {
                                *a = w.magazine;
                            }
                        }
                    }
                },
            },
            Counterfactual {
                name: "Wounds",
                detail: "seriously wounded −2 penalties",
                apply: |s, _| {
                    for c in &mut s.combatants {
                        if c.alive && !c.is_mortally_wounded() && c.is_seriously_wounded() {
                            c.hp = cpred_core::rules::seriously_wounded_threshold(c.max_hp());
                        }
                    }
                },
            },
        ]
    }

    fn kill_probability(s: &CombatState, attacker: CombatantId, target: CombatantId) -> f64 {
        ev::kill_probability(s, attacker, target)
    }

    fn hypothetical_turn(s: &CombatState, id: CombatantId) -> Option<CombatState> {
        s.hypothetical_turn(id)
    }

    fn scripted_action(s: &CombatState) -> Option<Action> {
        movegen::scripted_action(s)
    }

    fn turn_budget(s: &CombatState, actor: CombatantId) -> Option<String> {
        s.combatant(actor).map(turn_text)
    }
}
