-- The order videos were added in, as a number (the flaky
-- `video::the_list_keeps_the_order_videos_were_added_in`, 2026-10-01).
--
-- **Why not `created_at`.** It was the list's order, and it is a string: `now_rfc3339` wrote
-- the fraction of a second without its trailing zeros, so `…:05.1234Z` came after
-- `…:05.12345678Z` — later as text, earlier as time. Two videos added in the same second came
-- back in either order. A number given once, at insertion, and never rewritten by an update
-- cannot do that, and is the same however fast two videos are added.
--
-- The videos already kept get the order they were inserted in (`rowid`; nothing here runs
-- VACUUM, which is the one thing that may renumber it).
ALTER TABLE videos ADD COLUMN seq INTEGER;
UPDATE videos SET seq = rowid;
CREATE INDEX idx_videos_seq ON videos (seq);
