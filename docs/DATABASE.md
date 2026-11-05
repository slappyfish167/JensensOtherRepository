# Database schema

Source of truth: [`crates/pf2e-store/migrations/001_init.sql`](../crates/pf2e-store/migrations/001_init.sql).
Migrations are applied in order at startup and recorded in `schema_migrations`.

## Principles

* **Aggregates as JSON documents** (`data` column) plus a few **indexed scalar columns** used for
  listing/filtering. The JSON is exactly the serde form of the Rust model, so import/export and
  storage share one format.
* **Combats are event-sourced**: `combats.initial_state` (immutable) + `combat_commands` (append-only).
  There is no mutable "current state" column; it is derived by replay.
* **Portable SQL**: `TEXT` keys (UUID strings), `INTEGER`/`BIGINT`, epoch-second timestamps, no
  SQLite-only features. On PostgreSQL, declare `data`, `initial_state` and `command` as `JSONB` to
  gain JSON indexing; the application code does not change.

## Tables

```
characters                         monsters                          encounters
──────────                         ────────                          ──────────
id           TEXT PK               id          TEXT PK               id          TEXT PK
name         TEXT     (idx)        name        TEXT  (idx)           name        TEXT
level        INTEGER               level       INTEGER (idx)         party_level INTEGER
ancestry_id  TEXT                  is_hazard   INTEGER (0/1)         data        TEXT (Encounter JSON)
class_id     TEXT                  tags        TEXT (comma list)     created_at  BIGINT
is_legal     INTEGER (0/1)         data        TEXT (Monster JSON)   updated_at  BIGINT
data         TEXT (Character JSON) created_at  BIGINT
created_at   BIGINT                updated_at  BIGINT
updated_at   BIGINT

combats                                          combat_commands
───────                                          ───────────────
id            TEXT PK                            combat_id  TEXT FK → combats.id ON DELETE CASCADE
encounter_id  TEXT NULL FK → encounters.id       seq        INTEGER   ─┐ PRIMARY KEY (combat_id, seq)
              ON DELETE SET NULL                 command    TEXT (Command JSON)
name          TEXT                               created_at BIGINT
dice_mode     TEXT  'manual' | 'seeded'
dice_seed     BIGINT NULL
initial_state TEXT (CombatState JSON)
status        TEXT  'active' | 'ended'
created_at    BIGINT
updated_at    BIGINT
```

### Notes

* `characters.is_legal` caches the builder's verdict at save time so lists can flag illegal builds
  without re-validating. Validation is always re-run when a character enters combat.
* `combat_commands (combat_id, seq)` is the **optimistic-concurrency key**: two clients appending the
  same `seq` cannot both succeed (the second gets HTTP 409).
* Undo deletes the commands with `seq ≥ n`.
* The combat's `initial_state` embeds full stat blocks and spell definitions, so editing a monster
  or spell later never changes a recorded combat.

## Repository interface

```rust
pub trait Store: Send {
    fn list_characters(&self) -> StoreResult<Vec<CharacterSummary>>;
    fn get_character(&self, id: &str) -> StoreResult<Option<Character>>;
    fn put_character(&self, c: &Character, is_legal: bool) -> StoreResult<()>;
    fn delete_character(&self, id: &str) -> StoreResult<bool>;
    // … same shape for monsters and encounters …
    fn create_combat(&self, rec: &CombatRecord, initial: &CombatState) -> StoreResult<()>;
    fn get_combat(&self, id: &str) -> StoreResult<Option<(CombatRecord, CombatLog)>>;
    fn append_command(&self, combat_id: &str, seq: usize, cmd: &Command) -> StoreResult<()>;
    fn truncate_commands(&self, combat_id: &str, from: usize) -> StoreResult<()>;
}
```

A PostgreSQL implementation (`PgStore`) only needs to implement this trait.
