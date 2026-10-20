#![forbid(unsafe_code)]
use myagent_sandbox::{ProcessError, supervise_child};
use std::{process::Stdio, time::Duration};
use tokio_util::sync::CancellationToken;
#[test]
fn child_fixture() {
    if std::env::var_os("MYAGENT_TEST_SUPERVISED_CHILD").is_none() {
        return;
    }
    if std::env::var_os("MYAGENT_TEST_BOTH").is_some() {
        println!("{}", "o".repeat(600));
        eprintln!("{}", "e".repeat(600));
    }
    if std::env::var_os("MYAGENT_TEST_FLOOD").is_some() {
        for _ in 0..10000 {
            println!("{}", "x".repeat(1024));
        }
    }
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
fn child(flood: bool) -> tokio::process::Child {
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "child_fixture", "--nocapture"])
        .env("MYAGENT_TEST_SUPERVISED_CHILD", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if flood {
        command.env("MYAGENT_TEST_FLOOD", "1");
    }
    command.spawn().unwrap()
}
#[tokio::test]
async fn cancellation_kills_and_reaps_a_running_process() {
    let cancel = CancellationToken::new();
    let cloned = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cloned.cancel();
    });
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        supervise_child(child(false), cancel, 4096),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap_err(), ProcessError::Cancelled);
}
#[tokio::test]
async fn output_limit_is_enforced_while_reading_and_terminates_the_child() {
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        supervise_child(child(true), CancellationToken::new(), 1024),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap_err(), ProcessError::Limit);
}

#[tokio::test]
async fn stdout_and_stderr_share_one_budget_while_the_child_is_still_alive() {
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_fixture", "--nocapture"])
        .env("MYAGENT_TEST_SUPERVISED_CHILD", "1")
        .env("MYAGENT_TEST_BOTH", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        supervise_child(child, CancellationToken::new(), 1024),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap_err(), ProcessError::Limit);
}
