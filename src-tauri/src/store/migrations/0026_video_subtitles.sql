-- T696 (owner's decision B3) — the subtitle track drawn into the picture, chosen per video.
--
-- NULL — no subtitles, which is what every video had before and what a new one starts with.
-- Otherwise the index among the source's subtitle tracks (`0:s:<N>`), counted from zero.
ALTER TABLE videos ADD COLUMN subtitle_track INTEGER;
