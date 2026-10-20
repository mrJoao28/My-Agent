CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL CHECK (length(user_id) BETWEEN 1 AND 128),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 32768),
    cwd TEXT,
    workspace_path TEXT NOT NULL,
    retention_warned_at INTEGER CHECK (retention_warned_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
);
CREATE TABLE messages (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    generation_id TEXT,
    role TEXT NOT NULL CHECK (role IN ('USER', 'ASSISTANT', 'ERROR')),
    status TEXT NOT NULL CHECK (status IN ('IN_PROGRESS', 'COMPLETE', 'INTERRUPTED')),
    model TEXT NOT NULL CHECK (length(model) BETWEEN 1 AND 200),
    mode TEXT NOT NULL CHECK (mode IN ('PLAN', 'BUILD')),
    content TEXT NOT NULL,
    parts_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(parts_json) AND json_type(parts_json) = 'array'),
    duration INTEGER CHECK (duration BETWEEN 0 AND 2147483647),
    total_tokens INTEGER CHECK (total_tokens BETWEEN 0 AND 2147483647),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK (role = 'ASSISTANT' OR status <> 'IN_PROGRESS')
);
