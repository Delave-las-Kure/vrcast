-- T445 — which film a task belongs to, and which batch.
--
-- **What a batch looks like without this.** Ten films put in at once make thirty tasks, and
-- the task list shows thirty rows saying "measuring quality", "building a set", "measuring
-- quality" — with nothing to say which film any of them is. Watching a batch then means
-- watching a wall, and "stop this one" is a guess.
--
-- Two columns and not one. The identifier groups them, so "stop the batch" can mean
-- something; the label is what a person reads, kept beside each task rather than in a table
-- of its own. A task outlives the screen that made it and may outlive the file's name in the
-- library, and a label that has to be looked up somewhere else is a label that one day is not
-- there.
--
-- Old rows have both empty, which reads as "not part of a batch" — which they were not.

ALTER TABLE tasks ADD COLUMN batch_id TEXT;
ALTER TABLE tasks ADD COLUMN batch_label TEXT;
