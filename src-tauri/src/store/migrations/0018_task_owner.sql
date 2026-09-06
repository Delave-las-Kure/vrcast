-- T514 — the recovery learns whose task it is looking at.
--
-- **The same hazard as migration 0010, one table along, and left there.** That one taught the
-- process sweep whose record it was looking at, because minimising to the tray created a case
-- that had never existed before: the application goes on running with encodes in flight,
-- somebody starts it again, and the second instance tidies away the first one's work.
--
-- It stopped at `running_processes`. The tasks table got nothing, and the recovery reads
-- `WHERE state = 'running'` over the whole of it with no owner in sight — so a second instance
-- declares every task the first one is running "left over from the previous run" and rewrites
-- it to paused. The first instance's ffmpeg keeps going, because 0010 protects it; only the
-- row about it is taken. Then `restore_uploads` walks that same table, finds an unfinished
-- upload that is now paused, and raises it in the second instance: one press of "Continue" and
-- two processes are writing the same file on the server.
--
-- FR-151 says a second instance MUST NOT interrupt the first. Its own note scopes the remedy
-- to "the record of a started program" — the process row — which is exactly why the task rows
-- were left open.
--
-- **Why the identity and not the number alone**, verbatim from 0010's reasoning because it is
-- the same reasoning: process numbers are reused, and by the next start-up the old instance's
-- number may belong to a person's browser. The identity is the process start time.
--
-- Old rows have both columns empty, which reads as "owner unknown". Those are recovered
-- exactly as before: they were written by a version that could not have had a live owner
-- beside it.

ALTER TABLE tasks ADD COLUMN owner_pid INTEGER;
ALTER TABLE tasks ADD COLUMN owner_identity TEXT;
