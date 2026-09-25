ALTER TABLE thread_artifacts RENAME TO thread_attachments;
ALTER TABLE thread_attachments RENAME COLUMN artifact_type TO attachment_type;

DROP INDEX idx_thread_artifacts_thread_created_id;
CREATE INDEX idx_thread_attachments_thread_created_id
    ON thread_attachments(thread_id, created_at, id);
