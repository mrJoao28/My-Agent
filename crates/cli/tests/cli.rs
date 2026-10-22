#![forbid(unsafe_code)]

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_myagent"))
        .args(args)
        .output()
        .expect("failed to execute myagent")
}

#[test]
fn help_is_available_from_the_public_binary() {
    let cases: &[&[&str]] = &[&[], &["--help"], &["-h"]];
    for args in cases {
        let output = run(args);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).expect("help must be UTF-8");
        assert!(text.contains("myagent"));
        assert!(text.contains("programação de terminal"));
        assert!(text.contains("--help"));
        assert!(text.contains("--version"));
    }
}

#[test]
fn version_matches_the_package_and_exits_successfully() {
    for flag in ["--version", "-v"] {
        let output = run(&[flag]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).expect("version must be UTF-8"),
            format!("myagent {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn invalid_or_ambiguous_arguments_are_rejected() {
    let cases: &[&[&str]] = &[&["--unknown"], &["--help", "--version"]];
    for args in cases {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .expect("error must be UTF-8")
                .contains("--help")
        );
    }
}

#[test]
fn headless_demo_preserves_plan_and_explicit_approval_exit_codes() {
    for (mode, approval, expected) in [
        ("plan", "none", 0),
        ("build", "none", 3),
        ("build", "all", 0),
    ] {
        let output = run(&[
            "--demo",
            "exec",
            "demonstrar arquivos",
            "--mode",
            mode,
            "--approve",
            approval,
            "--json",
        ]);
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let summary = events.last().unwrap();
        assert_eq!(summary["type"], "exec-result");
        assert_eq!(summary["exitCode"], expected);
        assert_eq!(summary["mode"], mode.to_uppercase());
        if mode == "plan" {
            assert_eq!(summary["toolCalls"], 0);
        }
    }
}

#[test]
fn doctor_demo_and_invalid_exec_are_machine_readable() {
    let output = run(&["--demo", "doctor", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["exitCode"], 0);
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["name"] == "sqlite")
    );
    assert_eq!(run(&["--demo", "exec", " "]).status.code(), Some(2));
    let oversized = "x".repeat(9001);
    assert_eq!(run(&["--demo", "exec", &oversized]).status.code(), Some(2));
}
