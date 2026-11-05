-- PF2e Combat Engine schema, version 1.
--
-- Portable between SQLite and PostgreSQL:
--   * TEXT primary keys (UUID strings), INTEGER/BIGINT numbers, TEXT JSON.
--   * Timestamps are Unix epoch seconds (BIGINT).
--   * On PostgreSQL, the `data`/`state`/`command` columns can be declared
--     JSONB instead of TEXT with no application change.
--
-- Aggregates are stored as JSON documents with a few indexed scalar
-- columns for listing and filtering. Combats are event-sourced: an
-- immutable initial state plus an append-only command log.

CREATE TABLE IF NOT EXISTS schema_migrations (
    version     INTEGER PRIMARY KEY,
    applied_at  BIGINT  NOT NULL
);

CREATE TABLE IF NOT EXISTS characters (
    id           TEXT    PRIMARY KEY,
    name         TEXT    NOT NULL,
    level        INTEGER NOT NULL,
    ancestry_id  TEXT    NOT NULL,
    class_id     TEXT    NOT NULL,
    is_legal     INTEGER NOT NULL DEFAULT 0,
    data         TEXT    NOT NULL,
    created_at   BIGINT  NOT NULL,
    updated_at   BIGINT  NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_characters_name ON characters (name);

CREATE TABLE IF NOT EXISTS monsters (
    id          TEXT    PRIMARY KEY,
    name        TEXT    NOT NULL,
    level       INTEGER NOT NULL,
    is_hazard   INTEGER NOT NULL DEFAULT 0,
    tags        TEXT    NOT NULL DEFAULT '',
    data        TEXT    NOT NULL,
    created_at  BIGINT  NOT NULL,
    updated_at  BIGINT  NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_monsters_level ON monsters (level);
CREATE INDEX IF NOT EXISTS idx_monsters_name ON monsters (name);

CREATE TABLE IF NOT EXISTS encounters (
    id           TEXT    PRIMARY KEY,
    name         TEXT    NOT NULL,
    party_level  INTEGER NOT NULL,
    data         TEXT    NOT NULL,
    created_at   BIGINT  NOT NULL,
    updated_at   BIGINT  NOT NULL
);

CREATE TABLE IF NOT EXISTS combats (
    id             TEXT    PRIMARY KEY,
    encounter_id   TEXT    NULL REFERENCES encounters (id) ON DELETE SET NULL,
    name           TEXT    NOT NULL,
    dice_mode      TEXT    NOT NULL,           -- 'manual' | 'seeded'
    dice_seed      BIGINT  NULL,
    initial_state  TEXT    NOT NULL,           -- CombatState JSON
    status         TEXT    NOT NULL,           -- 'active' | 'ended'
    created_at     BIGINT  NOT NULL,
    updated_at     BIGINT  NOT NULL
);

CREATE TABLE IF NOT EXISTS combat_commands (
    combat_id   TEXT    NOT NULL REFERENCES combats (id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,              -- 0-based position in the log
    command     TEXT    NOT NULL,              -- Command JSON
    created_at  BIGINT  NOT NULL,
    PRIMARY KEY (combat_id, seq)
);
