-- Initial schema: email_stats tracks per-user deleted email counts.
-- IF NOT EXISTS keeps this safe to apply against a database that already
-- has the table (e.g. one previously initialized from create_tables.sql).
CREATE TABLE IF NOT EXISTS email_stats (
    email_hash TEXT PRIMARY KEY,
    deleted_count BIGINT NOT NULL
);

-- System-wide total record.
INSERT INTO email_stats (email_hash, deleted_count)
VALUES ('SYSTEM_TOTAL', 0)
ON CONFLICT (email_hash) DO NOTHING;
