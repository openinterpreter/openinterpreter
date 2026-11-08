ALTER TABLE queued_items ADD COLUMN admission_terminal_outcome TEXT;
ALTER TABLE queued_items ADD COLUMN admission_terminal_turn_id TEXT;
ALTER TABLE queued_items ADD COLUMN admission_terminal_goal_id TEXT;
ALTER TABLE queued_items ADD COLUMN admission_terminal_mission_revision INTEGER;
ALTER TABLE queued_items ADD COLUMN admission_terminal_lifecycle_revision INTEGER;
ALTER TABLE queued_items ADD COLUMN admission_terminal_objective TEXT;
ALTER TABLE queued_items ADD COLUMN admission_terminal_at_ms INTEGER;
