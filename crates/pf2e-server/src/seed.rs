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
    let (pf2e_empty, cpred_empty) = {
        let s = app.store();
        (
            s.list_monsters()?.is_empty() && s.list_characters()?.is_empty() && s.list_encounters()?.is_empty(),
            s.list_documents("cpred", "actor")?.is_empty() && s.list_documents("cpred", "encounter")?.is_empty(),
        )
    };
    if pf2e_empty {
        seed_pf2e(app)?;
        println!("seeded sample PF2e monsters, characters and an encounter");
    }
    // Databases created before multi-system support get the Cyberpunk RED samples too.
    if cpred_empty {
        seed_cpred(app)?;
        println!("seeded sample Cyberpunk RED actors and an encounter");
    }
    Ok(())
}

/// All sample content, every system.
#[cfg(test)]
pub fn seed(app: &App) -> StoreResult<()> {
    seed_pf2e(app)?;
    seed_cpred(app)
}

/// Cyberpunk RED sample actors and encounters, stored as documents.
pub fn seed_cpred(app: &App) -> StoreResult<()> {
    let store = app.store();
    for a in &app.cpred.actors {
        store.put_document("cpred", "actor", &a.id, &a.name, &serde_json::to_value(a)?)?;
    }
    for e in &app.cpred.encounters {
        store.put_document("cpred", "encounter", &e.id, &e.name, &serde_json::to_value(e)?)?;
    }
    Ok(())
}

pub fn seed_pf2e(app: &App) -> StoreResult<()> {
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
