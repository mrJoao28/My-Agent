#![forbid(unsafe_code)]
use myagent_core::{CancellationToken, Effect, Mode, ToolContext};
use myagent_tools::{Error, SafeWorkspace, ToolRegistry};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    tools: ToolRegistry,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("myagent m3 fixture ")
            .tempdir()
            .unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let tools = ToolRegistry::new(SafeWorkspace::new(&root).unwrap(), true);
        Self {
            _temp: temp,
            root,
            tools,
        }
    }
    fn write(&self, path: &str, text: impl AsRef<[u8]>) {
        let p = self.root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    async fn call(&self, name: &str, args: Value) -> myagent_tools::Result<Value> {
        self.tools.call(name, args, context("session-a")).await
    }
}
fn context(id: &str) -> ToolContext {
    ToolContext {
        session_id: id.into(),
        cancellation: CancellationToken::new(),
        tool_call_id: "fixture-call".into(),
        events: None,
    }
}
#[test]
fn safe_paths_cover_traversal_absent_ancestors_and_sensitive_names() {
    let f = Fixture::new();
    f.write("..valid/file.ts", "safe");
    f.write("file.txt", "safe");
    let w = SafeWorkspace::new(&f.root).unwrap();
    assert_eq!(
        w.resolve_safe_path("..valid/file.ts", true).unwrap(),
        f.root.join("..valid/file.ts")
    );
    assert_eq!(
        w.resolve_safe_path("new/deep/file.ts", false).unwrap(),
        f.root.join("new/deep/file.ts")
    );
    assert!(matches!(
        w.resolve_safe_path("missing", true),
        Err(Error::NotFound)
    ));
    for p in [
        "../outside",
        "a/../../escape",
        ".env",
        ".ENV.local",
        ".envrc",
        ".AWS/config",
        ".ssh/id_ed25519",
        ".myagent/api-token",
        ".docker/config.json",
        ".git/config",
        "a/private.pem",
        ".npmrc",
        "credentials.json",
        "secret.backup",
        "file:stream",
        "NUL.txt",
        "file.",
        "file ",
        "config.env",
        "chatgpt-credentials.json",
        "old.db",
        "trace.log",
        "C:relative",
        "D:/other/file",
        "//server/share/file",
        "\\\\server\\share\\file",
        "file.txt/missing/deep",
    ] {
        assert!(
            matches!(w.resolve_safe_path(p, false), Err(Error::PathBlocked)),
            "{p}"
        );
    }
    assert!(
        w.resolve_safe_path(f.root.parent().unwrap().to_str().unwrap(), false)
            .is_err()
    );
    assert!(w.resolve_safe_path(".env.example", false).is_ok());
    assert!(SafeWorkspace::new(Path::new(".")).is_err());
    assert!(SafeWorkspace::new(&f.root.join("file.txt")).is_err());
}
#[test]
fn hard_links_are_rejected_without_reading_contents() {
    let f = Fixture::new();
    f.write("source.txt", "fictitious");
    fs::hard_link(f.root.join("source.txt"), f.root.join("alias.txt")).unwrap();
    let w = SafeWorkspace::new(&f.root).unwrap();
    assert!(matches!(
        w.resolve_safe_path("alias.txt", true),
        Err(Error::PathBlocked)
    ));
}
#[cfg(unix)]
#[test]
fn symlinks_inside_outside_and_missing_descendants_are_blocked() {
    let f = Fixture::new();
    f.write("real/file.ts", "safe");
    let outside = tempfile::tempdir().unwrap();
    for (name, target) in [
        ("inside", f.root.join("real")),
        ("outside", outside.path().to_owned()),
    ] {
        std::os::unix::fs::symlink(target, f.root.join(name)).unwrap();
        let w = SafeWorkspace::new(&f.root).unwrap();
        assert!(matches!(
            w.resolve_safe_path(&format!("{name}/new/deep/file"), false),
            Err(Error::PathBlocked)
        ));
        assert!(SafeWorkspace::new(&f.root.join(name)).is_err());
    }
}
#[cfg(windows)]
#[test]
fn junctions_inside_outside_and_absent_descendants_are_blocked() {
    let f = Fixture::new();
    f.write("real/file.ts", "safe");
    let outside = tempfile::Builder::new()
        .prefix("myagent outside fixture ")
        .tempdir()
        .unwrap();
    for (name, target) in [
        ("inside", f.root.join("real")),
        ("outside", outside.path().to_owned()),
    ] {
        let alias = f.root.join(name);
        let output = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&alias)
            .arg(target)
            .output()
            .unwrap();
        assert!(output.status.success());
        let w = SafeWorkspace::new(&f.root).unwrap();
        assert!(matches!(
            w.resolve_safe_path(&format!("{name}/new/deep/file"), false),
            Err(Error::PathBlocked)
        ));
        assert!(SafeWorkspace::new(&alias).is_err());
        fs::remove_dir(alias).unwrap();
    }
}
#[cfg(windows)]
#[test]
fn windows_absolute_paths_drive_casing_and_volume_roots() {
    let f = Fixture::new();
    f.write("safe.txt", "safe");
    let w = SafeWorkspace::new(&f.root).unwrap();
    let ordinary = f
        .root
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_string();
    let absolute = format!("{ordinary}\\safe.txt");
    assert!(w.resolve_safe_path(&absolute, true).is_ok());
    let mut casing = absolute.clone();
    casing.replace_range(0..1, &absolute[0..1].to_ascii_lowercase());
    assert!(w.resolve_safe_path(&casing, true).is_ok());
    let volume = PathBuf::from(format!("{}\\", &ordinary[..2]));
    let root = SafeWorkspace::new(&volume).unwrap();
    assert_eq!(
        root.resolve_safe_path(".", true).unwrap(),
        fs::canonicalize(volume).unwrap()
    );
    assert!(
        w.resolve_safe_path(&format!("{ordinary}-sibling/file"), false)
            .is_err()
    );
}
#[tokio::test]
async fn registry_has_faithful_schemas_and_plan_never_mutates_project() {
    let f = Fixture::new();
    for mode in [Mode::Plan, Mode::Build] {
        let tools = f.tools.tools(mode);
        assert_eq!(tools.len(), 17);
        for tool in tools {
            let spec = tool.spec();
            assert!(matches!(spec.effect, Effect::Read | Effect::SessionState));
            assert_eq!(spec.parameters["additionalProperties"], false);
            assert!(!spec.description.is_empty());
        }
    }
    for (name, args) in [
        ("read_file", json!({"path":"missing","limit":0})),
        ("list_directory", json!({"limit":501})),
        ("grep", json!({"pattern":"x","context":21})),
        ("git_log", json!({"limit":101})),
        ("git_blame", json!({"path":"a","limit":201})),
        ("read_files", json!({"files":[]})),
        (
            "todo_write",
            json!({"todos":[{"content":"x","status":"invalid"}]}),
        ),
        ("environment_check", json!({"extra":true})),
    ] {
        assert!(
            matches!(f.call(name, args).await, Err(Error::InvalidInput)),
            "{name}"
        );
    }
    assert!(matches!(
        f.call("bash", json!({})).await,
        Err(Error::Unavailable)
    ));
}
#[tokio::test]
async fn read_pages_preserve_complete_unicode_lines_and_absolute_numbers() {
    let f = Fixture::new();
    let raw = (0..400)
        .map(|i| format!("line {i} {}", "🦀".repeat(20)))
        .collect::<Vec<_>>();
    f.write("sample.txt", raw.join("\r\n"));
    let mut offset = 0;
    let mut received = vec![];
    loop {
        let page = f
            .call(
                "read_file",
                json!({"path":"sample.txt","offset":offset,"numbered":false}),
            )
            .await
            .unwrap();
        assert!(page["content"].as_str().unwrap().encode_utf16().count() <= 10000);
        received.extend(
            page["content"]
                .as_str()
                .unwrap()
                .split('\n')
                .map(str::to_owned),
        );
        if page["truncated"] == false {
            break;
        }
        let next = page["nextOffset"].as_u64().unwrap();
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(received, raw);
    let page = f
        .call(
            "read_file",
            json!({"path":"sample.txt","offset":2,"limit":1}),
        )
        .await
        .unwrap();
    assert!(page["content"].as_str().unwrap().starts_with("3: line 2"));
    assert_eq!(page["nextOffset"], 3);
    f.write("empty.txt", "");
    let empty = f
        .call("read_file", json!({"path":"empty.txt"}))
        .await
        .unwrap();
    assert_eq!(empty["totalLines"], 0);
    assert_eq!(empty["content"], "");
    f.write("long.txt", "x".repeat(10001));
    assert!(matches!(
        f.call("read_file", json!({"path":"long.txt"})).await,
        Err(Error::LineTooLong)
    ));
}
#[tokio::test]
async fn read_size_binary_and_batch_limits_are_enforced() {
    let f = Fixture::new();
    f.write("huge.txt", vec![b'x'; 5 * 1024 * 1024 + 1]);
    assert!(matches!(
        f.call("read_file", json!({"path":"huge.txt","limit":1}))
            .await,
        Err(Error::FileTooLarge)
    ));
    f.write("binary.bin", [0, 1, 2]);
    f.write("large.txt", "x".repeat(20000));
    let batch=f.call("read_files",json!({"files":[{"path":"large.txt"},{"path":"large.txt"},{"path":"large.txt"},{"path":"large.txt"}]})).await.unwrap();
    assert_eq!(batch["files"][3]["error"], "BATCH_LIMIT");
    assert_eq!(batch["truncated"], true);
    let batch=f.call("read_files",json!({"files":[{"path":"binary.bin"},{"path":".env"},{"path":"missing"},{"path":"huge.txt"},{"path":"large.txt"}]})).await.unwrap();
    assert_eq!(batch["files"][0]["error"], "BINARY_FILE");
    assert_eq!(batch["files"][1]["error"], "PATH_BLOCKED");
    assert_eq!(batch["files"][2]["error"], "NOT_FOUND");
    assert_eq!(batch["files"][3]["error"], "FILE_TOO_LARGE");
    assert_eq!(batch["files"][4]["error"], "BATCH_LIMIT");
}
#[tokio::test]
async fn directory_pages_cover_safe_entries_and_skip_hard_links() {
    let f = Fixture::new();
    for i in 0..5 {
        f.write(&format!("{i}.txt"), "safe");
    }
    f.write("real.txt", "fictitious");
    fs::hard_link(f.root.join("real.txt"), f.root.join("alias.txt")).unwrap();
    let mut offset = 0;
    let mut names = vec![];
    loop {
        let page = f
            .call("list_directory", json!({"offset":offset,"limit":2}))
            .await
            .unwrap();
        for e in page["entries"].as_array().unwrap() {
            names.push(e["name"].as_str().unwrap().to_owned());
        }
        if page["nextOffset"].is_null() {
            break;
        }
        offset = page["nextOffset"].as_u64().unwrap();
    }
    names.sort();
    assert_eq!(names, vec!["0.txt", "1.txt", "2.txt", "3.txt", "4.txt"]);
}
#[tokio::test]
async fn search_honors_root_gitignore_and_literal_glob_metacharacters() {
    let f = Fixture::new();
    f.write(".gitignore","# rules\r\n/cache/\r\ngenerated/\r\n!generated/keep.ts\r\n*.tmp\r\n!keep.tmp\r\nsrc/private/*.ts\r\n!src/private/keep.ts\r\nliteral{a,b}.ts\r\n[ab].ignored.ts\r\n");
    for p in [
        "cache/a.ts",
        "nested/cache/a.ts",
        "generated/keep.ts",
        "drop.tmp",
        "keep.tmp",
        "src/a.ts",
        "src/private/drop.ts",
        "src/private/keep.ts",
        "literal{a,b}.ts",
        "literala.ts",
        "a.ignored.ts",
        "node_modules/allowed.ts",
    ] {
        f.write(p, "needle");
    }
    let expected = json!([
        "keep.tmp",
        "literala.ts",
        "nested/cache/a.ts",
        "src/a.ts",
        "src/private/keep.ts"
    ]);
    assert_eq!(
        f.call("grep", json!({"pattern":"needle","files_only":true}))
            .await
            .unwrap()["files"],
        expected
    );
    assert_eq!(
        f.call("glob", json!({"pattern":"**/*.ts","path":"src"}))
            .await
            .unwrap()["matches"],
        json!(["a.ts", "private/keep.ts"])
    );
    assert_eq!(
        f.call("glob", json!({"pattern":"**/*","path":"generated"}))
            .await
            .unwrap()["matches"],
        json!([])
    );
    f.write("file[1].ts", "x");
    assert_eq!(
        f.call("glob", json!({"pattern":"file[1].ts"}))
            .await
            .unwrap()["matches"],
        json!(["file[1].ts"])
    );
}
#[tokio::test]
async fn search_bounds_gitignore_rules_bytes_and_rejects_hardlinks() {
    let f = Fixture::new();
    f.write(".gitignore", "#".repeat(65537));
    assert!(matches!(
        f.call("glob", json!({"pattern":"*"})).await,
        Err(Error::FileTooLarge)
    ));
    f.write(
        ".gitignore",
        (0..2001).map(|n| format!("entry{n}\n")).collect::<String>(),
    );
    assert!(matches!(
        f.call("grep", json!({"pattern":"a"})).await,
        Err(Error::IgnoreTooLarge)
    ));
    fs::hard_link(f.root.join(".gitignore"), f.root.join("rules.txt")).unwrap();
    assert!(matches!(
        f.call("glob", json!({"pattern":"*"})).await,
        Err(Error::PathBlocked)
    ));
}
#[tokio::test]
async fn grep_context_include_invalid_regex_and_output_limit() {
    let f = Fixture::new();
    f.write(
        "src/a.ts",
        "first\r\n  before\r\nneedle\r\n  after\r\nneedle",
    );
    f.write("src/b.txt", "needle");
    let r = f
        .call(
            "grep",
            json!({"pattern":"needle","include":"*.ts","context":1}),
        )
        .await
        .unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 2);
    assert_eq!(
        r["matches"][0]["before"],
        json!([{"line":2,"text":"  before"}])
    );
    assert_eq!(
        f.call(
            "grep",
            json!({"pattern":"needle","path":"src","include":"*.txt","files_only":true})
        )
        .await
        .unwrap()["files"],
        json!(["src/b.txt"])
    );
    assert!(matches!(
        f.call("grep", json!({"pattern":"["})).await,
        Err(Error::InvalidPattern)
    ));
    f.write("literal.txt", "[a]");
    assert_eq!(
        f.call(
            "grep",
            json!({"pattern":"[a]","literal":true,"files_only":true})
        )
        .await
        .unwrap()["files"],
        json!(["literal.txt"])
    );
    f.write(
        "many.ts",
        (0..300)
            .map(|_| format!("needle {}\n", "x".repeat(400)))
            .collect::<String>(),
    );
    let r = f
        .call(
            "grep",
            json!({"pattern":"needle","path":"many.ts","context":20}),
        )
        .await
        .unwrap();
    assert_eq!(r["truncated"], true);
    assert!(r.to_string().encode_utf16().count() < 61_000);
    f.write("pathological.txt", format!("{}!", "a".repeat(100000)));
    let start = std::time::Instant::now();
    f.call(
        "grep",
        json!({"pattern":"(a+)+$","path":"pathological.txt"}),
    )
    .await
    .unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(15));
}
#[tokio::test]
async fn all_tools_honor_pre_cancellation_before_io() {
    let f = Fixture::new();
    for name in myagent_tools::TOOL_NAMES {
        let mut ctx = context("cancelled");
        ctx.cancellation.cancel();
        let args = match *name {
            "read_file" | "file_outline" | "git_blame" => json!({"path":"missing.ts"}),
            "read_files" => json!({"files":[{"path":"missing"}]}),
            "glob" | "grep" => json!({"pattern":"a"}),
            "symbol_search" => json!({"query":"a"}),
            "todo_write" => json!({"todos":[]}),
            _ => json!({}),
        };
        assert!(
            matches!(
                f.tools.call(name, args, ctx.clone()).await,
                Err(Error::Cancelled)
            ),
            "{name}"
        );
        ctx.cancellation = CancellationToken::new();
    }
}
#[tokio::test]
async fn ast_finds_declarations_in_all_supported_dialects_not_comments_or_references() {
    let f = Fixture::new();
    f.write("source.ts","export function greeting() { return 'function imaginary() {}'; }\nexport const greetValue = 1;\nclass WorkerClass { greetMethod() {} }\n// function commentOnly() {}\ngreeting();");
    let r = f
        .call("file_outline", json!({"path":"source.ts"}))
        .await
        .unwrap();
    let names: Vec<_> = r["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["greeting", "greetValue", "WorkerClass", "greetMethod"]
    );
    assert_eq!(r["symbols"][0]["exported"], true);
    assert_eq!(r["symbols"][1]["exported"], true);
    assert_eq!(
        f.call("symbol_search", json!({"query":"GREET","path":"source.ts"}))
            .await
            .unwrap()["matches"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        f.call(
            "symbol_search",
            json!({"query":"GREET","caseSensitive":true})
        )
        .await
        .unwrap()["matches"],
        json!([])
    );
    for extension in ["js", "jsx", "tsx", "mjs", "cjs", "mts", "cts"] {
        let path = format!("dialect.{extension}");
        f.write(&path, "export const Component = () => 1;");
        assert_eq!(
            f.call("file_outline", json!({"path":path})).await.unwrap()["symbols"][0]["name"],
            "Component"
        );
    }
    f.write(
        "many.ts",
        (0..220)
            .map(|n| format!("const item{n} = {n};\n"))
            .collect::<String>(),
    );
    let r = f
        .call("file_outline", json!({"path":"many.ts"}))
        .await
        .unwrap();
    assert_eq!(r["symbols"].as_array().unwrap().len(), 200);
    assert_eq!(r["truncated"], true);
    let r = f
        .call("symbol_search", json!({"query":"item"}))
        .await
        .unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 100);
    assert_eq!(r["truncated"], true);
    assert!(matches!(
        f.call("file_outline", json!({"path":"source.py"})).await,
        Err(Error::UnsupportedLanguage)
    ));
}
#[tokio::test]
async fn inspection_and_discovery_never_execute_manifest_commands() {
    let f = Fixture::new();
    f.write("package.json",json!({"scripts":{"test":"MUST_NOT_EXECUTE","test:unit":"MUST_NOT_EXECUTE","build":"MUST_NOT_EXECUTE"},"devDependencies":{"vitest":"fictional"}}).to_string());
    f.write("nested/package.json", "invalid fixture");
    f.write("vitest.config.ts", "throw new Error('MUST_NOT_EXECUTE')");
    for path in [
        "src/a.test.ts",
        "src/test_unit.py",
        "src/a_test.go",
        "node_modules/hidden.test.ts",
    ] {
        f.write(path, "fixture");
    }
    let r = f.call("project_inspect", json!({})).await.unwrap();
    assert!(!r.to_string().contains("MUST_NOT_EXECUTE"));
    assert_eq!(
        r["errors"],
        json!([{"path":"nested/package.json","error":"INVALID_PACKAGE_JSON"}])
    );
    assert_eq!(r["packages"][0]["dependencies"], json!(["vitest"]));
    let r = f.call("test_discover", json!({})).await.unwrap();
    assert_eq!(
        r["tests"],
        json!(["src/a.test.ts", "src/a_test.go", "src/test_unit.py"])
    );
    assert_eq!(r["packages"][0]["runners"], json!(["vitest"]));
    let r = f.call("environment_check", json!({})).await.unwrap();
    assert_eq!(r["sandbox"]["configured"], true);
    assert_eq!(r["sandbox"]["verified"], false);
    assert_eq!(r["sandbox"]["hostExecution"], false);
}
#[tokio::test]
async fn inspection_limits_and_todo_session_isolation() {
    let f = Fixture::new();
    f.write("package.json",json!({"scripts":(0..120).map(|n|(format!("test:{n}"),json!("fictional"))).collect::<serde_json::Map<String,Value>>()} ).to_string());
    let r = f.call("project_inspect", json!({})).await.unwrap();
    assert_eq!(r["packages"][0]["scripts"].as_array().unwrap().len(), 100);
    assert_eq!(r["truncated"], true);
    let r = f
        .call(
            "todo_write",
            json!({"todos":[{"content":"inspect","status":"in_progress"}]}),
        )
        .await
        .unwrap();
    assert_eq!(r["inProgress"], 1);
    assert_eq!(
        f.tools
            .session_todos("session-a")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(f.tools.session_todos("other").unwrap(), json!([]));
    f.tools.clear_session("session-a").unwrap();
    assert_eq!(f.tools.session_todos("session-a").unwrap(), json!([]));
}

// Trusted fixture setup only; production registry never exposes these commands.
fn fixture_git(root: &Path, args: &[&str], stdin: Option<&[u8]>) -> String {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(data) = stdin {
        child.stdin.take().unwrap().write_all(data).unwrap();
    } else {
        drop(child.stdin.take());
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "fixture Git failed");
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn seed_git(f: &Fixture) {
    fixture_git(&f.root, &["init", "-b", "main"], None);
    f.write("a.ts", "first\nsecond\n");
    f.write("b.ts", "other\n");
    for (name, content) in [("a.ts", "first\nsecond\n"), ("b.ts", "other\n")] {
        let hash = fixture_git(
            &f.root,
            &["hash-object", "-w", "--stdin"],
            Some(content.as_bytes()),
        );
        fixture_git(
            &f.root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{hash},{name}"),
            ],
            None,
        );
    }
    let tree = fixture_git(&f.root, &["write-tree"], None);
    let commit = format!(
        "tree {tree}\nauthor Fixture <fixture@example.invalid> 1700000000 +0000\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\n\ninitial fixture\n"
    );
    let hash = fixture_git(
        &f.root,
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        Some(commit.as_bytes()),
    );
    f.write(".git/refs/heads/main", format!("{hash}\n"));
}
#[tokio::test]
async fn protected_git_inspects_real_fixture_without_mutation() {
    let f = Fixture::new();
    seed_git(&f);
    f.write("a.ts", "first\nchanged\nthird\n");
    f.write("untracked.txt", "safe");
    let status = f.call("git_status", json!({})).await.unwrap();
    assert_eq!(status["branch"], "main");
    assert_eq!(status["clean"], false);
    assert!(
        status["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"] == "untracked.txt")
    );
    assert!(
        f.call("git_diff", json!({})).await.unwrap()["diff"]
            .as_str()
            .unwrap()
            .contains("+changed")
    );
    assert_eq!(
        f.call("git_diff", json!({"staged":true})).await.unwrap()["diff"],
        "(sem diferenças)"
    );
    f.write("b.ts", "staged\n");
    let staged_hash = fixture_git(
        &f.root,
        &["hash-object", "-w", "--stdin"],
        Some(b"staged\n"),
    );
    fixture_git(
        &f.root,
        &[
            "update-index",
            "--cacheinfo",
            &format!("100644,{staged_hash},b.ts"),
        ],
        None,
    );
    let staged = f
        .call("diff_summary", json!({"staged":true}))
        .await
        .unwrap();
    assert_eq!(staged["files"][0]["path"], "b.ts");
    assert_eq!(staged["additions"], 1);
    assert_eq!(staged["deletions"], 1);
    let summary = f.call("diff_summary", json!({})).await.unwrap();
    assert_eq!(summary["totalFiles"], 1);
    assert_eq!(summary["additions"], 2);
    assert_eq!(summary["deletions"], 1);
    assert_eq!(
        f.call("git_log", json!({})).await.unwrap()["commits"][0]["message"],
        "initial fixture"
    );
    assert!(
        f.call("git_show", json!({})).await.unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("initial fixture")
    );
    let blame = f
        .call("git_blame", json!({"path":"a.ts","limit":200}))
        .await
        .unwrap();
    assert_eq!(blame["totalLines"], 2);
    assert_eq!(blame["endLine"], 2);
    assert!(blame["text"].as_str().unwrap().contains("author Fixture"));
    assert!(matches!(
        f.call("git_blame", json!({"path":"a.ts","startLine":3}))
            .await,
        Err(Error::LineRange)
    ));
    assert!(matches!(
        f.call("git_show", json!({"revision":"--help"})).await,
        Err(Error::Revision)
    ));
    assert!(matches!(
        f.call("git_diff", json!({"path":".env"})).await,
        Err(Error::PathBlocked)
    ));
    assert_eq!(
        fixture_git(&f.root, &["diff", "--cached", "--name-only"], None),
        "b.ts"
    );
}
#[tokio::test]
async fn git_blocks_external_metadata_config_and_blame_files() {
    let f = Fixture::new();
    seed_git(&f);
    fixture_git(
        &f.root,
        &["config", "blame.ignoreRevsFile", "outside"],
        None,
    );
    assert!(matches!(
        f.call("git_blame", json!({"path":"a.ts"})).await,
        Err(Error::GitUnsafe)
    ));
    fixture_git(
        &f.root,
        &["config", "--unset-all", "blame.ignoreRevsFile"],
        None,
    );
    fixture_git(&f.root, &["config", "include.path", "outside"], None);
    assert!(matches!(
        f.call("git_log", json!({})).await,
        Err(Error::GitUnsafe)
    ));
    let g = Fixture::new();
    seed_git(&g);
    g.write(".git/objects/info/alternates", "outside");
    assert!(matches!(
        g.call("git_log", json!({})).await,
        Err(Error::GitUnsafe)
    ));
}
#[tokio::test]
async fn git_disables_external_diff_textconv_filters_and_mailmap() {
    let f = Fixture::new();
    seed_git(&f);
    f.write("a.ts", "changed\n");
    f.write(".gitattributes", "*.ts diff=malicious filter=malicious\n");
    for (key, value) in [
        ("diff.malicious.command", "MUST_NOT_EXECUTE"),
        ("diff.malicious.textconv", "MUST_NOT_EXECUTE"),
        ("filter.malicious.clean", "MUST_NOT_EXECUTE"),
        ("filter.malicious.required", "true"),
        ("core.fsmonitor", "MUST_NOT_EXECUTE"),
        ("mailmap.file", "MUST_NOT_READ"),
    ] {
        fixture_git(&f.root, &["config", key, value], None);
    }
    let r = f.call("git_diff", json!({})).await.unwrap();
    assert!(r["diff"].as_str().unwrap().contains("+changed"));
    assert!(f.call("git_show", json!({})).await.is_ok());
    assert!(f.call("git_blame", json!({"path":"a.ts"})).await.is_ok());
}

#[tokio::test]
async fn search_and_inspection_report_numeric_caps() {
    let f = Fixture::new();
    for i in 0..501 {
        f.write(&format!("files/{i:04}.ts"), "needle\n");
    }
    let r = f.call("glob", json!({"pattern":"**/*.ts"})).await.unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 500);
    assert_eq!(r["truncated"], true);
    let r = f.call("grep", json!({"pattern":"needle"})).await.unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 200);
    assert_eq!(r["truncated"], true);
    let r = f.call("project_inspect", json!({})).await.unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 100);
    assert_eq!(r["truncated"], true);
    f.write("large.ts", "x".repeat(262145));
    assert!(matches!(
        f.call("file_outline", json!({"path":"large.ts"})).await,
        Err(Error::FileTooLarge)
    ));
}
#[tokio::test]
async fn grep_total_read_budget_and_symbol_byte_budget_are_enforced() {
    let f = Fixture::new();
    for i in 0..21 {
        f.write(&format!("text/{i:02}.txt"), "x".repeat(1_000_000));
    }
    let r = f
        .call("grep", json!({"pattern":"absent","path":"text"}))
        .await
        .unwrap();
    assert_eq!(r["truncated"], true);
    assert_eq!(r["matches"], json!([]));
    let ending = "*/\nconst symbol = 1;";
    let content = format!("/*{}{}", "x".repeat(262_144 - 2 - ending.len()), ending);
    assert_eq!(content.len(), 262144);
    for i in 0..9 {
        f.write(&format!("source/{i}.ts"), &content);
    }
    let r = f
        .call("symbol_search", json!({"query":"absent","path":"source"}))
        .await
        .unwrap();
    assert_eq!(r["inspected"], 8);
    assert_eq!(r["truncated"], true);
}
#[tokio::test]
async fn search_cannot_reinclude_a_child_of_an_excluded_parent() {
    let f = Fixture::new();
    f.write(
        ".gitignore",
        "blocked/\n!blocked/keep.ts\npartial**/file.ts\n",
    );
    f.write("blocked/keep.ts", "needle");
    f.write("partial/deep/file.ts", "needle");
    f.write("partialx/file.ts", "needle");
    assert_eq!(
        f.call(
            "grep",
            json!({"pattern":"needle","path":"blocked/keep.ts","files_only":true})
        )
        .await
        .unwrap()["files"],
        json!([])
    );
    assert_eq!(
        f.call("glob", json!({"pattern":"**/*.ts"})).await.unwrap()["matches"],
        json!(["partial/deep/file.ts"])
    );
}
#[tokio::test]
async fn active_search_is_cancelled_and_never_returns_partial_success() {
    let f = Fixture::new();
    for i in 0..30 {
        f.write(&format!("{i:02}.txt"), "a".repeat(900_000));
    }
    let ctx = context("active");
    let cancel = ctx.cancellation.clone();
    let registry = f.tools.clone();
    let task = tokio::spawn(async move {
        registry
            .call("grep", json!({"pattern":"(a+)+$"}), ctx)
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    cancel.cancel();
    assert!(matches!(task.await.unwrap(), Err(Error::Cancelled)));
}

#[tokio::test]
async fn active_outline_is_cancelled_with_no_partial_success() {
    let f = Fixture::new();
    f.write(
        "complex.ts",
        "function example(){ return 1; }\n".repeat(7000),
    );
    let ctx = context("outline");
    let cancel = ctx.cancellation.clone();
    let registry = f.tools.clone();
    let task = tokio::spawn(async move {
        registry
            .call("file_outline", json!({"path":"complex.ts"}), ctx)
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    cancel.cancel();
    assert!(matches!(task.await.unwrap(), Err(Error::Cancelled)));
}
#[tokio::test]
async fn typescript_ast_covers_types_members_and_excludes_accessors_and_constructor() {
    let f = Fixture::new();
    f.write("types.ts","export interface Shape { width: number; draw(): void }\nexport type Alias = Shape;\nexport enum Kind { One }\nclass Box { field = 1; constructor() {} get value() { return 1; } set value(v: number) {} method() {} }");
    let r = f
        .call("file_outline", json!({"path":"types.ts"}))
        .await
        .unwrap();
    let names: Vec<_> = r["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Shape", "width", "draw", "Alias", "Kind", "Box", "field", "method"
        ]
    );
}

#[test]
fn workspace_authorization_is_bound_to_trusted_roots() {
    let f = Fixture::new();
    f.write("child/file.txt", "safe");
    let ordinary = PathBuf::from(f.root.to_string_lossy().trim_start_matches("\\\\?\\"));
    assert!(SafeWorkspace::authorize(&f.root.join("child"), std::slice::from_ref(&f.root)).is_ok());
    assert!(
        SafeWorkspace::authorize(&ordinary.join("child"), std::slice::from_ref(&ordinary)).is_ok()
    );
    assert!(SafeWorkspace::authorize(Path::new("."), std::slice::from_ref(&ordinary)).is_err());
    assert!(
        SafeWorkspace::authorize(
            &ordinary.join("child/file.txt"),
            std::slice::from_ref(&ordinary)
        )
        .is_err()
    );
    let outside = tempfile::tempdir().unwrap();
    assert!(SafeWorkspace::authorize(outside.path(), &[ordinary]).is_err());
}
