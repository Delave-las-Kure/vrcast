-- T714 (the owner's decision of 2026-10-09: «remove the old measurements») — throw away every
-- quality measurement taken by the recipe before T697.
--
-- The same reasoning as 0016, a second time. Until T697 a chunk was measured with a keyframe
-- every 48 frames, a ceiling in whole megabits (at 1 Mbit/s it let 2 through, where the rung
-- made from it may reach 1.1) and, for an HDR film, without the tonemapping the rung goes
-- through. So the stored scores describe files nobody makes any more, and a ladder chosen from
-- them, or a loan checked against a fresh measurement, would rest on a different recipe.
--
-- The films are measured again. A video not yet past its measurement then says «measure»
-- and «Start» measures first (`commands::video::forget_lost_measurement`); one already
-- building goes on with the rungs it was building; a finished set is not touched.
DELETE FROM quality_points;
DELETE FROM quality_measurements;
