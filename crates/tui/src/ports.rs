use crate::approval::Decision;
use myagent_core::{BoxFuture, CancellationToken, Result};
use myagent_protocol::{Message, Mode, Session, StreamEvent};
use tokio::sync::mpsc;
#[derive(Clone, Debug)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub configured: bool,
}
#[derive(Clone, Debug)]
pub struct SessionView {
    pub session: Session,
    pub messages: Vec<Message>,
    pub model: String,
    pub mode: Mode,
    pub measured_tokens: Option<u64>,
    pub unreported: usize,
}
#[derive(Clone, Debug)]
pub struct Submission {
    pub session: String,
    pub model: String,
    pub mode: Mode,
    pub content: Option<String>,
}
#[derive(Clone, Debug)]
pub enum AccountOperation {
    Status,
    Login(Option<String>),
    Activate(String),
    Logout(String),
}
#[derive(Clone, Debug, Default)]
pub struct AccountView {
    pub choices: Vec<(String, String)>,
    pub note: String,
}
pub trait Backend: Send + Sync + 'static {
    fn models(&self) -> Vec<ModelChoice>;
    fn refresh_models(
        &self,
        _cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<Vec<ModelChoice>>> {
        Box::pin(async { Ok(self.models()) })
    }
    fn account(
        &self,
        _op: AccountOperation,
        _cancel: CancellationToken,
        _urls: mpsc::Sender<String>,
    ) -> BoxFuture<'_, Result<AccountView>> {
        Box::pin(async { Err(myagent_core::AgentError::Configuration) })
    }
    fn sessions(&self) -> BoxFuture<'_, Result<Vec<Session>>>;
    fn new_session(&self) -> BoxFuture<'_, Result<SessionView>>;
    fn load_session(&self, id: String) -> BoxFuture<'_, Result<SessionView>>;
    fn generate(
        &self,
        input: Submission,
        cancel: CancellationToken,
        events: mpsc::Sender<StreamEvent>,
    ) -> BoxFuture<'_, Result<()>>;
    fn undo(
        &self,
        session: String,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<SessionView>>;
    fn approve(&self, session: &str, id: &str, choice: Decision) -> bool;
    fn forget(&self, session: &str);
    fn file_candidates(&self, prefix: String) -> BoxFuture<'_, Result<Vec<String>>>;
}
