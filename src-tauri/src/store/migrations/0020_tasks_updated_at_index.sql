-- T564 — an index for the retention purge (`tasks::store::purge_finished_before`).
--
-- The function's own WHERE clause is `state IN (...) AND updated_at < ?1` — `updated_at`,
-- not `created_at`. The two already diverge for any task that changed state after it was
-- created, which is every task that ever ran: `created_at` is written once and never again,
-- `updated_at` moves with every state change, every progress write, every stage change.
-- Indexing the wrong column would leave the predicate that actually decides which rows go
-- unindexed and scanning the whole table regardless.
CREATE INDEX idx_tasks_updated_at ON tasks (updated_at);
