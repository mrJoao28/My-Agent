//! Public-client OAuth for OSS applications. No credentials in Debug or events.
use crate::{HttpProvider, Model, Pricing, ProviderEndpoints, ProviderKind, StreamLimits};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use myagent_core::{
    AgentError, BoxFuture, CancellationToken, Provider, ProviderEvent, ProviderRequest, Result,
};
use myagent_store::private::{PrivateGuard, PrivateStore};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
#[derive(Clone)]
pub struct Endpoints {
    pub issuer: String,
    pub api: String,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            issuer: "https://auth.openai.com".into(),
            api: RESOURCE.into(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tokens {
    access: String,
    refresh: String,
    id: String,
    expires_at: u64,
    refresh_expires_at: u64,
    earliest_refresh_at: Option<u64>,
    scopes: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    client_id: String,
    issuer: String,
    subject: String,
    email: String,
    tokens: Option<Tokens>,
    models: Vec<Model>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    host_id: String,
    active: Option<String>,
    pending_client_id: Option<String>,
    accounts: Vec<Account>,
}
#[derive(Clone, Debug, Serialize)]
pub struct AccountStatus {
    pub client_id: String,
    pub label: String,
    pub active: bool,
    pub connected: bool,
    pub plan_allowed: bool,
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    id_token: Option<String>,
    token_type: String,
    expires_in: u64,
    scope: Option<String>,
    earliest_refresh_at: Option<u64>,
}
#[derive(Clone, Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    email: String,
    nonce: Option<String>,
}
#[derive(Deserialize)]
struct Metadata {
    issuer: String,
    jwks_uri: String,
    revocation_endpoint: String,
}

#[derive(Clone)]
pub struct ChatGpt {
    store: PrivateStore,
    client: Client,
    endpoints: Endpoints,
    connections: Arc<Mutex<Vec<(String, CancellationToken)>>>,
}
pub struct Login {
    pub authorization_url: String,
    listener: TcpListener,
    state: String,
    nonce: String,
    verifier: String,
    redirect: String,
    client_id: Option<String>,
    consumed: bool,
    expires_at: std::time::Instant,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
fn random() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| AgentError::Configuration)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn storage<T>(value: std::io::Result<T>) -> Result<T> {
    value.map_err(|e| {
        if e.kind() == std::io::ErrorKind::Interrupted {
            AgentError::Cancelled
        } else {
            AgentError::Persistence
        }
    })
}
fn label(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(160)
        .collect()
}
fn plan(tokens: &Tokens) -> bool {
    ["resource.invoke", "chatgpt.tokens.use.direct"]
        .iter()
        .all(|scope| tokens.scopes.iter().any(|s| s == scope))
}
fn validate_url(url: &str) -> Result<Url> {
    let url = Url::parse(url).map_err(|_| AgentError::Configuration)?;
    if !(url.scheme() == "https" || url.scheme() == "http" && url.host_str() == Some("127.0.0.1"))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AgentError::Configuration);
    }
    Ok(url)
}
impl ChatGpt {
    pub fn new(home: &Path) -> Result<Self> {
        Self::with_endpoints(home, Endpoints::default())
    }
    /// Explicit injection for loopback fixtures; normal CLI always uses official endpoints.
    pub fn with_endpoints(home: &Path, endpoints: Endpoints) -> Result<Self> {
        validate_url(&endpoints.issuer)?;
        validate_url(&endpoints.api)?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(15))
            .no_proxy()
            .build()
            .map_err(|_| AgentError::Configuration)?;
        Ok(Self {
            store: PrivateStore::new(home),
            client,
            endpoints,
            connections: Arc::new(Mutex::new(vec![])),
        })
    }
    async fn locked(&self, cancel: &CancellationToken) -> Result<PrivateGuard> {
        let store = self.store.clone();
        let cancel = cancel.clone();
        myagent_core::blocking(move || storage(store.lock(&cancel))).await
    }
    async fn json(
        &self,
        request: reqwest::RequestBuilder,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        let mut response = tokio::select! { biased; _=cancel.cancelled()=>return Err(AgentError::Cancelled), r=request.send()=>r.map_err(|_| AgentError::Network)? };
        if response.status().as_u16() == 401 {
            return Err(AgentError::Authentication);
        }
        if response.status().as_u16() == 403 {
            return Err(AgentError::PlanPermission);
        }
        if !response.status().is_success() {
            // Read only a bounded error code, never return the endpoint's body.
            let mut bytes = vec![];
            while let Some(chunk) = tokio::select! { _=cancel.cancelled()=>return Err(AgentError::Cancelled), r=response.chunk()=>r.map_err(|_| AgentError::Network)? }
            {
                if bytes.len() + chunk.len() > 16384 {
                    break;
                }
                bytes.extend_from_slice(&chunk);
            }
            if serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_owned))
                .is_some_and(|s| matches!(s.as_str(), "invalid_grant" | "invalid_client"))
            {
                return Err(AgentError::Authentication);
            }
            return Err(AgentError::Network);
        }
        if !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            })
        {
            return Err(AgentError::Protocol);
        }
        let mut bytes = vec![];
        while let Some(chunk) = tokio::select! { biased; _=cancel.cancelled()=>return Err(AgentError::Cancelled), r=response.chunk()=>r.map_err(|_| AgentError::Network)? }
        {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err(AgentError::Limit);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| AgentError::Protocol)
    }
    async fn metadata(&self, cancel: &CancellationToken) -> Result<Metadata> {
        let meta: Metadata = serde_json::from_value(
            self.json(
                self.client.get(format!(
                    "{}/.well-known/openid-configuration",
                    self.endpoints.issuer
                )),
                cancel,
            )
            .await?,
        )
        .map_err(|_| AgentError::Protocol)?;
        let issuer = validate_url(&self.endpoints.issuer)?;
        if meta.issuer != self.endpoints.issuer
            || [meta.jwks_uri.as_str(), meta.revocation_endpoint.as_str()]
                .iter()
                .any(|endpoint| {
                    validate_url(endpoint).map_or(true, |url| url.origin() != issuer.origin())
                })
        {
            return Err(AgentError::Authentication);
        }
        Ok(meta)
    }
    async fn verify(
        &self,
        token: &str,
        client_id: &str,
        nonce: Option<&str>,
        subject: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Claims> {
        if token.len() > 32768 {
            return Err(AgentError::Authentication);
        }
        let header = decode_header(token).map_err(|_| AgentError::Authentication)?;
        if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
            return Err(AgentError::Authentication);
        }
        let meta = self.metadata(cancel).await?;
        let keys: JwkSet =
            serde_json::from_value(self.json(self.client.get(meta.jwks_uri), cancel).await?)
                .map_err(|_| AgentError::Authentication)?;
        let key = keys
            .find(header.kid.as_deref().ok_or(AgentError::Authentication)?)
            .ok_or(AgentError::Authentication)?;
        let key = DecodingKey::from_jwk(key).map_err(|_| AgentError::Authentication)?;
        let mut validation = Validation::new(header.alg);
        validation.leeway = 0;
        validation.validate_nbf = true;
        validation.set_issuer(&[&self.endpoints.issuer]);
        validation.set_audience(&[client_id]);
        validation.set_required_spec_claims(&["sub", "exp", "iss", "aud"]);
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| AgentError::Authentication)?
            .claims;
        if claims.sub.is_empty()
            || claims.sub.len() > 500
            || nonce.is_some_and(|nonce| claims.nonce.as_deref() != Some(nonce))
            || subject.is_some_and(|sub| claims.sub != sub)
        {
            return Err(AgentError::Authentication);
        }
        Ok(claims)
    }
    pub async fn status(&self, cancel: &CancellationToken) -> Result<Vec<AccountStatus>> {
        let guard = self.locked(cancel).await?;
        let state: State = storage(guard.read())?;
        Ok(state
            .accounts
            .iter()
            .map(|a| AccountStatus {
                client_id: a.client_id.clone(),
                label: format!(
                    "{} · {}",
                    label(if a.email.is_empty() {
                        &a.subject
                    } else {
                        &a.email
                    }),
                    a.client_id
                ),
                active: state.active.as_deref() == Some(&a.client_id),
                connected: a.tokens.is_some(),
                plan_allowed: a.tokens.as_ref().is_some_and(plan),
            })
            .collect())
    }
    /// Polls the shared registration while a generation runs, including tool execution.
    pub async fn watch_account(
        &self,
        id: String,
        generation: CancellationToken,
        stop: CancellationToken,
    ) {
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => return,
                _ = generation.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
            let result = async {
                let guard = self.locked(&stop).await?;
                let state: State = storage(guard.read())?;
                Ok::<_, AgentError>(
                    state.active.as_deref() == Some(&id)
                        && state
                            .accounts
                            .iter()
                            .any(|a| a.client_id == id && a.tokens.is_some()),
                )
            }
            .await;
            if !matches!(result, Ok(true)) {
                if !stop.is_cancelled() {
                    generation.cancel();
                }
                return;
            }
        }
    }
    pub async fn start_login(
        &self,
        selected: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Login> {
        let guard = self.locked(cancel).await?;
        let mut state: State = storage(guard.read())?;
        if state.host_id.is_empty() {
            let mut bytes = [0u8; 16];
            getrandom::fill(&mut bytes).map_err(|_| AgentError::Configuration)?;
            bytes[6] = (bytes[6] & 15) | 64;
            bytes[8] = (bytes[8] & 63) | 128;
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            state.host_id = format!(
                "urn:uuid:{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            );
        }
        let account = selected
            .map(|id| {
                state
                    .accounts
                    .iter()
                    .find(|a| a.client_id == id)
                    .ok_or(AgentError::Authentication)
            })
            .transpose()?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| AgentError::Network)?;
        let redirect = format!(
            "http://127.0.0.1:{}/auth/callback",
            listener
                .local_addr()
                .map_err(|_| AgentError::Network)?
                .port()
        );
        let client_id = account
            .map(|a| a.client_id.clone())
            .or(state.pending_client_id.clone());
        let (nonce, verifier, pending_state) = (random()?, random()?, random()?);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = Url::parse(&format!("{}/api/accounts/authorize", self.endpoints.issuer))
            .map_err(|_| AgentError::Configuration)?;
        {
            let mut query = url.query_pairs_mut();
            query.extend_pairs([
                ("client_id", client_id.as_deref().unwrap_or(DYNAMIC_CLIENT)),
                ("ext_agent_host_id", &state.host_id),
                ("response_type", "code"),
                ("redirect_uri", &redirect),
                ("scope", SCOPES),
                ("resource", RESOURCE),
                ("state", &pending_state),
                ("nonce", &nonce),
                ("code_challenge_method", "S256"),
                ("code_challenge", &challenge),
            ]);
            if client_id.is_none() {
                query.append_pair("agent_name_hint", "My-Agent");
            }
            // No id_token_hint in a displayed URL: it would expose a retained secret.
            if let Some(a) = account
                && !a.email.is_empty()
            {
                query.append_pair("login_hint", &a.email);
            }
        }
        storage(guard.write(&state))?;
        Ok(Login {
            authorization_url: url.into(),
            listener,
            state: pending_state,
            nonce,
            verifier,
            redirect,
            client_id,
            consumed: false,
            expires_at: std::time::Instant::now() + Duration::from_secs(300),
        })
    }
    fn token_set(&self, response: TokenResponse, previous: Option<&Tokens>) -> Result<Tokens> {
        if response.access_token.is_empty()
            || previous.is_some() && response.refresh_token.is_empty()
            || response.access_token.len() > 32768
            || response.refresh_token.len() > 32768
            || !response.token_type.eq_ignore_ascii_case("bearer")
            || !(1..=86400).contains(&response.expires_in)
        {
            return Err(AgentError::Protocol);
        }
        let id = response
            .id_token
            .or_else(|| previous.map(|t| t.id.clone()))
            .filter(|s| !s.is_empty() && s.len() <= 32768)
            .ok_or(AgentError::Authentication)?;
        let scopes = response
            .scope
            .map(|s| s.split_whitespace().map(str::to_owned).collect())
            .or_else(|| previous.map(|t| t.scopes.clone()))
            .unwrap_or_default();
        let refresh_expires_at = if response.refresh_token.is_empty() {
            now()
        } else {
            now() + 30 * 86400
        };
        Ok(Tokens {
            access: response.access_token,
            refresh: response.refresh_token,
            id,
            expires_at: now().saturating_add(response.expires_in),
            refresh_expires_at,
            earliest_refresh_at: response.earliest_refresh_at,
            scopes,
        })
    }
    pub async fn complete_login(
        &self,
        login: &mut Login,
        callback: &str,
        cancel: &CancellationToken,
    ) -> Result<AccountStatus> {
        if login.consumed || std::time::Instant::now() >= login.expires_at {
            return Err(AgentError::Authentication);
        }
        let url = Url::parse(callback).map_err(|_| AgentError::Authentication)?;
        let redirect = Url::parse(&login.redirect).map_err(|_| AgentError::Authentication)?;
        if url.origin() != redirect.origin()
            || url.path() != redirect.path()
            || callback.len() > 16384
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AgentError::Authentication);
        }
        let params: Vec<_> = url.query_pairs().collect();
        let get = |name: &str| -> Result<Option<String>> {
            let values: Vec<_> = params.iter().filter(|(k, _)| k == name).collect();
            if values.len() > 1 {
                return Err(AgentError::Authentication);
            }
            Ok(values.first().map(|(_, v)| v.to_string()))
        };
        if get("state")?.as_deref() != Some(&login.state) {
            return Err(AgentError::Authentication);
        }
        login.consumed = true;
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        if get("error")?.is_some() {
            return Err(AgentError::PlanPermission);
        }
        let issued = get("client_id")?
            .or(login.client_id.clone())
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 200
                    && s != DYNAMIC_CLIENT
                    && !s.chars().any(char::is_control)
            })
            .ok_or(AgentError::Authentication)?;
        if login.client_id.as_ref().is_some_and(|id| id != &issued) {
            return Err(AgentError::Authentication);
        }
        let code = get("code")?
            .filter(|s| !s.is_empty() && s.len() <= 8192)
            .ok_or(AgentError::Authentication)?;
        let guard = self.locked(cancel).await?;
        let mut state: State = storage(guard.read())?;
        state.pending_client_id = Some(issued.clone());
        storage(guard.write(&state))?;
        let response = self
            .json(
                self.client
                    .post(format!(
                        "{}/api/accounts/oauth/token",
                        self.endpoints.issuer
                    ))
                    .form(&[
                        ("grant_type", "authorization_code"),
                        ("client_id", &issued),
                        ("code", &code),
                        ("code_verifier", &login.verifier),
                        ("redirect_uri", &login.redirect),
                        ("resource", RESOURCE),
                    ]),
                cancel,
            )
            .await?;
        let tokens = self.token_set(
            serde_json::from_value(response).map_err(|_| AgentError::Protocol)?,
            None,
        )?;
        let existing = state.accounts.iter().find(|a| a.client_id == issued);
        let claims = self
            .verify(
                &tokens.id,
                &issued,
                Some(&login.nonce),
                existing.map(|a| a.subject.as_str()),
                cancel,
            )
            .await?;
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        if state.accounts.len() >= 256 && existing.is_none() {
            return Err(AgentError::Limit);
        }
        let status = AccountStatus {
            client_id: issued.clone(),
            label: format!("{} · {issued}", label(&claims.email)),
            active: true,
            connected: true,
            plan_allowed: plan(&tokens),
        };
        state.accounts.retain(|a| a.client_id != issued);
        state.accounts.push(Account {
            client_id: issued.clone(),
            issuer: self.endpoints.issuer.clone(),
            subject: claims.sub,
            email: label(&claims.email),
            tokens: Some(tokens),
            models: vec![],
        });
        self.interrupt(state.active.as_deref());
        state.active = Some(issued);
        state.pending_client_id = None;
        storage(guard.write(&state))?;
        Ok(status)
    }
    pub async fn wait_login(
        &self,
        mut login: Login,
        cancel: &CancellationToken,
    ) -> Result<AccountStatus> {
        let deadline = tokio::time::sleep(Duration::from_secs(300));
        tokio::pin!(deadline);
        loop {
            let (mut socket, _) = tokio::select! { biased; _=cancel.cancelled()=>return Err(AgentError::Cancelled), _=&mut deadline=>return Err(AgentError::Timeout), r=login.listener.accept()=>r.map_err(|_| AgentError::Network)? };
            let callback = tokio::select! { _=cancel.cancelled()=>return Err(AgentError::Cancelled), _=&mut deadline=>return Err(AgentError::Timeout), r=read_callback(&mut socket, &login.redirect)=>r };
            let result = match callback {
                Ok(url) => self.complete_login(&mut login, &url, cancel).await,
                Err(e) => Err(e),
            };
            let success = result.is_ok();
            let text = if success {
                "ChatGPT conectado. Volte ao My-Agent."
            } else {
                "Callback recusado. Volte ao My-Agent."
            };
            let http = format!(
                "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{text}",
                if success { "200 OK" } else { "400 Bad Request" },
                text.len()
            );
            let _ = tokio::time::timeout(Duration::from_secs(2), socket.write_all(http.as_bytes()))
                .await;
            if login.consumed {
                return result;
            }
        }
    }
    fn interrupt(&self, id: Option<&str>) {
        if let Ok(mut connections) = self.connections.lock() {
            connections.retain(|(account, token)| {
                if id == Some(account.as_str()) {
                    token.cancel();
                    false
                } else {
                    !token.is_cancelled()
                }
            });
        }
    }
    pub async fn activate(&self, id: &str, cancel: &CancellationToken) -> Result<()> {
        let guard = self.locked(cancel).await?;
        let mut state: State = storage(guard.read())?;
        if !state
            .accounts
            .iter()
            .any(|a| a.client_id == id && a.tokens.is_some())
        {
            return Err(AgentError::Authentication);
        }
        self.interrupt(state.active.as_deref());
        state.active = Some(id.into());
        storage(guard.write(&state))
    }
    pub async fn logout(&self, id: &str, cancel: &CancellationToken) -> Result<bool> {
        self.interrupt(Some(id));
        let guard = self.locked(cancel).await?;
        let mut state: State = storage(guard.read())?;
        let account = state
            .accounts
            .iter_mut()
            .find(|a| a.client_id == id)
            .ok_or(AgentError::Authentication)?;
        let mut confirmed = account.tokens.is_none();
        if let Some(tokens) = &account.tokens
            && !tokens.refresh.is_empty()
            && let Ok(meta) = self.metadata(cancel).await
        {
            for delay in [0, 100, 300] {
                if cancel.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let request = self.client.post(&meta.revocation_endpoint).form(&[
                    ("token", tokens.refresh.as_str()),
                    ("token_type_hint", "refresh_token"),
                    ("client_id", id),
                ]);
                let response = tokio::select! { _=cancel.cancelled()=>break, r=request.send()=>r };
                if response.as_ref().is_ok_and(|r| r.status().as_u16() == 200) {
                    confirmed = true;
                    break;
                }
                if response
                    .as_ref()
                    .is_ok_and(|r| r.status().is_client_error())
                {
                    break;
                }
            }
        }
        account.tokens = None;
        account.models.clear();
        if state.active.as_deref() == Some(id) {
            state.active = None;
        }
        storage(guard.write(&state))?;
        Ok(confirmed)
    }
    async fn access(
        &self,
        expected: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<(String, String)> {
        let guard = self.locked(cancel).await?;
        let mut state: State = storage(guard.read())?;
        let active = state.active.clone().ok_or(AgentError::Authentication)?;
        if expected.is_some_and(|id| id != active) {
            return Err(AgentError::Cancelled);
        }
        let account = state
            .accounts
            .iter_mut()
            .find(|a| a.client_id == active && a.issuer == self.endpoints.issuer)
            .ok_or(AgentError::Authentication)?;
        let tokens = account.tokens.as_ref().ok_or(AgentError::Authentication)?;
        if !plan(tokens) {
            return Err(AgentError::PlanPermission);
        }
        if tokens.refresh_expires_at <= now() && tokens.expires_at <= now() {
            account.tokens = None;
            account.models.clear();
            storage(guard.write(&state))?;
            return Err(AgentError::Authentication);
        }
        if tokens.expires_at <= now() + 60
            && tokens.refresh_expires_at > now()
            && tokens.earliest_refresh_at.is_none_or(|t| t <= now())
        {
            let response = self
                .json(
                    self.client
                        .post(format!(
                            "{}/api/accounts/oauth/token",
                            self.endpoints.issuer
                        ))
                        .form(&[
                            ("grant_type", "refresh_token"),
                            ("client_id", &active),
                            ("refresh_token", &tokens.refresh),
                            ("resource", RESOURCE),
                        ]),
                    cancel,
                )
                .await;
            let response = match response {
                Err(AgentError::Authentication) => {
                    account.tokens = None;
                    account.models.clear();
                    storage(guard.write(&state))?;
                    return Err(AgentError::Authentication);
                }
                other => other?,
            };
            let returned: TokenResponse =
                serde_json::from_value(response).map_err(|_| AgentError::Protocol)?;
            if let Some(id) = &returned.id_token
                && let Err(error) = self
                    .verify(id, &active, None, Some(&account.subject), cancel)
                    .await
            {
                if matches!(error, AgentError::Authentication) {
                    account.tokens = None;
                    account.models.clear();
                    storage(guard.write(&state))?;
                }
                return Err(error);
            }
            let updated = self.token_set(returned, Some(tokens))?;
            account.tokens = Some(updated);
            storage(guard.write(&state))?;
        }
        let tokens = state
            .accounts
            .iter()
            .find(|a| a.client_id == active)
            .and_then(|a| a.tokens.as_ref())
            .ok_or(AgentError::Authentication)?;
        if tokens.expires_at <= now() {
            return Err(AgentError::Authentication);
        }
        if !plan(tokens) {
            return Err(AgentError::PlanPermission);
        }
        Ok((active, tokens.access.clone()))
    }
    async fn invalidate(&self, id: &str, rejected: &str, cancel: &CancellationToken) -> Result<()> {
        let guard = self.locked(cancel).await?;
        let mut state: State = storage(guard.read())?;
        if let Some(a) = state.accounts.iter_mut().find(|a| a.client_id == id)
            && a.tokens.as_ref().is_some_and(|t| t.access == rejected)
        {
            a.tokens = None;
            a.models.clear();
            storage(guard.write(&state))?;
        }
        self.interrupt(Some(id));
        Ok(())
    }
    pub async fn models(&self, cancel: &CancellationToken) -> Result<Vec<Model>> {
        let (id, access) = self.access(None, cancel).await?;
        let result = self
            .json(
                self.client
                    .get(format!("{}/models", self.endpoints.api))
                    .bearer_auth(&access),
                cancel,
            )
            .await;
        if matches!(result, Err(AgentError::Authentication)) {
            self.invalidate(&id, &access, cancel).await?;
        }
        let value = result?;
        let entries = value["models"]
            .as_array()
            .filter(|a| a.len() <= 1000)
            .ok_or(AgentError::Protocol)?;
        let mut models = vec![];
        let mut seen = std::collections::HashSet::new();
        for entry in entries.iter().filter(|v| v["visibility"] == "list") {
            let slug = entry["slug"]
                .as_str()
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 180
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                })
                .ok_or(AgentError::Protocol)?;
            if !seen.insert(slug) {
                return Err(AgentError::Protocol);
            }
            let name = entry["display_name"].as_str().unwrap_or(slug);
            models.push(Model {
                id: format!("chatgpt:{slug}"),
                provider: ProviderKind::Chatgpt,
                display_name: Some(label(name)),
                api_key_env: None,
                context_window: entry["context_window"]
                    .as_u64()
                    .filter(|n| (1024..=i32::MAX as u64).contains(n))
                    .map(|n| n as u32),
                pricing: Some(Pricing::default()),
            });
        }
        let guard = self.locked(cancel).await?;
        let mut state: State = storage(guard.read())?;
        if state.active.as_deref() != Some(&id) {
            return Err(AgentError::Cancelled);
        }
        let account = state
            .accounts
            .iter_mut()
            .find(|a| a.client_id == id)
            .ok_or(AgentError::Authentication)?;
        if account.tokens.as_ref().is_none_or(|t| t.access != access) {
            return Err(AgentError::Cancelled);
        }
        account.models = models.clone();
        storage(guard.write(&state))?;
        Ok(models)
    }
    pub async fn provider(
        &self,
        model: &str,
        cancel: &CancellationToken,
    ) -> Result<ChatGptProvider> {
        let guard = self.locked(cancel).await?;
        let state: State = storage(guard.read())?;
        let id = state.active.ok_or(AgentError::Authentication)?;
        if !state
            .accounts
            .iter()
            .any(|a| a.client_id == id && a.models.iter().any(|m| m.id == model))
        {
            return Err(AgentError::Configuration);
        }
        let token = CancellationToken::new();
        self.connections
            .lock()
            .map_err(|_| AgentError::Cleanup)?
            .push((id.clone(), token.clone()));
        Ok(ChatGptProvider {
            auth: self.clone(),
            id,
            changed: token,
        })
    }
}
pub struct ChatGptProvider {
    auth: ChatGpt,
    id: String,
    changed: CancellationToken,
}
impl ChatGptProvider {
    pub fn account_id(&self) -> &str {
        &self.id
    }
}
impl Provider for ChatGptProvider {
    fn stream(
        &self,
        mut request: ProviderRequest,
        cancel: CancellationToken,
        events: mpsc::Sender<ProviderEvent>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let (id, access) = self.auth.access(Some(&self.id), &cancel).await?;
            request.model = request
                .model
                .strip_prefix("chatgpt:")
                .ok_or(AgentError::Configuration)?
                .to_owned();
            let provider = HttpProvider::new(
                ProviderKind::Chatgpt,
                &access,
                ProviderEndpoints {
                    openai: format!("{}/responses", self.auth.endpoints.api),
                    ..ProviderEndpoints::default()
                },
                StreamLimits::default(),
            )?;
            let running = cancel.child_token();
            let result = tokio::select! { biased; _=self.changed.cancelled()=>{ running.cancel(); Err(AgentError::Cancelled) }, result=provider.stream(request, running.clone(), events)=>result };
            if matches!(result, Err(AgentError::Authentication)) {
                self.auth.invalidate(&id, &access, &cancel).await?;
            }
            result
        })
    }
}
async fn read_callback(socket: &mut TcpStream, redirect: &str) -> Result<String> {
    let mut bytes = vec![];
    tokio::time::timeout(Duration::from_secs(3), async {
        while !bytes.ends_with(b"\r\n\r\n") {
            if bytes.len() >= 16384 {
                return Err(AgentError::Limit);
            }
            let mut byte = [0];
            if socket
                .read(&mut byte)
                .await
                .map_err(|_| AgentError::Network)?
                == 0
            {
                return Err(AgentError::Protocol);
            }
            bytes.push(byte[0]);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| AgentError::Protocol)?;
        let mut lines = text.split("\r\n");
        let first: Vec<_> = lines.next().unwrap_or("").split(' ').collect();
        if first.len() != 3
            || first[0] != "GET"
            || first[2] != "HTTP/1.1"
            || !first[1].starts_with("/auth/callback?")
        {
            return Err(AgentError::Authentication);
        }
        let base = Url::parse(redirect).map_err(|_| AgentError::Authentication)?;
        let host = format!(
            "127.0.0.1:{}",
            base.port().ok_or(AgentError::Authentication)?
        );
        let hosts: Vec<_> = lines
            .filter_map(|l| l.split_once(':'))
            .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
            .collect();
        if hosts.len() != 1 || hosts[0].1.trim() != host {
            return Err(AgentError::Authentication);
        }
        Ok(format!(
            "{}{}",
            base.origin().ascii_serialization(),
            first[1]
        ))
    })
    .await
    .map_err(|_| AgentError::Timeout)?
}

#[cfg(test)]
mod tests;
