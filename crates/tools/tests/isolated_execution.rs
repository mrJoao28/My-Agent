#![forbid(unsafe_code)]
use myagent_core::{CancellationToken, Mode, ToolContext};
use myagent_sandbox::{
    Docker, DockerConfig, DockerDriver, DockerFuture, DockerOutput, SandboxError,
};
use myagent_tools::{Error, ExecutionRegistry, SafeWorkspace, ToolRegistry};
use serde_json::json;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct Driver {
    calls: Mutex<Vec<Vec<String>>>,
    unavailable: bool,
    missing_image: bool,
    timeout: bool,
    cleanup_fail: bool,
    wait: AtomicBool,
    cleanup_delay: Duration,
    run_started: tokio::sync::Notify,
    cleanup_started: tokio::sync::Notify,
    cleanup_finished: AtomicBool,
}
impl DockerDriver for Driver {
    fn invoke(
        &self,
        args: Vec<String>,
        _timeout: Duration,
        cancel: CancellationToken,
    ) -> DockerFuture<'_> {
        self.calls.lock().unwrap().push(args.clone());
        Box::pin(async move {
            if args[0] == "info" {
                return if self.unavailable {
                    Err(SandboxError::Unavailable)
                } else {
                    Ok(output("linux", 0))
                };
            }
            if args[0] == "image" {
                return Ok(output("", if self.missing_image { 1 } else { 0 }));
            }
            if args[0] == "rm" {
                self.cleanup_started.notify_one();
                tokio::time::sleep(self.cleanup_delay).await;
                self.cleanup_finished.store(true, Ordering::SeqCst);
                return if self.cleanup_fail {
                    Err(SandboxError::Docker)
                } else {
                    Ok(output("removed", 0))
                };
            }
            if args[0] == "run" {
                self.run_started.notify_one();
                assert!(args.iter().any(|s| s == "--network=none"));
                if self.wait.load(Ordering::SeqCst) {
                    cancel.cancelled().await;
                    return Err(SandboxError::Cancelled);
                }
                if self.timeout {
                    return Err(SandboxError::Timeout);
                }
                return Ok(output("fixture", 0));
            }
            Ok(output("", 0))
        })
    }
}

#[tokio::test]
async fn cancelling_agent_waits_for_docker_cleanup_and_persists_cleanup_failure() {
    use myagent_core::{
        Agent, AgentError, BoxFuture, GenerationInput, Provider, ProviderEvent, ProviderRequest,
        ToolCall,
    };
    use myagent_protocol::{MessageStatus, StreamEvent};
    use myagent_store::{Store, StoreConfig, StoreJournal, Workspace};
    use tokio::sync::mpsc;
    struct ShellProvider;
    impl Provider for ShellProvider {
        fn stream(
            &self,
            _: ProviderRequest,
            _: CancellationToken,
            events: mpsc::Sender<ProviderEvent>,
        ) -> BoxFuture<'_, myagent_core::Result<()>> {
            Box::pin(async move {
                events
                    .send(ProviderEvent::ToolCall(ToolCall {
                        id: "shell_call".into(),
                        name: "bash".into(),
                        args: json!({"command":"fixture"}).as_object().unwrap().clone(),
                    }))
                    .await
                    .map_err(|_| AgentError::Cancelled)?;
                events
                    .send(ProviderEvent::Done)
                    .await
                    .map_err(|_| AgentError::Cancelled)?;
                Ok(())
            })
        }
    }
    for cleanup_fail in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        let root = base.join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("fixture.rs"), "fixture").unwrap();
        let workspace = Workspace::resolve(&root).unwrap();
        let store = Store::open(StoreConfig {
            home: base.join("state"),
            retention_days: 0,
        })
        .unwrap();
        let session = store.create_session(&workspace).unwrap().id;
        let driver = Arc::new(Driver {
            cleanup_fail,
            cleanup_delay: Duration::from_secs(3),
            ..Driver::default()
        });
        driver.wait.store(true, Ordering::SeqCst);
        let registry = registry(&root, driver.clone()).await;
        let agent = Agent::new(
            Arc::new(ShellProvider),
            Arc::new(StoreJournal::new(store.clone(), workspace.clone())),
            registry.tools(Mode::Build),
            None,
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let input = GenerationInput {
            session_id: session.clone(),
            model: "fixture".into(),
            mode: Mode::Build,
            content: Some("run fixture".into()),
            resume: false,
            context_window: 65536,
            context_estimated: false,
            max_tool_calls: 40,
            cancellation: cancellation.clone(),
        };
        let (tx, mut rx) = mpsc::channel(32);
        let task = tokio::spawn(async move { agent.generate(input, tx).await });
        tokio::time::timeout(Duration::from_secs(5), driver.run_started.notified())
            .await
            .unwrap();
        cancellation.cancel();
        driver.cleanup_started.notified().await;
        let rows = store.list_messages(&workspace, &session, 100, 0).unwrap();
        assert_eq!(rows.last().unwrap().status, MessageStatus::InProgress);
        let expected = if cleanup_fail {
            AgentError::Cleanup
        } else {
            AgentError::Cancelled
        };
        assert_eq!(task.await.unwrap().unwrap_err(), expected);
        assert!(driver.cleanup_finished.load(Ordering::SeqCst));
        let rows = store.list_messages(&workspace, &session, 100, 0).unwrap();
        assert_eq!(rows.last().unwrap().status, MessageStatus::Interrupted);
        while let Some(event) = rx.recv().await {
            assert!(!matches!(event, StreamEvent::Done { .. }));
        }
        // Cleanup failure deliberately retains its snapshot; remove the fixture
        // only after the fake driver has confirmed it has stopped.
        if cleanup_fail {
            let calls = driver.calls.lock().unwrap();
            let mount = calls
                .iter()
                .find(|args| args[0] == "run")
                .unwrap()
                .iter()
                .find(|arg| arg.starts_with("type=bind,source="))
                .unwrap();
            let source = mount
                .strip_prefix("type=bind,source=")
                .unwrap()
                .split(",target=")
                .next()
                .unwrap();
            std::fs::remove_dir_all(source).unwrap();
        }
    }
}
fn output(stdout: &str, exit_code: i32) -> DockerOutput {
    DockerOutput {
        stdout: stdout.into(),
        stderr: String::new(),
        exit_code,
    }
}
fn context(token: CancellationToken) -> ToolContext {
    ToolContext {
        session_id: "session".into(),
        tool_call_id: "call".into(),
        cancellation: token,
        events: None,
    }
}
async fn registry(root: &Path, driver: Arc<Driver>) -> ExecutionRegistry {
    ExecutionRegistry::detect(
        SafeWorkspace::new(root).unwrap(),
        DockerConfig::default(),
        driver,
        CancellationToken::new(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn absent_docker_removes_all_process_tools_and_plan_never_has_bash() {
    let temp = tempfile::tempdir().unwrap();
    let absent = Arc::new(Driver {
        unavailable: true,
        ..Driver::default()
    });
    let registry = registry(temp.path(), absent.clone()).await;
    assert!(!registry.available());
    assert!(registry.tools(Mode::Build).is_empty());
    assert!(matches!(
        registry
            .call(
                Mode::Build,
                "bash",
                json!({"command":"never"}),
                context(CancellationToken::new())
            )
            .await,
        Err(Error::Unavailable)
    ));
    assert_eq!(absent.calls.lock().unwrap().len(), 1);
    let present = ExecutionRegistry::detect(
        SafeWorkspace::new(temp.path()).unwrap(),
        DockerConfig::default(),
        Arc::new(Driver::default()),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(present.tools(Mode::Plan).len(), 2);
    assert_eq!(present.tools(Mode::Build).len(), 3);
    assert!(
        present
            .tools(Mode::Plan)
            .iter()
            .all(|t| t.spec().name != "bash")
    );
    assert!(matches!(
        present
            .call(
                Mode::Plan,
                "bash",
                json!({"command":"never"}),
                context(CancellationToken::new())
            )
            .await,
        Err(Error::Denied)
    ));
    let combined =
        ToolRegistry::new(SafeWorkspace::new(temp.path()).unwrap(), false).with_execution(present);
    assert_eq!(combined.tools(Mode::Plan).len(), 19);
    assert_eq!(combined.tools(Mode::Build).len(), 20);
}
#[test]
fn configuration_is_bounded_and_cannot_inject_docker_options() {
    for (key, value) in [
        ("MYAGENT_SANDBOX_IMAGE", "--privileged"),
        ("MYAGENT_SANDBOX_IMAGE", "image,unsafe"),
        ("MYAGENT_SANDBOX_TMPFS_MIB", "0"),
        ("MYAGENT_SANDBOX_TMPFS_MIB", "4097"),
        ("MYAGENT_SANDBOX_TMPFS_MIB", "+64"),
        ("MYAGENT_SANDBOX_DEPENDENCIES", "yes"),
    ] {
        assert!(DockerConfig::from_lookup(|name| (name == key).then(|| value.into())).is_err());
    }
    let config = DockerConfig::from_lookup(|name| match name {
        "MYAGENT_SANDBOX_TMPFS_MIB" => Some("128".into()),
        "MYAGENT_SANDBOX_DEPENDENCIES" => Some("1".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(config.tmpfs_mib, 128);
    assert!(config.dependencies);
}
#[tokio::test]
async fn docker_probe_is_limited_to_three_seconds_and_honors_cancellation() {
    struct Hanging;
    impl DockerDriver for Hanging {
        fn invoke(&self, _: Vec<String>, _: Duration, _: CancellationToken) -> DockerFuture<'_> {
            Box::pin(std::future::pending())
        }
    }
    let start = std::time::Instant::now();
    assert!(
        Docker::detect(
            DockerConfig::default(),
            Arc::new(Hanging),
            CancellationToken::new()
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        Docker::detect(DockerConfig::default(), Arc::new(Hanging), cancel).await,
        Err(SandboxError::Cancelled)
    ));
}
#[tokio::test]
async fn execution_uses_only_filtered_mount_and_all_security_flags() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a"), b"fixture").unwrap();
    let driver = Arc::new(Driver {
        missing_image: true,
        ..Driver::default()
    });
    let registry = registry(temp.path(), driver.clone()).await;
    let result = registry
        .call(
            Mode::Build,
            "bash",
            json!({"command":"printf fixture"}),
            context(CancellationToken::new()),
        )
        .await
        .unwrap();
    assert_eq!(result["stdout"], "fixture");
    let calls = driver.calls.lock().unwrap();
    let args = calls.iter().find(|a| a[0] == "run").unwrap();
    for flag in [
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--pids-limit=128",
        "--memory=512m",
        "--memory-swap=512m",
        "--cpus=1",
        "--user=65534:65534",
        "--pull=never",
    ] {
        assert!(args.iter().any(|a| a == flag), "{flag}");
    }
    assert!(
        !args
            .iter()
            .any(|a| a == "--privileged" || a == "--env-file" || a == "-e")
    );
    assert!(
        args.iter()
            .any(|a| a == "/tmp:rw,nosuid,nodev,size=64m,mode=1777")
    );
    let mount = args.iter().find(|a| a.starts_with("type=bind,")).unwrap();
    assert!(mount.contains("readonly"));
    assert!(!mount.contains(&temp.path().display().to_string()));
    assert_eq!(calls.iter().filter(|a| a[0] == "pull").count(), 1);
    assert_eq!(calls.last().unwrap()[0], "rm");
    let source = mount
        .strip_prefix("type=bind,source=")
        .unwrap()
        .split(",target=")
        .next()
        .unwrap();
    assert!(!Path::new(source).exists());
}
#[tokio::test]
async fn runners_are_quoted_offline_and_keep_dependencies_read_only() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("tests")).unwrap();
    std::fs::write(temp.path().join("Cargo.toml"), b"fixture").unwrap();
    std::fs::write(temp.path().join("tests/smoke.rs"), b"fixture").unwrap();
    let driver = Arc::new(Driver::default());
    let registry = registry(temp.path(), driver.clone()).await;
    registry
        .call(
            Mode::Plan,
            "test_run",
            json!({"files":["tests/smoke.rs"]}),
            context(CancellationToken::new()),
        )
        .await
        .unwrap();
    for runner in ["bun", "node", "vitest", "jest", "pytest"] {
        registry
            .call(
                Mode::Plan,
                "test_run",
                json!({"runner":runner}),
                context(CancellationToken::new()),
            )
            .await
            .unwrap();
    }
    registry
        .call(
            Mode::Plan,
            "type_check",
            json!({}),
            context(CancellationToken::new()),
        )
        .await
        .unwrap();
    let calls = driver.calls.lock().unwrap();
    let scripts = calls
        .iter()
        .filter(|a| a[0] == "run")
        .map(|a| a.last().unwrap())
        .collect::<Vec<_>>();
    assert!(scripts[0].contains("'cargo' 'test' '--offline' '--locked'"));
    assert!(scripts[0].contains("'--test' 'smoke'"));
    assert!(scripts[0].contains("node_modules|.venv) ln -s"));
    assert!(
        scripts
            .last()
            .unwrap()
            .contains("'cargo' 'check' '--offline' '--locked'")
    );
}
#[tokio::test]
async fn malformed_paths_commands_and_timeouts_never_start_a_container() {
    let temp = tempfile::tempdir().unwrap();
    let driver = Arc::new(Driver::default());
    let registry = registry(temp.path(), driver.clone()).await;
    for (name, args) in [
        ("bash", json!({"command":"   "})),
        ("bash", json!({"command":"true","timeoutMs":120001})),
        ("test_run", json!({"path":"../outside"})),
        ("test_run", json!({"runner":"host"})),
        ("test_run", json!({"files":["-unsafe"]})),
    ] {
        assert!(
            registry
                .call(Mode::Build, name, args, context(CancellationToken::new()))
                .await
                .is_err()
        );
    }
    assert_eq!(driver.calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn timeout_and_abandoned_call_remove_container_before_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let driver = Arc::new(Driver {
        timeout: true,
        ..Driver::default()
    });
    let registry = registry(temp.path(), driver.clone()).await;
    let result = registry
        .call(
            Mode::Build,
            "bash",
            json!({"command":"sleep forever"}),
            context(CancellationToken::new()),
        )
        .await
        .unwrap();
    assert_eq!(result["status"], "timed_out");
    assert_eq!(result["exitCode"], 124);
    assert_eq!(driver.calls.lock().unwrap().last().unwrap()[0], "rm");
    let slow = Arc::new(Driver::default());
    slow.wait.store(true, Ordering::SeqCst);
    let registry = ExecutionRegistry::detect(
        SafeWorkspace::new(temp.path()).unwrap(),
        DockerConfig::default(),
        slow.clone(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let task = tokio::spawn(async move {
        registry
            .call(
                Mode::Build,
                "bash",
                json!({"command":"sleep forever"}),
                context(CancellationToken::new()),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if slow.calls.lock().unwrap().iter().any(|a| a[0] == "run") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if slow.calls.lock().unwrap().iter().any(|a| a[0] == "rm") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn failed_container_cleanup_preserves_mount_and_reports_failure() {
    let temp = tempfile::tempdir().unwrap();
    let driver = Arc::new(Driver {
        cleanup_fail: true,
        ..Driver::default()
    });
    let registry = registry(temp.path(), driver.clone()).await;
    assert!(matches!(
        registry
            .call(
                Mode::Build,
                "bash",
                json!({"command":"fixture"}),
                context(CancellationToken::new())
            )
            .await,
        Err(Error::Sandbox(SandboxError::Cleanup))
    ));
    let mount = driver
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|a| a[0] == "run")
        .unwrap()
        .iter()
        .find(|a| a.starts_with("type=bind,"))
        .unwrap()
        .clone();
    let source = Path::new(
        mount
            .strip_prefix("type=bind,source=")
            .unwrap()
            .split(",target=")
            .next()
            .unwrap(),
    );
    assert!(source.is_dir());
    // O driver é simulado; somente o temporário criado pelo teste é removido.
    let checked = std::fs::canonicalize(source).unwrap();
    assert_eq!(
        checked.parent().unwrap(),
        std::fs::canonicalize(std::env::temp_dir()).unwrap()
    );
    assert!(
        checked
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("myagent-snapshot-")
    );
    std::fs::remove_dir_all(checked).unwrap();
}

#[tokio::test]
async fn interactive_bash_requires_approval_even_through_direct_call_or_execute() {
    use myagent_protocol::StreamEvent;
    use myagent_tools::{ApprovalDecision, ApprovalManager};
    use tokio::sync::mpsc;
    let temp = tempfile::tempdir().unwrap();
    let driver = Arc::new(Driver::default());
    let approvals = ApprovalManager::default();
    let registry = registry(temp.path(), driver.clone())
        .await
        .with_approvals(approvals.clone());
    assert!(
        registry
            .call(
                Mode::Build,
                "bash",
                json!({"command":"echo fixture"}),
                context(CancellationToken::new())
            )
            .await
            .is_err()
    );
    let bash = registry
        .tools(Mode::Build)
        .into_iter()
        .find(|tool| tool.spec().name == "bash")
        .unwrap();
    assert!(
        bash.execute(
            json!({"command":"echo fixture"})
                .as_object()
                .unwrap()
                .clone(),
            context(CancellationToken::new())
        )
        .await
        .is_err()
    );
    assert!(
        !driver
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|args| args[0] == "run")
    );
    let (tx, mut rx) = mpsc::channel(4);
    let mut ctx = context(CancellationToken::new());
    ctx.events = Some(tx);
    let (result, ()) = tokio::join!(
        registry.call(Mode::Build, "bash", json!({"command":"echo fixture"}), ctx),
        async {
            let StreamEvent::ApprovalRequired {
                approval_id, args, ..
            } = rx.recv().await.unwrap()
            else {
                panic!("approval expected")
            };
            assert_eq!(args["command"], "echo fixture");
            assert!(approvals.decide("session", &approval_id, ApprovalDecision::Once));
        }
    );
    result.unwrap();
    assert!(
        driver
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|args| args[0] == "run")
    );
}
