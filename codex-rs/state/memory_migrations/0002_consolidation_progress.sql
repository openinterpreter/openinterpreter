-- A successful consolidation is the readiness boundary for the version experiment.
CREATE TABLE consolidation_progress (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    max_thread_count INTEGER NOT NULL DEFAULT 0
);
INSERT INTO consolidation_progress (singleton) VALUES (1);
