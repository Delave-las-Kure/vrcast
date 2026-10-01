-- T672 — a video in work: one film on its way from a plan to links, in one place.
--
-- **Why a table of its own and not a column on `tasks`.** A video outlives every task it goes
-- through: a measurement, then a build, then — after a restart or a retry — another build of
-- the same set. The tasks come and go and are purged after ninety days; the video is what a
-- person is looking at, and what has to carry on after the application is closed. It holds
-- what the tasks do not: the source, the medium in the library, the stage it is at and the
-- problem it stopped on.
--
-- **The tasks are found by batch, not by a list of ids.** Every task a video starts carries
-- the video's id as its batch (`tasks.batch_id`), and that is how the build the measurement
-- chains onto (T438) is recognised as this video's — it is created inside the measurement's
-- task, where this table cannot be told about it.
--
-- `server_id` cascades like the tasks' own: a video for a server that was removed has nowhere
-- to go.
CREATE TABLE videos (
    id               TEXT PRIMARY KEY,
    server_id        TEXT NOT NULL REFERENCES server_profiles (id) ON DELETE CASCADE,
    source_path      TEXT NOT NULL,
    title            TEXT NOT NULL,
    slug             TEXT NOT NULL,
    audio_track      INTEGER NOT NULL DEFAULT 0,
    stage            TEXT NOT NULL DEFAULT 'planned'
                     CHECK (stage IN ('planned', 'measuring', 'encoding', 'uploading',
                                      'cutting', 'verifying', 'done')),
    state            TEXT NOT NULL DEFAULT 'planning'
                     CHECK (state IN ('planning', 'ready', 'working', 'paused', 'problem',
                                      'cancelling', 'cancelled', 'done')),
    -- A pause a person pressed. Survives a restart as a pause: nothing carries it on unasked.
    paused_by_person INTEGER NOT NULL DEFAULT 0 CHECK (paused_by_person IN (0, 1)),
    -- «Start» was pressed while the plan was still being made: go as soon as it is.
    start_requested  INTEGER NOT NULL DEFAULT 0 CHECK (start_requested IN (0, 1)),
    -- The measurement of this film is over (its task said so): carrying on goes to the build.
    measured         INTEGER NOT NULL DEFAULT 0 CHECK (measured IN (0, 1)),
    -- The medium was made by this video, so nobody can be watching it before it is done: the
    -- build needs no «build anyway» against viewers (T571) for it.
    own_medium       INTEGER NOT NULL DEFAULT 0 CHECK (own_medium IN (0, 1)),
    -- A person pressed «build anyway»: past viewers and past the checker's objections.
    confirmed        INTEGER NOT NULL DEFAULT 0 CHECK (confirmed IN (0, 1)),
    -- The task of the current stage. Not a foreign key: tasks are purged, videos are not.
    task_id          TEXT,
    -- The medium in the library, once it has been made (or taken over).
    media_id         TEXT,
    -- What the source turned out to be (JSON `SourceFile`), the plan (JSON `VideoPlan`), the
    -- rungs a person set by hand (JSON `Rung[]`), and the problem it stopped on (JSON
    -- `{error, actions}`). JSON because each is a shape the core already serialises.
    source_json      TEXT,
    plan_json        TEXT,
    rungs_json       TEXT,
    problem_json     TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

CREATE INDEX idx_videos_server ON videos (server_id);

-- What this machine's encoders have actually done, in pixels of output a second (T672): what
-- the time in a plan is corrected by. One row per encoded rung; read back as a middle value.
CREATE TABLE encode_speeds (
    encoder      TEXT NOT NULL,
    pixels_per_s REAL NOT NULL CHECK (pixels_per_s > 0),
    measured_at  TEXT NOT NULL
);

CREATE INDEX idx_encode_speeds_encoder ON encode_speeds (encoder, measured_at);
