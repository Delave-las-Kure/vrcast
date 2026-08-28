-- T415 — a task keeps what it had to say.
--
-- **What was going wrong.** A task can finish the work and still have something a person
-- needs: three variants were taken from a previous run rather than made again, the
-- measurement stopped short of the whole grid so the optimum may not have been found, the
-- graphics card refused and the processor did it four times slower. All three are worked out
-- by the core and written in both languages — and all three ended in a log line or were
-- dropped outright.
--
-- The event carrying them is not enough on its own. It reaches whoever is looking at that
-- moment; somebody on another screen when a two-hour build ends learns nothing, and after a
-- restart there is nothing to learn from.
--
-- Stored as JSON, in the same shape and for the same reason as `error`: a code and its
-- numbers, never a sentence. A task that finished a week ago must still explain itself in
-- whatever language is chosen today.
--
-- Old rows have it empty, which reads as "said nothing" — which is what they did.

ALTER TABLE tasks ADD COLUMN notices TEXT;
