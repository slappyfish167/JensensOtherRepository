-- Version 2: multiple game systems.
--
--   * Every combat records the game system that runs it ('pf2e', 'cpred',
--     ...). Its initial state and commands are that system's JSON.
--   * `documents` stores the actors, encounters and other aggregates of
--     systems that do not have dedicated tables, keyed by (system, kind, id),
--     so adding a game system needs no schema change.

ALTER TABLE combats ADD COLUMN system TEXT NOT NULL DEFAULT 'pf2e';
-- `encounter_id` references the PF2e encounters table; other systems'
-- encounters are documents, referenced here instead (no foreign key).
ALTER TABLE combats ADD COLUMN encounter_ref TEXT NULL;
CREATE INDEX IF NOT EXISTS idx_combats_system ON combats (system);

CREATE TABLE IF NOT EXISTS documents (
    system      TEXT    NOT NULL,
    kind        TEXT    NOT NULL,              -- e.g. 'actor', 'encounter'
    id          TEXT    NOT NULL,
    name        TEXT    NOT NULL,
    data        TEXT    NOT NULL,              -- JSON (JSONB on PostgreSQL)
    created_at  BIGINT  NOT NULL,
    updated_at  BIGINT  NOT NULL,
    PRIMARY KEY (system, kind, id)
);
CREATE INDEX IF NOT EXISTS idx_documents_name ON documents (system, kind, name);
