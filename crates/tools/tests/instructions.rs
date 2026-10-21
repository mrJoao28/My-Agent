#![forbid(unsafe_code)]
use myagent_core::instructions::{InstructionSource, ProjectInstructions};
use myagent_tools::FilesystemInstructions;
use std::{fs, sync::Arc};

#[test]
fn system_temporary_instruction_root_is_accepted_without_following_user_aliases() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("AGENTS.md"), "temporary fixture rules").unwrap();
    let source = FilesystemInstructions::new(temp.path()).unwrap();
    assert_eq!(
        source
            .read(std::path::Path::new("AGENTS.md"))
            .unwrap()
            .as_deref(),
        Some("temporary fixture rules")
    );
}
#[test]
fn file_source_is_read_only_scoped_and_never_reads_prohibited_names() {
    let root = tempfile::Builder::new()
        .prefix("myagent instructions fixture ")
        .tempdir()
        .unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    fs::write(base.join("AGENTS.md"), "root\r\nrules 🦀").unwrap();
    fs::create_dir(base.join("src")).unwrap();
    fs::write(base.join("src/AGENTS.md"), "nested").unwrap();
    let source = FilesystemInstructions::new(&base).unwrap();
    assert!(source.read(std::path::Path::new("config.env")).is_err());
    assert!(source.read(std::path::Path::new("../AGENTS.md")).is_err());
    let mut notes = ProjectInstructions::new(Arc::new(source));
    assert!(notes.for_paths(&[]).unwrap().contains("rules"));
    assert!(
        notes
            .for_paths(&["src/lib.rs".into()])
            .unwrap()
            .contains("nested")
    );
    assert_eq!(
        fs::read_to_string(base.join("AGENTS.md")).unwrap(),
        "root\r\nrules 🦀"
    );
}
#[test]
fn hard_linked_agents_is_blocked_without_reading_its_contents() {
    let root = tempfile::Builder::new()
        .prefix("myagent instructions fixture ")
        .tempdir()
        .unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    fs::write(base.join("fixture.txt"), "fictitious").unwrap();
    fs::hard_link(base.join("fixture.txt"), base.join("AGENTS.md")).unwrap();
    assert!(
        FilesystemInstructions::new(&base)
            .unwrap()
            .read(std::path::Path::new("AGENTS.md"))
            .is_err()
    );
}
#[cfg(unix)]
#[test]
fn symlinked_instruction_ancestors_are_blocked() {
    let root = tempfile::Builder::new()
        .prefix("myagent instructions fixture ")
        .tempdir()
        .unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    fs::create_dir(base.join("real")).unwrap();
    fs::write(base.join("real/AGENTS.md"), "fictitious").unwrap();
    std::os::unix::fs::symlink(base.join("real"), base.join("linked")).unwrap();
    assert!(
        FilesystemInstructions::new(&base)
            .unwrap()
            .read(std::path::Path::new("linked/AGENTS.md"))
            .is_err()
    );
}
#[cfg(windows)]
#[test]
fn junctioned_instruction_ancestors_are_blocked() {
    let root = tempfile::Builder::new()
        .prefix("myagent instructions fixture ")
        .tempdir()
        .unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    fs::create_dir(base.join("real")).unwrap();
    fs::write(base.join("real/AGENTS.md"), "fictitious").unwrap();
    let alias = base.join("linked");
    let output = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&alias)
        .arg(base.join("real"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let denied = FilesystemInstructions::new(&base)
        .unwrap()
        .read(std::path::Path::new("linked/AGENTS.md"))
        .is_err();
    fs::remove_dir(alias).unwrap();
    assert!(denied);
}
