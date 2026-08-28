-- T434 — what the material actually is, kept instead of thrown away.
--
-- **All of it is already read and all of it is already discarded.** `prepare()` probes the
-- source and keeps five numbers; the codec's full name, the pixel format, the transfer curve,
-- the length and the peak go straight in the bin. Then lending compares five fields and calls
-- that "the same material" — and one of the five is a boolean, "is this HEVC", so AV1 and VP9
-- are compared as though they were H.264 (T431).
--
-- **Columns added, never a table rebuilt.** `quality_points` refers to this table with
-- `ON DELETE CASCADE`, and rebuilding a parent with foreign keys on has already emptied a
-- child once in this project (tasks.md:1387). `ALTER TABLE ADD COLUMN` touches no row and
-- drops no reference.
--
-- Old rows have them empty, which reads as "not known". Lending treats an unknown field as
-- one it cannot vouch for and refuses, which is the safe direction: a measurement made before
-- these columns existed says nothing about the material it was made on.

ALTER TABLE quality_measurements ADD COLUMN source_codec TEXT;
ALTER TABLE quality_measurements ADD COLUMN pix_fmt TEXT;
ALTER TABLE quality_measurements ADD COLUMN color_transfer TEXT;
ALTER TABLE quality_measurements ADD COLUMN duration_s REAL;
ALTER TABLE quality_measurements ADD COLUMN peak_bps INTEGER;
