#![forbid(unsafe_code)]
use myagent_core::{
    Agent, BoxFuture, CancellationToken, GenerationInput, Mode, Provider, ProviderEvent,
    ProviderRequest, SharedJournal, ToolCall, ToolContext,
};
use myagent_protocol::{StreamEvent, Validate};
use myagent_store::{Store, StoreConfig, StoreJournal, Workspace};
use myagent_tools::{
    ApprovalDecision, ApprovalManager, CheckpointStore, Error, GitSettings, MutationRegistry,
    PushRequest, PushTransport, SafeWorkspace, ToolRegistry,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tokio::sync::mpsc;

struct ScriptedProvider {
    steps: Mutex<std::collections::VecDeque<Vec<ProviderEvent>>>,
    requests: Mutex<Vec<ProviderRequest>>,
    observed_path: PathBuf,
    observed_contents: Mutex<Vec<String>>,
}
impl ScriptedProvider {
    fn new(path: PathBuf, steps: Vec<Vec<ProviderEvent>>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            requests: Mutex::new(vec![]),
            observed_path: path,
            observed_contents: Mutex::new(vec![]),
        })
    }
}
impl Provider for ScriptedProvider {
    fn stream(
        &self,
        request: ProviderRequest,
        _: CancellationToken,
        events: mpsc::Sender<ProviderEvent>,
    ) -> BoxFuture<'_, myagent_core::Result<()>> {
        Box::pin(async move {
            self.observed_contents
                .lock()
                .unwrap()
                .push(fs::read_to_string(&self.observed_path).unwrap());
            self.requests.lock().unwrap().push(request);
            let step = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected inference");
            for event in step {
                events
                    .send(event)
                    .await
                    .map_err(|_| myagent_core::AgentError::Cancelled)?;
            }
            Ok(())
        })
    }
}
fn scripted_call(id: &str, name: &str, args: Value) -> ProviderEvent {
    ProviderEvent::ToolCall(ToolCall {
        id: id.into(),
        name: name.into(),
        args: args.as_object().unwrap().clone(),
    })
}
async fn run_scripted(f: &Fixture, agent: &Agent) {
    let (tx, mut rx) = mpsc::channel(64);
    let manager = f.manager.clone();
    let session = f.session.clone();
    let events = f.events.clone();
    let observer = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let StreamEvent::ApprovalRequired { approval_id, .. } = &event {
                assert!(manager.decide(&session, approval_id, ApprovalDecision::Once));
            }
            events.lock().unwrap().push(event);
        }
    });
    agent
        .generate(
            GenerationInput {
                session_id: f.session.clone(),
                model: "fixture".into(),
                mode: Mode::Build,
                content: Some("perform the fixture action".into()),
                resume: false,
                context_window: 65536,
                context_estimated: false,
                max_tool_calls: 40,
                cancellation: CancellationToken::new(),
            },
            tx,
        )
        .await
        .unwrap();
    observer.await.unwrap();
}

#[tokio::test]
async fn nested_instructions_defer_the_entire_batch_before_approval_or_writes() {
    for path_kind in ["relative", "absolute", "backslash"] {
        let f = Fixture::new();
        f.write(
            "src/AGENTS.md",
            "Scoped fixture rule: review before editing.",
        );
        f.write("src/file.rs", "before\n");
        f.write("root.txt", "root fixture\n");
        let path = match path_kind {
            "absolute" => f.root.join("src/file.rs").to_string_lossy().into_owned(),
            "backslash" => "src\\file.rs".into(),
            _ => "src/file.rs".into(),
        };
        let edit = json!({"path":path,"old_str":"before","new_str":"after"});
        let provider = ScriptedProvider::new(
            f.root.join("src/file.rs"),
            vec![
                vec![
                    scripted_call("read_deferred", "read_file", json!({"path":"root.txt"})),
                    scripted_call("edit_deferred", "edit_file", edit.clone()),
                    ProviderEvent::Done,
                ],
                vec![
                    scripted_call("edit_reviewed", "edit_file", edit),
                    ProviderEvent::Done,
                ],
                vec![ProviderEvent::Text("done".into()), ProviderEvent::Done],
            ],
        );
        let registry = ToolRegistry::new(SafeWorkspace::new(&f.root).unwrap(), false)
            .with_mutations(f.engine.clone());
        let source = Arc::new(myagent_tools::FilesystemInstructions::new(&f.root).unwrap());
        let agent = Agent::new(
            provider.clone(),
            f.journal(),
            registry.tools(Mode::Build),
            Some(source),
        )
        .unwrap();
        run_scripted(&f, &agent).await;
        assert_eq!(
            *provider.observed_contents.lock().unwrap(),
            vec!["before\n", "before\n", "after\n"]
        );
        let requests = provider.requests.lock().unwrap();
        assert!(requests[1].messages.iter().flat_map(|m| &m.content).any(|part| {
        matches!(part, myagent_protocol::MessagePart::ToolResult { result, .. } if result.contains("Scoped fixture rule"))
    }));
        let events = f.events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamEvent::ApprovalRequired { .. }))
                .count(),
            1
        );
        let rows = f
            .store
            .list_messages(&f.workspace, &f.session, 100, 0)
            .unwrap();
        let parts = &rows.last().unwrap().parts;
        for id in ["read_deferred", "edit_deferred"] {
            assert!(parts.iter().any(|part| matches!(part, myagent_protocol::MessagePart::ToolCall { id: call, execution: Some(myagent_protocol::ExecutionState::Pending), .. } if call == id)));
        }
        assert_eq!(f.steps().len(), 1);
    }
}

#[tokio::test]
async fn persisted_local_undo_allows_the_same_read_in_the_next_generation() {
    let f = Fixture::new();
    f.write("file.rs", "before\n");
    f.run(
        "edit_file",
        json!({"path":"file.rs","old_str":"before","new_str":"after"}),
    )
    .await
    .unwrap();
    let read = json!({"path":"file.rs","numbered":false});
    let provider = ScriptedProvider::new(
        f.root.join("file.rs"),
        vec![
            vec![
                scripted_call("first_read", "read_file", read.clone()),
                ProviderEvent::Done,
            ],
            vec![
                ProviderEvent::Text("first complete".into()),
                ProviderEvent::Done,
            ],
            vec![
                scripted_call("second_read", "read_file", read),
                ProviderEvent::Done,
            ],
            vec![
                ProviderEvent::Text("second complete".into()),
                ProviderEvent::Done,
            ],
        ],
    );
    let registry = ToolRegistry::new(SafeWorkspace::new(&f.root).unwrap(), false);
    let agent = Agent::new(
        provider.clone(),
        f.journal(),
        registry.tools(Mode::Build),
        None,
    )
    .unwrap();
    run_scripted(&f, &agent).await;
    f.undo().await.unwrap();
    // The next generation reconstructs history from SQLite, including USER/[UNDO].
    run_scripted(&f, &agent).await;
    let rows = f
        .store
        .list_messages(&f.workspace, &f.session, 100, 0)
        .unwrap();
    let result = rows
        .last()
        .unwrap()
        .parts
        .iter()
        .find_map(|part| match part {
            myagent_protocol::MessagePart::ToolResult { id, result, .. } if id == "second_read" => {
                Some(result)
            }
            _ => None,
        })
        .unwrap();
    assert!(result.contains("before"), "{result}");
    assert!(!result.contains("after"));
}
#[derive(Default)]
struct FakePush {
    requests: Mutex<Vec<(String, String, String)>>,
}
impl PushTransport for FakePush {
    fn push(
        &self,
        request: PushRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, myagent_tools::Result<()>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            self.requests.lock().unwrap().push((
                request.url().into(),
                request.branch().into(),
                request.refspec(),
            ));
            Ok(())
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    base: PathBuf,
    store: Store,
    workspace: Workspace,
    session: String,
    manager: ApprovalManager,
    engine: MutationRegistry,
    push: Arc<FakePush>,
    events: Arc<Mutex<Vec<StreamEvent>>>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("myagent m4 fixture ")
            .tempdir()
            .unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let root = base.join("workspace");
        fs::create_dir(&root).unwrap();
        let workspace = Workspace::resolve(&root).unwrap();
        let store = Store::open(StoreConfig {
            home: base.join("state"),
            retention_days: 0,
        })
        .unwrap();
        let session = store.create_session(&workspace).unwrap().id;
        let manager = ApprovalManager::default();
        let push = Arc::new(FakePush::default());
        let git = GitSettings {
            identity: Some(("Fixture".into(), "fixture@example.invalid".into())),
            transport: push.clone(),
            use_external_credentials: false,
            ..GitSettings::default()
        };
        let engine = MutationRegistry::new(
            SafeWorkspace::new(&root).unwrap(),
            CheckpointStore::new(base.join("checkpoints")),
            manager.clone(),
            Arc::new(StoreJournal::new(store.clone(), workspace.clone())),
            git,
        );
        Self {
            _temp: temp,
            root,
            base,
            store,
            workspace,
            session,
            manager,
            engine,
            push,
            events: Arc::new(Mutex::new(vec![])),
        }
    }
    fn write(&self, path: &str, data: impl AsRef<[u8]>) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
    fn bytes(&self, path: &str) -> Vec<u8> {
        fs::read(self.root.join(path)).unwrap()
    }
    fn steps(&self) -> Vec<PathBuf> {
        let dir = self.base.join("checkpoints").join(&self.session);
        if !dir.exists() {
            return vec![];
        }
        let mut files = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|s| s == "json"))
            .collect::<Vec<_>>();
        files.sort();
        files
    }
    async fn run(&self, name: &str, args: Value) -> myagent_tools::Result<Value> {
        self.run_choice(name, args, ApprovalDecision::Once).await
    }
    async fn run_choice(
        &self,
        name: &str,
        args: Value,
        choice: ApprovalDecision,
    ) -> myagent_tools::Result<Value> {
        let (tx, mut rx) = mpsc::channel(8);
        let manager = self.manager.clone();
        let session = self.session.clone();
        let events = self.events.clone();
        let responder = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let StreamEvent::ApprovalRequired {
                    approval_id,
                    confirmation,
                    ..
                } = &event
                {
                    let decision = if confirmation.is_some() {
                        ApprovalDecision::ConfirmMain
                    } else {
                        choice
                    };
                    if !manager.decide(&session, approval_id, decision) {
                        assert!(manager.decide(&session, approval_id, ApprovalDecision::Deny));
                    }
                }
                events.lock().unwrap().push(event);
            }
        });
        let ctx = context(&self.session, Some(tx));
        let result = self.engine.call(Mode::Build, name, args, ctx).await;
        responder.await.unwrap();
        result
    }
    async fn undo(&self) -> myagent_tools::Result<myagent_tools::UndoResult> {
        self.engine
            .try_local_command(&self.session, "/undo", CancellationToken::new())
            .await
            .map(|result| result.expect("local undo handled"))
    }
    fn journal(&self) -> SharedJournal {
        Arc::new(StoreJournal::new(
            self.store.clone(),
            self.workspace.clone(),
        ))
    }
}
fn context(session: &str, events: Option<mpsc::Sender<StreamEvent>>) -> ToolContext {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    ToolContext {
        session_id: session.into(),
        cancellation: CancellationToken::new(),
        tool_call_id: format!("call{}", NEXT.fetch_add(1, Ordering::SeqCst)),
        events,
    }
}
#[tokio::test]
async fn no_consent_or_plan_never_changes_the_project() {
    let f = Fixture::new();
    f.write("a.txt", "before");
    assert!(matches!(
        f.run_choice(
            "write_file",
            json!({"path":"a.txt","content":"after"}),
            ApprovalDecision::Deny
        )
        .await,
        Err(Error::Denied)
    ));
    assert!(matches!(
        f.engine
            .call(
                Mode::Build,
                "write_file",
                json!({"path":"a.txt","content":"after"}),
                context(&f.session, None)
            )
            .await,
        Err(Error::Denied)
    ));
    assert!(matches!(
        f.engine
            .call(
                Mode::Plan,
                "write_file",
                json!({"path":"a.txt","content":"after"}),
                context(&f.session, None)
            )
            .await,
        Err(Error::Denied)
    ));
    assert_eq!(f.bytes("a.txt"), b"before");
    assert!(f.steps().is_empty());
    assert!(f.engine.tools(Mode::Plan).is_empty());
    let tools = ToolRegistry::new(SafeWorkspace::new(&f.root).unwrap(), false)
        .with_mutations(f.engine.clone());
    assert_eq!(tools.tools(Mode::Plan).len(), 17);
    assert_eq!(tools.tools(Mode::Build).len(), 26);
    let tool = f
        .engine
        .tools(Mode::Build)
        .into_iter()
        .find(|t| t.spec().name == "write_file")
        .unwrap();
    assert!(
        tool.execute(
            json!({"path":"a.txt","content":"bypass"})
                .as_object()
                .unwrap()
                .clone(),
            context(&f.session, None)
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn exact_crlf_and_modes_survive_reloaded_undo_and_local_note() {
    let f = Fixture::new();
    f.store
        .add_user_message(&f.workspace, &f.session, "fixture", Mode::Plan, "fixture")
        .unwrap();
    f.write("a.ts", "one\r\ntwo\r\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(f.root.join("a.ts"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let result = f
        .run(
            "edit_file",
            json!({"path":"a.ts","old_str":"one\ntwo","new_str":"three\nfour"}),
        )
        .await
        .unwrap();
    assert_eq!(f.bytes("a.ts"), b"three\r\nfour\r\n");
    assert!(result["checkpoint"]["id"].is_string());
    assert_eq!(result["approval"]["choice"], "once");
    let engine = MutationRegistry::new(
        SafeWorkspace::new(&f.root).unwrap(),
        CheckpointStore::new(f.base.join("checkpoints")),
        f.manager.clone(),
        f.journal(),
        GitSettings::default(),
    );
    engine
        .undo(&f.session, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(f.bytes("a.ts"), b"one\r\ntwo\r\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(f.root.join("a.ts"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
    let history = f
        .store
        .list_messages(&f.workspace, &f.session, 500, 0)
        .unwrap();
    assert!(history.last().unwrap().content.starts_with("[UNDO]"));
    assert_eq!(history.last().unwrap().mode, Mode::Plan);
    assert_eq!(history.last().unwrap().model, "fixture");
    assert!(f.steps().is_empty());
}
#[tokio::test]
async fn backups_new_parents_and_multiple_undo_steps_preserve_the_chain() {
    let f = Fixture::new();
    f.write("a", "original");
    f.run(
        "write_file",
        json!({"path":"a","content":"second","backup":true}),
    )
    .await
    .unwrap();
    assert_eq!(f.bytes("a.bak"), b"original");
    f.run(
        "edit_file",
        json!({"path":"a","old_str":"second","new_str":"third"}),
    )
    .await
    .unwrap();
    f.undo().await.unwrap();
    assert_eq!(f.bytes("a"), b"second");
    f.undo().await.unwrap();
    assert_eq!(f.bytes("a"), b"original");
    assert!(!f.root.join("a.bak").exists());
    f.run(
        "write_file",
        json!({"path":"new/deep/file.ts","content":"new"}),
    )
    .await
    .unwrap();
    f.undo().await.unwrap();
    assert!(!f.root.join("new").exists());
}
#[tokio::test]
async fn rewriting_identical_bytes_is_a_distinct_step() {
    let f = Fixture::new();
    f.write("a", "original");
    f.run("write_file", json!({"path":"a","content":"updated"}))
        .await
        .unwrap();
    f.run("write_file", json!({"path":"a","content":"updated"}))
        .await
        .unwrap();
    assert_eq!(f.steps().len(), 2);
    f.undo().await.unwrap();
    assert_eq!(f.bytes("a"), b"updated");
    f.undo().await.unwrap();
    assert_eq!(f.bytes("a"), b"original");
}
#[tokio::test]
async fn binary_untracked_directory_move_and_delete_restore_all_bytes() {
    let f = Fixture::new();
    let bytes = vec![0, 1, 2, 255, 13, 10, 0];
    f.write("source/data.bin", &bytes);
    f.run("move_file", json!({"from":"source","to":"destination"}))
        .await
        .unwrap();
    assert_eq!(f.bytes("destination/data.bin"), bytes);
    f.undo().await.unwrap();
    assert_eq!(f.bytes("source/data.bin"), bytes);
    f.run("delete_file", json!({"path":"source","recursive":true}))
        .await
        .unwrap();
    assert!(!f.root.join("source").exists());
    f.undo().await.unwrap();
    assert_eq!(f.bytes("source/data.bin"), bytes);
    assert!(!f.root.join(".git").exists());
}
#[tokio::test]
async fn conflicts_preflight_all_paths_without_partial_restoration() {
    let f = Fixture::new();
    f.write("a", "source");
    f.write("b", "destination");
    f.run("move_file", json!({"from":"a","to":"b","overwrite":true}))
        .await
        .unwrap();
    f.write("a", "user work");
    assert!(matches!(f.undo().await, Err(Error::Conflict { .. })));
    assert_eq!(f.bytes("a"), b"user work");
    assert_eq!(f.bytes("b"), b"source");
    assert_eq!(f.steps().len(), 1);
}
#[tokio::test]
async fn same_bytes_recreated_and_mtime_restored_still_conflict_on_identity_or_ctime() {
    let f = Fixture::new();
    f.write("a", "before");
    f.run("write_file", json!({"path":"a","content":"after"}))
        .await
        .unwrap();
    let path = f.root.join("a");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::rename(&path, f.root.join("saved-inode")).unwrap();
    f.write("a", "after");
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert!(matches!(f.undo().await, Err(Error::Conflict { .. })));
    assert_eq!(f.bytes("a"), b"after");
}
#[tokio::test]
async fn extra_directory_content_conflicts_and_modes_are_verified() {
    let f = Fixture::new();
    f.write("folder/file", "before");
    f.run("move_file", json!({"from":"folder","to":"moved"}))
        .await
        .unwrap();
    f.write("moved/extra", "user work");
    assert!(matches!(f.undo().await, Err(Error::Conflict { .. })));
    assert_eq!(f.bytes("moved/extra"), b"user work");
    let g = Fixture::new();
    g.write("a", "before");
    g.run("write_file", json!({"path":"a","content":"after"}))
        .await
        .unwrap();
    let path = g.root.join("a");
    #[cfg(windows)]
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
    }
    let conflict = g.undo().await;
    #[cfg(windows)]
    {
        fs::set_permissions(&path, original_permissions).unwrap();
    }
    assert!(matches!(conflict, Err(Error::Conflict { .. })));
}
#[tokio::test]
async fn multi_file_patch_is_one_step_check_only_is_read_only_and_index_untouched() {
    let f = Fixture::new();
    git(&f.root, &["init", "-b", "main"], None);
    f.write("a", "one\n");
    f.write("b", "two\n");
    git(&f.root, &["add", "--", "a", "b"], None);
    let index = fs::read(f.root.join(".git/index")).unwrap();
    let patch = "--- a/a\n+++ b/a\n@@ -1 +1 @@\n-one\n+edited\n--- a/b\n+++ b/b\n@@ -1 +1 @@\n-two\n+changed\n";
    let checked = f
        .run("apply_patch", json!({"patch":patch,"checkOnly":true}))
        .await
        .unwrap();
    assert_eq!(checked["valid"], true);
    assert_eq!(f.bytes("a"), b"one\n");
    assert!(f.steps().is_empty());
    f.run("apply_patch", json!({"patch":patch})).await.unwrap();
    assert_eq!(f.steps().len(), 1);
    assert_eq!(f.bytes("a"), b"edited\n");
    assert_eq!(f.bytes("b"), b"changed\n");
    f.undo().await.unwrap();
    assert_eq!(f.bytes("a"), b"one\n");
    assert_eq!(f.bytes("b"), b"two\n");
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), index);
}
#[tokio::test]
async fn patch_preflights_all_hunks_and_rejects_unsafe_or_unsupported_headers() {
    let f = Fixture::new();
    f.write("a", "one\n");
    f.write("b", "different\n");
    let patch = "--- a/a\n+++ b/a\n@@ -1 +1 @@\n-one\n+edited\n--- a/b\n+++ b/b\n@@ -1 +1 @@\n-two\n+changed\n";
    assert!(matches!(
        f.run("apply_patch", json!({"patch":patch})).await,
        Err(Error::Patch)
    ));
    assert_eq!(f.bytes("a"), b"one\n");
    assert!(f.steps().is_empty());
    for patch in [
        "--- a/.env\n+++ b/.env\n@@ -1 +1 @@\n-a\n+b\n",
        "diff --git a/a b/a\nnew mode 120000\n",
        "GIT binary patch\n",
        "diff --git a/a b/a\nrename from a\nrename to b\n",
        "--- a/file with space\n+++ b/file with space\n@@ -1 +1 @@\n-a\n+b\n",
    ] {
        assert!(f.run("apply_patch", json!({"patch":patch})).await.is_err());
    }
}
#[tokio::test]
async fn patch_create_delete_and_no_final_newline_restore_exactly() {
    let f = Fixture::new();
    f.write("old", "before");
    let patch = "--- a/old\n+++ /dev/null\n@@ -1 +0,0 @@\n-before\n\\ No newline at end of file\n--- /dev/null\n+++ b/new\n@@ -0,0 +1 @@\n+after\n\\ No newline at end of file\n";
    f.run("apply_patch", json!({"patch":patch})).await.unwrap();
    assert!(!f.root.join("old").exists());
    assert_eq!(f.bytes("new"), b"after");
    f.undo().await.unwrap();
    assert_eq!(f.bytes("old"), b"before");
    assert!(!f.root.join("new").exists());
}
#[tokio::test]
async fn session_approvals_are_exact_and_once_does_not_remember() {
    let f = Fixture::new();
    f.write("a", "one");
    f.write("b", "one");
    f.run_choice(
        "edit_file",
        json!({"path":"a","old_str":"one","new_str":"two"}),
        ApprovalDecision::Session,
    )
    .await
    .unwrap();
    let count = f.events.lock().unwrap().len();
    let cached = f
        .run(
            "edit_file",
            json!({"path":"./a","old_str":"two","new_str":"three"}),
        )
        .await
        .unwrap();
    assert_eq!(cached["approval"]["choice"], "cached");
    assert_eq!(f.events.lock().unwrap().len(), count);
    f.run(
        "edit_file",
        json!({"path":"b","old_str":"one","new_str":"two"}),
    )
    .await
    .unwrap();
    assert_eq!(f.events.lock().unwrap().len(), count + 1);
    f.run("write_file", json!({"path":"a","content":"four"}))
        .await
        .unwrap();
    f.run("write_file", json!({"path":"a","content":"five"}))
        .await
        .unwrap();
    assert_eq!(f.events.lock().unwrap().len(), count + 3);
    f.engine.forget_session(&f.session);
    f.run(
        "edit_file",
        json!({"path":"a","old_str":"five","new_str":"six"}),
    )
    .await
    .unwrap();
    assert_eq!(f.events.lock().unwrap().len(), count + 4);
}
#[tokio::test]
async fn individual_tools_never_accept_session_approval() {
    let f = Fixture::new();
    f.write("a", "safe");
    assert!(matches!(
        f.run_choice(
            "delete_file",
            json!({"path":"a"}),
            ApprovalDecision::Session
        )
        .await,
        Err(Error::Denied)
    ));
    assert_eq!(f.bytes("a"), b"safe");
    assert!(matches!(
        f.run_choice("git_init", json!({}), ApprovalDecision::Session)
            .await,
        Err(Error::Denied)
    ));
    assert!(!f.root.join(".git").exists());
}
#[tokio::test]
async fn approval_cannot_be_replayed_modified_or_used_after_cancellation() {
    let f = Fixture::new();
    f.write("a", "before");
    let tool = f
        .engine
        .tools(Mode::Build)
        .into_iter()
        .find(|t| t.spec().name == "write_file")
        .unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    let ctx = context(&f.session, Some(tx));
    let args = json!({"path":"a","content":"after"})
        .as_object()
        .unwrap()
        .clone();
    let manager = f.manager.clone();
    let session = f.session.clone();
    let respond = tokio::spawn(async move {
        if let StreamEvent::ApprovalRequired { approval_id, .. } = rx.recv().await.unwrap() {
            assert!(!manager.decide("other-session", &approval_id, ApprovalDecision::Once));
            assert!(manager.decide(&session, &approval_id, ApprovalDecision::Once));
            assert!(!manager.decide(&session, &approval_id, ApprovalDecision::Once));
        }
    });
    tool.authorize(&args, ctx.clone()).await.unwrap();
    respond.await.unwrap();
    ctx.cancellation.cancel();
    assert!(tool.execute(args, ctx).await.is_err());
    assert_eq!(f.bytes("a"), b"before");
}
#[tokio::test]
async fn changes_after_preview_invalidate_the_approved_plan() {
    let f = Fixture::new();
    f.write("a", "before");
    let tool = f
        .engine
        .tools(Mode::Build)
        .into_iter()
        .find(|t| t.spec().name == "write_file")
        .unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    let ctx = context(&f.session, Some(tx));
    let args = json!({"path":"a","content":"after"})
        .as_object()
        .unwrap()
        .clone();
    let manager = f.manager.clone();
    let session = f.session.clone();
    let respond = tokio::spawn(async move {
        if let StreamEvent::ApprovalRequired { approval_id, .. } = rx.recv().await.unwrap() {
            assert!(manager.decide(&session, &approval_id, ApprovalDecision::Once));
        }
    });
    tool.authorize(&args, ctx.clone()).await.unwrap();
    respond.await.unwrap();
    f.write("a", "user changed");
    assert!(tool.execute(args.clone(), ctx.clone()).await.is_err());
    assert!(tool.execute(args, ctx).await.is_err());
    assert_eq!(f.bytes("a"), b"user changed");
    assert!(f.steps().is_empty());
}
#[tokio::test]
async fn undo_refuses_active_generation_foreign_workspace_and_lost_lease() {
    let f = Fixture::new();
    f.write("a", "before");
    f.run("write_file", json!({"path":"a","content":"after"}))
        .await
        .unwrap();
    let guard = myagent_core::SessionActionGuard::acquire(&f.session).unwrap();
    assert!(matches!(f.undo().await, Err(Error::GenerationActive)));
    drop(guard);
    let assistant = f
        .store
        .begin_assistant(&f.workspace, &f.session, "fixture", Mode::Build)
        .unwrap();
    assert!(f.undo().await.is_err());
    f.store
        .finish_assistant(
            &f.workspace,
            &assistant.id,
            myagent_protocol::MessageStatus::Interrupted,
            Some(0),
            None,
        )
        .unwrap();
    f.store.release_lease(&f.workspace, &f.session).unwrap();
    assert!(matches!(f.undo().await, Err(Error::Session)));
    assert_eq!(f.bytes("a"), b"after");
}
#[tokio::test]
async fn checkpoint_io_failure_prevents_project_writes_and_no_phantom_step() {
    let f = Fixture::new();
    f.write("a", "before");
    fs::write(f.base.join("checkpoints"), "not a directory").unwrap();
    assert!(
        f.run("write_file", json!({"path":"a","content":"after"}))
            .await
            .is_err()
    );
    assert_eq!(f.bytes("a"), b"before");
    let g = Fixture::new();
    g.write("a", "before");
    assert!(matches!(
        g.run(
            "edit_file",
            json!({"path":"a","old_str":"absent","new_str":"after"})
        )
        .await,
        Err(Error::Occurrences(0))
    ));
    assert!(g.steps().is_empty());
}
#[tokio::test]
async fn limits_and_sensitive_paths_fail_before_project_effects() {
    let f = Fixture::new();
    f.write("large", vec![0; 5 * 1024 * 1024 + 1]);
    assert!(matches!(
        f.run("write_file", json!({"path":"large","content":"small"}))
            .await,
        Err(Error::CheckpointLimit)
    ));
    assert_eq!(
        fs::metadata(f.root.join("large")).unwrap().len(),
        5 * 1024 * 1024 + 1
    );
    for path in [
        ".",
        ".env",
        "config.env",
        "a/secret.key",
        "../outside",
        &format!("{}/file", vec!["deep"; 33].join("/")),
    ] {
        assert!(
            f.run("write_file", json!({"path":path,"content":"no"}))
                .await
                .is_err(),
            "{path}"
        );
    }
    assert!(f.steps().is_empty());
}
#[tokio::test]
async fn completed_history_is_bounded_to_twenty_steps() {
    let f = Fixture::new();
    for n in 0..23 {
        f.run(
            "write_file",
            json!({"path":"counter","content":n.to_string()}),
        )
        .await
        .unwrap();
    }
    assert_eq!(f.steps().len(), 20);
    for _ in 0..20 {
        f.undo().await.unwrap();
    }
    assert_eq!(f.bytes("counter"), b"2");
    assert!(matches!(f.undo().await, Err(Error::NoCheckpoint)));
}
fn git(root: &Path, args: &[&str], input: Option<&[u8]>) -> String {
    let mut cmd = Command::new("git");
    cmd.current_dir(root).env_clear();
    for key in ["PATH", "Path", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    } else {
        drop(child.stdin.take());
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "fixture git failed");
    String::from_utf8(output.stdout).unwrap().trim().into()
}
#[tokio::test]
async fn git_bootstrap_selective_commit_and_remote_require_individual_consents() {
    let f = Fixture::new();
    f.write("a.txt", "initial\n");
    f.run("git_init", json!({})).await.unwrap();
    assert!(f.root.join(".git").is_dir());
    f.write("unrelated.txt", "staged outside scope\n");
    git(&f.root, &["add", "--", "unrelated.txt"], None);
    let committed = f
        .run(
            "git_commit",
            json!({"message":"feat: fixture initial","files":["a.txt"]}),
        )
        .await
        .unwrap();
    assert!(committed["commitHash"].is_string());
    assert_eq!(
        git(&f.root, &["diff", "--cached", "--name-only"], None),
        "unrelated.txt"
    );
    assert_eq!(git(&f.root, &["show", "HEAD:a.txt"], None), "initial");
    let remote = f
        .run(
            "git_remote_add",
            json!({"url":"https://github.com/example/fixture.git"}),
        )
        .await
        .unwrap();
    assert_eq!(remote["added"], true);
    assert!(matches!(
        f.run(
            "git_remote_add",
            json!({"url":"https://github.com/example/different.git"})
        )
        .await,
        Err(Error::GitTarget)
    ));
    assert!(f.steps().is_empty());
    assert!(f.undo().await.is_err());
}
#[tokio::test]
async fn git_push_binds_url_branch_and_requires_two_distinct_main_decisions() {
    let f = Fixture::new();
    f.run("git_init", json!({})).await.unwrap();
    f.write("a", "initial");
    f.run(
        "git_commit",
        json!({"message":"feat: fixture","files":["a"]}),
    )
    .await
    .unwrap();
    f.run(
        "git_remote_add",
        json!({"url":"https://github.com/example/fixture.git"}),
    )
    .await
    .unwrap();
    let before = f.events.lock().unwrap().len();
    let pushed = f.run("git_push", json!({"branch":"main"})).await.unwrap();
    assert_eq!(pushed["awaitingRelease"], true);
    assert_eq!(f.events.lock().unwrap().len(), before + 2);
    assert_eq!(
        f.push.requests.lock().unwrap().as_slice(),
        &[(
            "https://github.com/example/fixture.git".into(),
            "main".into(),
            "refs/heads/main:refs/heads/main".into()
        )]
    );
    assert!(
        f.run("git_push", json!({"branch":"different"}))
            .await
            .is_err()
    );
    assert!(matches!(
        f.run(
            "git_push",
            json!({"branch":"main","url":"https://github.com/example/other.git"})
        )
        .await,
        Err(Error::StaleApproval)
    ));
    for event in &*f.events.lock().unwrap() {
        if let StreamEvent::ApprovalRequired {
            tool_name,
            allow_session,
            args,
            ..
        } = event
            && tool_name == "git_push"
        {
            assert_eq!(*allow_session, Some(false));
            assert_eq!(args["url"], "https://github.com/example/fixture.git");
            assert_eq!(args["branch"], "main");
        }
    }
}
#[tokio::test]
async fn malformed_git_targets_are_never_offered_or_executed() {
    let f = Fixture::new();
    f.run("git_init", json!({})).await.unwrap();
    for url in [
        "http://github.com/a/b",
        "https://user:pass@github.com/a/b",
        "https://example.invalid/a/b",
        "https://github.com/a/b?token=fake",
        "https://github.com/a/%62",
        "https://github.com/",
    ] {
        assert!(f.run("git_remote_add", json!({"url":url})).await.is_err());
    }
    assert!(
        f.run(
            "git_remote_add",
            json!({"remote":"--bad","url":"https://github.com/a/b"})
        )
        .await
        .is_err()
    );
    assert!(f.push.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ctime_alone_detects_same_inode_same_bytes_and_restored_mtime() {
    let f = Fixture::new();
    f.write("a", "before");
    f.run("write_file", json!({"path":"a","content":"after"}))
        .await
        .unwrap();
    let path = f.root.join("a");
    let previous = fs::metadata(&path).unwrap();
    let modified = previous.modified().unwrap();
    let before = myagent_platform::change_time(&fs::File::open(&path).unwrap()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(&path, b"other").unwrap();
    fs::write(&path, b"after").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let after = myagent_platform::change_time(&fs::File::open(&path).unwrap()).unwrap();
    assert_ne!(before, after);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert_eq!(f.bytes("a"), b"after");
    assert!(matches!(f.undo().await, Err(Error::Conflict { .. })));
}
#[tokio::test]
async fn concurrent_approved_creations_serialize_and_only_one_can_publish() {
    let f = Fixture::new();
    let first = f
        .engine
        .tools(Mode::Build)
        .into_iter()
        .find(|t| t.spec().name == "write_file")
        .unwrap();
    let second = f
        .engine
        .tools(Mode::Build)
        .into_iter()
        .find(|t| t.spec().name == "write_file")
        .unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    let ctx1 = context(&f.session, Some(tx.clone()));
    let ctx2 = context(&f.session, Some(tx));
    let manager = f.manager.clone();
    let session = f.session.clone();
    let responder = tokio::spawn(async move {
        for _ in 0..2 {
            if let StreamEvent::ApprovalRequired { approval_id, .. } = rx.recv().await.unwrap() {
                assert!(manager.decide(&session, &approval_id, ApprovalDecision::Once));
            }
        }
    });
    let a = json!({"path":"new","content":"one","overwrite":false})
        .as_object()
        .unwrap()
        .clone();
    let b = json!({"path":"new","content":"two","overwrite":false})
        .as_object()
        .unwrap()
        .clone();
    let (x, y) = tokio::join!(
        first.authorize(&a, ctx1.clone()),
        second.authorize(&b, ctx2.clone())
    );
    x.unwrap();
    y.unwrap();
    responder.await.unwrap();
    let (x, y) = tokio::join!(first.execute(a, ctx1), second.execute(b, ctx2));
    assert_ne!(x.is_ok(), y.is_ok());
    assert_eq!(f.steps().len(), 1);
    assert!([b"one".to_vec(), b"two".to_vec()].contains(&f.bytes("new")));
}
#[tokio::test]
async fn corrupt_preimage_hash_is_rejected_before_any_restore() {
    let f = Fixture::new();
    f.write("a", "before");
    f.run("write_file", json!({"path":"a","content":"after"}))
        .await
        .unwrap();
    let path = &f.steps()[0];
    let mut record: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    record["before"][0]["entries"][0]["data"] = json!("YmFk");
    fs::write(path, record.to_string()).unwrap();
    assert!(matches!(f.undo().await, Err(Error::InvalidCheckpoint)));
    assert_eq!(f.bytes("a"), b"after");
    assert_eq!(f.steps().len(), 1);
}
struct ToolProvider {
    tool: &'static str,
    args: Value,
    calls: AtomicUsize,
    extra: bool,
}
impl Provider for ToolProvider {
    fn stream(
        &self,
        _request: ProviderRequest,
        cancel: CancellationToken,
        events: mpsc::Sender<ProviderEvent>,
    ) -> BoxFuture<'_, myagent_core::Result<()>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(myagent_core::AgentError::Cancelled);
            }
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let values = if call == 0 {
                let mut values = vec![ProviderEvent::ToolCall(ToolCall {
                    id: "effect-call".into(),
                    name: self.tool.into(),
                    args: self.args.as_object().unwrap().clone(),
                })];
                if self.extra {
                    values.push(ProviderEvent::ToolCall(ToolCall {
                        id: "extra-call".into(),
                        name: "write_file".into(),
                        args: json!({"path":"must-not-exist","content":"no"})
                            .as_object()
                            .unwrap()
                            .clone(),
                    }));
                }
                values.push(ProviderEvent::Done);
                values
            } else {
                vec![ProviderEvent::Text("done".into()), ProviderEvent::Done]
            };
            for event in values {
                events
                    .send(event)
                    .await
                    .map_err(|_| myagent_core::AgentError::Cancelled)?;
            }
            Ok(())
        })
    }
}
async fn generation(
    f: &Fixture,
    provider: Arc<ToolProvider>,
) -> myagent_core::Result<myagent_core::GenerationResult> {
    let agent = Agent::new(provider, f.journal(), f.engine.tools(Mode::Build), None).unwrap();
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);
    let manager = f.manager.clone();
    let session = f.session.clone();
    let observer = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            event.validate().unwrap();
            if let StreamEvent::ApprovalRequired {
                approval_id,
                confirmation,
                ..
            } = event
            {
                if confirmation.is_some() {
                    assert!(!manager.decide(&session, &approval_id, ApprovalDecision::Once));
                    assert!(manager.decide(&session, &approval_id, ApprovalDecision::ConfirmMain));
                } else {
                    assert!(manager.decide(&session, &approval_id, ApprovalDecision::Once));
                }
            }
        }
    });
    let result = agent
        .generate(
            GenerationInput {
                session_id: f.session.clone(),
                model: "fixture".into(),
                mode: Mode::Build,
                content: Some("perform the fixture action".into()),
                resume: false,
                context_window: 65536,
                context_estimated: false,
                max_tool_calls: 40,
                cancellation: CancellationToken::new(),
            },
            tx,
        )
        .await;
    observer.await.unwrap();
    result
}
#[tokio::test]
async fn agent_persists_approval_checkpoint_and_stops_after_push() {
    let f = Fixture::new();
    let provider = Arc::new(ToolProvider {
        tool: "write_file",
        args: json!({"path":"a","content":"initial"}),
        calls: AtomicUsize::new(0),
        extra: false,
    });
    generation(&f, provider.clone()).await.unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let messages = f
        .store
        .list_messages(&f.workspace, &f.session, 500, 0)
        .unwrap();
    assert!(messages.last().unwrap().parts.iter().any(|p|matches!(p,myagent_protocol::MessagePart::ToolResult{result,..} if result.contains("checkpoint")&&result.contains("approval"))));
    f.run("git_init", json!({})).await.unwrap();
    f.run(
        "git_commit",
        json!({"message":"feat: fixture","files":["a"]}),
    )
    .await
    .unwrap();
    f.run(
        "git_remote_add",
        json!({"url":"https://github.com/example/fixture.git"}),
    )
    .await
    .unwrap();
    let provider = Arc::new(ToolProvider {
        tool: "git_push",
        args: json!({"branch":"main"}),
        calls: AtomicUsize::new(0),
        extra: false,
    });
    generation(&f, provider.clone()).await.unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.push.requests.lock().unwrap().len(), 1);
    assert_eq!(
        f.store
            .list_messages(&f.workspace, &f.session, 500, 0)
            .unwrap()
            .last()
            .unwrap()
            .status,
        myagent_protocol::MessageStatus::Complete
    );
}
#[tokio::test]
async fn push_cannot_share_a_provider_batch_with_another_side_effect() {
    let f = Fixture::new();
    let provider = Arc::new(ToolProvider {
        tool: "git_push",
        args: json!({"branch":"main"}),
        calls: AtomicUsize::new(0),
        extra: true,
    });
    assert!(generation(&f, provider).await.is_err());
    assert!(f.push.requests.lock().unwrap().is_empty());
    assert!(!f.root.join("must-not-exist").exists());
}

#[tokio::test]
async fn checkpoint_storage_never_exceeds_sixty_four_mib_with_large_steps() {
    let f = Fixture::new();
    let mut content = "x".repeat(5 * 1024 * 1024);
    f.write("large.txt", &content);
    for n in 0..8 {
        content.replace_range(0..1, &n.to_string());
        f.run("write_file", json!({"path":"large.txt","content":content}))
            .await
            .unwrap();
        let used: u64 = f
            .steps()
            .iter()
            .map(|p| fs::metadata(p).unwrap().len())
            .sum();
        assert!(used <= 64 * 1024 * 1024);
    }
    assert!(f.steps().len() < 8);
    f.undo().await.unwrap();
    assert_eq!(f.bytes("large.txt")[0], b'6');
}
#[tokio::test]
async fn committing_a_deleted_file_preserves_unrelated_staged_entries() {
    let f = Fixture::new();
    f.write("a", "initial");
    f.run("git_init", json!({})).await.unwrap();
    f.run(
        "git_commit",
        json!({"message":"feat: fixture","files":["a"]}),
    )
    .await
    .unwrap();
    f.write("unrelated", "staged");
    git(&f.root, &["add", "--", "unrelated"], None);
    f.run("delete_file", json!({"path":"a"})).await.unwrap();
    f.run(
        "git_commit",
        json!({"message":"fix: remove fixture","files":["a"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        git(&f.root, &["diff", "--cached", "--name-only"], None),
        "unrelated"
    );
    assert_eq!(git(&f.root, &["ls-tree", "--name-only", "HEAD"], None), "");
}
#[cfg(windows)]
#[tokio::test]
async fn failed_readonly_replacement_leaves_no_temporary_project_file() {
    let f = Fixture::new();
    f.write("a", "before");
    let path = f.root.join("a");
    let original = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let result = f
        .run("write_file", json!({"path":"a","content":"after"}))
        .await;
    assert!(result.is_err());
    assert_eq!(f.bytes("a"), b"before");
    let names = fs::read_dir(&f.root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["a"]);
    assert!(f.steps().is_empty());
    fs::set_permissions(path, original).unwrap();
}
