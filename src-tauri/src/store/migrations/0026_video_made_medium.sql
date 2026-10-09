-- T700 — a video remembers whether it made its medium itself.
--
-- «Start» makes the film's medium in the library before the first byte. A video cancelled and
-- removed before its set was filed used to leave that medium behind, empty: «0 files · 0 B»
-- in the library, a name nobody asked for. Removing such a video removes the medium too — but
-- only one this video made, never one a person chose («Build a set», T675) or one «Replace»
-- took over (T676): those were somebody's before the video came.
--
-- `own_medium` cannot tell them apart: it is about viewers (T571) and is set for all three.
ALTER TABLE videos ADD COLUMN made_medium INTEGER NOT NULL DEFAULT 0
    CHECK (made_medium IN (0, 1));
