-- T173 — the application's settings.
--
-- One row per setting rather than one row with a column each. A column per setting means a
-- migration for every new one, and half of them are added the day somebody wants them; a
-- name and a value need no migration at all.
--
-- Deliberately NOT where secrets go. Those live in the operating system's own store
-- (constitution, principle IV), and this file is an ordinary one in a person's profile.
CREATE TABLE settings (
    name  TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
