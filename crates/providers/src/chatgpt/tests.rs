use super::*;
use jsonwebtoken::{EncodingKey, Header, encode};
use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
use serde_json::json;
use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

fn signing() -> &'static (EncodingKey, Value) {
    static KEY: OnceLock<(EncodingKey, Value)> = OnceLock::new();
    KEY.get_or_init(|| {
        let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let jwk = json!({"keys":[{"kty":"RSA","kid":"fixture","alg":"RS256","use":"sig","n":URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(key.e().to_bytes_be())}]});
        (EncodingKey::from_rsa_der(key.to_pkcs1_der().unwrap().as_bytes()), jwk)
    })
}
struct Mock {
    claims: Mutex<Value>,
    scopes: Mutex<String>,
    stream: Mutex<String>,
    refresh_error: Mutex<bool>,
    model_error: Mutex<bool>,
    slow: Mutex<bool>,
    tokens: AtomicUsize,
    refreshes: AtomicUsize,
    inferences: AtomicUsize,
    catalogs: AtomicUsize,
    revokes: AtomicUsize,
}
struct Fixture {
    auth: ChatGpt,
    mock: Arc<Mock>,
    dir: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let _ = signing();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let mock = Arc::new(Mock {
            claims: Mutex::new(json!({})),
            scopes: Mutex::new(SCOPES.into()),
            stream: Mutex::new("complete".into()),
            refresh_error: Mutex::new(false),
            model_error: Mutex::new(false),
            slow: Mutex::new(false),
            tokens: AtomicUsize::new(0),
            refreshes: AtomicUsize::new(0),
            inferences: AtomicUsize::new(0),
            catalogs: AtomicUsize::new(0),
            revokes: AtomicUsize::new(0),
        });
        let worker = mock.clone();
        let base = issuer.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let mock = worker.clone();
                let base = base.clone();
                tokio::spawn(async move {
                    serve(socket, mock, base).await;
                });
            }
        });
        let dir = tempfile::tempdir().unwrap();
        drop(
            PrivateStore::new(dir.path())
                .lock(&CancellationToken::new())
                .expect("private OAuth fixture initialization"),
        );
        let auth = ChatGpt::with_endpoints(
            dir.path(),
            Endpoints {
                api: format!("{issuer}/v1"),
                issuer,
            },
        )
        .unwrap();
        Self {
            auth,
            mock,
            dir,
            server,
        }
    }
    async fn pending(&self, client: &str, selected: Option<&str>) -> (Login, String) {
        let login = self
            .auth
            .start_login(selected, &CancellationToken::new())
            .await
            .unwrap();
        let query: std::collections::HashMap<_, _> = Url::parse(&login.authorization_url)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(query["resource"], RESOURCE);
        assert_eq!(query["code_challenge_method"], "S256");
        *self.mock.claims.lock().unwrap() = json!({"iss":self.auth.endpoints.issuer,"aud":client,"sub":"subject-fixture","email":"same@example.test","nonce":query["nonce"],"exp":now()+3600});
        let mut callback = Url::parse(&query["redirect_uri"]).unwrap();
        callback.query_pairs_mut().extend_pairs([
            ("state", query["state"].as_str()),
            ("code", "fake-code"),
            ("client_id", client),
        ]);
        (login, callback.into())
    }
    async fn connected(&self, client: &str) {
        let (mut login, url) = self.pending(client, None).await;
        assert!(
            self.auth
                .complete_login(&mut login, &url, &CancellationToken::new())
                .await
                .unwrap()
                .plan_allowed
        );
    }
    async fn expire(&self) {
        let guard = self.auth.locked(&CancellationToken::new()).await.unwrap();
        let mut state: State = guard.read().unwrap();
        let active = state.active.clone().unwrap();
        state
            .accounts
            .iter_mut()
            .find(|a| a.client_id == active)
            .unwrap()
            .tokens
            .as_mut()
            .unwrap()
            .expires_at = now() - 1;
        guard.write(&state).unwrap();
    }
}
async fn serve(mut socket: TcpStream, mock: Arc<Mock>, issuer: String) {
    let mut bytes = vec![];
    let end;
    loop {
        let mut buf = [0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap_or(0);
        if n == 0 || bytes.len() > 1024 * 1024 {
            return;
        }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(i) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
            end = i + 4;
            break;
        }
    }
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let length = headers
        .lines()
        .filter_map(|s| s.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, v)| v.trim().parse::<usize>().unwrap());
    while bytes.len() < end + length {
        let mut buf = [0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let target = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let body = String::from_utf8(bytes[end..].to_vec()).unwrap();
    let mut status = 200;
    let mut mime = "application/json";
    let response=match target {
        "/.well-known/openid-configuration"=>json!({"issuer":issuer,"jwks_uri":format!("{issuer}/jwks"),"revocation_endpoint":format!("{issuer}/revoke")}).to_string(),
        "/jwks"=>signing().1.to_string(),
        "/api/accounts/oauth/token"=>{
            mock.tokens.fetch_add(1,Ordering::SeqCst);
            let slow=*mock.slow.lock().unwrap();
            if slow { tokio::time::sleep(Duration::from_millis(300)).await; }
            let params:std::collections::HashMap<_,_>=Url::parse(&format!("http://127.0.0.1/?{body}")).unwrap().query_pairs().map(|(k,v)|(k.to_string(),v.to_string())).collect();
            assert_ne!(params["client_id"],DYNAMIC_CLIENT); assert_eq!(params["resource"],RESOURCE);
            if params["grant_type"]=="refresh_token" {
                let previous=mock.refreshes.fetch_add(1,Ordering::SeqCst);
                if *mock.refresh_error.lock().unwrap() || previous>0 || params["refresh_token"]!="refresh-0" {
                    status=400; json!({"error":"invalid_grant","description":"must never escape"}).to_string()
                } else { json!({"access_token":"access-rotated","refresh_token":"refresh-rotated","token_type":"Bearer","expires_in":3600,"earliest_refresh_at":now()+100}).to_string() }
            } else {
                let mut header=Header::new(Algorithm::RS256); header.kid=Some("fixture".into());
                let id=encode(&header,&*mock.claims.lock().unwrap(),&signing().0).unwrap();
                json!({"access_token":"access-login","refresh_token":"refresh-0","id_token":id,"token_type":"Bearer","expires_in":3600,"scope":*mock.scopes.lock().unwrap()}).to_string()
            }
        }
        "/revoke"=>{ mock.revokes.fetch_add(1,Ordering::SeqCst); String::new() }
        "/v1/models"=>{
            mock.catalogs.fetch_add(1,Ordering::SeqCst);
            assert!(headers.to_ascii_lowercase().contains("authorization: bearer access-"));
            if *mock.model_error.lock().unwrap() { status=401; json!({"error":"revoked"}).to_string() }
            else { json!({"models":[{"slug":"fixture","display_name":"Fixture","visibility":"list","context_window":32768},{"slug":"hidden","visibility":"hidden"}]}).to_string() }
        }
        "/v1/responses"=>{
            mock.inferences.fetch_add(1,Ordering::SeqCst);
            let value:Value=serde_json::from_str(&body).unwrap();
            assert_eq!(value["model"],"fixture"); assert_eq!(value["stream"],true); assert_eq!(value["store"],false);
            mime="text/event-stream";
            let delta="data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n";
            match mock.stream.lock().unwrap().as_str() {
                "eof"=>delta.into(),
                "failed"=>format!("{delta}data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"code\":\"fixture_error\"}}}}}}\n\n"),
                "plan-failed"=>format!("{delta}data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"code\":\"subscription_sharing_usage_unavailable\"}}}}}}\n\n"),
                "html"=>{ mime="text/html"; "<html>not a stream</html>".into() }
                _=>format!("{delta}data: {{\"type\":\"response.completed\",\"response\":{{\"status\":\"completed\",\"output\":[],\"usage\":{{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}}}\n\n"),
            }
        }
        _=>{ status=404; "{}".into() }
    };
    let headers = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    );
    let _ = socket.write_all(headers.as_bytes()).await;
    let _ = socket.write_all(response.as_bytes()).await;
}
#[tokio::test]
async fn rejects_state_nonce_audience_expiry_subject_and_callback_replay() {
    let f = Fixture::new().await;
    let cancel = CancellationToken::new();
    let (mut login, url) = f.pending("client-a", None).await;
    let mut bad = Url::parse(&url).unwrap();
    bad.set_query(Some("state=wrong&code=fake&client_id=client-a"));
    assert!(
        f.auth
            .complete_login(&mut login, bad.as_str(), &cancel)
            .await
            .is_err()
    );
    assert_eq!(f.mock.tokens.load(Ordering::SeqCst), 0);
    assert!(
        f.auth
            .complete_login(&mut login, &url, &cancel)
            .await
            .is_ok()
    );
    assert!(
        f.auth
            .complete_login(&mut login, &url, &cancel)
            .await
            .is_err()
    );
    assert_eq!(f.mock.tokens.load(Ordering::SeqCst), 1);
    for (claim, value) in [
        ("nonce", json!("wrong")),
        ("aud", json!("other-client")),
        ("iss", json!("https://invalid.example")),
        ("exp", json!(now() - 1)),
        ("sub", json!("other-subject")),
    ] {
        let (mut login, url) = f.pending("client-a", Some("client-a")).await;
        f.mock.claims.lock().unwrap()[claim] = value;
        assert!(
            f.auth
                .complete_login(&mut login, &url, &cancel)
                .await
                .is_err(),
            "accepted {claim}"
        );
    }
    assert_eq!(f.auth.status(&cancel).await.unwrap().len(), 1);
}
#[tokio::test]
async fn same_email_registrations_logout_and_missing_plan_are_explicit() {
    let f = Fixture::new().await;
    let cancel = CancellationToken::new();
    f.connected("client-a").await;
    f.connected("client-b").await;
    let accounts = f.auth.status(&cancel).await.unwrap();
    assert_eq!(accounts.len(), 2);
    assert_ne!(accounts[0].label, accounts[1].label);
    assert!(accounts[1].active);
    assert_eq!(f.auth.models(&cancel).await.unwrap().len(), 1);
    assert!(f.auth.logout("client-b", &cancel).await.unwrap());
    assert_eq!(f.mock.revokes.load(Ordering::SeqCst), 1);
    let login = f.auth.start_login(Some("client-b"), &cancel).await.unwrap();
    assert!(login.authorization_url.contains("client_id=client-b"));
    assert!(!login.authorization_url.contains("agent_name_hint"));
    *f.mock.scopes.lock().unwrap() = "openid profile email".into();
    let (mut login, url) = f.pending("client-c", None).await;
    assert!(
        !f.auth
            .complete_login(&mut login, &url, &cancel)
            .await
            .unwrap()
            .plan_allowed
    );
    let prior = f.mock.catalogs.load(Ordering::SeqCst);
    assert!(matches!(
        f.auth.models(&cancel).await,
        Err(AgentError::PlanPermission)
    ));
    assert_eq!(f.mock.catalogs.load(Ordering::SeqCst), prior);
    assert_eq!(f.mock.inferences.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn cancellation_waiting_and_exchanging_preserves_active_identity() {
    let f = Fixture::new().await;
    f.connected("client-a").await;
    let cancel = CancellationToken::new();
    let login = f.auth.start_login(None, &cancel).await.unwrap();
    cancel.cancel();
    assert!(matches!(
        f.auth.wait_login(login, &cancel).await,
        Err(AgentError::Cancelled)
    ));
    let cancel = CancellationToken::new();
    let (mut login, url) = f.pending("client-b", None).await;
    *f.mock.slow.lock().unwrap() = true;
    let before = f.mock.tokens.load(Ordering::SeqCst);
    let auth = f.auth.clone();
    let token = cancel.clone();
    let task = tokio::spawn(async move { auth.complete_login(&mut login, &url, &token).await });
    while f.mock.tokens.load(Ordering::SeqCst) == before {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    assert!(matches!(task.await.unwrap(), Err(AgentError::Cancelled)));
    let accounts = f.auth.status(&CancellationToken::new()).await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert!(accounts[0].active);
}
#[tokio::test]
async fn loopback_callback_checks_host_and_consumes_once() {
    let f = Fixture::new().await;
    let (login, url) = f.pending("client-a", None).await;
    let auth = f.auth.clone();
    let task = tokio::spawn(async move { auth.wait_login(login, &CancellationToken::new()).await });
    let client = Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        client
            .get(&url)
            .header("Host", "invalid.example")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(task.await.unwrap().is_ok());
}
#[tokio::test]
async fn rotation_saves_tokens_expiry_scopes_and_revokes_only_the_affected_account() {
    let f = Fixture::new().await;
    let cancel = CancellationToken::new();
    f.connected("client-a").await;
    f.expire().await;
    f.auth.models(&cancel).await.unwrap();
    let guard = f.auth.locked(&cancel).await.unwrap();
    let state: State = guard.read().unwrap();
    let tokens = state.accounts[0].tokens.as_ref().unwrap();
    assert_eq!(tokens.access, "access-rotated");
    assert_eq!(tokens.refresh, "refresh-rotated");
    assert!(tokens.expires_at > now() + 3000);
    assert!(tokens.earliest_refresh_at.unwrap() > now());
    assert!(plan(tokens));
    drop(guard);
    f.connected("client-b").await;
    f.expire().await;
    *f.mock.refresh_error.lock().unwrap() = true;
    assert!(matches!(
        f.auth.models(&cancel).await,
        Err(AgentError::Authentication)
    ));
    let accounts = f.auth.status(&cancel).await.unwrap();
    assert!(accounts[0].connected);
    assert!(!accounts[1].connected);
    f.auth.activate("client-a", &cancel).await.unwrap();
    *f.mock.model_error.lock().unwrap() = true;
    assert!(matches!(
        f.auth.models(&cancel).await,
        Err(AgentError::Authentication)
    ));
    assert!(!f.auth.status(&cancel).await.unwrap()[0].connected);
}
#[tokio::test]
async fn streaming_errors_keep_partials_and_never_retry_or_fallback() {
    use myagent_core::{ModelMessage, ModelRole};
    let f = Fixture::new().await;
    let cancel = CancellationToken::new();
    f.connected("client-a").await;
    f.auth.models(&cancel).await.unwrap();
    for outcome in ["eof", "failed", "plan-failed", "html", "complete"] {
        *f.mock.stream.lock().unwrap() = outcome.into();
        let provider = f.auth.provider("chatgpt:fixture", &cancel).await.unwrap();
        let request = ProviderRequest {
            model: "chatgpt:fixture".into(),
            system: "fixture".into(),
            messages: vec![ModelMessage::text(ModelRole::User, "hello")],
            tools: vec![],
            max_output_tokens: 128,
        };
        let (tx, mut rx) = mpsc::channel(64);
        let result = provider.stream(request, cancel.clone(), tx).await;
        assert_eq!(result.is_ok(), outcome == "complete");
        if outcome == "plan-failed" {
            assert!(matches!(result, Err(AgentError::PlanPermission)));
        }
        let mut partial = false;
        let mut done = false;
        while let Some(event) = rx.recv().await {
            match event {
                ProviderEvent::Text(t) => partial |= t == "partial",
                ProviderEvent::Done => done = true,
                _ => {}
            }
        }
        assert_eq!(partial, outcome != "html");
        assert_eq!(done, outcome == "complete");
    }
    assert_eq!(f.mock.inferences.load(Ordering::SeqCst), 5);
    assert_eq!(f.mock.refreshes.load(Ordering::SeqCst), 0);
}
#[test]
fn refresh_child() {
    let Some(home) = std::env::var_os("MYAGENT_TEST_OAUTH_HOME") else {
        return;
    };
    let issuer = std::env::var("MYAGENT_TEST_OAUTH_ISSUER").unwrap();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let auth = ChatGpt::with_endpoints(
                Path::new(&home),
                Endpoints {
                    api: format!("{issuer}/v1"),
                    issuer,
                },
            )
            .unwrap();
            assert!(auth.models(&CancellationToken::new()).await.is_ok());
        });
}

#[tokio::test]
async fn another_client_instance_switching_accounts_cancels_the_generation() {
    let f = Fixture::new().await;
    f.connected("client-a").await;
    f.connected("client-b").await;
    let other = ChatGpt::with_endpoints(f.dir.path(), f.auth.endpoints.clone()).unwrap();
    let generation = CancellationToken::new();
    let stop = CancellationToken::new();
    let auth = f.auth.clone();
    let token = generation.clone();
    let halt = stop.clone();
    let task =
        tokio::spawn(async move { auth.watch_account("client-b".into(), token, halt).await });
    other
        .activate("client-a", &CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), generation.cancelled())
        .await
        .unwrap();
    stop.cancel();
    task.await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_processes_refresh_the_same_rotating_token_only_once() {
    let f = Fixture::new().await;
    f.connected("client-a").await;
    f.expire().await;
    let spawn = || {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["chatgpt::tests::refresh_child", "--exact"])
            .env("MYAGENT_TEST_OAUTH_HOME", f.dir.path())
            .env("MYAGENT_TEST_OAUTH_ISSUER", &f.auth.endpoints.issuer)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.spawn().unwrap()
    };
    let mut one = spawn();
    let mut two = spawn();
    let results = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(one.wait(), two.wait())
    })
    .await
    .unwrap();
    assert!(results.0.unwrap().success());
    assert!(results.1.unwrap().success());
    assert_eq!(f.mock.refreshes.load(Ordering::SeqCst), 1);
}
