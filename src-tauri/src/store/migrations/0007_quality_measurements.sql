-- T236 — what a quality measurement found, kept between runs.
--
-- A measurement costs about half an hour of a person's machine, so losing it to a restart,
-- a cancellation or a crash is not acceptable. Points are written one at a time, as they
-- are measured, and a run that comes back picks up the grid where it stopped.
--
-- **The header and the points are separate tables on purpose.** A run is identified by the
-- material and the target codec together: a ladder measured for H.264 says nothing about
-- HEVC — AV1's advantage over H.264 melts as the bitrate rises (+5.42 VMAF at 4 Mbit/s
-- against +1.09 at 22), so there is no multiplier to carry one to the other, and the points
-- where the resolution should change move as well.
CREATE TABLE quality_measurements (
    -- The material: its size and its name. A file edited in place is a different film and
    -- has to be measured again; a file merely moved is the same one.
    source_key    TEXT NOT NULL,
    -- The codec the ladder is being measured FOR, not the source's own.
    codec         TEXT NOT NULL,
    source_path   TEXT NOT NULL,
    width         INTEGER NOT NULL,
    height        INTEGER NOT NULL,
    fps           INTEGER NOT NULL,
    -- The source's own bitrate and whether it is in a codec heavier than H.264.
    -- Kept because the ladder made from these points is capped by them: above the
    -- source there is no detail to find, only weight.
    source_bitrate_bps INTEGER NOT NULL,
    heavier_codec      INTEGER NOT NULL,
    -- The height the material really has, when it was upscaled. Told by the person.
    native_height INTEGER,
    -- What the complexity probe found; the grid is built around it.
    anchor_mbps   INTEGER NOT NULL,
    -- Where the reference chunks start, in seconds, separated by commas.
    --
    -- Stored rather than recomputed: reusing a measurement between episodes of a season
    -- means reusing THESE chunks. Chosen afresh, the percentiles would be the same but the
    -- scenes different, and the difference between episodes would mix into the difference
    -- between rungs.
    chunk_starts  TEXT NOT NULL,
    chunk_s       INTEGER NOT NULL,
    -- Set when this measurement was taken from another file rather than made here. A rung
    -- resting on a borrowed measurement is not a measured rung and must not be shown as one.
    borrowed_from TEXT,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (source_key, codec)
) STRICT;

CREATE TABLE quality_points (
    source_key   TEXT NOT NULL,
    codec        TEXT NOT NULL,
    bitrate_mbps INTEGER NOT NULL,
    height       INTEGER NOT NULL,
    vmaf         REAL NOT NULL,
    -- What the encode actually weighed: the target is asked for, not obeyed.
    actual_bps   INTEGER NOT NULL,
    measured_at  TEXT NOT NULL,
    PRIMARY KEY (source_key, codec, bitrate_mbps, height),
    FOREIGN KEY (source_key, codec)
        REFERENCES quality_measurements (source_key, codec) ON DELETE CASCADE
) STRICT;
