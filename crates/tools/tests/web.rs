#![forbid(unsafe_code)]
use myagent_core::{BoxFuture, CancellationToken, Mode, ToolContext};
use myagent_protocol::StreamEvent;
use myagent_tools::{
    ApprovalDecision, ApprovalManager, Error, Result, WebClient, WebRegistry, WebRequest,
    WebResolver, WebResponse, WebTransport,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::mpsc;

struct Resolver {
    calls: AtomicUsize,
    addresses: Vec<IpAddr>,
    hang: bool,
}
impl WebResolver for Resolver {
    fn resolve(
        &self,
        _host: String,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<Vec<IpAddr>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.hang {
                cancel.cancelled().await;
                return Err(Error::Cancelled);
            }
            Ok(self.addresses.clone())
        })
    }
}
#[derive(Default)]
struct Transport {
    responses: Mutex<VecDeque<WebResponse>>,
    requests: Mutex<Vec<WebRequest>>,
}
impl WebTransport for Transport {
    fn request(
        &self,
        request: WebRequest,
        _cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<WebResponse>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async move { self.responses.lock().unwrap().pop_front().ok_or(Error::Web) })
    }
}
fn response(status: u16, location: Option<&str>, body: &str) -> WebResponse {
    WebResponse {
        status,
        location: location.map(str::to_owned),
        content_type: "text/html".into(),
        body: body.as_bytes().to_vec(),
    }
}
struct Fixture {
    registry: WebRegistry,
    manager: ApprovalManager,
    resolver: Arc<Resolver>,
    transport: Arc<Transport>,
}
impl Fixture {
    fn new(addresses: &[&str], responses: Vec<WebResponse>) -> Self {
        let resolver = Arc::new(Resolver {
            calls: AtomicUsize::new(0),
            addresses: addresses.iter().map(|s| s.parse().unwrap()).collect(),
            hang: false,
        });
        let transport = Arc::new(Transport {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(vec![]),
        });
        let manager = ApprovalManager::default();
        let registry = WebRegistry::new(
            WebClient::new(
                resolver.clone(),
                transport.clone(),
                Some("fixture-tavily-key".into()),
            ),
            manager.clone(),
        );
        Self {
            registry,
            manager,
            resolver,
            transport,
        }
    }
    async fn call(
        &self,
        name: &str,
        args: Value,
        session: &str,
        choice: ApprovalDecision,
    ) -> Result<Value> {
        let (tx, mut rx) = mpsc::channel(4);
        let manager = self.manager.clone();
        let session = session.to_owned();
        let other = session.clone();
        let responder = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let StreamEvent::ApprovalRequired { approval_id, .. } = event {
                    assert!(manager.decide(&other, &approval_id, choice));
                }
            }
        });
        let result = self
            .registry
            .call(
                name,
                args,
                context(&session, Some(tx), CancellationToken::new()),
            )
            .await;
        responder.await.unwrap();
        result
    }
}
fn context(
    session: &str,
    events: Option<mpsc::Sender<StreamEvent>>,
    cancellation: CancellationToken,
) -> ToolContext {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    ToolContext {
        session_id: session.into(),
        tool_call_id: format!("call{}", NEXT.fetch_add(1, Ordering::SeqCst)),
        events,
        cancellation,
    }
}
#[tokio::test]
async fn web_requires_consent_before_dns_and_execute_cannot_bypass_it() {
    let f = Fixture::new(&["8.8.8.8"], vec![]);
    assert!(matches!(
        f.call(
            "web_fetch",
            json!({"url":"https://docs.example/a"}),
            "session",
            ApprovalDecision::Deny
        )
        .await,
        Err(Error::Denied)
    ));
    let tool = f
        .registry
        .tools(Mode::Plan)
        .into_iter()
        .find(|t| t.spec().name == "web_fetch")
        .unwrap();
    assert!(
        tool.execute(
            json!({"url":"https://docs.example/a"})
                .as_object()
                .unwrap()
                .clone(),
            context("session", None, CancellationToken::new())
        )
        .await
        .is_err()
    );
    assert_eq!(f.resolver.calls.load(Ordering::SeqCst), 0);
    assert!(f.transport.requests.lock().unwrap().is_empty());
}
#[tokio::test]
async fn grants_are_exact_origins_and_session_tool_port_scheme_are_isolated() {
    let f = Fixture::new(
        &["8.8.8.8"],
        vec![response(200, None, "ok"), response(200, None, "again")],
    );
    f.call(
        "web_fetch",
        json!({"url":"https://docs.example/a"}),
        "session",
        ApprovalDecision::Session,
    )
    .await
    .unwrap();
    f.registry
        .call(
            "web_fetch",
            json!({"url":"https://docs.example/b"}),
            context("session", None, CancellationToken::new()),
        )
        .await
        .unwrap();
    for (session, url) in [
        ("session", "http://docs.example/a"),
        ("session", "https://docs.example:444/a"),
        ("session", "https://other.example/a"),
        ("different", "https://docs.example/a"),
    ] {
        assert!(
            f.registry
                .call(
                    "web_fetch",
                    json!({"url":url}),
                    context(session, None, CancellationToken::new())
                )
                .await
                .is_err()
        );
    }
    assert!(
        f.registry
            .call(
                "web_search",
                json!({"query":"fixture"}),
                context("session", None, CancellationToken::new())
            )
            .await
            .is_err()
    );
    assert_eq!(f.transport.requests.lock().unwrap().len(), 2);
    f.registry.forget_session("session");
    assert!(
        f.registry
            .call(
                "web_fetch",
                json!({"url":"https://docs.example/c"}),
                context("session", None, CancellationToken::new())
            )
            .await
            .is_err()
    );
}
#[tokio::test]
async fn pinned_addresses_cannot_rebind_and_mixed_answers_fail_closed() {
    let f = Fixture::new(
        &["93.184.216.34"],
        vec![response(200, None, "<script>bad</script><p>public</p>")],
    );
    let result = f
        .call(
            "web_fetch",
            json!({"url":"https://rebind.example"}),
            "session",
            ApprovalDecision::Once,
        )
        .await
        .unwrap();
    assert_eq!(result["content"], "public");
    assert_eq!(f.resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.transport.requests.lock().unwrap()[0].addresses[0].ip(),
        "93.184.216.34".parse::<IpAddr>().unwrap()
    );
    for addresses in [
        vec!["8.8.8.8", "127.0.0.1"],
        vec!["169.254.169.254"],
        vec!["::ffff:8.8.8.8"],
        vec![],
    ] {
        let f = Fixture::new(&addresses, vec![]);
        assert!(matches!(
            f.call(
                "web_fetch",
                json!({"url":"https://mixed.example"}),
                "session",
                ApprovalDecision::Once
            )
            .await,
            Err(Error::Ssrf)
        ));
        assert!(f.transport.requests.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn every_redirect_revalidates_dns_and_other_origins_never_receive_requests() {
    for next in [
        "https://other.example/a",
        "http://docs.example/a",
        "https://docs.example:444/a",
    ] {
        let f = Fixture::new(&["8.8.8.8"], vec![response(302, Some(next), "")]);
        assert!(matches!(
            f.call(
                "web_fetch",
                json!({"url":"https://docs.example/a"}),
                "session",
                ApprovalDecision::Once
            )
            .await,
            Err(Error::CrossOrigin)
        ));
        assert_eq!(f.transport.requests.lock().unwrap().len(), 1);
    }
    let f = Fixture::new(
        &["8.8.8.8"],
        vec![
            response(302, Some("/next"), ""),
            response(200, None, "next"),
        ],
    );
    assert_eq!(
        f.call(
            "web_fetch",
            json!({"url":"https://docs.example/a"}),
            "session",
            ApprovalDecision::Once
        )
        .await
        .unwrap()["content"],
        "next"
    );
    assert_eq!(f.resolver.calls.load(Ordering::SeqCst), 2);
    let f = Fixture::new(
        &["8.8.8.8"],
        (0..6).map(|_| response(302, Some("/again"), "")).collect(),
    );
    assert!(
        f.call(
            "web_fetch",
            json!({"url":"https://docs.example/a"}),
            "session",
            ApprovalDecision::Once
        )
        .await
        .is_err()
    );
    assert_eq!(f.transport.requests.lock().unwrap().len(), 6);
}
#[tokio::test]
async fn results_and_response_sizes_are_bounded_without_approving_results() {
    let text = "😀".repeat(6000);
    let f = Fixture::new(&["8.8.8.8"], vec![response(200, None, &text)]);
    let r = f
        .call(
            "web_fetch",
            json!({"url":"https://docs.example"}),
            "session",
            ApprovalDecision::Once,
        )
        .await
        .unwrap();
    assert_eq!(
        r["content"].as_str().unwrap().encode_utf16().count(),
        10_000
    );
    assert_eq!(r["truncated"], true);
    let huge = "x".repeat(5 * 1024 * 1024 + 1);
    let f = Fixture::new(&["8.8.8.8"], vec![response(200, None, &huge)]);
    assert!(matches!(
        f.call(
            "web_fetch",
            json!({"url":"https://docs.example"}),
            "session",
            ApprovalDecision::Once
        )
        .await,
        Err(Error::OutputLimit)
    ));
    let data = json!({"answer":"a".repeat(2100),"results":[{"title":"t".repeat(400),"url":"https://result.example","content":"c".repeat(600)}]});
    let f = Fixture::new(&["8.8.8.8"], vec![response(200, None, &data.to_string())]);
    let result = f
        .call(
            "web_search",
            json!({"query":" fixture ","maxResults":1}),
            "session",
            ApprovalDecision::Session,
        )
        .await
        .unwrap();
    assert_eq!(result["answer"].as_str().unwrap().len(), 2000);
    assert_eq!(result["results"][0]["title"].as_str().unwrap().len(), 300);
    assert_eq!(result["results"][0]["snippet"].as_str().unwrap().len(), 500);
    {
        let requests = f.transport.requests.lock().unwrap();
        assert_eq!(requests[0].url.as_str(), "https://api.tavily.com/search");
        assert_eq!(requests[0].post.as_ref().unwrap()["query"], "fixture");
    }
    assert!(
        f.registry
            .call(
                "web_fetch",
                json!({"url":"https://result.example"}),
                context("session", None, CancellationToken::new())
            )
            .await
            .is_err()
    );
}
#[tokio::test]
async fn malformed_search_status_redirects_and_missing_keys_fail_explicitly() {
    for body in [
        "null",
        "{}x",
        r#"{"results":[{"title":4}]}"#,
        r#"{"answer":false}"#,
    ] {
        let f = Fixture::new(&["8.8.8.8"], vec![response(200, None, body)]);
        assert!(
            f.call(
                "web_search",
                json!({"query":"fixture"}),
                "session",
                ApprovalDecision::Once
            )
            .await
            .is_err()
        );
    }
    let f = Fixture::new(
        &["8.8.8.8"],
        vec![response(302, Some("https://other.example"), "")],
    );
    assert!(
        f.call(
            "web_search",
            json!({"query":"fixture"}),
            "session",
            ApprovalDecision::Once
        )
        .await
        .is_err()
    );
    assert_eq!(f.transport.requests.lock().unwrap().len(), 1);
    let registry = WebRegistry::new(WebClient::default(), ApprovalManager::default());
    assert!(matches!(
        registry
            .call(
                "web_search",
                json!({"query":"fixture"}),
                context("session", None, CancellationToken::new())
            )
            .await,
        Err(Error::SearchKey)
    ));
    for args in [
        json!({"query":" "}),
        json!({"query":"fixture","maxResults":11}),
        json!({"query":"fixture","maxResults":0}),
        json!({"query":"fixture","api_key":"fictional"}),
    ] {
        assert!(
            registry
                .call(
                    "web_search",
                    args,
                    context("session", None, CancellationToken::new())
                )
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn cancellation_closes_pending_dns_and_approval_is_not_replayable() {
    let resolver = Arc::new(Resolver {
        calls: AtomicUsize::new(0),
        addresses: vec![],
        hang: true,
    });
    let transport = Arc::new(Transport::default());
    let manager = ApprovalManager::default();
    let registry = WebRegistry::new(
        WebClient::new(resolver.clone(), transport.clone(), None),
        manager.clone(),
    );
    let tool = registry
        .tools(Mode::Build)
        .into_iter()
        .find(|t| t.spec().name == "web_fetch")
        .unwrap();
    let args = json!({"url":"https://docs.example"})
        .as_object()
        .unwrap()
        .clone();
    let (tx, mut rx) = mpsc::channel(1);
    let cancel = CancellationToken::new();
    let ctx = context("session", Some(tx), cancel.clone());
    let responder = tokio::spawn(async move {
        if let Some(StreamEvent::ApprovalRequired { approval_id, .. }) = rx.recv().await {
            assert!(manager.decide("session", &approval_id, ApprovalDecision::Once));
        }
    });
    tool.authorize(&args, ctx.clone()).await.unwrap();
    responder.await.unwrap();
    let copied = tool.clone();
    let cloned_args = args.clone();
    let cloned_ctx = ctx.clone();
    let task = tokio::spawn(async move { copied.execute(cloned_args, cloned_ctx).await });
    while resolver.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    assert!(task.await.unwrap().is_err());
    assert!(tool.execute(args, ctx).await.is_err());
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rebinding_on_a_same_origin_redirect_is_rejected_before_second_connection() {
    struct Changing(AtomicUsize);
    impl WebResolver for Changing {
        fn resolve(
            &self,
            _host: String,
            _cancel: CancellationToken,
        ) -> BoxFuture<'_, Result<Vec<IpAddr>>> {
            let first = self.0.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                Ok(vec![
                    if first { "8.8.8.8" } else { "127.0.0.1" }.parse().unwrap(),
                ])
            })
        }
    }
    let transport = Arc::new(Transport {
        responses: Mutex::new(vec![response(302, Some("/again"), "")].into()),
        requests: Mutex::new(vec![]),
    });
    let resolver = Arc::new(Changing(AtomicUsize::new(0)));
    let manager = ApprovalManager::default();
    let registry = WebRegistry::new(
        WebClient::new(resolver.clone(), transport.clone(), None),
        manager.clone(),
    );
    let (tx, mut rx) = mpsc::channel(1);
    let responder = tokio::spawn(async move {
        if let Some(StreamEvent::ApprovalRequired { approval_id, .. }) = rx.recv().await {
            assert!(manager.decide("session", &approval_id, ApprovalDecision::Once));
        }
    });
    let result = registry
        .call(
            "web_fetch",
            json!({"url":"https://rebind.example"}),
            context("session", Some(tx), CancellationToken::new()),
        )
        .await;
    responder.await.unwrap();
    assert!(matches!(result, Err(Error::Ssrf)));
    assert_eq!(resolver.0.load(Ordering::SeqCst), 2);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}
