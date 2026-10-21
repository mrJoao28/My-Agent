use crate::{
    Error, Result, SafeWorkspace,
    checkpoints::{capture, fingerprint},
    git::Git,
    io::Budget,
    mutation_io::*,
    registry::text,
};
use myagent_core::BoxFuture;
use serde_json::{Map, Value, json};
use std::{fs, sync::Arc};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub struct GitSettings {
    pub allowed_hosts: Vec<String>,
    pub identity: Option<(String, String)>,
    pub transport: Arc<dyn PushTransport>,
    pub use_external_credentials: bool,
}
impl Default for GitSettings {
    fn default() -> Self {
        Self {
            allowed_hosts: vec!["github.com".into()],
            identity: None,
            transport: Arc::new(NativePush),
            use_external_credentials: true,
        }
    }
}
impl GitSettings {
    pub fn from_environment() -> Self {
        let mut settings = Self::default();
        if let Ok(hosts) = std::env::var("MYAGENT_GIT_ALLOWED_HOSTS") {
            settings.allowed_hosts = hosts
                .split(',')
                .map(|h| h.trim().to_ascii_lowercase())
                .filter(|h| !h.is_empty())
                .collect();
        }
        settings
    }
}
pub struct PushRequest {
    pub(crate) workspace: SafeWorkspace,
    pub(crate) url: String,
    pub(crate) branch: String,
    pub(crate) options: Vec<String>,
}
impl PushRequest {
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn branch(&self) -> &str {
        &self.branch
    }
    pub fn refspec(&self) -> String {
        format!("refs/heads/{0}:refs/heads/{0}", self.branch)
    }
}
pub trait PushTransport: Send + Sync {
    fn push(&self, request: PushRequest, cancel: CancellationToken) -> BoxFuture<'_, Result<()>>;
}
pub struct NativePush;
impl PushTransport for NativePush {
    fn push(&self, request: PushRequest, cancel: CancellationToken) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let git = Git::new(&request.workspace, cancel).await?;
            let mut args = request.options.clone();
            args.extend(strings(&[
                "-c",
                "push.followTags=false",
                "push",
                "--porcelain",
                "--no-verify",
                "--signed=false",
                "--recurse-submodules=no",
            ]));
            args.push(request.url.clone());
            args.push(request.refspec());
            git.run_policy(args, 60, false, true).await?;
            Ok(())
        })
    }
}
pub(crate) struct GitPlan {
    pub name: String,
    pub args: Map<String, Value>,
    pub paths: Vec<String>,
    pub binding: String,
    pub preview: String,
    pub main: Option<String>,
    pub identity: Option<(String, String)>,
    pub options: Vec<String>,
}
fn strings(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).into()).collect()
}
pub fn validate_git_url(value: &str, hosts: &[String]) -> Result<String> {
    if value
        .bytes()
        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        || value.contains(['\\', '%'])
    {
        return Err(Error::GitTarget);
    }
    let url = url::Url::parse(value).map_err(|_| Error::GitTarget)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|p| p != 443)
        || url.path() == "/"
        || !url
            .host_str()
            .is_some_and(|h| hosts.iter().any(|allowed| allowed.eq_ignore_ascii_case(h)))
    {
        return Err(Error::GitTarget);
    }
    Ok(url.to_string())
}
async fn optional(git: &Git, args: Vec<String>) -> Result<Option<String>> {
    match git.run(args, 5).await {
        Ok(s) => Ok(Some(s.trim().into())),
        Err(Error::Git) => Ok(None),
        Err(e) => Err(e),
    }
}
async fn identity(git: &Git, settings: &GitSettings) -> Result<(String, String)> {
    if let Some(identity) = &settings.identity {
        return checked_identity(identity.clone());
    }
    let mut values = vec![];
    for key in ["user.name", "user.email"] {
        let local = optional(
            git,
            strings(&["config", "--local", "--no-includes", "--get", key]),
        )
        .await?;
        let value = if local.as_ref().is_some_and(|v| !v.is_empty()) {
            local.unwrap_or_default()
        } else {
            git.run_policy(
                strings(&["config", "--global", "--no-includes", "--get", key]),
                5,
                true,
                false,
            )
            .await
            .map_err(|_| Error::GitIdentity)?
            .trim()
            .to_owned()
        };
        values.push(value);
    }
    checked_identity((values.remove(0), values.remove(0)))
}
fn checked_identity(identity: (String, String)) -> Result<(String, String)> {
    if [&identity.0, &identity.1]
        .iter()
        .any(|v| v.trim().is_empty() || v.len() > 512 || v.contains(['\r', '\n', '\0']))
    {
        return Err(Error::GitIdentity);
    }
    Ok(identity)
}
async fn network_options(git: &Git, external: bool) -> Result<Vec<String>> {
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
    if keys.split('\0').any(|k| {
        let k = k.to_ascii_lowercase();
        [
            "include.",
            "includeif.",
            "url.",
            "http.",
            "https.",
            "credential.",
        ]
        .iter()
        .any(|p| k.starts_with(p))
            || k == "core.gitproxy"
            || (k.starts_with("remote.")
                && [".proxy", ".uploadpack", ".receivepack", ".vcs"]
                    .iter()
                    .any(|s| k.ends_with(s)))
    }) {
        return Err(Error::GitUnsafe);
    }
    let mut options = strings(&[
        "-c",
        "protocol.https.allow=always",
        "-c",
        "http.sslVerify=true",
        "-c",
        "http.followRedirects=false",
        "-c",
        "http.proxy=",
        "-c",
        "credential.interactive=false",
    ]);
    let mut helpers = Vec::new();
    for scope in ["--system", "--global"].into_iter().filter(|_| external) {
        match git
            .run_policy(
                strings(&[
                    "config",
                    scope,
                    "--no-includes",
                    "--null",
                    "--get-all",
                    "credential.helper",
                ]),
                5,
                true,
                false,
            )
            .await
        {
            Ok(value) => {
                for helper in value.split_terminator('\0') {
                    if helper.is_empty() {
                        helpers.clear();
                    } else {
                        helpers.push(helper.to_owned());
                    }
                }
            }
            Err(Error::Git) => {}
            Err(e) => return Err(e),
        }
    }
    for helper in helpers {
        options.push("-c".into());
        options.push(format!("credential.helper={helper}"));
    }
    Ok(options)
}
pub(crate) async fn prepare(
    ws: &SafeWorkspace,
    name: &str,
    a: &Map<String, Value>,
    settings: &GitSettings,
    cancel: CancellationToken,
) -> Result<GitPlan> {
    let mut args = a.clone();
    let mut paths = vec![];
    let mut current_identity = None;
    let mut main = None;
    let mut options = vec![];
    let mut binding_data = String::new();
    if name == "git_init" {
        let root = ws.resolve_safe_path(".", true)?;
        for parent in root.ancestors() {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            for marker in [".git", "HEAD"] {
                match fs::symlink_metadata(parent.join(marker)) {
                    Ok(_) => return Err(Error::GitUnsafe),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(Error::Io),
                }
            }
        }
        return Ok(GitPlan {
            name: name.into(),
            args,
            paths,
            binding: workspace_key(ws.root()),
            preview:
                "Initialize a new Git repository on main. Commit and push are separate approvals."
                    .into(),
            main,
            identity: None,
            options,
        });
    }
    let git = Git::new(ws, cancel.clone()).await?;
    let preview = match name {
        "git_commit" => {
            let files = a["files"].as_array().ok_or(Error::InvalidInput)?;
            for file in files {
                let safe =
                    ws.resolve_safe_path(file.as_str().ok_or(Error::InvalidInput)?, false)?;
                let relative = ws.relative(&safe)?;
                if relative == "." || safe.is_dir() {
                    return Err(Error::PathBlocked);
                }
                crate::git::control_files(ws, &relative, &Budget::new(cancel.clone(), 300))?;
                if !paths.contains(&relative) {
                    paths.push(relative);
                }
            }
            // Validate all staged names before reading any staged blob.
            let staged = git
                .run(strings(&["diff", "--cached", "--name-only", "-z"]), 15)
                .await?;
            for file in staged.split('\0').filter(|p| !p.is_empty()) {
                if crate::is_sensitive_path(file) {
                    return Err(Error::Denied);
                }
                ws.resolve_safe_path(file, false)?;
            }
            let identity = identity(&git, settings).await?;
            binding_data.push_str(&serde_json::to_string(&identity).map_err(|_| Error::Internal)?);
            current_identity = Some(identity);
            args.insert("files".into(), json!(paths));
            let mut diff = strings(&[
                "diff",
                "--stat",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--no-color",
            ]);
            if optional(&git, strings(&["rev-parse", "--verify", "HEAD"]))
                .await?
                .is_some()
            {
                diff.push("HEAD".into());
            }
            diff.push("--".into());
            diff.extend(paths.clone());
            let diff = git.run(diff, 15).await?;
            let owned = ws.clone();
            let requested = paths.clone();
            let token = cancel.clone();
            let digest = tokio::task::spawn_blocking(move || {
                fingerprint(&capture(
                    &owned,
                    &requested,
                    false,
                    &Budget::new(token, 300),
                )?)
            })
            .await
            .map_err(|_| Error::Internal)??;
            binding_data.push_str(&digest);
            format!(
                "Commit message: {}\nFiles: {}\nDiff --stat:\n{}",
                text(a, "message", ""),
                paths.join(", "),
                diff
            )
        }
        "git_remote_add" => {
            let remote = text(a, "remote", "origin");
            let url = validate_git_url(text(a, "url", ""), &settings.allowed_hosts)?;
            let old = optional(
                &git,
                vec![
                    "config".into(),
                    "--local".into(),
                    "--no-includes".into(),
                    "--get-all".into(),
                    format!("remote.{remote}.url"),
                ],
            )
            .await?;
            if old.as_ref().is_some_and(|old| old != &url) {
                return Err(Error::GitTarget);
            }
            binding_data.push_str(old.as_deref().unwrap_or("<absent>"));
            args.insert("url".into(), url.clone().into());
            args.insert("remote".into(), remote.into());
            format!("Add remote {remote}: {url}. Existing URLs are never replaced.")
        }
        "git_push" => {
            let remote = text(a, "remote", "origin");
            let branch = text(a, "branch", "");
            if branch.is_empty()
                || branch.starts_with('-')
                || branch == "HEAD"
                || branch.contains("@{")
            {
                return Err(Error::GitTarget);
            }
            git.run(strings(&["check-ref-format", "--branch", branch]), 5)
                .await
                .map_err(|_| Error::GitTarget)?;
            let current = git
                .run(strings(&["symbolic-ref", "--short", "HEAD"]), 5)
                .await?;
            if current.trim() != branch {
                return Err(Error::GitTarget);
            }
            let url = git
                .run(
                    vec![
                        "config".into(),
                        "--local".into(),
                        "--no-includes".into(),
                        "--get-all".into(),
                        format!("remote.{remote}.url"),
                    ],
                    5,
                )
                .await?;
            let url = validate_git_url(url.trim(), &settings.allowed_hosts)?;
            if optional(
                &git,
                vec![
                    "config".into(),
                    "--local".into(),
                    "--no-includes".into(),
                    "--get-all".into(),
                    format!("remote.{remote}.pushurl"),
                ],
            )
            .await?
            .is_some_and(|s| !s.is_empty())
            {
                return Err(Error::GitTarget);
            }
            if a.get("url")
                .and_then(Value::as_str)
                .is_some_and(|requested| requested != url)
            {
                return Err(Error::StaleApproval);
            }
            options = network_options(&git, settings.use_external_credentials).await?;
            args.insert("remote".into(), remote.into());
            args.insert("branch".into(), branch.into());
            args.insert("url".into(), url.clone().into());
            if matches!(branch, "main" | "master") {
                main = Some(branch.into());
            }
            format!(
                "Push branch: {branch}\nRemote: {remote}\nURL: {url}\nNo force, tags, mirror, delete or submodule recursion. Stop after push."
            )
        }
        _ => return Err(Error::Unavailable),
    };
    // Includes index, HEAD and config so changes while waiting invalidate approval.
    for path in ["index", "HEAD", "config"] {
        let path = ws.root().join(".git").join(path);
        if path.try_exists()? {
            binding_data.push_str(&hash(&bounded_file(&path, 20 * 1024 * 1024)?));
        }
    }
    if let Some(head) = optional(&git, strings(&["rev-parse", "--verify", "HEAD"])).await? {
        binding_data.push_str(&head);
    }
    binding_data.push_str(&serde_json::to_string(&args).map_err(|_| Error::Internal)?);
    binding_data.push_str(&serde_json::to_string(&options).map_err(|_| Error::Internal)?);
    Ok(GitPlan {
        name: name.into(),
        args,
        paths,
        binding: hash(binding_data.as_bytes()),
        preview,
        main,
        identity: current_identity,
        options,
    })
}
pub(crate) async fn execute(
    ws: &SafeWorkspace,
    plan: GitPlan,
    settings: &GitSettings,
    cancel: CancellationToken,
    confirmed: Option<&str>,
) -> Result<Value> {
    if plan.name == "git_init" {
        Git::for_init(ws, cancel)?
            .run(strings(&["init", "-b", "main"]), 15)
            .await?;
        return Ok(json!({"initialized":true,"branch":"main"}));
    }
    let git = Git::new(ws, cancel.clone()).await?;
    match plan.name.as_str() {
        "git_commit" => {
            let (name, email) = plan.identity.ok_or(Error::GitIdentity)?;
            let mut add = strings(&["add", "--"]);
            add.extend(plan.paths.clone());
            git.run(add, 15).await?;
            // No protected staged names or obvious key material may reach commit.
            let staged = git
                .run(strings(&["diff", "--cached", "--name-only", "-z"]), 15)
                .await?;
            let secret=regex::Regex::new(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|AKIA[A-Z0-9]{16}|gh[pousr]_[A-Za-z0-9]{30,}|sk-(?:proj-)?[A-Za-z0-9_-]{30,}").map_err(|_|Error::Internal)?;
            for file in staged.split('\0').filter(|s| !s.is_empty()) {
                if crate::is_sensitive_path(file) {
                    return Err(Error::Denied);
                }
                if git
                    .run(strings(&["ls-files", "-z", "--", file]), 5)
                    .await?
                    .is_empty()
                {
                    continue;
                }
                let blob = git.run(vec!["show".into(), format!(":{file}")], 5).await?;
                if secret.is_match(&blob) {
                    return Err(Error::Denied);
                }
            }
            let message = text(&plan.args, "message", "");
            let mut commit = vec![
                "-c".into(),
                format!("user.name={name}"),
                "-c".into(),
                format!("user.email={email}"),
            ];
            commit.extend(strings(&[
                "commit",
                "--only",
                "--no-verify",
                "--no-gpg-sign",
                "-m",
                message,
                "--",
            ]));
            commit.extend(plan.paths.clone());
            git.run(commit, 15).await?;
            let hash = git
                .run(strings(&["rev-parse", "--short", "HEAD"]), 5)
                .await?;
            Ok(json!({"commitHash":hash.trim(),"message":message,"files":plan.paths}))
        }
        "git_remote_add" => {
            let remote = text(&plan.args, "remote", "origin");
            let url = text(&plan.args, "url", "");
            let exists = optional(
                &git,
                vec![
                    "config".into(),
                    "--local".into(),
                    "--no-includes".into(),
                    "--get-all".into(),
                    format!("remote.{remote}.url"),
                ],
            )
            .await?
            .is_some();
            if !exists {
                git.run(strings(&["remote", "add", remote, url]), 15)
                    .await?;
            }
            Ok(json!({"remote":remote,"url":url,"added":!exists}))
        }
        "git_push" => {
            let branch = text(&plan.args, "branch", "");
            if plan.main.is_some() && confirmed != Some(branch) {
                return Err(Error::Denied);
            }
            let url = text(&plan.args, "url", "");
            settings
                .transport
                .push(
                    PushRequest {
                        workspace: ws.clone(),
                        url: url.into(),
                        branch: branch.into(),
                        options: plan.options,
                    },
                    cancel,
                )
                .await?;
            Ok(
                json!({"remote":text(&plan.args,"remote","origin"),"url":url,"branch":branch,"pushed":true,"awaitingRelease":true,"message":"Branch pushed. Stop and wait for the user's GitHub release before continuing."}),
            )
        }
        _ => Err(Error::Unavailable),
    }
}
