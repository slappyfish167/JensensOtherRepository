//! Ready-made combat states built from the bundled sample content, for
//! tests, benchmarks and the WASM smoke test. Small and allocation-only.

use pf2e_core::builder::{build, content::Content};
use pf2e_core::combat::{CombatSetup, CombatState, CombatantKind, Command, PendingInput, Phase, SetupParticipant};
use pf2e_core::models::encounter::{Encounter, ParticipantSource};
use pf2e_core::models::Character;
use pf2e_core::rng::{AnswerRoll, SeededDice};
use std::collections::HashMap;

const CHARACTERS: [&str; 4] = [
    include_str!("../../../content/characters/fighter.json"),
    include_str!("../../../content/characters/rogue.json"),
    include_str!("../../../content/characters/cleric.json"),
    include_str!("../../../content/characters/wizard.json"),
];
const CRYPT_AMBUSH: &str = include_str!("../../../content/encounters/crypt-ambush.json");

/// The bundled *Crypt Ambush* encounter in `Phase::Setup`.
pub fn crypt_ambush_setup() -> CombatState {
    let content = Content::bundled().expect("bundled content");
    let encounter: Encounter = serde_json::from_str(CRYPT_AMBUSH).expect("sample encounter");
    let characters: HashMap<String, Character> = CHARACTERS
        .iter()
        .map(|s| serde_json::from_str::<Character>(s).expect("sample character"))
        .map(|c| (c.id.clone(), c))
        .collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let parts = encounter
        .participants
        .iter()
        .map(|p| {
            let (statblock, kind) = match &p.source {
                ParticipantSource::Character(id) => {
                    (build(&characters[id], &content).statblock.expect("legal sample build"), CombatantKind::Character)
                }
                ParticipantSource::Monster(id) => {
                    let m = content.monsters.iter().find(|m| &m.id == id).expect("sample monster");
                    let kind = if m.is_hazard() { CombatantKind::Hazard } else { CombatantKind::Monster };
                    (m.statblock.clone(), kind)
                }
                ParticipantSource::Inline(sb) => ((**sb).clone(), CombatantKind::Monster),
            };
            let n = seen.entry(statblock.name.clone()).or_default();
            *n += 1;
            SetupParticipant {
                name: format!("{} {}", statblock.name, n),
                side: p.side,
                kind,
                statblock,
                position: p.position,
                current_hp: p.current_hp,
            }
        })
        .collect();
    CombatSetup::default().build(encounter.battlefield, parts, &content).expect("sample encounter builds")
}

/// *Crypt Ambush* with initiative rolled (seeded) and the first turn waiting.
pub fn crypt_ambush_active(seed: u64) -> CombatState {
    let mut s = crypt_ambush_setup();
    s.apply(&Command::StartCombat).expect("start");
    let mut dice = SeededDice::new(seed);
    while s.phase == Phase::RollingInitiative {
        match s.pending() {
            Some(PendingInput::Roll(r)) => {
                s.apply(&Command::ProvideRoll { request_id: r.request_id, value: dice.answer(&r), extra_modifier: 0 }).expect("roll");
            }
            _ => break,
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Engine state → snapshot (what the Foundry extractor would produce)
// ---------------------------------------------------------------------------

use crate::snapshot::*;
use pf2e_core::combat::LifeState;
use pf2e_core::models::battlefield::Terrain;
use pf2e_core::models::statblock::{DamageRider, ReactionKind, SpellcastingKind, StrikeRange};
fn formula(components: &[pf2e_core::models::statblock::DamageComponent], flat: i32) -> String {
    let parts: Vec<String> = components
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let dice = if i == 0 { c.dice.with_modifier(flat) } else { c.dice.clone() };
            let flavor = if c.precision { format!("precision,{}", c.damage_type.label()) } else { c.damage_type.label().to_string() };
            format!("({dice})[{flavor}]")
        })
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// Renders an engine state as a `CombatSnapshot`, field by field the way
/// the TypeScript extractor fills it from Foundry documents. Used for
/// round-trip tests and to generate the shared JSON fixture.
pub fn snapshot_from_state(s: &CombatState) -> CombatSnapshot {
    let token = |id: pf2e_core::CombatantId| format!("tok{:02}", id.0);
    let mut terrain = Vec::new();
    for y in 0..s.battlefield.height as i32 {
        for x in 0..s.battlefield.width as i32 {
            let kind = match s.battlefield.terrain(pf2e_core::grid::Pos::new(x, y)) {
                Terrain::Open => continue,
                Terrain::Difficult => TerrainKind::Difficult,
                Terrain::GreaterDifficult => TerrainKind::GreaterDifficult,
                Terrain::Obscured => TerrainKind::Obscured,
                Terrain::Wall => TerrainKind::Wall,
            };
            terrain.push(TerrainCell { x, y, kind });
        }
    }
    let combatants = s
        .combatants
        .iter()
        .filter(|c| c.life != LifeState::Dead)
        .map(|c| {
            let sb = &c.statblock;
            let flat: i32 = sb.damage_riders.iter().map(|r| if let DamageRider::FlatDamage { value, .. } = r { *value } else { 0 }).sum();
            let sneak = sb.damage_riders.iter().find_map(|r| if let DamageRider::SneakAttack { dice } = r { Some(dice.to_string()) } else { None });
            CombatantSnapshot {
                token_id: token(c.id),
                actor_id: format!("act{:02}", c.id.0),
                name: c.name.clone(),
                kind: c.kind,
                side: c.side,
                x: c.position.x,
                y: c.position.y,
                level: sb.level,
                size: sb.size,
                traits: sb.traits.clone(),
                hp: c.hp,
                max_hp: sb.max_hp,
                temp_hp: c.temp_hp,
                ac: sb.ac,
                fortitude: sb.fortitude,
                reflex: sb.reflex,
                will: sb.will,
                perception: sb.perception,
                speed: sb.speed,
                attributes: sb.attributes,
                skills: sb.skills.clone(),
                strikes: sb
                    .strikes
                    .iter()
                    .map(|st| StrikeSnapshot {
                        name: st.name.clone(),
                        attack_bonus: st.attack_bonus,
                        damage_formula: formula(&st.damage, flat),
                        damage_type: None,
                        traits: st.traits.iter().map(|t| t.to_string()).collect(),
                        melee: st.is_melee(),
                        reach_ft: match st.range {
                            StrikeRange::Melee { reach_ft } => Some(reach_ft),
                            _ => None,
                        },
                        range_increment_ft: match st.range {
                            StrikeRange::Ranged { increment_ft } => Some(increment_ft),
                            _ => None,
                        },
                        attribute: st.attribute,
                        damage_includes_conditions: false,
                    })
                    .collect(),
                spellcasting: sb
                    .spellcasting
                    .iter()
                    .enumerate()
                    .map(|(ei, e)| SpellcastingSnapshot {
                        name: e.name.clone(),
                        tradition: Some(format!("{:?}", e.tradition).to_lowercase()),
                        kind: match e.kind {
                            SpellcastingKind::Prepared => "prepared",
                            SpellcastingKind::Spontaneous => "spontaneous",
                            SpellcastingKind::Innate => "innate",
                        }
                        .into(),
                        attribute: e.attribute,
                        attack_bonus: e.attack_bonus,
                        dc: e.dc,
                        slots: c.spell_slots.get(ei).cloned().unwrap_or_default(),
                        spells: e
                            .cantrips
                            .iter()
                            .map(|id| (id, 1u8, true))
                            .chain(e.spells.iter().map(|k| (&k.spell_id, k.rank, false)))
                            .map(|(id, rank, cantrip)| {
                                let def = s.spells.get(id);
                                SpellSnapshot {
                                    slug: id.clone(),
                                    name: def.map(|d| d.name.clone()).unwrap_or_else(|| id.clone()),
                                    rank,
                                    base_rank: def.map(|d| d.rank),
                                    cantrip,
                                    actions: def.map(|d| d.actions),
                                    range_ft: def.and_then(|d| d.range_ft),
                                    area: None,
                                    target_count: None,
                                    targets_allies: false,
                                    targets_self: false,
                                    defense: None,
                                    damage: vec![],
                                    healing: None,
                                    heightening: None,
                                    traits: def.map(|d| d.traits.clone()).unwrap_or_default(),
                                }
                            })
                            .collect(),
                    })
                    .collect(),
                iwr: IwrSnapshot {
                    immunities: sb.iwr.immunities.iter().map(selector_name).collect(),
                    weaknesses: sb.iwr.weaknesses.iter().map(|w| IwrValueSnapshot { kind: selector_name(&w.applies_to), value: w.value }).collect(),
                    resistances: sb.iwr.resistances.iter().map(|w| IwrValueSnapshot { kind: selector_name(&w.applies_to), value: w.value }).collect(),
                },
                reactions: ReactionsSnapshot {
                    reactive_strike: sb.has_reaction(ReactionKind::ReactiveStrike),
                    shield_block: sb.has_reaction(ReactionKind::ShieldBlock),
                },
                shield: sb.shield.as_ref().map(|sh| ShieldSnapshot {
                    name: sh.name.clone(),
                    ac_bonus: sh.ac_bonus,
                    hardness: sh.hardness,
                    hp: c.shield_hp,
                    max_hp: sh.max_hp,
                    broken_threshold: sh.broken_threshold,
                    raised: c.has_tag(&pf2e_core::effects::EffectTag::ShieldRaised),
                }),
                conditions: c
                    .conditions
                    .0
                    .iter()
                    .map(|x| ConditionSnapshot { slug: x.kind.label().replace("offguard", "off-guard").replace('_', "-"), value: x.kind.is_valued().then_some(x.value) })
                    .collect(),
                persistent: c.persistent.iter().map(|p| PersistentSnapshot { formula: p.dice.to_string(), damage_type: p.damage_type.label().into() }).collect(),
                class_dc: sb.class_dc,
                sneak_attack: sneak,
                initiative: s.initiative.iter().find(|e| e.combatant == c.id).map(|e| e.value as f64),
                hazard: sb.hazard.as_ref().map(|h| HazardSnapshot {
                    complex: h.complex,
                    stealth_dc: h.stealth_dc,
                    disable: h.disable.iter().map(|d| DisableSnapshot { skill: d.skill, dc: d.dc }).collect(),
                }),
                reaction_used: !c.reaction_available,
                uses_dying_rules: sb.uses_dying_rules,
            }
        })
        .collect();
    let current = s.current_actor();
    CombatSnapshot {
        version: SNAPSHOT_VERSION,
        scene: SceneSnapshot {
            id: "scene-crypt".into(),
            name: s.battlefield.notes.chars().take(40).collect(),
            grid: GridKind::Square,
            grid_size_px: 100.0,
            grid_distance_ft: 5.0,
            origin_px: [0.0, 0.0],
            width: s.battlefield.width,
            height: s.battlefield.height,
            walls: vec![],
            terrain,
        },
        combatants,
        turn: TurnSnapshot {
            started: current.is_some(),
            round: s.round,
            current: current.map(token),
            order: s.initiative.iter().map(|e| token(e.combatant)).collect(),
            actions_spent: current.and_then(|id| s.combatant(id)).map(|c| 3u8.saturating_sub(c.actions_remaining)).unwrap_or(0),
            attacks_made: current.and_then(|id| s.combatant(id)).map(|c| c.attacks_this_turn).unwrap_or(0),
        },
        options: SnapshotOptions::default(),
    }
}

fn selector_name(s: &pf2e_core::damage::DamageSelector) -> String {
    use pf2e_core::damage::DamageSelector::*;
    match s {
        Type(t) => t.label().into(),
        Physical => "physical".into(),
        Energy => "energy".into(),
        Precision => "precision".into(),
        All => "all-damage".into(),
    }
}
