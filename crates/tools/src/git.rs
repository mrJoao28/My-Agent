use crate::{
    Error, Result, SafeWorkspace,
    io::{Budget, cut, units},
    registry::{flag, limit_output, number, text},
};
use myagent_sandbox::{ProcessError, supervise_child};
use serde_json::{Map, Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
const NULL: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };
fn metadata(ws: &SafeWorkspace, b: &Budget) -> Result<()> {
    let root = ws.root().join(".git");
    let meta = fs::symlink_metadata(&root).map_err(|_| Error::GitUnsafe)?;
    if !meta.is_dir() || crate::path::linked(&meta) {
        return Err(Error::GitUnsafe);
    }
    let mut stack = vec![root.clone()];
    let mut visited = 0;
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path)? {
            b.check()?;
            visited += 1;
            if visited > 20_000 {
                return Err(Error::GitUnsafe);
            }
            let p = entry?.path();
            if crate::is_sensitive_path(&p.file_name().ok_or(Error::GitUnsafe)?.to_string_lossy()) {
                return Err(Error::GitUnsafe);
            }
            let m = fs::symlink_metadata(&p)?;
            if crate::path::linked(&m)
                || crate::path::hardlinked(&p, &m)?
                || p == root.join("commondir")
                || p == root.join("objects/info/alternates")
                || p == root.join("objects/info/http-alternates")
            {
                return Err(Error::GitUnsafe);
            }
            if m.is_dir() {
                stack.push(p);
            }
        }
    }
    // Includes must be refused before Git can follow them. Never interpret values.
    let p = root.join("config");
    let file = fs::File::open(p)?;
    crate::path::check_handle(&file)?;
    use std::io::Read;
    let mut bytes = vec![];
    file.take(262_145).read_to_end(&mut bytes)?;
    if bytes.len() > 262_144 || bytes.contains(&0) {
        return Err(Error::GitUnsafe);
    }
    let config = String::from_utf8(bytes).map_err(|_| Error::GitUnsafe)?;
    for line in config.lines() {
        let trimmed = line.trim_start();
        if let Some(section) = trimmed.strip_prefix('[') {
            let compact: String = section.chars().filter(|c| !c.is_whitespace()).collect();
            if compact.to_ascii_lowercase().starts_with("include") {
                return Err(Error::GitUnsafe);
            }
        }
    }
    control_files(ws, ".", b)?;
    Ok(())
}
fn control_files(ws: &SafeWorkspace, file: &str, b: &Budget) -> Result<()> {
    let mut path = Path::new(file).parent();
    for name in [".gitattributes", ".gitignore", ".mailmap"] {
        ws.resolve_safe_path(name, false)?;
    }
    while let Some(parent) = path {
        b.check()?;
        for name in [".gitattributes", ".gitignore"] {
            let p = parent.join(name).to_string_lossy().replace('\\', "/");
            ws.resolve_safe_path(&p, false)?;
        }
        path = parent.parent();
    }
    Ok(())
}
struct Git {
    ws: SafeWorkspace,
    filters: Vec<String>,
    cancel: CancellationToken,
    executable: PathBuf,
}
fn executable(ws: &SafeWorkspace) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").ok_or(Error::Git)?;
    for directory in std::env::split_paths(&path) {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(if cfg!(windows) { "git.exe" } else { "git" });
        if let Ok(canonical) = fs::canonicalize(candidate)
            && canonical.is_file()
            && ws.relative(&canonical).is_err()
        {
            return Ok(canonical);
        }
    }
    Err(Error::Git)
}
impl Git {
    async fn new(ws: &SafeWorkspace, cancel: CancellationToken) -> Result<Self> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let owned = ws.clone();
        let token = cancel.clone();
        tokio::task::spawn_blocking(move || metadata(&owned, &Budget::new(token, 15)))
            .await
            .map_err(|_| Error::Internal)??;
        let mut git = Self {
            executable: executable(ws)?,
            ws: ws.clone(),
            filters: vec![],
            cancel,
        };
        let keys = git
            .run(
                vec![
                    "config".into(),
                    "--local".into(),
                    "--no-includes".into(),
                    "--null".into(),
                    "--name-only".into(),
                    "--list".into(),
                ],
                5,
            )
            .await?;
        for key in keys
            .split('\0')
            .filter(|s| s.to_ascii_lowercase().starts_with("filter."))
        {
            let Some((driver, _)) = key.rsplit_once('.') else {
                return Err(Error::GitUnsafe);
            };
            for suffix in ["clean=", "smudge=", "process=", "required=false"] {
                git.filters.push("-c".into());
                git.filters.push(format!("{driver}.{suffix}"));
            }
        }
        Ok(git)
    }
    async fn run(&self, args: Vec<String>, seconds: u64) -> Result<String> {
        if self.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let mut cmd = Command::new(&self.executable);
        cmd.current_dir(self.ws.root()).env_clear();
        for name in [
            "PATH",
            "Path",
            "SystemRoot",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "TMPDIR",
        ] {
            if let Some(v) = std::env::var_os(name) {
                cmd.env(name, v);
            }
        }
        cmd.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", NULL)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C");
        // Git for Windows does not interpret Rust's canonical extended prefix here.
        let root = self.ws.root().to_string_lossy();
        let root = root.strip_prefix("\\\\?\\").unwrap_or(&root);
        cmd.args(["--no-pager", "--no-replace-objects", "--literal-pathspecs"])
            .arg(format!(
                "--git-dir={}",
                Path::new(root).join(".git").display()
            ))
            .arg(format!("--work-tree={root}"));
        for option in [
            "core.fsmonitor=false",
            &format!("core.hooksPath={NULL}"),
            &format!("core.attributesFile={NULL}"),
            &format!("core.excludesFile={NULL}"),
            "commit.gpgSign=false",
            "log.showSignature=false",
            "submodule.recurse=false",
            "credential.helper=",
            "diff.external=",
            "protocol.allow=never",
            "core.quotePath=false",
        ] {
            cmd.args(["-c", option]);
        }
        cmd.args(&self.filters)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let child = cmd.spawn().map_err(|_| Error::Git)?;
        let token = self.cancel.child_token();
        let _guard = token.clone().drop_guard();
        let mut task = Box::pin(supervise_child(child, token.clone(), 10 * 1024 * 1024));
        let result = tokio::select! {biased;
            _=tokio::time::sleep(Duration::from_secs(seconds))=>{token.cancel();let _=task.await;return Err(Error::Timeout);},
            r=&mut task=>r
        };
        let out = result.map_err(|e| match e {
            ProcessError::Cancelled => Error::Cancelled,
            ProcessError::Limit => Error::OutputLimit,
            ProcessError::Io => Error::Git,
        })?;
        if !out.status.success() {
            return Err(Error::Git);
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    async fn files(&self, path: Option<&str>) -> Result<Vec<String>> {
        self.ws.resolve_safe_path(path.unwrap_or("."), false)?;
        let out = self
            .run(strings(&["ls-files", "-z", "--", path.unwrap_or(".")]), 15)
            .await?;
        let ws = self.ws.clone();
        let cancel = self.cancel.clone();
        tokio::task::spawn_blocking(move || {
            let b = Budget::new(cancel, 15);
            let mut files = vec![];
            for file in out.split('\0').filter(|s| !s.is_empty()) {
                b.check()?;
                match ws.resolve_safe_path(file, false) {
                    Ok(_) => {
                        control_files(&ws, file, &b)?;
                        files.push(file.into());
                    }
                    Err(Error::PathBlocked) => continue,
                    Err(e) => return Err(e),
                }
                if files.len() > 1000 {
                    return Err(Error::InputLimit);
                }
            }
            Ok(files)
        })
        .await
        .map_err(|_| Error::Internal)?
    }
    async fn revision(&self, revision: &str) -> Result<String> {
        let hash =
            (4..=40).contains(&revision.len()) && revision.bytes().all(|c| c.is_ascii_hexdigit());
        let head = revision == "HEAD"
            || revision.strip_prefix("HEAD~").is_some_and(|n| {
                !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit())
            });
        if revision.len() > 40 || (!hash && !head) {
            return Err(Error::Revision);
        }
        let value = self
            .run(
                vec![
                    "rev-parse".into(),
                    "--verify".into(),
                    format!("{revision}^{{commit}}"),
                ],
                5,
            )
            .await?;
        let value = value.trim();
        if !(40..=64).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Revision);
        }
        Ok(value.into())
    }
}
fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).into()).collect()
}
fn truncated(text: String) -> Value {
    json!({"truncated":units(&text)>20_000,"text":cut(&text,20_000)})
}
fn status(code: u8) -> &'static str {
    match code {
        b' ' => "unchanged",
        b'M' => "modified",
        b'A' => "added",
        b'D' => "deleted",
        b'R' => "renamed",
        b'C' => "copied",
        b'U' => "unmerged",
        b'?' => "untracked",
        b'!' => "ignored",
        _ => "unknown",
    }
}
pub(crate) async fn execute(
    ws: &SafeWorkspace,
    name: &str,
    a: &Map<String, Value>,
    cancel: CancellationToken,
) -> Result<Value> {
    let git = Git::new(ws, cancel.clone()).await?;
    let path = a.get("path").and_then(Value::as_str);
    let files = git.files(path).await?;
    let value = match name {
        "git_status" => {
            let mut args = strings(&[
                "status",
                "--ignore-submodules=all",
                "--untracked-files=no",
                "--no-renames",
                "--porcelain=v1",
                "-z",
                "--branch",
                "--",
            ]);
            args.extend(files.clone());
            // With no safe files do not allow Git to inspect all protected tracked files.
            if files.is_empty() {
                args.push("__myagent_no_safe_paths__".into());
            }
            let out = git.run(args, 15).await?;
            let mut records = out.split('\0').filter(|s| !s.is_empty());
            let branchline = records.next().unwrap_or("");
            let branch = branchline
                .strip_prefix("## ")
                .unwrap_or("unknown")
                .split("...")
                .next()
                .unwrap_or("unknown")
                .split(" [")
                .next()
                .unwrap_or("unknown");
            let count = |label: &str| {
                branchline
                    .split(label)
                    .nth(1)
                    .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0)
            };
            let mut changed = vec![];
            for record in records {
                if record.len() >= 3 {
                    let path = &record[3..];
                    if ws.resolve_safe_path(path, false).is_ok() {
                        changed.push(json!({"path":path,"staged":status(record.as_bytes()[0]),"unstaged":status(record.as_bytes()[1])}));
                    }
                }
            }
            let workspace = ws.clone();
            let tracked = files;
            let token = cancel.clone();
            let untracked = tokio::task::spawn_blocking(move || {
                crate::search::untracked(&workspace, &tracked, &Budget::new(token, 15))
            })
            .await
            .map_err(|_| Error::Internal)??;
            for path in untracked.0 {
                changed.push(json!({"path":path,"staged":"untracked","unstaged":"untracked"}));
            }
            json!({"branch":branch,"ahead":count("ahead "),"behind":count("behind "),"clean":changed.is_empty(),"files":changed,"truncated":untracked.1})
        }
        "git_diff" | "diff_summary" => {
            if files.is_empty() {
                if name == "git_diff" {
                    json!({"diff":"(sem diferenças)","truncated":false})
                } else {
                    json!({"files":[],"totalFiles":0,"additions":0,"deletions":0,"binaryFiles":0,"truncated":false})
                }
            } else {
                let mut args = strings(&[
                    "diff",
                    "--ignore-submodules=all",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-renames",
                    "--no-color",
                    "--relative",
                ]);
                if name == "diff_summary" {
                    args.extend(strings(&["--numstat", "-z"]));
                }
                if flag(a, "staged", false) {
                    args.push("--staged".into());
                }
                args.push("--".into());
                args.extend(files);
                let out = git.run(args, 15).await?;
                if name == "git_diff" {
                    json!({"diff":if out.is_empty(){"(sem diferenças)".into()}else{cut(&out,20_000)},"truncated":units(&out)>20_000})
                } else {
                    let mut entries = vec![];
                    let mut additions = 0;
                    let mut deletions = 0;
                    let mut binaries = 0;
                    for line in out.split('\0').filter(|s| !s.is_empty()) {
                        let mut parts = line.splitn(3, '\t');
                        let added = parts.next().ok_or(Error::Git)?;
                        let removed = parts.next().ok_or(Error::Git)?;
                        let path = parts.next().ok_or(Error::Git)?;
                        let binary = added == "-" || removed == "-";
                        let add = if binary {
                            0
                        } else {
                            added.parse::<u64>().map_err(|_| Error::Git)?
                        };
                        let del = if binary {
                            0
                        } else {
                            removed.parse::<u64>().map_err(|_| Error::Git)?
                        };
                        additions += add;
                        deletions += del;
                        binaries += usize::from(binary);
                        entries.push(
                            json!({"path":path,"additions":add,"deletions":del,"binary":binary}),
                        );
                    }
                    let count = entries.len();
                    entries.truncate(100);
                    limit_output(
                        json!({"files":entries,"totalFiles":count,"additions":additions,"deletions":deletions,"binaryFiles":binaries,"truncated":count>100}),
                    )?
                }
            }
        }
        "git_log" => {
            if files.is_empty() {
                json!({"commits":[],"count":0})
            } else {
                let mut args = strings(&[
                    "log",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-notes",
                    "--date=short",
                    "--pretty=format:%h%x09%ad%x09%an%x09%s",
                    "-n",
                ]);
                args.push(number(a, "limit", 20).to_string());
                args.push("--".into());
                args.extend(files);
                let out = git.run(args, 15).await?;
                let commits:Vec<_>=out.lines().map(|line|{let mut parts=line.splitn(4,'\t');json!({"hash":parts.next().unwrap_or(""),"date":parts.next().unwrap_or(""),"author":parts.next().unwrap_or(""),"message":parts.next().unwrap_or("")})}).collect();
                json!({"count":commits.len(),"commits":commits})
            }
        }
        "git_show" => {
            let commit = git.revision(text(a, "revision", "HEAD")).await?;
            if files.is_empty() {
                json!({"commit":commit,"text":"(nenhum arquivo seguro rastreado neste escopo)","truncated":false})
            } else {
                let mut args = strings(&[
                    "show",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-renames",
                    "--no-color",
                    "--no-notes",
                    "--format=commit %H%nAuthor: %an%nDate: %aI%nSubject: %s",
                ]);
                args.push(commit.clone());
                args.push("--".into());
                args.extend(files);
                let mut result = truncated(git.run(args, 15).await?);
                result["commit"] = commit.into();
                result
            }
        }
        "git_blame" => {
            let keys = git
                .run(
                    strings(&[
                        "config",
                        "--local",
                        "--no-includes",
                        "--null",
                        "--name-only",
                        "--list",
                    ]),
                    5,
                )
                .await?;
            if keys
                .split('\0')
                .any(|k| k.eq_ignore_ascii_case("blame.ignorerevsfile"))
            {
                return Err(Error::GitUnsafe);
            }
            let file = ws.relative(&ws.resolve_safe_path(text(a, "path", ""), false)?)?;
            if !files.contains(&file) {
                return Err(Error::NotTracked);
            }
            let commit = git.revision(text(a, "revision", "HEAD")).await?;
            let object = format!("{commit}:{file}");
            let size = git
                .run(vec!["cat-file".into(), "-s".into(), object.clone()], 5)
                .await?
                .trim()
                .parse::<usize>()
                .map_err(|_| Error::Git)?;
            if size > 1_048_576 {
                return Err(Error::FileTooLarge);
            }
            let content = git
                .run(vec!["cat-file".into(), "blob".into(), object], 5)
                .await?;
            if content.contains('\0') {
                return Err(Error::BinaryFile);
            }
            let lines = if content.is_empty() {
                0
            } else {
                content.split('\n').count() - usize::from(content.ends_with('\n'))
            };
            let start = number(a, "startLine", 1);
            if lines == 0 {
                json!({"path":file,"commit":commit,"startLine":start,"text":"","truncated":false})
            } else {
                if start > lines {
                    return Err(Error::LineRange);
                }
                let end = lines.min(start + number(a, "limit", 100) - 1);
                let args = vec![
                    "-c".into(),
                    format!("mailmap.file={NULL}"),
                    "-c".into(),
                    "mailmap.blob=".into(),
                    "blame".into(),
                    "--ignore-revs-file".into(),
                    "".into(),
                    "--no-textconv".into(),
                    "--line-porcelain".into(),
                    "-L".into(),
                    format!("{start},{end}"),
                    commit.clone(),
                    "--".into(),
                    file.clone(),
                ];
                let mut value = truncated(git.run(args, 15).await?);
                let map = value.as_object_mut().ok_or(Error::Internal)?;
                map.extend(json!({"path":file,"commit":commit,"startLine":start,"endLine":end,"totalLines":lines,"nextLine":if end<lines{Some(end+1)}else{None}}).as_object().ok_or(Error::Internal)?.clone());
                value
            }
        }
        _ => return Err(Error::Unavailable),
    };
    if value.to_string().len() > myagent_core::ToolLimits::default().output_bytes {
        return Err(Error::OutputLimit);
    }
    Ok(value)
}
