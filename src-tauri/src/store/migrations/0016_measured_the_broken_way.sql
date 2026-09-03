-- Throw away every quality measurement taken before 2026-09-03.
--
-- Not housekeeping. Until that day the chunk being scored was muxed into mp4, and the same
-- encoded bytes read back out of an mp4 score up to twenty-three VMAF below what they score
-- after a stream copy into Matroska (media/vmaf.rs, `chunk_args`). Every stored point is
-- therefore some six to twenty-three low, by an amount that differs from chunk to chunk.
--
-- Keeping them would be worse than losing them, and in two ways at once. A ladder chosen from
-- them puts its top wherever the grid happened to end, because a target of 96 was unreachable.
-- And lending compares a stored measurement against a freshly taken one: a donor measured the
-- old way against a borrower measured the new one differs by about twelve points, so the check
-- after a loan (T437, threshold one point) would refuse every legitimate loan there is.
--
-- Half an hour of somebody's machine goes into a measurement, and this discards it. That is
-- the cost of having measured the wrong thing; the alternative is a ladder built on it.
DELETE FROM quality_points;
DELETE FROM quality_measurements;
