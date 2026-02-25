-- Task hierarchy: add parent_task_id for subtask grouping and cancelled status.

-- SQLite cannot ALTER CHECK constraints, so we recreate the table.

-- 1. Rename old table
ALTER TABLE tasks RENAME TO _tasks_old;

-- 2. Create new table with parent_task_id and expanded status CHECK
CREATE TABLE tasks (
    id TEXT PRIMARY KEY NOT NULL,
    task_type TEXT NOT NULL,
    payload TEXT NOT NULL,  -- JSON
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    progress INTEGER NOT NULL DEFAULT 0
        CHECK (progress >= 0 AND progress <= 100),
    message TEXT,
    result TEXT,  -- JSON result on completion
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    started_at TEXT,
    completed_at TEXT,
    error_message TEXT,
    parent_task_id TEXT REFERENCES tasks(id)
);

-- 3. Copy existing data (parent_task_id will be NULL for all existing rows)
INSERT INTO tasks (id, task_type, payload, status, progress, message, result, created_at, started_at, completed_at, error_message)
SELECT id, task_type, payload, status, progress, message, result, created_at, started_at, completed_at, error_message
FROM _tasks_old;

-- 4. Drop old table
DROP TABLE _tasks_old;

-- 5. Recreate existing indexes
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at);

-- 6. New index for parent lookup
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id);
