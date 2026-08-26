-- T235 — the database learns that measuring quality is a kind of task.
--
-- **A kind added in the code and not here cannot be started at all.** The `kind` column
-- carries a CHECK constraint listing every kind by name, and SQLite refuses a row that is
-- not in it. `measure_quality` was added to the code and not to the list, so the very first
-- attempt to measure anything failed at the database — after the interface had already said
-- the task was starting.
--
-- Nothing in the unit tests could see it: they check the engine with kinds that already
-- existed. It was found the first time the whole thing was run end to end, which is exactly
-- what that check was written for.
--
-- SQLite cannot alter a constraint, so the table is made afresh and the rows are carried
-- across — the ordinary way of doing this, and safe here because it happens inside the
-- migration's own transaction.

CREATE TABLE tasks_new (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL
                  CHECK (kind IN ('probe', 'convert', 'upload', 'build_ladder',
                                  'measure_quality',
                                  'deploy', 'upgrade_server', 'diagnose')),
    -- NULL для чисто локальных задач (разбор исходника, подготовка файла).
    server_id     TEXT REFERENCES server_profiles (id) ON DELETE CASCADE,
    state         TEXT NOT NULL
                  CHECK (state IN ('queued', 'running', 'paused',
                                   'completed', 'failed', 'cancelled')),
    progress      REAL NOT NULL DEFAULT 0
                  CHECK (progress BETWEEN 0 AND 1),
    stage         TEXT,
    speed_bps     INTEGER,
    eta_s         INTEGER,
    -- Позиция возобновления: переданные байты, готовые ступени, выполненные шаги.
    resume_token  TEXT,
    -- Человеческая формулировка (FR-105), уже прошедшая вырезание секретов.
    error         TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    queue_order   INTEGER NOT NULL DEFAULT 0
);

INSERT INTO tasks_new
    (id, kind, server_id, state, progress, stage, speed_bps, eta_s,
     resume_token, error, created_at, updated_at, queue_order)
SELECT
     id, kind, server_id, state, progress, stage, speed_bps, eta_s,
     resume_token, error, created_at, updated_at, queue_order
FROM tasks;

DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

-- The indexes went with the old table.
CREATE INDEX idx_tasks_state ON tasks (state);
CREATE INDEX idx_tasks_server ON tasks (server_id);
