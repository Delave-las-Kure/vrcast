-- QA-25 — what a video's controls need to survive a restart (T683).
--
-- `remove_requested` (T683, the owner's decision of 2026-10-02): «Remove» on a video whose
-- work is alive stops the work first, like «Cancel», and takes the video off the list only
-- once everything has stopped. Kept, not held in memory: a restart in the middle of the stop
-- must still end with the video gone, not with a cancelled card nobody asked to keep.
ALTER TABLE videos ADD COLUMN remove_requested INTEGER NOT NULL DEFAULT 0
    CHECK (remove_requested IN (0, 1));
