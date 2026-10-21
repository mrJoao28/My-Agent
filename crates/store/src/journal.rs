use crate::{Error, Result as StoreResult, Store, Workspace, records};
use myagent_core::{
    AgentError, GenerationState, Journal, ModelMessage, PrepareGeneration, Result, context,
};
use myagent_protocol::{Message, MessagePart, MessageStatus, Role, SessionContext, Validate};
use rusqlite::{TransactionBehavior, params};
#[derive(Clone)]
pub struct StoreJournal {
    store: Store,
    workspace: Workspace,
}
impl StoreJournal {
    pub fn new(store: Store, workspace: Workspace) -> Self {
        Self { store, workspace }
    }
}
fn map_error(error: Error) -> AgentError {
    match error {
        Error::SessionBusy | Error::GenerationActive => AgentError::Busy,
        Error::LeaseLost => AgentError::LeaseLost,
        _ => AgentError::Persistence,
    }
}
impl Store {
    /// Recuperação inicial: não altera gerações cobertas por lease vivo de outro processo.
    pub fn recover_orphaned_generations(&self, workspace: &Workspace) -> StoreResult<usize> {
        let workspace = workspace.clone();
        self.call(move|state|{
            let now=state.now()?;
            Ok(state.connection.execute("UPDATE messages SET status='INTERRUPTED'
                WHERE role='ASSISTANT' AND status='IN_PROGRESS'
                AND session_id IN (SELECT id FROM sessions WHERE workspace_path=?1 AND user_id='local-user')
                AND NOT EXISTS (SELECT 1 FROM session_leases l WHERE l.session_id=messages.session_id AND l.expires_at>?2)",
                params![workspace.key,now])?)
        })
    }
    fn interrupt_recovered(&self, workspace: &Workspace, session: &str) -> StoreResult<()> {
        let workspace = workspace.clone();
        let session = Self::checked_session_id(session)?;
        self.call(move|state|{
            let now=state.now()?;
            let tx=state.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let valid=tx.query_row("SELECT count(*) FROM sessions s JOIN session_leases l ON l.session_id=s.id
                WHERE s.id=?1 AND s.workspace_path=?2 AND s.user_id='local-user' AND l.owner=?3 AND l.expires_at>?4",
                params![session,workspace.key,state.owner,now],|row|row.get::<_,u32>(0))?;
            if valid!=1||state.lost.contains(&session){return Err(Error::LeaseLost);}
            tx.execute("UPDATE messages SET status='INTERRUPTED' WHERE session_id=?1 AND role='ASSISTANT' AND status='IN_PROGRESS'",[session])?;
            tx.commit()?;Ok(())
        })
    }
    fn update_usage(&self, workspace: &Workspace, id: &str, total: Option<u32>) -> StoreResult<()> {
        let workspace = workspace.clone();
        let id = Self::checked_session_id(id)?;
        self.call(move|state|{
            let now=state.now()?;
            let tx=state.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let message=records::message(&tx,&id)?;
            let valid=tx.query_row("SELECT count(*) FROM sessions s JOIN session_leases l ON l.session_id=s.id
                WHERE s.id=?1 AND s.workspace_path=?2 AND s.user_id='local-user' AND l.owner=?3 AND l.expires_at>?4",
                params![message.session_id,workspace.key,state.owner,now],|row|row.get::<_,u32>(0))?;
            if valid!=1||state.lost.contains(&message.session_id){return Err(Error::LeaseLost);}
            let mut checked=message.clone();checked.total_tokens=total;checked.validate()?;
            let changed=tx.execute("UPDATE messages SET total_tokens=?1 WHERE id=?2 AND status='IN_PROGRESS' AND role='ASSISTANT'",params![total,id])?;
            if changed!=1{return Err(Error::MessageNotInProgress);}
            tx.commit()?;Ok(())
        })
    }
}
impl Journal for StoreJournal {
    fn check_session(&self, session: &str, workspace: &str, idle: bool) -> Result<()> {
        let matches = if cfg!(windows) {
            workspace.eq_ignore_ascii_case(&self.workspace.path)
        } else {
            workspace == self.workspace.path
        };
        if !matches {
            return Err(AgentError::Configuration);
        }
        self.store
            .check_local_session(&self.workspace, session, idle)
            .map_err(map_error)
    }
    fn local_note(&self, session: &str, note: &str) -> Result<()> {
        self.store
            .append_undo_note(&self.workspace, session, note)
            .map(|_| ())
            .map_err(map_error)
    }
    fn prepare(&self, input: PrepareGeneration) -> Result<GenerationState> {
        if self
            .store
            .load_session(&self.workspace, &input.session_id)
            .map_err(map_error)?
            .is_none()
        {
            return Err(AgentError::Persistence);
        }
        self.store
            .interrupt_recovered(&self.workspace, &input.session_id)
            .map_err(map_error)?;
        let mut rows: Vec<Message> = Vec::new();
        let mut offset = 0;
        loop {
            let page = self
                .store
                .list_messages(&self.workspace, &input.session_id, 500, offset)
                .map_err(map_error)?;
            let len = page.len();
            rows.extend(page);
            if len < 500 {
                break;
            }
            offset += 500;
            if offset > 10000 {
                return Err(AgentError::Limit);
            }
        }
        let mut previous = Vec::new();
        let mut generation = None;
        if input.resume {
            let last = rows.last().ok_or(AgentError::Configuration)?;
            if last.model != input.model
                || last.mode != input.mode
                || !(last.role == Role::User
                    || last.role == Role::Assistant && last.status == MessageStatus::Interrupted)
            {
                return Err(AgentError::Configuration);
            }
            if last.role == Role::Assistant {
                generation = Some(
                    last.generation_id
                        .clone()
                        .unwrap_or_else(|| last.id.clone()),
                );
                for row in &rows {
                    if row.generation_id.as_ref() == generation.as_ref()
                        || Some(&row.id) == generation.as_ref()
                    {
                        previous.extend(row.parts.clone());
                    }
                }
            }
        } else {
            let content = input.content.as_deref().ok_or(AgentError::Configuration)?;
            let user = self
                .store
                .add_user_message(
                    &self.workspace,
                    &input.session_id,
                    &input.model,
                    input.mode,
                    content,
                )
                .map_err(map_error)?;
            rows.push(user);
        }
        let mut history = context::history(&rows);
        // Reaplica somente metadados associados ao mesmo conjunto de tool IDs.
        if generation.is_some()
            && let Ok(saved) = self.store.load_context(&self.workspace, &input.session_id)
        {
            let prior: Vec<ModelMessage> = saved
                .messages
                .into_iter()
                .filter_map(|value| serde_json::from_value(value).ok())
                .collect();
            for message in &mut history {
                let ids: Vec<_> = message
                    .content
                    .iter()
                    .filter_map(|p| match p {
                        MessagePart::ToolCall { id, .. }
                            if previous.iter().any(
                                |p| matches!(p,MessagePart::ToolCall{id:old,..} if old==id),
                            ) =>
                        {
                            Some(id)
                        }
                        _ => None,
                    })
                    .collect();
                if ids.is_empty() {
                    continue;
                }
                if let Some(old) = prior.iter().find(|m| {
                    m.content
                        .iter()
                        .any(|p| matches!(p,MessagePart::ToolCall{id,..} if ids.contains(&id)))
                }) {
                    message.provider_metadata = old.provider_metadata.clone();
                }
            }
        }
        let message = self
            .store
            .begin_assistant_generation(
                &self.workspace,
                &input.session_id,
                &input.model,
                input.mode,
                generation.as_deref(),
            )
            .map_err(map_error)?;
        Ok(GenerationState {
            message,
            history,
            previous,
        })
    }
    fn append(&self, id: &str, part: MessagePart) -> Result<()> {
        self.store
            .append_part(&self.workspace, id, part)
            .map(|_| ())
            .map_err(map_error)
    }
    fn started(&self, id: &str, call: &str) -> Result<()> {
        self.store
            .mark_tool_started(&self.workspace, id, call)
            .map(|_| ())
            .map_err(map_error)
    }
    fn usage(&self, id: &str, total: Option<u32>) -> Result<()> {
        self.store
            .update_usage(&self.workspace, id, total)
            .map_err(map_error)
    }
    fn finish(
        &self,
        id: &str,
        status: MessageStatus,
        duration: u32,
        tokens: Option<u32>,
    ) -> Result<()> {
        self.store
            .finish_assistant(&self.workspace, id, status, Some(duration), tokens)
            .map(|_| ())
            .map_err(map_error)
    }
    fn context(
        &self,
        session: &str,
        messages: Vec<ModelMessage>,
        limit: u32,
        estimated: u32,
        truncated: bool,
    ) -> Result<Vec<ModelMessage>> {
        let encoded = messages
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| AgentError::Context)?;
        let saved = self
            .store
            .save_context(
                &self.workspace,
                SessionContext {
                    session_id: session.into(),
                    messages: encoded,
                    token_limit: limit,
                    estimated_tokens: estimated,
                    truncated,
                    updated_at: 0,
                },
            )
            .map_err(map_error)?;
        saved
            .messages
            .into_iter()
            .map(|v| serde_json::from_value(v).map_err(|_| AgentError::Context))
            .collect()
    }
    fn check_lease(&self, session: &str) -> Result<()> {
        let session = session.to_owned();
        let workspace = self.workspace.clone();
        self.store.call(move|state|{
            let now=state.now()?;
            if state.lost.contains(&session){return Err(Error::LeaseLost);}
            let valid=state.connection.query_row("SELECT count(*) FROM sessions s JOIN session_leases l ON l.session_id=s.id
                WHERE s.id=?1 AND s.workspace_path=?2 AND s.user_id='local-user' AND l.owner=?3 AND l.expires_at>?4",
                params![session,workspace.key,state.owner,now],|row|row.get::<_,u32>(0))?;
            if valid!=1{return Err(Error::LeaseLost);}Ok(())
        }).map_err(map_error)
    }
}
