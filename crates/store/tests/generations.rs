#![forbid(unsafe_code)]
use myagent_core::*;
use myagent_protocol::{ExecutionState, MessagePart, MessageStatus, Mode, StreamEvent};
use myagent_store::{Store, StoreConfig, StoreJournal, Workspace};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::{Notify, mpsc};
enum Step {
    Events(Vec<ProviderEvent>),
    Fail(Vec<ProviderEvent>),
    Wait,
}
struct Simulated {
    steps: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<ProviderRequest>>,
    started: Notify,
}
impl Simulated {
    fn new(steps: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            requests: Mutex::new(vec![]),
            started: Notify::new(),
        })
    }
}
impl Provider for Simulated {
    fn stream(
        &self,
        request: ProviderRequest,
        cancel: CancellationToken,
        sender: mpsc::Sender<ProviderEvent>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            let step = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(AgentError::Protocol)?;
            self.started.notify_one();
            let (events, fail) = match step {
                Step::Events(events) => (events, false),
                Step::Fail(events) => (events, true),
                Step::Wait => {
                    cancel.cancelled().await;
                    return Err(AgentError::Cancelled);
                }
            };
            for event in events {
                tokio::select! {_=cancel.cancelled()=>return Err(AgentError::Cancelled),r=sender.send(event)=>r.map_err(|_|AgentError::Cancelled)?}
            }
            if fail {
                Err(AgentError::Network)
            } else {
                Ok(())
            }
        })
    }
}
struct FakeTool {
    name: &'static str,
    effect: Effect,
    calls: AtomicUsize,
    deny: bool,
}
impl FakeTool {
    fn new(name: &'static str, effect: Effect) -> Arc<Self> {
        Arc::new(Self {
            name,
            effect,
            calls: AtomicUsize::new(0),
            deny: false,
        })
    }
}
impl Tool for FakeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.into(),
            description: "fictitious tool".into(),
            parameters: json!({"type":"object","properties":{}}),
            effect: self.effect,
        }
    }
    fn authorize<'a>(
        &'a self,
        _: &'a serde_json::Map<String, Value>,
        _: ToolContext,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if self.deny {
                Err(AgentError::ToolDenied)
            } else {
                Ok(())
            }
        })
    }
    fn execute<'a>(
        &'a self,
        _: serde_json::Map<String, Value>,
        _: ToolContext,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("fictitious result".into())
        })
    }
}
struct Fixture {
    store: Store,
    workspace: Workspace,
    session: String,
    _root: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(root.path()).unwrap();
        let workspace = Workspace::resolve(&base).unwrap();
        let store = Store::open(StoreConfig {
            home: base.join("home"),
            retention_days: 0,
        })
        .unwrap();
        let session = store.create_session(&workspace).unwrap().id;
        Self {
            store,
            workspace,
            session,
            _root: root,
        }
    }
    fn journal(&self) -> Arc<StoreJournal> {
        Arc::new(StoreJournal::new(
            self.store.clone(),
            self.workspace.clone(),
        ))
    }
    fn input(&self) -> GenerationInput {
        GenerationInput {
            session_id: self.session.clone(),
            model: "fixture".into(),
            mode: Mode::Build,
            content: Some("fictitious request".into()),
            resume: false,
            context_window: 65536,
            context_estimated: true,
            max_tool_calls: 40,
            cancellation: CancellationToken::new(),
        }
    }
    fn rows(&self) -> Vec<myagent_protocol::Message> {
        self.store
            .list_messages(&self.workspace, &self.session, 100, 0)
            .unwrap()
    }
}
fn call(id: &str, name: &str, args: Value) -> ProviderEvent {
    ProviderEvent::ToolCall(ToolCall {
        id: id.into(),
        name: name.into(),
        args: serde_json::from_value(args).unwrap(),
    })
}
fn complete(text: &str) -> Step {
    Step::Events(vec![ProviderEvent::Text(text.into()), ProviderEvent::Done])
}
async fn run(
    agent: &Agent,
    input: GenerationInput,
) -> (Result<GenerationResult>, Vec<StreamEvent>) {
    let (tx, mut rx) = mpsc::channel(32);
    let reader = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    });
    let result = agent.generate(input, tx).await;
    (result, reader.await.unwrap())
}
#[tokio::test]
async fn generation_persists_deltas_tools_usage_and_only_then_completes() {
    let f = Fixture::new();
    let tool = FakeTool::new("read_file", Effect::Read);
    let provider = Simulated::new(vec![
        Step::Events(vec![
            ProviderEvent::Reasoning("private reasoning".into()),
            ProviderEvent::Text("partial ".into()),
            call("call_1", "read_file", json!({"path":"file.rs"})),
            ProviderEvent::Usage(7),
            ProviderEvent::Usage(7),
            ProviderEvent::Done,
        ]),
        Step::Events(vec![
            ProviderEvent::Text("answer".into()),
            ProviderEvent::Usage(3),
            ProviderEvent::Done,
        ]),
    ]);
    let agent = Agent::new(provider.clone(), f.journal(), vec![tool.clone()], None).unwrap();
    let (result, events) = run(&agent, f.input()).await;
    let result = result.unwrap();
    assert_eq!(result.total_tokens, Some(10));
    assert_eq!(result.tool_calls, 1);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    let rows = f.rows();
    let saved = rows.last().unwrap();
    assert_eq!(saved.status, MessageStatus::Complete);
    assert_eq!(saved.content, "partial answer");
    assert_eq!(saved.total_tokens, Some(10));
    assert!(saved.parts.iter().any(|p| matches!(
        p,
        MessagePart::ToolCall {
            execution: Some(ExecutionState::Started),
            ..
        }
    )));
    assert!(
        saved
            .parts
            .iter()
            .any(|p| matches!(p,MessagePart::ToolResult{id,..} if id=="call_1"))
    );
    assert!(matches!(
        events.first(),
        Some(StreamEvent::GenerationStarted { .. })
    ));
    assert!(matches!(events.last(), Some(StreamEvent::Done { .. })));
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    let context = f.store.load_context(&f.workspace, &f.session).unwrap();
    assert!(context.estimated_tokens > 0);
}
#[tokio::test]
async fn missing_usage_remains_unknown_and_new_turn_omits_reasoning() {
    let f = Fixture::new();
    let provider = Simulated::new(vec![
        Step::Events(vec![
            ProviderEvent::Reasoning("reason".into()),
            ProviderEvent::Text("first".into()),
            ProviderEvent::Done,
        ]),
        complete("second"),
    ]);
    let agent = Agent::new(provider.clone(), f.journal(), vec![], None).unwrap();
    assert_eq!(run(&agent, f.input()).await.0.unwrap().total_tokens, None);
    run(&agent, f.input()).await.0.unwrap();
    assert!(
        provider.requests.lock().unwrap()[1]
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .all(|p| !matches!(p, MessagePart::Reasoning { .. }))
    );
}
#[tokio::test]
async fn eof_and_network_failure_are_interrupted_with_partial_text_and_reported_usage() {
    for step in [
        Step::Events(vec![ProviderEvent::Text("partial".into())]),
        Step::Fail(vec![
            ProviderEvent::Text("partial".into()),
            ProviderEvent::Usage(4),
        ]),
    ] {
        let f = Fixture::new();
        let provider = Simulated::new(vec![step]);
        let agent = Agent::new(provider, f.journal(), vec![], None).unwrap();
        let (result, events) = run(&agent, f.input()).await;
        assert!(result.is_err());
        let rows = f.rows();
        assert_eq!(rows.last().unwrap().status, MessageStatus::Interrupted);
        assert_eq!(rows.last().unwrap().content, "partial");
        assert!(matches!(events.last(), Some(StreamEvent::Error { .. })));
    }
}
#[tokio::test]
async fn plan_list_cannot_expose_writes_even_when_tool_effect_is_misclassified() {
    let f = Fixture::new();
    let write = FakeTool::new("write_file", Effect::Read);
    let read = FakeTool::new("read_file", Effect::Read);
    let provider = Simulated::new(vec![Step::Events(vec![
        call("forbidden", "write_file", json!({"path":"f.rs"})),
        ProviderEvent::Done,
    ])]);
    let agent = Agent::new(
        provider.clone(),
        f.journal(),
        vec![write.clone(), read],
        None,
    )
    .unwrap();
    let mut input = f.input();
    input.mode = Mode::Plan;
    assert_eq!(
        run(&agent, input).await.0.unwrap_err(),
        AgentError::ToolDenied
    );
    assert!(
        provider.requests.lock().unwrap()[0]
            .tools
            .iter()
            .all(|t| t.name != "write_file")
    );
    assert_eq!(write.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn synthesis_removes_all_tools_when_two_calls_remain() {
    let f = Fixture::new();
    let tool = FakeTool::new("read_file", Effect::Read);
    let provider = Simulated::new(vec![
        Step::Events(vec![
            call("one", "read_file", json!({"offset":1})),
            ProviderEvent::Done,
        ]),
        complete("partial answer"),
    ]);
    let agent = Agent::new(provider.clone(), f.journal(), vec![tool], None).unwrap();
    let mut input = f.input();
    input.max_tool_calls = 3;
    let (result, events) = run(&agent, input).await;
    result.unwrap();
    let requests = provider.requests.lock().unwrap();
    assert!(!requests[0].tools.is_empty());
    assert!(requests[1].tools.is_empty());
    assert!(requests[1].system.contains("SEM ferramentas"));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolLimitWarning { limit: 3 }))
    );
}

#[tokio::test]
async fn compact_plan_keeps_verification_tools_without_exposing_bash_or_writes() {
    let f = Fixture::new();
    let provider = Simulated::new(vec![complete("plan")]);
    let tools = vec![
        FakeTool::new("test_run", Effect::IsolatedProcess) as Arc<dyn Tool>,
        FakeTool::new("type_check", Effect::IsolatedProcess),
        FakeTool::new("bash", Effect::IsolatedProcess),
        FakeTool::new("write_file", Effect::Write),
    ];
    let agent = Agent::new(provider.clone(), f.journal(), tools, None).unwrap();
    let mut input = f.input();
    input.mode = Mode::Plan;
    input.context_window = 16384;
    run(&agent, input).await.0.unwrap();
    let requests = provider.requests.lock().unwrap();
    let names: Vec<_> = requests[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(names, vec!["test_run", "type_check"]);
}
#[tokio::test]
async fn identical_calls_are_not_reexecuted_and_fourth_repeat_interrupts() {
    let f = Fixture::new();
    let tool = FakeTool::new("read_file", Effect::Read);
    let steps = (0..4)
        .map(|i| {
            Step::Events(vec![
                call(&format!("call_{i}"), "read_file", json!({"path":"f.rs"})),
                ProviderEvent::Done,
            ])
        })
        .collect();
    let provider = Simulated::new(steps);
    let agent = Agent::new(provider, f.journal(), vec![tool.clone()], None).unwrap();
    assert_eq!(
        run(&agent, f.input()).await.0.unwrap_err(),
        AgentError::ToolLoop
    );
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.rows().last().unwrap().status, MessageStatus::Interrupted);
}
#[tokio::test]
async fn resuming_preserves_generation_id_and_refuses_started_side_effect_replay() {
    for execution in [Some(ExecutionState::Started), None] {
        let f = Fixture::new();
        f.store
            .add_user_message(&f.workspace, &f.session, "fixture", Mode::Build, "request")
            .unwrap();
        let old = f
            .store
            .begin_assistant(&f.workspace, &f.session, "fixture", Mode::Build)
            .unwrap();
        f.store
            .append_part(
                &f.workspace,
                &old.id,
                MessagePart::ToolCall {
                    id: "old_call".into(),
                    name: "write_file".into(),
                    args: serde_json::from_value(json!({"path":"f.rs","content":"same"})).unwrap(),
                    execution,
                },
            )
            .unwrap();
        f.store
            .finish_assistant(
                &f.workspace,
                &old.id,
                MessageStatus::Interrupted,
                Some(1),
                None,
            )
            .unwrap();
        let tool = FakeTool::new("write_file", Effect::Write);
        let provider = Simulated::new(vec![Step::Events(vec![
            call(
                "retry",
                "write_file",
                json!({"content":"same","path":"f.rs"}),
            ),
            ProviderEvent::Done,
        ])]);
        let agent = Agent::new(provider, f.journal(), vec![tool.clone()], None).unwrap();
        let mut input = f.input();
        input.resume = true;
        input.content = None;
        assert_eq!(run(&agent, input).await.0.unwrap_err(), AgentError::Replay);
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
        let rows = f.rows();
        let resumed = rows.last().unwrap();
        assert_ne!(resumed.id, old.id);
        assert_eq!(resumed.generation_id, Some(old.id));
        assert_eq!(resumed.status, MessageStatus::Interrupted);
    }
}
#[tokio::test]
async fn orphan_recovery_preserves_live_foreign_leases() {
    let f = Fixture::new();
    f.store
        .add_user_message(&f.workspace, &f.session, "fixture", Mode::Build, "request")
        .unwrap();
    let message = f
        .store
        .begin_assistant(&f.workspace, &f.session, "fixture", Mode::Build)
        .unwrap();
    let other = Store::open(StoreConfig {
        home: f._root.path().join("home"),
        retention_days: 0,
    })
    .unwrap();
    assert_eq!(other.recover_orphaned_generations(&f.workspace).unwrap(), 0);
    f.store.release_lease(&f.workspace, &f.session).unwrap();
    assert_eq!(other.recover_orphaned_generations(&f.workspace).unwrap(), 1);
    assert_eq!(
        other
            .list_messages(&f.workspace, &f.session, 100, 0)
            .unwrap()
            .last()
            .unwrap()
            .id,
        message.id
    );
    assert_eq!(
        other
            .list_messages(&f.workspace, &f.session, 100, 0)
            .unwrap()
            .last()
            .unwrap()
            .status,
        MessageStatus::Interrupted
    );
}
#[tokio::test]
async fn concurrent_generation_is_rejected_before_writing_another_user_message() {
    let f = Fixture::new();
    let provider = Simulated::new(vec![Step::Wait, complete("next")]);
    let agent = Agent::new(provider.clone(), f.journal(), vec![], None).unwrap();
    let input = f.input();
    let cancel = input.cancellation.clone();
    let worker = agent.clone();
    let first = tokio::spawn(async move { run(&worker, input).await });
    provider.started.notified().await;
    assert_eq!(
        run(&agent, f.input()).await.0.unwrap_err(),
        AgentError::Busy
    );
    assert_eq!(f.rows().len(), 2);
    cancel.cancel();
    assert_eq!(first.await.unwrap().0.unwrap_err(), AgentError::Cancelled);
    assert_eq!(f.rows().last().unwrap().status, MessageStatus::Interrupted);
    run(&agent, f.input()).await.0.unwrap();
}
#[tokio::test(start_paused = true)]
async fn global_five_minute_deadline_interrupts_and_persists() {
    let f = Fixture::new();
    let provider = Simulated::new(vec![Step::Wait]);
    let agent = Agent::new(provider.clone(), f.journal(), vec![], None).unwrap();
    let input = f.input();
    let worker = tokio::spawn(async move { run(&agent, input).await });
    loop {
        tokio::select! {_=provider.started.notified()=>break,_=tokio::task::yield_now()=>{}}
    }
    tokio::time::advance(std::time::Duration::from_secs(301)).await;
    assert_eq!(worker.await.unwrap().0.unwrap_err(), AgentError::Timeout);
    assert_eq!(f.rows().last().unwrap().status, MessageStatus::Interrupted);
}
#[tokio::test]
async fn failed_authorization_never_starts_or_executes_a_tool() {
    let f = Fixture::new();
    let tool = Arc::new(FakeTool {
        name: "write_file",
        effect: Effect::Write,
        calls: AtomicUsize::new(0),
        deny: true,
    });
    let provider = Simulated::new(vec![Step::Events(vec![
        call("one", "write_file", json!({})),
        ProviderEvent::Done,
    ])]);
    let agent = Agent::new(provider, f.journal(), vec![tool.clone()], None).unwrap();
    assert_eq!(
        run(&agent, f.input()).await.0.unwrap_err(),
        AgentError::ToolDenied
    );
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    assert!(f.rows().last().unwrap().parts.iter().all(|p| !matches!(
        p,
        MessagePart::ToolCall {
            execution: Some(ExecutionState::Started),
            ..
        }
    )));
}
struct FailingContext(Arc<StoreJournal>);
impl Journal for FailingContext {
    fn prepare(&self, i: PrepareGeneration) -> Result<GenerationState> {
        self.0.prepare(i)
    }
    fn append(&self, id: &str, p: MessagePart) -> Result<()> {
        self.0.append(id, p)
    }
    fn started(&self, id: &str, call: &str) -> Result<()> {
        self.0.started(id, call)
    }
    fn usage(&self, id: &str, n: Option<u32>) -> Result<()> {
        self.0.usage(id, n)
    }
    fn finish(&self, id: &str, s: MessageStatus, d: u32, n: Option<u32>) -> Result<()> {
        self.0.finish(id, s, d, n)
    }
    fn context(
        &self,
        _: &str,
        _: Vec<ModelMessage>,
        _: u32,
        _: u32,
        _: bool,
    ) -> Result<Vec<ModelMessage>> {
        Err(AgentError::Persistence)
    }
    fn check_lease(&self, s: &str) -> Result<()> {
        self.0.check_lease(s)
    }
}
#[tokio::test]
async fn persistence_failure_prevents_inference() {
    let f = Fixture::new();
    let provider = Simulated::new(vec![complete("must not be requested")]);
    let agent = Agent::new(
        provider.clone(),
        Arc::new(FailingContext(f.journal())),
        vec![],
        None,
    )
    .unwrap();
    assert_eq!(
        run(&agent, f.input()).await.0.unwrap_err(),
        AgentError::Persistence
    );
    assert!(provider.requests.lock().unwrap().is_empty());
    assert_eq!(f.rows().last().unwrap().status, MessageStatus::Interrupted);
}

#[tokio::test]
async fn more_than_forty_proposed_calls_are_rejected_before_any_effect() {
    let f = Fixture::new();
    let tool = FakeTool::new("read_file", Effect::Read);
    let mut events: Vec<_> = (0..41)
        .map(|n| call(&format!("c{n}"), "read_file", json!({"offset":n})))
        .collect();
    events.push(ProviderEvent::Done);
    let provider = Simulated::new(vec![Step::Events(events)]);
    let agent = Agent::new(provider, f.journal(), vec![tool.clone()], None).unwrap();
    assert_eq!(
        run(&agent, f.input()).await.0.unwrap_err(),
        AgentError::Limit
    );
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.rows()
            .last()
            .unwrap()
            .parts
            .iter()
            .filter(|p| matches!(p, MessagePart::ToolCall { .. }))
            .count(),
        40
    );
}
#[tokio::test]
async fn resume_preserves_the_original_call_budget_and_pending_user_is_not_duplicated() {
    let f = Fixture::new();
    f.store
        .add_user_message(&f.workspace, &f.session, "fixture", Mode::Build, "pending")
        .unwrap();
    let provider = Simulated::new(vec![complete("first"), complete("summary")]);
    let tool = FakeTool::new("read_file", Effect::Read);
    let agent = Agent::new(provider.clone(), f.journal(), vec![tool], None).unwrap();
    let mut resume = f.input();
    resume.resume = true;
    resume.content = None;
    run(&agent, resume.clone()).await.0.unwrap();
    assert_eq!(
        f.rows()
            .iter()
            .filter(|m| m.role == myagent_protocol::Role::User)
            .count(),
        1
    );
    let old = f
        .store
        .begin_assistant(&f.workspace, &f.session, "fixture", Mode::Build)
        .unwrap();
    for n in 0..40 {
        f.store
            .append_part(
                &f.workspace,
                &old.id,
                MessagePart::ToolCall {
                    id: format!("old{n}"),
                    name: "read_file".into(),
                    args: serde_json::from_value(json!({"offset":n})).unwrap(),
                    execution: Some(ExecutionState::Pending),
                },
            )
            .unwrap();
    }
    f.store
        .finish_assistant(
            &f.workspace,
            &old.id,
            MessageStatus::Interrupted,
            None,
            None,
        )
        .unwrap();
    let result = run(&agent, resume).await.0.unwrap();
    assert_eq!(result.tool_calls, 40);
    assert!(
        provider
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .tools
            .is_empty()
    );
}
#[tokio::test]
async fn dropping_the_consumer_cancels_the_worker_and_persists_interruption() {
    let f = Fixture::new();
    let provider = Simulated::new(vec![Step::Wait]);
    let agent = Agent::new(provider.clone(), f.journal(), vec![], None).unwrap();
    let input = f.input();
    let task = tokio::spawn(async move { run(&agent, input).await });
    provider.started.notified().await;
    task.abort();
    let _ = task.await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if f.rows()
                .last()
                .is_some_and(|m| m.status == MessageStatus::Interrupted)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn history_preserves_text_and_tool_result_order_across_steps() {
    let f = Fixture::new();
    let tool = FakeTool::new("read_file", Effect::Read);
    let provider = Simulated::new(vec![
        Step::Events(vec![
            ProviderEvent::Text("before".into()),
            call("c", "read_file", json!({})),
            ProviderEvent::Done,
        ]),
        complete("after"),
        complete("new turn"),
    ]);
    let agent = Agent::new(provider.clone(), f.journal(), vec![tool], None).unwrap();
    run(&agent, f.input()).await.0.unwrap();
    run(&agent, f.input()).await.0.unwrap();
    let requests = provider.requests.lock().unwrap();
    let history = &requests[2].messages;
    let tool_index = history
        .iter()
        .position(|m| m.role == ModelRole::Tool)
        .unwrap();
    let after_index = history
        .iter()
        .position(|m| {
            m.content
                .iter()
                .any(|p| matches!(p,MessagePart::Text{text} if text=="after"))
        })
        .unwrap();
    assert!(tool_index < after_index);
}

#[tokio::test]
async fn session_token_counter_separates_measured_usage_from_unreported_messages() {
    let f = Fixture::new();
    let provider = Simulated::new(vec![
        Step::Events(vec![ProviderEvent::Usage(7), ProviderEvent::Done]),
        complete("unknown usage"),
    ]);
    let agent = Agent::new(provider, f.journal(), vec![], None).unwrap();
    run(&agent, f.input()).await.0.unwrap();
    run(&agent, f.input()).await.0.unwrap();
    let usage = token_usage(&f.rows());
    assert_eq!(usage.measured_tokens, Some(7));
    assert_eq!(usage.unreported_messages, 1);
}

struct CancellableTool {
    started: Arc<Notify>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
}
impl Tool for CancellableTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "simulated process".into(),
            parameters: json!({"type":"object"}),
            effect: Effect::IsolatedProcess,
        }
    }
    fn authorize<'a>(
        &'a self,
        _: &'a serde_json::Map<String, Value>,
        _: ToolContext,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn execute<'a>(
        &'a self,
        _: serde_json::Map<String, Value>,
        context: ToolContext,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            self.started.notify_one();
            context.cancellation.cancelled().await;
            self.stopped.store(true, Ordering::SeqCst);
            Err(AgentError::Cancelled)
        })
    }
}
#[tokio::test]
async fn cancellation_waits_for_tool_cleanup_before_marking_interrupted() {
    let f = Fixture::new();
    let started = Arc::new(Notify::new());
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tool = Arc::new(CancellableTool {
        started: started.clone(),
        stopped: stopped.clone(),
    });
    let provider = Simulated::new(vec![Step::Events(vec![
        call("c", "read_file", json!({})),
        ProviderEvent::Done,
    ])]);
    let agent = Agent::new(provider, f.journal(), vec![tool], None).unwrap();
    let input = f.input();
    let cancel = input.cancellation.clone();
    let task = tokio::spawn(async move { run(&agent, input).await });
    started.notified().await;
    cancel.cancel();
    assert_eq!(task.await.unwrap().0.unwrap_err(), AgentError::Cancelled);
    assert!(stopped.load(Ordering::SeqCst));
    assert_eq!(f.rows().last().unwrap().status, MessageStatus::Interrupted);
}

#[tokio::test]
async fn signed_provider_state_survives_resume_but_is_not_reintroduced_on_a_new_user_turn() {
    let f = Fixture::new();
    let tool = FakeTool::new("read_file", Effect::Read);
    let metadata = json!({"google":{"parts":[{"functionCall":{"name":"read_file","args":{}},"thoughtSignature":"fictitious"}]}});
    let first = Simulated::new(vec![
        Step::Events(vec![
            call("c", "read_file", json!({})),
            ProviderEvent::Metadata(metadata),
            ProviderEvent::Done,
        ]),
        Step::Wait,
    ]);
    let agent = Agent::new(first.clone(), f.journal(), vec![tool.clone()], None).unwrap();
    let input = f.input();
    let cancel = input.cancellation.clone();
    let running = tokio::spawn(async move { run(&agent, input).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if first.requests.lock().unwrap().len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        first.requests.lock().unwrap()[1]
            .messages
            .iter()
            .any(|m| m.provider_metadata.is_some())
    );
    cancel.cancel();
    assert_eq!(running.await.unwrap().0.unwrap_err(), AgentError::Cancelled);
    let provider = Simulated::new(vec![complete("resumed"), complete("fresh")]);
    let agent = Agent::new(provider.clone(), f.journal(), vec![tool], None).unwrap();
    let mut resume = f.input();
    resume.resume = true;
    resume.content = None;
    run(&agent, resume).await.0.unwrap();
    assert!(
        provider.requests.lock().unwrap()[0]
            .messages
            .iter()
            .any(|m| m.provider_metadata.is_some())
    );
    run(&agent, f.input()).await.0.unwrap();
    assert!(
        provider.requests.lock().unwrap()[1]
            .messages
            .iter()
            .all(|m| m.provider_metadata.is_none())
    );
}

struct LeaseFailure {
    inner: Arc<StoreJournal>,
    lost: Arc<std::sync::atomic::AtomicBool>,
}
impl Journal for LeaseFailure {
    fn prepare(&self, i: PrepareGeneration) -> Result<GenerationState> {
        self.inner.prepare(i)
    }
    fn append(&self, id: &str, p: MessagePart) -> Result<()> {
        self.inner.append(id, p)
    }
    fn started(&self, id: &str, call: &str) -> Result<()> {
        self.inner.started(id, call)
    }
    fn usage(&self, id: &str, n: Option<u32>) -> Result<()> {
        self.inner.usage(id, n)
    }
    fn finish(&self, id: &str, s: MessageStatus, d: u32, n: Option<u32>) -> Result<()> {
        self.inner.finish(id, s, d, n)
    }
    fn context(
        &self,
        s: &str,
        m: Vec<ModelMessage>,
        l: u32,
        e: u32,
        t: bool,
    ) -> Result<Vec<ModelMessage>> {
        self.inner.context(s, m, l, e, t)
    }
    fn check_lease(&self, s: &str) -> Result<()> {
        if self.lost.load(Ordering::SeqCst) {
            Err(AgentError::LeaseLost)
        } else {
            self.inner.check_lease(s)
        }
    }
}
#[tokio::test(start_paused = true)]
async fn losing_a_lease_cancels_a_long_tool_before_the_global_timeout() {
    let f = Fixture::new();
    let started = Arc::new(Notify::new());
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let journal = Arc::new(LeaseFailure {
        inner: f.journal(),
        lost: lost.clone(),
    });
    let provider = Simulated::new(vec![Step::Events(vec![
        call("c", "read_file", json!({})),
        ProviderEvent::Done,
    ])]);
    let tool = Arc::new(CancellableTool {
        started: started.clone(),
        stopped: stopped.clone(),
    });
    let agent = Agent::new(provider, journal, vec![tool], None).unwrap();
    let input = f.input();
    let task = tokio::spawn(async move { run(&agent, input).await });
    loop {
        tokio::select! {_=started.notified()=>break,_=tokio::task::yield_now()=>{}}
    }
    lost.store(true, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(6)).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !task.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "lease monitor did not cancel tool"
        );
        tokio::task::yield_now().await;
    }
    assert_eq!(task.await.unwrap().0.unwrap_err(), AgentError::LeaseLost);
    assert!(stopped.load(Ordering::SeqCst));
    assert_eq!(f.rows().last().unwrap().status, MessageStatus::Interrupted);
}

#[tokio::test]
async fn interrupted_display_previews_never_execute_or_persist_as_tool_calls() {
    let f = Fixture::new();
    let tool = FakeTool::new("write_file", Effect::Write);
    let provider = Simulated::new(vec![Step::Fail(vec![ProviderEvent::ToolPreview {
        id: "preview_only".into(),
        name: "write_file".into(),
        arguments: r#"{"path":"file.rs","content":"partial"#.into(),
    }])]);
    let agent = Agent::new(provider, f.journal(), vec![tool.clone()], None).unwrap();
    let (result, events) = run(&agent, f.input()).await;
    assert_eq!(result.unwrap_err(), AgentError::Network);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolPreview { .. }))
    );
    let rows = f.rows();
    assert_eq!(rows.last().unwrap().status, MessageStatus::Interrupted);
    assert!(rows.last().unwrap().parts.is_empty());
}
