"""Idempotent SQLite schema setup.

There is no migration framework at the MVP — every CREATE is `IF NOT EXISTS`,
and new columns can be added later via a small `_ensure_column` helper.
"""
from __future__ import annotations

import sqlite3


SCHEMA_STATEMENTS = (
    """
    CREATE TABLE IF NOT EXISTS events (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        ts            TEXT    NOT NULL,
        kind          TEXT    NOT NULL,
        payload       TEXT    NOT NULL,
        session_id    TEXT,
        consolidated  INTEGER NOT NULL DEFAULT 0
    )
    """,
    "CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts)",
    "CREATE INDEX IF NOT EXISTS idx_events_consolidated ON events(consolidated)",
    """
    CREATE TABLE IF NOT EXISTS plans (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        goal        TEXT    NOT NULL,
        tasks       TEXT    NOT NULL,
        status      TEXT    NOT NULL DEFAULT 'active',
        created_at  TEXT    NOT NULL
    )
    """,
    "CREATE INDEX IF NOT EXISTS idx_plans_status ON plans(status)",
)


def apply_schema(conn: sqlite3.Connection) -> None:
    cur = conn.cursor()
    for statement in SCHEMA_STATEMENTS:
        cur.execute(statement)
    conn.commit()
