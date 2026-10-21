use crate::{
    DAY_MS, Error, LEASE_TIMEOUT_MS, RETENTION_GRACE_MS, Result, State, Store, Workspace, records,
};
use myagent_protocol::{
    ExecutionState, Message, MessagePart, MessageStatus, Mode, Role, Session, SessionContext,
    SessionLease, Validate, ValidationError, validate_id, validate_user_content,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

const LOCAL_USER: &str = "local-user";
const INACTIVE: &str = "user_id='local-user' AND updated_at<=?1
AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.session_id=sessions.id
    AND (m.created_at>?1 OR m.status='IN_PROGRESS'))
AND NOT EXISTS (SELECT 1 FROM session_leases l WHERE l.session_id=sessions.id AND l.expires_at>?2)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetentionWarning {
    pub session_id: String,
    pub delete_after: i64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionReport {
    pub warned: Vec<RetentionWarning>,
    pub deleted: usize,
}

fn scope(db: &Connection, workspace: &Workspace, id: &str) -> Result<bool> {
    Ok(db
        .query_row(
            "SELECT 1 FROM sessions WHERE id=?1 AND workspace_path=?2 AND user_id=?3",
            params![id, workspace.key, LOCAL_USER],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
fn require_scope(db: &Connection, workspace: &Workspace, id: &str) -> Result<()> {
    if !scope(db, workspace, id)? {
        return Err(Error::SessionNotFound);
    }
    Ok(())
}
fn lease(db: &Connection, id: &str) -> Result<Option<SessionLease>> {
    let value = db
        .query_row(
            "SELECT session_id,owner,expires_at FROM session_leases WHERE session_id=?1",
            [id],
            |row| {
                Ok(SessionLease {
                    session_id: row.get(0)?,
                    owner: row.get(1)?,
                    expires_at: row.get(2)?,
                })
            },
        )
        .optional()?;
    if let Some(value) = &value {
        value.validate()?;
    }
    Ok(value)
}
fn require_lease(
    db: &Connection,
    id: &str,
    owner: &str,
    now: i64,
    lost: &std::collections::HashSet<String>,
) -> Result<()> {
    if lost.contains(id) {
        return Err(Error::LeaseLost);
    }
    if !lease(db, id)?.is_some_and(|lease| lease.owner == owner && lease.expires_at > now) {
        return Err(Error::LeaseLost);
    }
    Ok(())
}
fn claim(db: &Connection, id: &str, owner: &str, now: i64) -> Result<SessionLease> {
    let expires_at = now
        .checked_add(LEASE_TIMEOUT_MS)
        .ok_or(ValidationError::Field("clock"))?;
    let changed = db.execute(
        "INSERT INTO session_leases(session_id,owner,expires_at) VALUES (?1,?2,?3)
         ON CONFLICT(session_id) DO UPDATE SET owner=excluded.owner, expires_at=excluded.expires_at
         WHERE session_leases.expires_at<=?4 OR session_leases.owner=excluded.owner",
        params![id, owner, expires_at, now],
    )?;
    if changed != 1 {
        return Err(Error::SessionBusy);
    }
    Ok(SessionLease {
        session_id: id.into(),
        owner: owner.into(),
        expires_at,
    })
}
fn touch(db: &Connection, id: &str, now: i64) -> Result<()> {
    let changed = db.execute(
        "UPDATE sessions SET updated_at=MAX(updated_at,?1),retention_warned_at=NULL WHERE id=?2",
        params![now, id],
    )?;
    if changed != 1 {
        return Err(Error::SessionNotFound);
    }
    Ok(())
}
fn identifier(db: &Connection) -> Result<String> {
    Ok(db.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?)
}
fn page(limit: u32, offset: u32) -> Result<(u32, u32)> {
    if !(1..=500).contains(&limit) {
        return Err(ValidationError::Field("limit").into());
    }
    Ok((limit, offset))
}
fn no_generation(db: &Connection, id: &str) -> Result<()> {
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id=?1 AND role='ASSISTANT' AND status='IN_PROGRESS')",
        [id], |row| row.get(0))?;
    if active {
        return Err(Error::GenerationActive);
    }
    Ok(())
}
fn write_message(db: &Connection, message: &Message) -> Result<()> {
    let _validated = records::json_text(message)?;
    let parts = serde_json::to_string(&message.parts).map_err(|_| ValidationError::Json)?;
    db.execute("INSERT INTO messages
        (id,session_id,generation_id,role,status,model,mode,content,parts_json,duration,total_tokens,created_at)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![message.id, message.session_id, message.generation_id, records::label(&message.role)?,
            records::label(&message.status)?, message.model, records::label(&message.mode)?, message.content,
            parts, message.duration, message.total_tokens, message.created_at])?;
    touch(db, &message.session_id, message.created_at)
}
fn update_message(db: &Connection, message: &Message, now: i64) -> Result<()> {
    let _validated = records::json_text(message)?;
    let parts = serde_json::to_string(&message.parts).map_err(|_| ValidationError::Json)?;
    let changed = db.execute(
        "UPDATE messages SET content=?1,parts_json=?2,status=?3,duration=?4,total_tokens=?5
         WHERE id=?6 AND status='IN_PROGRESS' AND role='ASSISTANT'",
        params![
            message.content,
            parts,
            records::label(&message.status)?,
            message.duration,
            message.total_tokens,
            message.id
        ],
    )?;
    if changed != 1 {
        return Err(Error::MessageNotInProgress);
    }
    touch(db, &message.session_id, now)
}

impl Store {
    pub(crate) fn check_local_session(
        &self,
        workspace: &Workspace,
        session: &str,
        idle: bool,
    ) -> Result<()> {
        let workspace = workspace.clone();
        let session = Self::checked_session_id(session)?;
        self.call(move |state| {
            let now = state.now()?;
            let tx = state
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_scope(&tx, &workspace, &session)?;
            require_lease(&tx, &session, &state.owner, now, &state.lost)?;
            if idle {
                no_generation(&tx, &session)?;
            }
            tx.commit()?;
            Ok(())
        })
    }
    pub fn append_undo_note(
        &self,
        workspace: &Workspace,
        session: &str,
        note: &str,
    ) -> Result<Message> {
        validate_user_content(note)?;
        if !note.starts_with("[UNDO]") {
            return Err(ValidationError::Field("undo note").into());
        }
        let workspace = workspace.clone();
        let session = Self::checked_session_id(session)?;
        let note = note.to_owned();
        self.call(move |state| {
            let now = state.now()?;
            let tx = state
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_scope(&tx, &workspace, &session)?;
            require_lease(&tx, &session, &state.owner, now, &state.lost)?;
            no_generation(&tx, &session)?;
            let (model, mode) = tx
                .query_row(
                    "SELECT model, mode FROM messages WHERE session_id=?1 ORDER BY rowid DESC LIMIT 1",
                    [&session],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?
                .unwrap_or_else(|| ("local".into(), "BUILD".into()));
            let mode = match mode.as_str() {
                "PLAN" => Mode::Plan,
                "BUILD" => Mode::Build,
                _ => return Err(ValidationError::Field("mode").into()),
            };
            let message = Message {
                id: identifier(&tx)?,
                session_id: session,
                generation_id: None,
                role: Role::User,
                status: MessageStatus::Complete,
                model,
                mode,
                content: note,
                parts: vec![],
                duration: None,
                total_tokens: None,
                created_at: now,
            };
            write_message(&tx, &message)?;
            tx.commit()?;
            Ok(message)
        })
    }
    /// Cria sessão, contexto e lease na mesma transação.
    pub fn create_session(&self, workspace: &Workspace) -> Result<Session> {
        let workspace = workspace.clone();
        self.call(move |state| {
            let now = state.now()?;
            let tx = state.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let id = identifier(&tx)?;
            let session = Session { id: id.clone(), user_id: LOCAL_USER.into(), title: workspace.path.clone(),
                cwd: Some(workspace.path.clone()), workspace_path: workspace.key.clone(),
                retention_warned_at: None, created_at: now, updated_at: now };
            session.validate()?;
            tx.execute("INSERT INTO sessions(id,user_id,title,cwd,workspace_path,created_at,updated_at) VALUES (?1,?2,?3,?3,?4,?5,?5)",
                params![id, LOCAL_USER, workspace.path, workspace.key, now])?;
            tx.execute("INSERT INTO session_contexts(session_id,updated_at) VALUES (?1,?2)", params![id, now])?;
            claim(&tx, &id, &state.owner, now)?;
            tx.commit()?;
            state.active.insert(id);
            Ok(session)
        })
    }

    pub fn list_sessions(
        &self,
        workspace: &Workspace,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Session>> {
        let (limit, offset) = page(limit, offset)?;
        let workspace = workspace.clone();
        self.call(move |state| records::many(&state.connection,
            &format!("{} WHERE workspace_path=?1 AND user_id=?2 ORDER BY updated_at DESC,rowid DESC LIMIT ?3 OFFSET ?4", records::SESSION_JSON),
            params![workspace.key, LOCAL_USER, limit, offset]))
    }

    /// Abrir cancela o aviso de retenção e adquire o lease antes de retornar.
    pub fn load_session(&self, workspace: &Workspace, id: &str) -> Result<Option<Session>> {
        let id = Self::checked_session_id(id)?;
        let workspace = workspace.clone();
        self.call(move |state| {
            let now = state.now()?;
            let tx = state
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            if !scope(&tx, &workspace, &id)? {
                return Ok(None);
            }
            claim(&tx, &id, &state.owner, now)?;
            touch(&tx, &id, now)?;
            let result = records::session(&tx, &id)?;
            tx.commit()?;
            state.active.insert(id.clone());
            state.lost.remove(&id);
            Ok(Some(result))
        })
    }

    pub fn acquire_lease(&self, workspace: &Workspace, id: &str) -> Result<SessionLease> {
        let id = Self::checked_session_id(id)?;
        let workspace = workspace.clone();
        self.call(move |state| {
            let now = state.now()?;
            let tx = state
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_scope(&tx, &workspace, &id)?;
            let result = claim(&tx, &id, &state.owner, now)?;
            tx.commit()?;
            state.active.insert(id.clone());
            state.lost.remove(&id);
            Ok(result)
        })
    }
    pub fn session_lease(&self, workspace: &Workspace, id: &str) -> Result<Option<SessionLease>> {
        let id = Self::checked_session_id(id)?;
        let workspace = workspace.clone();
        self.call(move |state| {
            require_scope(&state.connection, &workspace, &id)?;
            lease(&state.connection, &id)
        })
    }
    pub fn release_lease(&self, workspace: &Workspace, id: &str) -> Result<()> {
        let id = Self::checked_session_id(id)?;
        let workspace = workspace.clone();
        self.call(move |state| {
            require_scope(&state.connection, &workspace, &id)?;
            state.connection.execute(
                "DELETE FROM session_leases WHERE session_id=?1 AND owner=?2",
                params![id, state.owner],
            )?;
            state.active.remove(&id);
            state.lost.remove(&id);
            Ok(())
        })
    }

    pub fn add_user_message(
        &self,
        workspace: &Workspace,
        session_id: &str,
        model: &str,
        mode: Mode,
        content: &str,
    ) -> Result<Message> {
        validate_user_content(content)?;
        let session_id = Self::checked_session_id(session_id)?;
        let workspace = workspace.clone();
        let model = model.to_owned();
        let content = content.trim().to_owned();
        self.call(move |state| {
            let now = state.now()?;
            let tx = state
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_scope(&tx, &workspace, &session_id)?;
            require_lease(&tx, &session_id, &state.owner, now, &state.lost)?;
            no_generation(&tx, &session_id)?;
            let message = Message {
                id: identifier(&tx)?,
                session_id,
                generation_id: None,
                role: Role::User,
                status: MessageStatus::Complete,
                model,
                mode,
                content,
                parts: vec![],
                duration: None,
                total_tokens: None,
                created_at: now,
            };
            write_message(&tx, &message)?;
            tx.commit()?;
            Ok(message)
        })
    }

    pub fn begin_assistant(
        &self,
        workspace: &Workspace,
        session_id: &str,
        model: &str,
        mode: Mode,
    ) -> Result<Message> {
        self.begin_assistant_generation(workspace, session_id, model, mode, None)
    }

    pub fn begin_assistant_generation(
        &self,
        workspace: &Workspace,
        session_id: &str,
        model: &str,
        mode: Mode,
        generation_id: Option<&str>,
    ) -> Result<Message> {
        let generation_id = generation_id.map(Self::checked_session_id).transpose()?;
        let session_id = Self::checked_session_id(session_id)?;
        let workspace = workspace.clone();
        let model = model.to_owned();
        self.call(move |state| {
            let now = state.now()?;
            let tx = state
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_scope(&tx, &workspace, &session_id)?;
            require_lease(&tx, &session_id, &state.owner, now, &state.lost)?;
            no_generation(&tx, &session_id)?;
            let id = identifier(&tx)?;
            let message = Message {
                generation_id: Some(generation_id.unwrap_or_else(|| id.clone())),
                id,
                session_id,
                role: Role::Assistant,
                status: MessageStatus::InProgress,
                model,
                mode,
                content: String::new(),
                parts: vec![],
                duration: None,
                total_tokens: None,
                created_at: now,
            };
            write_message(&tx, &message)?;
            tx.commit()?;
            Ok(message)
        })
    }

    pub fn list_messages(
        &self,
        workspace: &Workspace,
        session_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Message>> {
        let (limit, offset) = page(limit, offset)?;
        let id = Self::checked_session_id(session_id)?;
        let workspace = workspace.clone();
        self.call(move |state| {
            require_scope(&state.connection, &workspace, &id)?;
            records::many(
                &state.connection,
                &format!(
                    "{} WHERE session_id=?1 ORDER BY created_at,rowid LIMIT ?2 OFFSET ?3",
                    records::MESSAGE_JSON
                ),
                params![id, limit, offset],
            )
        })
    }

    pub fn append_part(
        &self,
        workspace: &Workspace,
        message_id: &str,
        part: MessagePart,
    ) -> Result<Message> {
        part.validate()?;
        self.edit_assistant(workspace, message_id, move |message| {
            match &part {
                MessagePart::ToolCall { id, .. } => {
                    if message.parts.iter().any(|p| matches!(p, MessagePart::ToolCall { id: old, .. } if old == id)) {
                        return Err(Error::ToolSequence);
                    }
                }
                MessagePart::ToolResult { id, name, .. } => {
                    if !message.parts.iter().any(|p| matches!(p, MessagePart::ToolCall { id: old, name: tool, .. } if old == id && tool == name)) {
                        return Err(Error::ToolSequence);
                    }
                    if let Some(old) = message.parts.iter().find(|p| matches!(p, MessagePart::ToolResult { id: old, .. } if old == id)) {
                        return if old == &part { Ok(()) } else { Err(Error::ToolSequence) };
                    }
                }
                _ => {}
            }
            if let MessagePart::Text { text } = &part {
                message.content.push_str(text);
            }
            myagent_core::context::push_part(&mut message.parts,part);
            Ok(())
        })
    }

    /// Deve ser persistido pelo core antes de iniciar qualquer efeito de ferramenta.
    pub fn mark_tool_started(
        &self,
        workspace: &Workspace,
        message_id: &str,
        call_id: &str,
    ) -> Result<Message> {
        validate_id(call_id)?;
        let call_id = call_id.to_owned();
        self.edit_assistant(workspace, message_id, move |message| {
            let part = message
                .parts
                .iter_mut()
                .find(|p| matches!(p, MessagePart::ToolCall { id, .. } if id == &call_id))
                .ok_or(Error::ToolSequence)?;
            if let MessagePart::ToolCall { execution, .. } = part {
                *execution = Some(ExecutionState::Started);
            }
            Ok(())
        })
    }

    pub fn finish_assistant(
        &self,
        workspace: &Workspace,
        message_id: &str,
        status: MessageStatus,
        duration: Option<u32>,
        total_tokens: Option<u32>,
    ) -> Result<Message> {
        if status == MessageStatus::InProgress {
            return Err(Error::MessageNotInProgress);
        }
        self.edit_assistant(workspace, message_id, move |message| {
            if status == MessageStatus::Complete {
                for part in &message.parts {
                    if let MessagePart::ToolCall { id, name, .. } = part
                        && !message.parts.iter().any(|p| matches!(p, MessagePart::ToolResult { id: result, name: tool, .. } if result == id && tool == name))
                    { return Err(Error::ToolSequence); }
                }
            }
            message.status = status;
            message.duration = duration;
            message.total_tokens = total_tokens;
            Ok(())
        })
    }

    fn edit_assistant(
        &self,
        workspace: &Workspace,
        message_id: &str,
        edit: impl FnOnce(&mut Message) -> Result<()> + Send + 'static,
    ) -> Result<Message> {
        let id = Self::checked_session_id(message_id)?;
        let workspace = workspace.clone();
        self.call(move |state| {
            let now = state.now()?;
            let tx = state
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut message = records::message(&tx, &id)?;
            require_scope(&tx, &workspace, &message.session_id)?;
            require_lease(&tx, &message.session_id, &state.owner, now, &state.lost)?;
            if message.role != Role::Assistant || message.status != MessageStatus::InProgress {
                return Err(Error::MessageNotInProgress);
            }
            edit(&mut message)?;
            update_message(&tx, &message, now)?;
            tx.commit()?;
            Ok(message)
        })
    }

    pub fn load_context(&self, workspace: &Workspace, session_id: &str) -> Result<SessionContext> {
        let id = Self::checked_session_id(session_id)?;
        let workspace = workspace.clone();
        self.call(move |state| {
            require_scope(&state.connection, &workspace, &id)?;
            records::context(&state.connection, &id)
        })
    }
    pub fn save_context(
        &self,
        workspace: &Workspace,
        mut context: SessionContext,
    ) -> Result<SessionContext> {
        let _validated = records::json_text(&context)?;
        let workspace = workspace.clone();
        self.call(move |state| {
            let now = state.now()?;
            let tx = state.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_scope(&tx, &workspace, &context.session_id)?;
            require_lease(&tx, &context.session_id, &state.owner, now, &state.lost)?;
            context.updated_at = now;
            let _validated = records::json_text(&context)?;
            let messages = serde_json::to_string(&context.messages).map_err(|_| ValidationError::Json)?;
            tx.execute("INSERT INTO session_contexts(session_id,messages_json,token_limit,estimated_tokens,truncated,updated_at)
                VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id) DO UPDATE SET
                messages_json=excluded.messages_json,token_limit=excluded.token_limit,
                estimated_tokens=excluded.estimated_tokens,truncated=excluded.truncated,updated_at=excluded.updated_at",
                params![context.session_id,messages,context.token_limit,context.estimated_tokens,context.truncated,context.updated_at])?;
            tx.commit()?;
            Ok(context)
        })
    }

    pub fn heartbeat(&self) -> Result<()> {
        self.call(State::heartbeat)
    }
    /// Permite ao futuro loop cancelar gerações que perderam a exclusão.
    pub fn lost_sessions(&self) -> Result<Vec<String>> {
        self.call(|state| Ok(state.lost.iter().cloned().collect()))
    }
    pub fn cleanup(&self) -> Result<RetentionReport> {
        self.call(State::cleanup)
    }
}

impl State {
    pub fn heartbeat(&mut self) -> Result<()> {
        let now = self.now()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut lost = Vec::new();
        for id in &self.active {
            let changed = tx.execute(
                "UPDATE session_leases SET expires_at=MAX(expires_at,?1)
                WHERE session_id=?2 AND owner=?3 AND expires_at>?4",
                params![now + LEASE_TIMEOUT_MS, id, self.owner, now],
            )?;
            if changed != 1 {
                lost.push(id.clone());
            }
        }
        tx.commit()?;
        for id in lost {
            self.active.remove(&id);
            self.lost.insert(id);
        }
        Ok(())
    }

    pub fn cleanup(&mut self) -> Result<RetentionReport> {
        if self.retention_days == 0 {
            return Ok(RetentionReport::default());
        }
        let now = self.now()?;
        let cutoff = now.saturating_sub(i64::from(self.retention_days) * DAY_MS);
        let grace = now.saturating_sub(RETENTION_GRACE_MS);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sql = format!("UPDATE sessions SET retention_warned_at=?2
            WHERE id IN (SELECT id FROM sessions WHERE {INACTIVE} AND retention_warned_at IS NULL LIMIT 100)
            RETURNING id");
        let warned = {
            let mut statement = tx.prepare(&sql)?;
            let rows = statement.query_map(params![cutoff, now], |r| r.get::<_, String>(0))?;
            let ids = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            ids.into_iter()
                .map(|session_id| RetentionWarning {
                    session_id,
                    delete_after: now.saturating_add(RETENTION_GRACE_MS),
                })
                .collect()
        };
        let deleted = tx.execute(
            &format!("DELETE FROM sessions WHERE {INACTIVE} AND retention_warned_at IS NOT NULL AND retention_warned_at<=?3"),
            params![cutoff, now, grace])?;
        tx.commit()?;
        Ok(RetentionReport { warned, deleted })
    }
}
