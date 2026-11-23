ALTER TABLE queued_items ADD COLUMN admission_id TEXT;
ALTER TABLE queued_items ADD COLUMN admission_goal_binding_json TEXT;
ALTER TABLE queued_items ADD COLUMN admission_state TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE queued_items ADD COLUMN admission_blocker TEXT;
ALTER TABLE queued_items ADD COLUMN dispatched_at_ms INTEGER;
ALTER TABLE queued_items ADD COLUMN dispatch_attempt_at_ms INTEGER;
ALTER TABLE queued_items ADD COLUMN dispatch_turn_id TEXT;
ALTER TABLE queued_items ADD COLUMN admission_dispatch_mode TEXT NOT NULL DEFAULT 'queue';
ALTER TABLE queued_items ADD COLUMN admission_expected_turn_id TEXT;

CREATE UNIQUE INDEX queued_items_thread_admission_idx
    ON queued_items(thread_id, admission_id)
    WHERE admission_id IS NOT NULL;

CREATE INDEX queued_items_thread_pending_order_idx
    ON queued_items(thread_id, queue_order)
    WHERE admission_state = 'pending';
