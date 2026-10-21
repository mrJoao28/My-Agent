#![forbid(unsafe_code)]
use myagent_core::{CancellationToken, Mode, ToolContext};
use myagent_sandbox::{DockerConfig, NativeDocker};
use myagent_tools::{ExecutionRegistry, SafeWorkspace};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
#[ignore = "Docker real: requer MYAGENT_TEST_DOCKER_IMAGE e execução explícita com --ignored"]
async fn docker_real_confines_files_network_identity_and_timeout() {
    let image = std::env::var("MYAGENT_TEST_DOCKER_IMAGE")
        .expect("Set MYAGENT_TEST_DOCKER_IMAGE to a trusted local Linux image with /bin/sh");
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("fixture"), b"exact\r\nbytes").unwrap();
    let registry = ExecutionRegistry::detect(
        SafeWorkspace::new(temp.path()).unwrap(),
        DockerConfig {
            image,
            ..DockerConfig::default()
        },
        Arc::new(NativeDocker),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(
        registry.available(),
        "Docker Linux daemon required; never execute on host"
    );
    let context = || ToolContext {
        session_id: "fixture".into(),
        tool_call_id: "fixture".into(),
        events: None,
        cancellation: CancellationToken::new(),
    };
    let script = r#"set -eu
test "$(id -u)" = 65534
grep -Eq '^CapEff:[[:space:]]+0+$' /proc/self/status
awk '$2 == "/" { if ($4 !~ /(^|,)ro(,|$)/) exit 1; found=1 } END { if (!found) exit 1 }' /proc/mounts
awk '$2 == "/workspace" { if ($4 !~ /(^|,)ro(,|$)/) exit 1; found=1 } END { if (!found) exit 1 }' /proc/mounts
test -z "${TAVILY_API_KEY-}${MYAGENT_TEST_SERVER_SENTINEL-}"
test ! -e /var/run/docker.sock
test ! -e /workspace/.git
test -z "$(awk 'NR>1 {print}' /proc/net/route)"
if printf changed > /workspace/fixture 2>/dev/null; then exit 1; fi
printf disposable > /tmp/fixture
printf confined
"#;
    let result = registry
        .call(Mode::Build, "bash", json!({"command":script}), context())
        .await
        .unwrap();
    assert_eq!(result["exitCode"], 0, "{result}");
    assert_eq!(result["stdout"], "confined");
    assert_eq!(
        std::fs::read(temp.path().join("fixture")).unwrap(),
        b"exact\r\nbytes"
    );
    let timeout = registry
        .call(
            Mode::Build,
            "bash",
            json!({"command":"sleep 20","timeoutMs":1000}),
            context(),
        )
        .await
        .unwrap();
    assert_eq!(timeout["timedOut"], true);
}
