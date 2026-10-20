use crate::{Error, Result};
use myagent_protocol::{Message, Session, SessionContext, Validate, decode_json, encode_json};
use rusqlite::{Connection, OptionalExtension, Params};
use serde::de::DeserializeOwned;

pub(crate) const SESSION_JSON: &str = "SELECT json_object(
'id',id,'userId',user_id,'title',title,'cwd',cwd,'workspacePath',workspace_path,
'retentionWarnedAt',retention_warned_at,'createdAt',created_at,'updatedAt',updated_at) FROM sessions";
pub(crate) const MESSAGE_JSON: &str = "SELECT json_object(
'id',id,'sessionId',session_id,'generationId',generation_id,'role',role,'status',status,
'model',model,'mode',mode,'content',content,'parts',json(parts_json),
'duration',duration,'totalTokens',total_tokens,'createdAt',created_at) FROM messages";
pub(crate) const CONTEXT_JSON: &str = "SELECT json_object(
'sessionId',session_id,'messages',json(messages_json),'tokenLimit',token_limit,
'estimatedTokens',estimated_tokens,'truncated',json(CASE WHEN truncated=1 THEN 'true' ELSE 'false' END),
'updatedAt',updated_at) FROM session_contexts";

pub(crate) fn one<T: DeserializeOwned + Validate>(
    db: &Connection,
    sql: &str,
    params: impl Params,
) -> Result<Option<T>> {
    let value: Option<String> = db.query_row(sql, params, |r| r.get(0)).optional()?;
    value
        .map(|s| decode_json(s.as_bytes()).map_err(Error::from))
        .transpose()
}
pub(crate) fn many<T: DeserializeOwned + Validate>(
    db: &Connection,
    sql: &str,
    params: impl Params,
) -> Result<Vec<T>> {
    let mut query = db.prepare(sql)?;
    let rows = query.query_map(params, |r| r.get::<_, String>(0))?;
    let mut result = Vec::new();
    let mut remaining = myagent_protocol::MAX_JSON_BYTES;
    for row in rows {
        let body = row?;
        if body.len() > remaining {
            return Err(myagent_protocol::ValidationError::Limit.into());
        }
        remaining -= body.len();
        result.push(decode_json(body.as_bytes())?);
    }
    Ok(result)
}
pub(crate) fn session(db: &Connection, id: &str) -> Result<Session> {
    one(db, &format!("{SESSION_JSON} WHERE id=?1"), [id])?.ok_or(Error::SessionNotFound)
}
pub(crate) fn message(db: &Connection, id: &str) -> Result<Message> {
    one(db, &format!("{MESSAGE_JSON} WHERE id=?1"), [id])?.ok_or(Error::MessageNotInProgress)
}
pub(crate) fn context(db: &Connection, id: &str) -> Result<SessionContext> {
    one(db, &format!("{CONTEXT_JSON} WHERE session_id=?1"), [id])?.ok_or(Error::SessionNotFound)
}
pub(crate) fn json_text<T: serde::Serialize + Validate>(value: &T) -> Result<String> {
    String::from_utf8(encode_json(value)?)
        .map_err(|_| myagent_protocol::ValidationError::Json.into())
}
pub(crate) fn label<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| myagent_protocol::ValidationError::Json.into())
}
