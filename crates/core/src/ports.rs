use myagent_protocol::{Message, MessagePart, MessageStatus, Mode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub type Result<T> = std::result::Result<T, AgentError>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentError {
    #[error("generation already active")]
    Busy,
    #[error("generation cancelled")]
    Cancelled,
    #[error("generation deadline exceeded")]
    Timeout,
    #[error("invalid provider response")]
    Protocol,
    #[error("provider request failed")]
    Network,
    #[error("provider authentication missing or rejected")]
    Authentication,
    #[error("resource limit exceeded")]
    Limit,
    #[error("invalid configuration")]
    Configuration,
    #[error("context cannot fit the current request")]
    Context,
    #[error("persistence failed; generation cannot continue")]
    Persistence,
    #[error("session lease lost")]
    LeaseLost,
    #[error("tool is unavailable in this mode")]
    ToolDenied,
    #[error("identical tool call repeated too often")]
    ToolLoop,
    #[error("previous side effect must not be replayed")]
    Replay,
    #[error("tool execution failed")]
    Tool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    User,
    Assistant,
    Tool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelMessage {
    pub role: ModelRole,
    pub content: Vec<MessagePart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}
impl ModelMessage {
    pub fn text(role: ModelRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![MessagePart::Text { text: text.into() }],
            provider_metadata: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Read,
    Network,
    IsolatedProcess,
    Write,
}
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub effect: Effect,
}
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Map<String, Value>,
}
#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub cancellation: CancellationToken,
}
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    /// Deve validar schema e obter aprovação antes de qualquer I/O de projeto.
    fn authorize<'a>(
        &'a self,
        args: &'a Map<String, Value>,
        context: ToolContext,
    ) -> BoxFuture<'a, Result<()>>;
    /// O futuro deve respeitar o token e encerrar processos antes de terminar.
    fn execute<'a>(
        &'a self,
        args: Map<String, Value>,
        context: ToolContext,
    ) -> BoxFuture<'a, Result<String>>;
}
#[derive(Clone, Debug)]
pub struct ProviderRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<ToolSpec>,
    pub max_output_tokens: u32,
}
#[derive(Clone, Debug)]
pub enum ProviderEvent {
    Text(String),
    Reasoning(String),
    ToolCall(ToolCall),
    Usage(u32),
    /// Estado opaco para signatures de thinking/function calls; nunca permissões.
    Metadata(Value),
    Done,
}
pub trait Provider: Send + Sync {
    fn stream(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        events: mpsc::Sender<ProviderEvent>,
    ) -> BoxFuture<'_, Result<()>>;
}
pub struct GenerationState {
    pub message: Message,
    pub history: Vec<ModelMessage>,
    pub previous: Vec<MessagePart>,
}
pub struct PrepareGeneration {
    pub session_id: String,
    pub model: String,
    pub mode: Mode,
    pub content: Option<String>,
    pub resume: bool,
}
/// Interface síncrona: o core faz a ponte com spawn_blocking.
pub trait Journal: Send + Sync + 'static {
    fn prepare(&self, input: PrepareGeneration) -> Result<GenerationState>;
    fn append(&self, id: &str, part: MessagePart) -> Result<()>;
    fn started(&self, id: &str, call: &str) -> Result<()>;
    fn usage(&self, id: &str, total: Option<u32>) -> Result<()>;
    fn finish(
        &self,
        id: &str,
        status: MessageStatus,
        duration: u32,
        tokens: Option<u32>,
    ) -> Result<()>;
    fn context(
        &self,
        session: &str,
        messages: Vec<ModelMessage>,
        limit: u32,
        estimated: u32,
        truncated: bool,
    ) -> Result<Vec<ModelMessage>>;
    fn check_lease(&self, session: &str) -> Result<()>;
}
pub async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| AgentError::Persistence)?
}
pub type SharedJournal = Arc<dyn Journal>;
