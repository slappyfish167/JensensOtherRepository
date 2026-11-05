//! # pf2e-store
//!
//! Persistence behind a repository trait. The bundled implementation uses
//! SQLite; the schema (see `migrations/`) is written to be PostgreSQL
//! compatible so a `PgStore` can implement the same trait later.

use pf2e_core::combat::{CombatLog, CombatState, Command};
use pf2e_core::models::{Character, Encounter, Monster};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/001_init.sql"))];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0} not found")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterSummary {
    pub id: String,
    pub name: String,
    pub level: i64,
    pub ancestry_id: String,
    pub class_id: String,
    pub is_legal: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonsterSummary {
    pub id: String,
    pub name: String,
    pub level: i64,
    pub is_hazard: bool,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncounterSummary {
    pub id: String,
    pub name: String,
    pub party_level: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum DiceMode {
    /// Every roll is entered by a human (the default).
    Manual,
    /// Humans may ask the server to roll the pending request with a seeded
    /// generator; results are still recorded as ordinary commands.
    Seeded { seed: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatRecord {
    pub id: String,
    pub encounter_id: Option<String>,
    pub name: String,
    pub dice: DiceMode,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Repository interface used by the API layer.
pub trait Store: Send {
    fn list_characters(&self) -> StoreResult<Vec<CharacterSummary>>;
    fn get_character(&self, id: &str) -> StoreResult<Option<Character>>;
    fn put_character(&self, c: &Character, is_legal: bool) -> StoreResult<()>;
    fn delete_character(&self, id: &str) -> StoreResult<bool>;

    fn list_monsters(&self) -> StoreResult<Vec<MonsterSummary>>;
    fn get_monster(&self, id: &str) -> StoreResult<Option<Monster>>;
    fn put_monster(&self, m: &Monster) -> StoreResult<()>;
    fn delete_monster(&self, id: &str) -> StoreResult<bool>;

    fn list_encounters(&self) -> StoreResult<Vec<EncounterSummary>>;
    fn get_encounter(&self, id: &str) -> StoreResult<Option<Encounter>>;
    fn put_encounter(&self, e: &Encounter) -> StoreResult<()>;
    fn delete_encounter(&self, id: &str) -> StoreResult<bool>;

    fn create_combat(&self, rec: &CombatRecord, initial: &CombatState) -> StoreResult<()>;
    fn list_combats(&self) -> StoreResult<Vec<CombatRecord>>;
    fn get_combat(&self, id: &str) -> StoreResult<Option<(CombatRecord, CombatLog)>>;
    /// Appends at exactly `seq`; fails with `Conflict` if that slot is taken
    /// (optimistic concurrency for two browser tabs on one combat).
    fn append_command(&self, combat_id: &str, seq: usize, cmd: &Command) -> StoreResult<()>;
    /// Removes commands with `seq >= from` (undo).
    fn truncate_commands(&self, combat_id: &str, from: usize) -> StoreResult<()>;
    fn set_combat_status(&self, id: &str, status: &str) -> StoreResult<()>;
    fn delete_combat(&self, id: &str) -> StoreResult<bool>;
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> StoreResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> StoreResult<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let store = SqliteStore { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> StoreResult<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at BIGINT NOT NULL);",
        )?;
        for (version, sql) in MIGRATIONS {
            let done: Option<i64> = self
                .conn
                .query_row("SELECT version FROM schema_migrations WHERE version = ?1", [version], |r| r.get(0))
                .optional()?;
            if done.is_none() {
                self.conn.execute_batch(&format!("BEGIN; {sql}\nCOMMIT;"))?;
                self.conn.execute("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)", params![version, now()])?;
            }
        }
        Ok(())
    }

    pub fn schema_version(&self) -> StoreResult<i64> {
        Ok(self.conn.query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |r| r.get(0))?)
    }

    fn get_doc<T: serde::de::DeserializeOwned>(&self, table: &str, id: &str) -> StoreResult<Option<T>> {
        let sql = format!("SELECT data FROM {table} WHERE id = ?1");
        let text: Option<String> = self.conn.query_row(&sql, [id], |r| r.get(0)).optional()?;
        Ok(match text {
            Some(t) => Some(serde_json::from_str(&t)?),
            None => None,
        })
    }

    fn delete_row(&self, table: &str, id: &str) -> StoreResult<bool> {
        Ok(self.conn.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])? > 0)
    }
}

impl Store for SqliteStore {
    fn list_characters(&self) -> StoreResult<Vec<CharacterSummary>> {
        let mut st = self.conn.prepare(
            "SELECT id, name, level, ancestry_id, class_id, is_legal, updated_at FROM characters ORDER BY name, id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(CharacterSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                level: r.get(2)?,
                ancestry_id: r.get(3)?,
                class_id: r.get(4)?,
                is_legal: r.get::<_, i64>(5)? != 0,
                updated_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn get_character(&self, id: &str) -> StoreResult<Option<Character>> {
        self.get_doc("characters", id)
    }

    fn put_character(&self, c: &Character, is_legal: bool) -> StoreResult<()> {
        let t = now();
        self.conn.execute(
            "INSERT INTO characters (id, name, level, ancestry_id, class_id, is_legal, data, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, level = excluded.level,
               ancestry_id = excluded.ancestry_id, class_id = excluded.class_id, is_legal = excluded.is_legal,
               data = excluded.data, updated_at = excluded.updated_at",
            params![c.id, c.name, c.level as i64, c.ancestry_id, c.class_id, is_legal as i64, serde_json::to_string(c)?, t],
        )?;
        Ok(())
    }

    fn delete_character(&self, id: &str) -> StoreResult<bool> {
        self.delete_row("characters", id)
    }

    fn list_monsters(&self) -> StoreResult<Vec<MonsterSummary>> {
        let mut st = self.conn.prepare("SELECT id, name, level, is_hazard, tags FROM monsters ORDER BY level, name, id")?;
        let rows = st.query_map([], |r| {
            let tags: String = r.get(4)?;
            Ok(MonsterSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                level: r.get(2)?,
                is_hazard: r.get::<_, i64>(3)? != 0,
                tags: tags.split(',').filter(|t| !t.is_empty()).map(String::from).collect(),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn get_monster(&self, id: &str) -> StoreResult<Option<Monster>> {
        self.get_doc("monsters", id)
    }

    fn put_monster(&self, m: &Monster) -> StoreResult<()> {
        let t = now();
        self.conn.execute(
            "INSERT INTO monsters (id, name, level, is_hazard, tags, data, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, level = excluded.level, is_hazard = excluded.is_hazard,
               tags = excluded.tags, data = excluded.data, updated_at = excluded.updated_at",
            params![m.id, m.name(), m.level() as i64, m.is_hazard() as i64, m.tags.join(","), serde_json::to_string(m)?, t],
        )?;
        Ok(())
    }

    fn delete_monster(&self, id: &str) -> StoreResult<bool> {
        self.delete_row("monsters", id)
    }

    fn list_encounters(&self) -> StoreResult<Vec<EncounterSummary>> {
        let mut st = self.conn.prepare("SELECT id, name, party_level, updated_at FROM encounters ORDER BY name, id")?;
        let rows = st.query_map([], |r| {
            Ok(EncounterSummary { id: r.get(0)?, name: r.get(1)?, party_level: r.get(2)?, updated_at: r.get(3)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn get_encounter(&self, id: &str) -> StoreResult<Option<Encounter>> {
        self.get_doc("encounters", id)
    }

    fn put_encounter(&self, e: &Encounter) -> StoreResult<()> {
        let t = now();
        self.conn.execute(
            "INSERT INTO encounters (id, name, party_level, data, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, party_level = excluded.party_level,
               data = excluded.data, updated_at = excluded.updated_at",
            params![e.id, e.name, e.party_level as i64, serde_json::to_string(e)?, t],
        )?;
        Ok(())
    }

    fn delete_encounter(&self, id: &str) -> StoreResult<bool> {
        self.delete_row("encounters", id)
    }

    fn create_combat(&self, rec: &CombatRecord, initial: &CombatState) -> StoreResult<()> {
        let (mode, seed) = match rec.dice {
            DiceMode::Manual => ("manual", None),
            DiceMode::Seeded { seed } => ("seeded", Some(seed as i64)),
        };
        self.conn.execute(
            "INSERT INTO combats (id, encounter_id, name, dice_mode, dice_seed, initial_state, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![rec.id, rec.encounter_id, rec.name, mode, seed, serde_json::to_string(initial)?, rec.status, rec.created_at],
        )?;
        Ok(())
    }

    fn list_combats(&self) -> StoreResult<Vec<CombatRecord>> {
        let mut st = self.conn.prepare(
            "SELECT id, encounter_id, name, dice_mode, dice_seed, status, created_at, updated_at FROM combats ORDER BY created_at DESC, id",
        )?;
        let rows = st.query_map([], row_to_record)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn get_combat(&self, id: &str) -> StoreResult<Option<(CombatRecord, CombatLog)>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, encounter_id, name, dice_mode, dice_seed, status, created_at, updated_at, initial_state FROM combats WHERE id = ?1",
                [id],
                |r| Ok((row_to_record(r)?, r.get::<_, String>(8)?)),
            )
            .optional()?;
        let Some((rec, initial)) = row else { return Ok(None) };
        let initial: CombatState = serde_json::from_str(&initial)?;
        let mut st = self.conn.prepare("SELECT command FROM combat_commands WHERE combat_id = ?1 ORDER BY seq")?;
        let commands = st
            .query_map([id], |r| r.get::<_, String>(0))?
            .map(|r| -> StoreResult<Command> { Ok(serde_json::from_str(&r?)?) })
            .collect::<StoreResult<Vec<_>>>()?;
        Ok(Some((rec, CombatLog { initial, commands })))
    }

    fn append_command(&self, combat_id: &str, seq: usize, cmd: &Command) -> StoreResult<()> {
        let res = self.conn.execute(
            "INSERT INTO combat_commands (combat_id, seq, command, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![combat_id, seq as i64, serde_json::to_string(cmd)?, now()],
        );
        match res {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                return Err(StoreError::Conflict(format!("command #{seq} already exists for combat {combat_id}")));
            }
            Err(e) => return Err(e.into()),
        }
        self.conn.execute("UPDATE combats SET updated_at = ?2 WHERE id = ?1", params![combat_id, now()])?;
        Ok(())
    }

    fn truncate_commands(&self, combat_id: &str, from: usize) -> StoreResult<()> {
        self.conn.execute("DELETE FROM combat_commands WHERE combat_id = ?1 AND seq >= ?2", params![combat_id, from as i64])?;
        Ok(())
    }

    fn set_combat_status(&self, id: &str, status: &str) -> StoreResult<()> {
        self.conn.execute("UPDATE combats SET status = ?2, updated_at = ?3 WHERE id = ?1", params![id, status, now()])?;
        Ok(())
    }

    fn delete_combat(&self, id: &str) -> StoreResult<bool> {
        self.delete_row("combats", id)
    }
}

fn row_to_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<CombatRecord> {
    let mode: String = r.get(3)?;
    let seed: Option<i64> = r.get(4)?;
    Ok(CombatRecord {
        id: r.get(0)?,
        encounter_id: r.get(1)?,
        name: r.get(2)?,
        dice: match (mode.as_str(), seed) {
            ("seeded", Some(s)) => DiceMode::Seeded { seed: s as u64 },
            _ => DiceMode::Manual,
        },
        status: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf2e_core::builder::content::Content;
    use pf2e_core::combat::{CombatSetup, Command};
    use pf2e_core::models::Battlefield;

    #[test]
    fn migrations_are_idempotent() {
        let s = SqliteStore::in_memory().unwrap();
        assert_eq!(s.schema_version().unwrap(), 1);
        s.migrate().unwrap();
        assert_eq!(s.schema_version().unwrap(), 1);
    }

    #[test]
    fn monster_crud() {
        let s = SqliteStore::in_memory().unwrap();
        let content = Content::bundled().unwrap();
        let m = content.monsters[0].clone();
        s.put_monster(&m).unwrap();
        assert_eq!(s.get_monster(&m.id).unwrap().unwrap(), m);
        assert_eq!(s.list_monsters().unwrap().len(), 1);
        let mut m2 = m.clone();
        m2.statblock.max_hp += 5;
        s.put_monster(&m2).unwrap();
        assert_eq!(s.get_monster(&m.id).unwrap().unwrap().statblock.max_hp, m.statblock.max_hp + 5);
        assert!(s.delete_monster(&m.id).unwrap());
        assert!(s.get_monster(&m.id).unwrap().is_none());
    }

    #[test]
    fn combat_log_roundtrip_and_conflict() {
        let s = SqliteStore::in_memory().unwrap();
        let content = Content::bundled().unwrap();
        let state = CombatSetup::default().build(Battlefield::open(5, 5), vec![], &content).unwrap();
        let rec = CombatRecord {
            id: "c1".into(),
            encounter_id: None,
            name: "Test".into(),
            dice: DiceMode::Seeded { seed: 42 },
            status: "active".into(),
            created_at: now(),
            updated_at: now(),
        };
        s.create_combat(&rec, &state).unwrap();
        s.append_command("c1", 0, &Command::StartCombat).unwrap();
        assert!(matches!(s.append_command("c1", 0, &Command::StartCombat), Err(StoreError::Conflict(_))));
        let (r, log) = s.get_combat("c1").unwrap().unwrap();
        assert_eq!(r.dice, DiceMode::Seeded { seed: 42 });
        assert_eq!(log.commands, vec![Command::StartCombat]);
        assert_eq!(log.initial, state);
        s.truncate_commands("c1", 0).unwrap();
        assert!(s.get_combat("c1").unwrap().unwrap().1.commands.is_empty());
        assert!(s.delete_combat("c1").unwrap());
    }
}
