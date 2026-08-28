-- T392 — the sweep learns whose record it is looking at.
--
-- **The hazard.** At start-up the application finishes off programs that survived the
-- previous run: on Linux nothing closes a grandchild when the parent dies, so without this
-- an orphaned ffmpeg goes on writing into a file and spoils it quietly.
--
-- Until now that sweep knew only that a program was alive and that there was a record of
-- it. It did not know **whose** record. Today that is harmless, because closing the window
-- ends the process and there is never a second instance alongside a first. Minimising to
-- the tray ends that: the application goes on running with encodes in flight, somebody
-- starts it again, and the second instance sweeps away the first one's work. Hours of
-- encoding, killed by opening the application.
--
-- So a record now carries the instance that made it. A record whose owner is still running
-- is not a survivor of anything and is not the sweep's to touch.
--
-- **Why the identity and not the number alone.** Process numbers are reused, and by the
-- next start-up the old instance's number may belong to a person's browser. The identity is
-- the process start time — the same mark, and for the same reason, as the one migration
-- 0004 added for the child process.
--
-- Old rows have both columns empty, which reads as "owner unknown". Those are swept exactly
-- as before: they were written by a version that could not have had a live owner beside it.

ALTER TABLE running_processes ADD COLUMN owner_pid INTEGER;
ALTER TABLE running_processes ADD COLUMN owner_identity TEXT;
