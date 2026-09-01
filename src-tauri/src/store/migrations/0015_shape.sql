-- T435 — the shape of a film's weight, kept instead of thrown away.
--
-- **Computed on every measurement and dropped every time.** Each one reads the size of every
-- packet to work out where the light, middling and heavy chunks fall — a full second-by-second
-- weight profile of the film — and keeps three timestamps out of it. The rest went nowhere.
--
-- It is the richest thing this application ever learns about material, and the only one that
-- describes the picture rather than the container: two episodes of a season have profiles that
-- look alike, an episode and a trailer do not, whatever their codec and frame size agree
-- about. Lending compares eight fields (T431) and every one of them is a container property.
--
-- Five numbers rather than the row itself. A two-hour film is seven thousand of them, and
-- keeping them all per measurement would grow the store with the library while answering no
-- question these five cannot.
--
-- Columns added, no table rebuilt: `quality_points` refers to this one with `ON DELETE
-- CASCADE`, and rebuilding a parent with foreign keys on has emptied a child once already.
--
-- Old rows have them empty, which reads as "not known" — which is what they are.

ALTER TABLE quality_measurements ADD COLUMN shape_median_bps INTEGER;
ALTER TABLE quality_measurements ADD COLUMN shape_p90_bps INTEGER;
ALTER TABLE quality_measurements ADD COLUMN shape_peak_bps INTEGER;
ALTER TABLE quality_measurements ADD COLUMN shape_peak_to_median_x100 INTEGER;
ALTER TABLE quality_measurements ADD COLUMN shape_walls INTEGER;
