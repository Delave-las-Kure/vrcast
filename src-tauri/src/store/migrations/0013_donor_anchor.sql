-- T429 — a borrowed measurement keeps both anchors.
--
-- **What was lost.** The anchor is the top of the grid this film's own complexity probe asked
-- for — the one number measured on *this* material before anything was borrowed. Lending
-- overwrote it with the donor's, so the borrowed ladder and any check that would one day
-- compare the two both stood on the donor's figure, and nothing was left to disagree with it.
--
-- The check that wants them is the one after a loan (T437): measure one cell of the grid on
-- the borrower and compare it with the donor's at the same bitrate and height. Two anchors
-- that differ by a lot are the first sign the material differs too.
--
-- Old rows have it empty, which reads as "not borrowed, so there is no donor's anchor" —
-- which is what they were.

ALTER TABLE quality_measurements ADD COLUMN donor_anchor_mbps INTEGER;
