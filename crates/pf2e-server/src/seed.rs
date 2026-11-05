//! Sample data inserted into an empty database so the app is usable at once.

use crate::app::App;
use pf2e_core::models::{Character, Encounter};
use pf2e_store::StoreResult;

const CHARACTERS: [&str; 4] = [
    include_str!("../../../content/characters/fighter.json"),
    include_str!("../../../content/characters/rogue.json"),
    include_str!("../../../content/characters/cleric.json"),
    include_str!("../../../content/characters/wizard.json"),
];
const ENCOUNTERS: [&str; 1] = [include_str!("../../../content/encounters/crypt-ambush.json")];

pub fn seed_if_empty(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let empty = {
        let s = app.store();
        s.list_monsters()?.is_empty() && s.list_characters()?.is_empty() && s.list_encounters()?.is_empty()
    };
    if !empty {
        return Ok(());
    }
    seed(app)?;
    println!("seeded sample monsters, characters and an encounter");
    Ok(())
}

pub fn seed(app: &App) -> StoreResult<()> {
    for m in &app.content.monsters {
        app.store().put_monster(m)?;
    }
    for text in CHARACTERS {
        let c: Character = serde_json::from_str(text)?;
        let legal = app.build_character(&c).report.is_legal();
        app.store().put_character(&c, legal)?;
    }
    for text in ENCOUNTERS {
        let e: Encounter = serde_json::from_str(text)?;
        app.store().put_encounter(&e)?;
    }
    Ok(())
}
