CREATE TABLE session_contexts (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    messages_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(messages_json) AND json_type(messages_json) = 'array'),
    token_limit INTEGER NOT NULL DEFAULT 16384 CHECK (token_limit BETWEEN 1 AND 2147483647),
    estimated_tokens INTEGER NOT NULL DEFAULT 0 CHECK (estimated_tokens BETWEEN 0 AND token_limit),
    truncated INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
);
CREATE TABLE session_leases (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner TEXT NOT NULL CHECK (length(owner) BETWEEN 1 AND 128),
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0)
);
