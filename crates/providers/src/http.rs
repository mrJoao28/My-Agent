use crate::{
    registry::{Model, ProviderKind},
    sse::SseDecoder,
    wire::{self, WireParser},
};
use myagent_core::{
    AgentError, BoxFuture, CancellationToken, Provider, ProviderEvent, ProviderRequest, Result,
};
use reqwest::{
    Client, Url,
    header::{ACCEPT, CONTENT_TYPE, HeaderValue},
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::sync::mpsc;
static REQUEST_NONCE: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Debug)]
pub struct StreamLimits {
    pub frame_bytes: usize,
    pub response_bytes: usize,
}
impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            frame_bytes: 1024 * 1024,
            response_bytes: 16 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ProviderEndpoints {
    pub anthropic: String,
    pub openai: String,
    pub google: String,
    pub ollama: String,
}
impl Default for ProviderEndpoints {
    fn default() -> Self {
        Self {
            anthropic: "https://api.anthropic.com/v1/messages".into(),
            openai: "https://api.openai.com/v1/responses".into(),
            google: "https://generativelanguage.googleapis.com/v1beta/models/".into(),
            ollama: "http://localhost:11434/v1".into(),
        }
    }
}
pub struct HttpProvider {
    client: Client,
    kind: ProviderKind,
    key: HeaderValue,
    endpoint: Url,
    limits: StreamLimits,
}
impl std::fmt::Debug for HttpProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpProvider")
            .field("kind", &self.kind)
            .field("credentials", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
impl HttpProvider {
    /// lookup injetável; testes usam somente valores fictícios, sem ler ambiente real.
    pub fn resolve(model: &Model, lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let mut endpoints = ProviderEndpoints::default();
        if model.provider == ProviderKind::Ollama
            && let Some(base) = lookup("OLLAMA_BASE_URL")
        {
            endpoints.ollama = base;
        }
        let key = if model.provider == ProviderKind::Ollama {
            lookup("OLLAMA_API_KEY").unwrap_or_else(|| "ollama".into())
        } else {
            lookup(&model.api_key_env().ok_or(AgentError::Authentication)?)
                .filter(|s| !s.is_empty())
                .ok_or(AgentError::Authentication)?
        };
        Self::new(model.provider, &key, endpoints, StreamLimits::default())
    }
    pub fn new(
        kind: ProviderKind,
        key: &str,
        endpoints: ProviderEndpoints,
        limits: StreamLimits,
    ) -> Result<Self> {
        if key.is_empty() || limits.frame_bytes == 0 || limits.response_bytes < limits.frame_bytes {
            return Err(AgentError::Configuration);
        }
        let base = match kind {
            ProviderKind::Anthropic => endpoints.anthropic,
            ProviderKind::Openai | ProviderKind::Chatgpt => endpoints.openai,
            ProviderKind::Google => endpoints.google,
            ProviderKind::Ollama => format!(
                "{}/chat/completions",
                endpoints.ollama.trim_end_matches('/')
            ),
        };
        let endpoint = Url::parse(&base).map_err(|_| AgentError::Configuration)?;
        if !matches!(endpoint.scheme(), "https" | "http")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(AgentError::Configuration);
        }
        // HTTP cloud somente em loopback; Ollama aceita base explicitamente configurada.
        if endpoint.scheme() == "http"
            && kind != ProviderKind::Ollama
            && !matches!(
                endpoint.host_str(),
                Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
            )
        {
            return Err(AgentError::Configuration);
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .no_proxy()
            .build()
            .map_err(|_| AgentError::Configuration)?;
        let mut key = HeaderValue::from_str(key).map_err(|_| AgentError::Authentication)?;
        key.set_sensitive(true);
        Ok(Self {
            client,
            kind,
            key,
            endpoint,
            limits,
        })
    }
    async fn run(
        &self,
        request: ProviderRequest,
        cancel: CancellationToken,
        events: mpsc::Sender<ProviderEvent>,
    ) -> Result<()> {
        let body = wire::request(self.kind, &request)?;
        let bytes = serde_json::to_vec(&body).map_err(|_| AgentError::Configuration)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(AgentError::Limit);
        }
        let mut endpoint = self.endpoint.clone();
        if self.kind == ProviderKind::Google {
            endpoint
                .path_segments_mut()
                .map_err(|_| AgentError::Configuration)?
                .pop_if_empty()
                .push(&format!("{}:streamGenerateContent", request.model));
            endpoint.query_pairs_mut().append_pair("alt", "sse");
        }
        let mut http = self
            .client
            .post(endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream");
        http = match self.kind {
            ProviderKind::Anthropic => http
                .header("x-api-key", self.key.clone())
                .header("anthropic-version", "2023-06-01"),
            ProviderKind::Google => http.header("x-goog-api-key", self.key.clone()),
            _ => {
                let raw = self.key.to_str().map_err(|_| AgentError::Authentication)?;
                let mut bearer = HeaderValue::from_str(&format!("Bearer {raw}"))
                    .map_err(|_| AgentError::Authentication)?;
                bearer.set_sensitive(true);
                http.header(reqwest::header::AUTHORIZATION, bearer)
            }
        };
        let mut response = tokio::select! {biased;_=cancel.cancelled()=>return Err(AgentError::Cancelled),r=http.body(bytes).send()=>r.map_err(|_|AgentError::Network)?};
        if self.kind == ProviderKind::Chatgpt && response.status().as_u16() == 403 {
            return Err(AgentError::PlanPermission);
        }
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(AgentError::Authentication);
        }
        if !response.status().is_success() {
            return Err(AgentError::Network);
        }
        if !response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
            })
        {
            return Err(AgentError::Protocol);
        }
        if response
            .content_length()
            .is_some_and(|n| n > self.limits.response_bytes as u64)
        {
            return Err(AgentError::Limit);
        }
        let mut decoder = SseDecoder::new(self.limits.frame_bytes, self.limits.response_bytes);
        let mut parser = WireParser::new(self.kind, request_nonce());
        loop {
            let chunk = tokio::select! {biased;_=cancel.cancelled()=>return Err(AgentError::Cancelled),chunk=response.chunk()=>chunk.map_err(|_|AgentError::Network)?};
            let Some(chunk) = chunk else { break };
            for frame in decoder.push(&chunk)? {
                if self.kind == ProviderKind::Chatgpt {
                    let value: serde_json::Value =
                        serde_json::from_str(&frame.data).unwrap_or_default();
                    let code = value
                        .pointer("/response/error/code")
                        .or_else(|| value.pointer("/error/code"))
                        .and_then(serde_json::Value::as_str);
                    if matches!(
                        code,
                        Some(
                            "subscription_sharing_usage_limit_exceeded"
                                | "subscription_sharing_usage_unavailable"
                        )
                    ) {
                        return Err(AgentError::PlanPermission);
                    }
                }
                for event in parser.feed(&frame.data)? {
                    send(&events, &cancel, event).await?;
                }
                if parser.done {
                    return Ok(());
                }
            }
        }
        decoder.finish()?;
        for event in parser.eof()? {
            send(&events, &cancel, event).await?;
        }
        Ok(())
    }
}
impl Provider for HttpProvider {
    fn stream(
        &self,
        request: ProviderRequest,
        cancel: CancellationToken,
        events: mpsc::Sender<ProviderEvent>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.run(request, cancel, events))
    }
}
async fn send(
    sender: &mpsc::Sender<ProviderEvent>,
    cancel: &CancellationToken,
    event: ProviderEvent,
) -> Result<()> {
    tokio::select! {biased;_=cancel.cancelled()=>Err(AgentError::Cancelled),result=sender.send(event)=>result.map_err(|_|AgentError::Cancelled)}
}

fn request_nonce() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |t| t.as_nanos() as u64);
    now.wrapping_add(u64::from(std::process::id()) << 32)
        .wrapping_add(
            REQUEST_NONCE
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_mul(0x9e3779b97f4a7c15),
        )
}
