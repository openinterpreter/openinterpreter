-- Initial managed-Goal creation has an explicit absent/no-Goal precondition.
-- Keep its durable creation receipt replayable after restart without inventing
-- a predecessor snapshot.  Completed-predecessor receipts still retain every
-- prior Goal field and continue to be non-null in practice.
CREATE TABLE thread_goal_creation_receipts_v2 (
    thread_id TEXT NOT NULL,
    creation_id TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    previous_goal_id TEXT,
    previous_objective TEXT,
    previous_status TEXT CHECK(previous_status IN (
        'active',
        'paused',
        'blocked',
        'usage_limited',
        'budget_limited',
        'complete'
    )),
    previous_token_budget INTEGER,
    previous_tokens_used INTEGER,
    previous_time_used_seconds INTEGER,
    previous_mission_revision INTEGER,
    previous_lifecycle_revision INTEGER,
    previous_created_at_ms INTEGER,
    previous_updated_at_ms INTEGER,
    goal_id TEXT NOT NULL,
    objective TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'active',
        'paused',
        'blocked',
        'usage_limited',
        'budget_limited',
        'complete'
    )),
    token_budget INTEGER,
    tokens_used INTEGER NOT NULL,
    time_used_seconds INTEGER NOT NULL,
    mission_revision INTEGER NOT NULL,
    lifecycle_revision INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    document_path TEXT NOT NULL,
    document_sha256 TEXT NOT NULL,
    document_byte_length INTEGER NOT NULL,
    document_updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(thread_id, creation_id)
);

INSERT INTO thread_goal_creation_receipts_v2
SELECT
    thread_id,
    creation_id,
    request_digest,
    previous_goal_id,
    previous_objective,
    previous_status,
    previous_token_budget,
    previous_tokens_used,
    previous_time_used_seconds,
    previous_mission_revision,
    previous_lifecycle_revision,
    previous_created_at_ms,
    previous_updated_at_ms,
    goal_id,
    objective,
    status,
    token_budget,
    tokens_used,
    time_used_seconds,
    mission_revision,
    lifecycle_revision,
    created_at_ms,
    updated_at_ms,
    document_path,
    document_sha256,
    document_byte_length,
    document_updated_at_ms
FROM thread_goal_creation_receipts;

DROP TABLE thread_goal_creation_receipts;
ALTER TABLE thread_goal_creation_receipts_v2 RENAME TO thread_goal_creation_receipts;
