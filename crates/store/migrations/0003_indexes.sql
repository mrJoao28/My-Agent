CREATE INDEX sessions_workspace_activity ON sessions(user_id, workspace_path, updated_at DESC, id DESC);
CREATE INDEX sessions_retention ON sessions(updated_at, retention_warned_at);
CREATE INDEX messages_session_order ON messages(session_id, created_at, id);
CREATE INDEX messages_generation ON messages(generation_id);
CREATE INDEX messages_activity ON messages(session_id, status, created_at);
CREATE INDEX leases_expiry ON session_leases(expires_at);
