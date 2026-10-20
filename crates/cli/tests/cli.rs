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
