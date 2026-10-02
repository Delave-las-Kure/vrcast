-- QA-25 №3 — a stop on the server that has to survive the application (T682).
--
-- The cutting runs on the server apart from the application (`server::hls_package`, T605):
-- a process group of its own, found by a mark in its environment that is unique to one start.
-- «Cancel» is over only once the server confirms nothing carrying that mark is alive. Killed
-- in between, the application used to forget which start it was, and a restart called the
-- video cancelled with the cutting perhaps still writing into the set.
--
-- So the start is written down the moment it is known, and struck out only once the work has
-- ended and its end is confirmed: which mark (`var`=`mark`), and the server and account it
-- runs under — a stop is sent there and nowhere else, never to wherever the profile points by
-- then. Never a pattern or a directory: a cutting of the same set started by another copy of
-- the application carries another mark and is not ours to stop.
CREATE TABLE remote_runs (
    task_id    TEXT PRIMARY KEY REFERENCES tasks (id) ON DELETE CASCADE,
    var        TEXT NOT NULL,
    mark       TEXT NOT NULL,
    host       TEXT NOT NULL,
    port       INTEGER NOT NULL,
    user       TEXT NOT NULL,
    created_at TEXT NOT NULL
);
