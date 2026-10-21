#![forbid(unsafe_code)]

//! Contratos versionáveis sem acesso a arquivos, rede ou credenciais.
//! Nas bordas, use decode_json/encode_json e Validate, não apenas os derives serde.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use std::io::{self, Write};

pub const MAX_MESSAGE_LENGTH: usize = 9_000;
pub const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PARTS: usize = 10_000;
pub const SESSION_APPROVAL_FORBIDDEN: &[&str] = &[
    "git_commit",
    "delete_file",
    "git_push",
    "git_remote_add",
    "git_init",
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("invalid field: {0}")]
    Field(&'static str),
    #[error("invalid JSON payload")]
    Json,
    #[error("payload exceeds the resource limit")]
    Limit,
}

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

pub fn decode_json<T: DeserializeOwned + Validate>(bytes: &[u8]) -> Result<T, ValidationError> {
    if bytes.len() > MAX_JSON_BYTES {
        return Err(ValidationError::Limit);
    }
    let value: T = serde_json::from_slice(bytes).map_err(|_| ValidationError::Json)?;
    value.validate()?;
    Ok(value)
}

struct LimitedBuffer(Vec<u8>);
impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_JSON_BYTES - self.0.len() {
            return Err(io::Error::other("JSON size limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn encode_json<T: Serialize + Validate>(value: &T) -> Result<Vec<u8>, ValidationError> {
    value.validate()?;
    let mut buffer = LimitedBuffer(Vec::new());
    serde_json::to_writer(&mut buffer, value).map_err(|_| ValidationError::Limit)?;
    Ok(buffer.0)
}

pub fn validate_id(value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ValidationError::Field("id"));
    }
    Ok(())
}

pub fn validate_user_content(value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.trim().encode_utf16().count() > MAX_MESSAGE_LENGTH {
        return Err(ValidationError::Field("content"));
    }
    Ok(())
}

fn text(value: &str, field: &'static str, maximum: usize) -> Result<(), ValidationError> {
    if value.len() > maximum || value.contains('\0') {
        return Err(ValidationError::Field(field));
    }
    Ok(())
}

fn required(value: &str, field: &'static str, maximum: usize) -> Result<(), ValidationError> {
    text(value, field, maximum)?;
    if value.trim().is_empty() {
        return Err(ValidationError::Field(field));
    }
    Ok(())
}

fn timestamp(value: i64) -> Result<(), ValidationError> {
    if value < 0 {
        return Err(ValidationError::Field("timestamp"));
    }
    Ok(())
}

fn count(value: u32) -> Result<(), ValidationError> {
    if value > i32::MAX as u32 {
        return Err(ValidationError::Field("count"));
    }
    Ok(())
}

fn json_value(value: &Value) -> Result<(), ValidationError> {
    let mut pending = vec![(value, 0_u8)];
    let mut entries = 0;
    while let Some((value, depth)) = pending.pop() {
        entries += 1;
        if entries > 100_000 || depth > 64 {
            return Err(ValidationError::Limit);
        }
        match value {
            Value::Array(items) => pending.extend(items.iter().map(|v| (v, depth + 1))),
            Value::Object(items) => {
                for (key, value) in items {
                    text(key, "JSON key", 4096)?;
                    pending.push((value, depth + 1));
                }
            }
            Value::String(value) => text(value, "JSON string", MAX_JSON_BYTES)?,
            _ => {}
        }
    }
    Ok(())
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Mode {
    Plan,
    Build,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Role {
    User,
    Assistant,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageStatus {
    InProgress,
    Complete,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionState {
    Pending,
    Started,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MessagePart {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        args: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution: Option<ExecutionState>,
    },
    ToolResult {
        id: String,
        name: String,
        result: String,
    },
    ContextWarning {
        context_window: u32,
        estimated: bool,
        tool_count: u32,
        message: String,
    },
    ToolLimitWarning {
        limit: u16,
    },
}

impl Validate for MessagePart {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Text { text: value } | Self::Reasoning { text: value } => {
                text(value, "text", MAX_JSON_BYTES)
            }
            Self::ToolCall { id, name, args, .. } => {
                validate_id(id)?;
                required(name, "tool name", 200)?;
                json_value(&Value::Object(args.clone()))
            }
            Self::ToolResult { id, name, result } => {
                validate_id(id)?;
                required(name, "tool name", 200)?;
                text(result, "result", MAX_JSON_BYTES)
            }
            Self::ContextWarning {
                context_window,
                tool_count,
                message,
                ..
            } => {
                if *context_window == 0 {
                    return Err(ValidationError::Field("contextWindow"));
                }
                count(*context_window)?;
                count(*tool_count)?;
                text(message, "message", MAX_JSON_BYTES)
            }
            Self::ToolLimitWarning { limit } if (1..=200).contains(limit) => Ok(()),
            Self::ToolLimitWarning { .. } => Err(ValidationError::Field("limit")),
        }
    }
}

pub fn validate_parts(parts: &[MessagePart]) -> Result<(), ValidationError> {
    if parts.len() > MAX_PARTS {
        return Err(ValidationError::Limit);
    }
    for part in parts {
        part.validate()?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub workspace_path: String,
    pub retention_warned_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}
impl Validate for Session {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.id)?;
        validate_id(&self.user_id)?;
        required(&self.title, "title", 32768)?;
        required(&self.workspace_path, "workspacePath", 32768)?;
        if let Some(cwd) = &self.cwd {
            required(cwd, "cwd", 32768)?;
        }
        timestamp(self.created_at)?;
        timestamp(self.updated_at)?;
        if let Some(time) = self.retention_warned_at {
            timestamp(time)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub generation_id: Option<String>,
    pub role: Role,
    pub status: MessageStatus,
    pub model: String,
    pub mode: Mode,
    pub content: String,
    pub parts: Vec<MessagePart>,
    /// Segundos, como Message.duration no legado; eventos done usam milissegundos.
    pub duration: Option<u32>,
    pub total_tokens: Option<u32>,
    pub created_at: i64,
}
impl Validate for Message {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.id)?;
        validate_id(&self.session_id)?;
        if let Some(id) = &self.generation_id {
            validate_id(id)?;
        }
        required(&self.model, "model", 200)?;
        text(&self.content, "content", MAX_JSON_BYTES)?;
        if self.role == Role::User {
            validate_user_content(&self.content)?;
            if self.status != MessageStatus::Complete {
                return Err(ValidationError::Field("status"));
            }
        } else if self.role == Role::Error && self.status == MessageStatus::InProgress {
            return Err(ValidationError::Field("status"));
        }
        validate_parts(&self.parts)?;
        if let Some(value) = self.duration {
            count(value)?;
        }
        if let Some(value) = self.total_tokens {
            count(value)?;
        }
        timestamp(self.created_at)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionContext {
    pub session_id: String,
    /// Envelopes JSON preservados, incluindo metadados específicos de providers.
    pub messages: Vec<Value>,
    pub token_limit: u32,
    pub estimated_tokens: u32,
    pub truncated: bool,
    pub updated_at: i64,
}
impl Validate for SessionContext {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.session_id)?;
        if self.token_limit == 0
            || self.estimated_tokens > self.token_limit
            || self.messages.len() > MAX_PARTS
        {
            return Err(ValidationError::Field("context"));
        }
        count(self.token_limit)?;
        count(self.estimated_tokens)?;
        for value in &self.messages {
            json_value(value)?;
            let object = value
                .as_object()
                .ok_or(ValidationError::Field("messages"))?;
            if !matches!(
                object.get("role").and_then(Value::as_str),
                Some("user" | "assistant" | "tool" | "system")
            ) || !object
                .get("content")
                .is_some_and(|v| v.is_string() || v.is_array())
            {
                return Err(ValidationError::Field("messages"));
            }
            if let Some(parts) = object.get("content").and_then(Value::as_array) {
                for part in parts {
                    let kind = part
                        .as_object()
                        .and_then(|part| part.get("type"))
                        .and_then(Value::as_str)
                        .ok_or(ValidationError::Field("messages.content"))?;
                    required(kind, "messages.content.type", 200)?;
                }
            }
        }
        timestamp(self.updated_at)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionLease {
    pub session_id: String,
    pub owner: String,
    pub expires_at: i64,
}
impl Validate for SessionLease {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_id(&self.session_id)?;
        validate_id(&self.owner)?;
        timestamp(self.expires_at)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum StreamEvent {
    GenerationStarted {
        message_id: String,
    },
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        args: Map<String, Value>,
    },
    /// Ephemeral, potentially incomplete JSON; not execution evidence.
    ToolPreview {
        tool_call_id: String,
        tool_name: String,
        arguments: String,
    },
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        result: String,
    },
    Usage {
        total_tokens: u32,
    },
    ApprovalRequired {
        approval_id: String,
        tool_name: String,
        args: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirmation: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allow_session: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
    },
    ContextWarning {
        context_window: u32,
        estimated: bool,
        tool_count: u32,
        message: String,
    },
    ToolLimitWarning {
        limit: u16,
    },
    Done {
        message_id: String,
        duration_ms: u64,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<StreamErrorKind>,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parts: Option<Vec<MessagePart>>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamErrorKind {
    Context,
}

impl Validate for StreamEvent {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::GenerationStarted { message_id } => validate_id(message_id),
            Self::Done {
                message_id,
                duration_ms,
            } => {
                validate_id(message_id)?;
                if *duration_ms > i64::MAX as u64 {
                    return Err(ValidationError::Field("durationMs"));
                }
                Ok(())
            }
            Self::TextDelta { text: value } | Self::ReasoningDelta { text: value } => {
                text(value, "text", MAX_JSON_BYTES)
            }
            Self::Usage { total_tokens } => count(*total_tokens),
            Self::ToolPreview {
                tool_call_id,
                tool_name,
                arguments,
            } => {
                validate_id(tool_call_id)?;
                validate_id(tool_name)?;
                text(arguments, "preview arguments", 65536)
            }
            Self::ToolCall {
                tool_call_id,
                tool_name,
                args,
            } => MessagePart::ToolCall {
                id: tool_call_id.clone(),
                name: tool_name.clone(),
                args: args.clone(),
                execution: None,
            }
            .validate(),
            Self::ToolResult {
                tool_call_id,
                tool_name,
                result,
            } => MessagePart::ToolResult {
                id: tool_call_id.clone(),
                name: tool_name.clone(),
                result: result.clone(),
            }
            .validate(),
            Self::ApprovalRequired {
                approval_id,
                tool_name,
                args,
                confirmation,
                scope,
                tool_call_id,
                ..
            } => {
                validate_id(approval_id)?;
                required(tool_name, "toolName", 200)?;
                json_value(&Value::Object(args.clone()))?;
                if let Some(value) = confirmation {
                    text(value, "confirmation", 200)?;
                }
                if let Some(value) = scope {
                    text(value, "scope", 32768)?;
                }
                if let Some(value) = tool_call_id {
                    validate_id(value)?;
                }
                Ok(())
            }
            Self::ContextWarning {
                context_window,
                estimated,
                tool_count,
                message,
            } => MessagePart::ContextWarning {
                context_window: *context_window,
                estimated: *estimated,
                tool_count: *tool_count,
                message: message.clone(),
            }
            .validate(),
            Self::ToolLimitWarning { limit } => {
                MessagePart::ToolLimitWarning { limit: *limit }.validate()
            }
            Self::Error {
                message,
                message_id,
                parts,
                ..
            } => {
                text(message, "message", MAX_JSON_BYTES)?;
                if let Some(id) = message_id {
                    validate_id(id)?;
                }
                if let Some(parts) = parts {
                    validate_parts(parts)?;
                }
                Ok(())
            }
        }
    }
}
