//! The contract every rules engine fulfils. The server, the store and the
//! analysis layer are written against this trait only, so supporting a new
//! game system means implementing it — nothing else changes.

use crate::geom::Pos;
use crate::ids::CombatantId;
use crate::rng::SeededDice;
use crate::view::{ActionView, PhaseView, PromptView, TableView};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;

pub trait GameSystem: Sized + Send + Sync + 'static {
    /// Stable identifier used in URLs and storage, e.g. `pf2e`, `cpred`.
    const ID: &'static str;
    const NAME: &'static str;

    type State: Clone + PartialEq + Debug + Serialize + DeserializeOwned + Send + Sync + 'static;
    type Command: Clone + PartialEq + Debug + Serialize + DeserializeOwned + Send + Sync + 'static;
    type Action: Clone + PartialEq + Debug + Serialize + DeserializeOwned + Send + Sync + 'static;
    type Event: Clone + Debug + Serialize + Send + Sync + 'static;
    /// The system's own description of the input it waits for.
    type Pending: Clone + Debug + Serialize + Send + Sync + 'static;

    // ----- the rules engine -------------------------------------------------

    /// Applies a command atomically: on error the state is unchanged.
    fn apply(state: &mut Self::State, cmd: &Self::Command) -> Result<Vec<Self::Event>, String>;

    /// Fast path for search. On error the state may be partially modified;
    /// callers discard it.
    fn apply_in_place(state: &mut Self::State, cmd: &Self::Command) -> Result<(), String> {
        Self::apply(state, cmd).map(|_| ())
    }

    fn pending(state: &Self::State) -> Option<Self::Pending>;
    fn current_actor(state: &Self::State) -> Option<CombatantId>;
    fn phase(state: &Self::State) -> PhaseView;
    /// Current round (0 before combat starts).
    fn round(state: &Self::State) -> u32;
    fn is_over(state: &Self::State) -> bool {
        Self::phase(state).kind == "ended"
    }
    /// Every legal action for the creature whose turn it is (empty while input is pending).
    fn legal_actions(state: &Self::State) -> Vec<Self::Action>;

    // ----- command constructors ---------------------------------------------

    fn start_combat() -> Self::Command;
    fn act(actor: CombatantId, action: Self::Action) -> Self::Command;
    fn end_turn(actor: CombatantId) -> Self::Command;
    /// `faces` carries individual dice when the prompt asked for them.
    fn answer_roll(request_id: u32, value: i32, faces: Vec<i32>, extra_modifier: i32) -> Self::Command;
    fn decide(request_id: u32, accept: bool) -> Self::Command;
    /// The pending roll answered with seeded dice (None if no roll is pending).
    fn auto_answer(state: &Self::State, dice: &mut SeededDice) -> Option<Self::Command>;

    // ----- presentation -----------------------------------------------------

    fn table(state: &Self::State) -> TableView;
    fn prompt(state: &Self::State) -> Option<PromptView>;
    fn describe_action(state: &Self::State, actor: CombatantId, action: &Self::Action) -> String;
    fn action_group(action: &Self::Action) -> &'static str;
    fn action_move_to(_action: &Self::Action) -> Option<Pos> {
        None
    }
    fn describe_event(state: &Self::State, event: &Self::Event) -> Option<String>;

    fn action_views(state: &Self::State) -> Vec<ActionView> {
        let Some(actor) = Self::current_actor(state) else { return vec![] };
        Self::legal_actions(state)
            .into_iter()
            .map(|a| ActionView {
                description: Self::describe_action(state, actor, &a),
                group: Self::action_group(&a).to_string(),
                move_to: Self::action_move_to(&a),
                action: serde_json::to_value(&a).unwrap_or_default(),
            })
            .collect()
    }

    // ----- determinism ------------------------------------------------------

    /// Stable FNV-1a hash of the canonical JSON form; used to verify replays.
    fn fingerprint(state: &Self::State) -> String {
        fingerprint_json(&serde_json::to_string(state).unwrap_or_default())
    }

    /// Re-applies a command log. Error: (index of the failing command, reason).
    fn replay(initial: &Self::State, commands: &[Self::Command]) -> Result<(Self::State, Vec<Self::Event>), (usize, String)> {
        let mut s = initial.clone();
        let mut events = Vec::new();
        for (i, c) in commands.iter().enumerate() {
            events.extend(Self::apply(&mut s, c).map_err(|e| (i, e))?);
        }
        Ok((s, events))
    }
}

pub fn fingerprint_json(json: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in json.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}
