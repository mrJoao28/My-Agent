use crate::{
    Error, Result, SafeWorkspace,
    approval::{ApprovalGrant, ApprovalManager, Scope},
    checkpoints::{CheckpointStore, UndoResult, capture, expanded_paths, fingerprint},
    file_mutations::{self, FilePlan},
    git_mutations::{self, GitPlan, GitSettings},
    io::Budget,
    mutation_io::*,
};
use myagent_core::{
    BoxFuture, Effect, Mode, SessionActionGuard, SharedJournal, Tool, ToolContext, ToolLimits,
    ToolSpec,
};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub const MUTATION_NAMES: &[&str] = &[
    "write_file",
    "edit_file",
    "delete_file",
    "move_file",
    "apply_patch",
    "git_commit",
    "git_init",
    "git_remote_add",
    "git_push",
];
const INDIVIDUAL: &[&str] = &[
    "delete_file",
    "git_commit",
    "git_init",
    "git_remote_add",
    "git_push",
];
pub const MUTATION_INPUT_BYTES: usize = 8 * 1024 * 1024;
enum Plan {
    File {
        plan: FilePlan,
        roots: Vec<String>,
        before: String,
    },
    Git(GitPlan),
}
struct Permit {
    plan: Plan,
    arguments: String,
    grant: ApprovalGrant,
    expires: Instant,
    cancel: myagent_core::CancellationToken,
}
#[derive(Clone)]
pub struct MutationRegistry {
    workspace: SafeWorkspace,
    checkpoints: CheckpointStore,
    approvals: ApprovalManager,
    journal: SharedJournal,
    git: GitSettings,
    permits: Arc<Mutex<HashMap<(String, String), Permit>>>,
}
impl MutationRegistry {
    /// Despacho de comando local: o chamador deve tratá-lo antes de enviar texto ao modelo.
    pub async fn try_local_command(
        &self,
        session: &str,
        input: &str,
        cancel: myagent_core::CancellationToken,
    ) -> Result<Option<UndoResult>> {
        let command = input.trim();
        if command == "/undo" {
            return self.undo(session, cancel).await.map(Some);
        }
        if command.split_whitespace().next() == Some("/undo") {
            return Err(Error::InvalidInput);
        }
        Ok(None)
    }
    pub fn new(
        workspace: SafeWorkspace,
        checkpoints: CheckpointStore,
        approvals: ApprovalManager,
        journal: SharedJournal,
        git: GitSettings,
    ) -> Self {
        Self {
            workspace,
            checkpoints,
            approvals,
            journal,
            git,
            permits: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn tools(&self, mode: Mode) -> Vec<Arc<dyn Tool>> {
        if mode != Mode::Build {
            return vec![];
        }
        MUTATION_NAMES
            .iter()
            .map(|name| {
                Arc::new(MutationTool {
                    name,
                    registry: self.clone(),
                }) as Arc<dyn Tool>
            })
            .collect()
    }
    pub fn forget_session(&self, session: &str) {
        self.approvals.forget(session);
        if let Ok(mut permits) = self.permits.lock() {
            permits.retain(|(id, _), _| id != session);
        }
    }
    fn check(&self, session: &str, idle: bool) -> Result<()> {
        self.journal
            .check_session(session, &self.workspace.root().to_string_lossy(), idle)
            .map_err(|e| {
                if e == myagent_core::AgentError::Busy {
                    Error::GenerationActive
                } else {
                    Error::Session
                }
            })
    }
    /// API local explícita; não é registrada como ferramenta do modelo.
    pub async fn undo(
        &self,
        session: &str,
        cancel: myagent_core::CancellationToken,
    ) -> Result<UndoResult> {
        let activity = SessionActionGuard::acquire(session).map_err(|_| Error::GenerationActive)?;
        let owned = self.clone();
        let session = session.to_owned();
        let token = cancel.child_token();
        let _cancel = token.clone().drop_guard();
        tokio::spawn(async move {
            let lock = lock_workspace(&owned.workspace, &token).await?;
            tokio::task::spawn_blocking(move || {
                let _lock = lock;
                let _activity = activity;
                owned.check(&session, true)?;
                let budget = Budget::new(token, 300);
                budget.check()?;
                owned.checkpoints.undo_locked(
                    &owned.workspace,
                    &session,
                    &budget,
                    || owned.check(&session, true),
                    |result| {
                        owned
                            .journal
                            .local_note(&session, &result.message)
                            .map_err(|_| Error::Session)
                    },
                )
            })
            .await
            .map_err(|_| Error::Internal)?
        })
        .await
        .map_err(|_| Error::Internal)?
    }
    pub async fn call(
        &self,
        mode: Mode,
        name: &str,
        args: Value,
        context: ToolContext,
    ) -> Result<Value> {
        if mode != Mode::Build {
            return Err(Error::Denied);
        }
        let args = args.as_object().ok_or(Error::InvalidInput)?.clone();
        self.authorize(name, &args, &context).await?;
        self.execute(name, args, context).await
    }
    async fn authorize(
        &self,
        name: &str,
        args: &Map<String, Value>,
        ctx: &ToolContext,
    ) -> Result<()> {
        validate(name, args)?;
        if ctx.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        myagent_protocol::validate_id(&ctx.tool_call_id).map_err(|_| Error::InvalidInput)?;
        let owned = self.clone();
        let session = ctx.session_id.clone();
        tokio::task::spawn_blocking(move || owned.check(&session, false))
            .await
            .map_err(|_| Error::Internal)??;
        let lock = lock_workspace(&self.workspace, &ctx.cancellation).await?;
        let plan = if name.starts_with("git_") {
            let git = git_mutations::prepare(
                &self.workspace,
                name,
                args,
                &self.git,
                ctx.cancellation.clone(),
            )
            .await?;
            drop(lock);
            Plan::Git(git)
        } else {
            let ws = self.workspace.clone();
            let args = args.clone();
            let name = name.to_owned();
            let token = ctx.cancellation.clone();
            tokio::task::spawn_blocking(move || {
                let _lock = lock;
                let budget = Budget::new(token, 300);
                let plan = file_mutations::prepare(&ws, &name, &args, &budget)?;
                let roots = expanded_paths(&ws, &plan.paths)?;
                let before = fingerprint(&capture(&ws, &roots, false, &budget)?)?;
                Ok::<Plan, Error>(Plan::File {
                    plan,
                    roots,
                    before,
                })
            })
            .await
            .map_err(|_| Error::Internal)??
        };
        let (paths, preview, main) = match &plan {
            Plan::File { plan, .. } => (plan.paths.clone(), plan.preview.clone(), None),
            Plan::Git(plan) => (plan.paths.clone(), plan.preview.clone(), plan.main.clone()),
        };
        let mut scope_paths = paths.clone();
        if cfg!(windows) {
            for path in &mut scope_paths {
                *path = path.to_ascii_lowercase();
            }
        }
        scope_paths.sort();
        scope_paths.dedup();
        let scope_key = hash(
            &serde_json::to_vec(&(
                name,
                workspace_key(self.workspace.root()),
                scope_paths,
                match &plan {
                    Plan::Git(g) => Some(g.args.clone()),
                    _ => None,
                },
            ))
            .map_err(|_| Error::Internal)?,
        );
        let label = if paths.is_empty() {
            format!("Workspace: {}", self.workspace.root().display())
        } else {
            format!("Paths: {}", paths.join(", "))
        };
        if crate::io::units(&label) > 32768 {
            return Err(Error::InputLimit);
        }
        let scope = Scope {
            key: scope_key,
            label,
            allow_session: !INDIVIDUAL.contains(&name),
            main,
        };
        let (preview, truncated) = file_mutations::preview(&preview);
        let mut display = args.clone();
        for key in ["content", "old_str", "new_str", "patch"] {
            display.remove(key);
        }
        display.insert("preview".into(), preview.into());
        display.insert("previewTruncated".into(), truncated.into());
        if let Plan::Git(git) = &plan {
            display = git.args.clone();
            display.insert(
                "preview".into(),
                file_mutations::preview(&git.preview).0.into(),
            );
            display.insert("previewTruncated".into(), truncated.into());
        }
        let grant = self.approvals.request(name, display, &scope, ctx).await?;
        if ctx.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let key = (ctx.session_id.clone(), ctx.tool_call_id.clone());
        let mut permits = self.permits.lock().map_err(|_| Error::Internal)?;
        permits.retain(|_, p| p.expires > Instant::now() && !p.cancel.is_cancelled());
        if permits.contains_key(&key) {
            return Err(Error::StaleApproval);
        }
        permits.insert(
            key,
            Permit {
                plan,
                arguments: myagent_core::guard::key(name, args),
                grant,
                expires: Instant::now() + Duration::from_secs(120),
                cancel: ctx.cancellation.clone(),
            },
        );
        Ok(())
    }
    async fn execute(
        &self,
        name: &str,
        args: Map<String, Value>,
        ctx: ToolContext,
    ) -> Result<Value> {
        validate(name, &args)?;
        let permit = self
            .permits
            .lock()
            .map_err(|_| Error::Internal)?
            .remove(&(ctx.session_id.clone(), ctx.tool_call_id.clone()))
            .ok_or(Error::Denied)?;
        if permit.arguments != myagent_core::guard::key(name, &args)
            || permit.expires <= Instant::now()
            || permit.cancel.is_cancelled()
            || ctx.cancellation.is_cancelled()
            || self.approvals.epoch(&ctx.session_id)? != permit.grant.epoch
        {
            return Err(Error::StaleApproval);
        }
        let owned = self.clone();
        let name = name.to_owned();
        let token = ctx.cancellation.child_token();
        let _cancel = token.clone().drop_guard();
        tokio::spawn(async move {
            let lock = lock_workspace(&owned.workspace, &token).await?;
            let session = ctx.session_id.clone();
            let checked = owned.clone();
            tokio::task::spawn_blocking(move || checked.check(&session, false))
                .await
                .map_err(|_| Error::Internal)??;
            let grant = permit.grant;
            let result = match permit.plan {
                Plan::File {
                    plan,
                    roots,
                    before,
                } => {
                    let session = ctx.session_id.clone();
                    let checkpoint = owned.checkpoints.clone();
                    let ws = owned.workspace.clone();
                    let verifier = owned.clone();
                    tokio::task::spawn_blocking(move || {
                        let _lock = lock;
                        let b = Budget::new(token, 300);
                        verifier.check(&session, false)?;
                        b.check()?;
                        if name == "apply_patch" && crate::registry::flag(&args, "checkOnly", false)
                        {
                            return Ok::<Value, Error>(plan.result);
                        }
                        let (mut result, step) = checkpoint.run(
                            &ws,
                            &session,
                            crate::checkpoints::Step {
                                tool: &name,
                                paths: &roots,
                                approved: &before,
                                addition: plan.addition,
                                backup: plan.backup,
                            },
                            &b,
                            || {
                                file_mutations::execute(&ws, &plan, &b, || {
                                    verifier.check(&session, false)
                                })
                            },
                        )?;
                        if let Some(id) = step {
                            result["checkpoint"] = json!({"id":id,"undo":"/undo"});
                        }
                        Ok(result)
                    })
                    .await
                    .map_err(|_| Error::Internal)??
                }
                Plan::Git(plan) => {
                    let refreshed = git_mutations::prepare(
                        &owned.workspace,
                        &plan.name,
                        &plan.args,
                        &owned.git,
                        token.clone(),
                    )
                    .await?;
                    if refreshed.binding != plan.binding {
                        return Err(Error::StaleApproval);
                    }
                    let result = git_mutations::execute(
                        &owned.workspace,
                        refreshed,
                        &owned.git,
                        token,
                        grant.confirmation.as_deref(),
                    )
                    .await?;
                    drop(lock);
                    result
                }
            };
            let mut result = result;
            result["approval"] = serde_json::to_value(grant).map_err(|_| Error::Internal)?;
            Ok(result)
        })
        .await
        .map_err(|_| Error::Internal)?
    }
}
struct MutationTool {
    name: &'static str,
    registry: MutationRegistry,
}
impl Tool for MutationTool {
    fn cleanup_timeout(&self) -> Duration {
        // After-images must be durable even when cancellation follows a write.
        Duration::from_secs(310)
    }
    fn spec(&self) -> ToolSpec {
        let (description, parameters) = schema(self.name);
        ToolSpec {
            name: self.name.into(),
            description: description.into(),
            parameters,
            effect: Effect::Write,
        }
    }
    fn limits(&self) -> ToolLimits {
        ToolLimits {
            input_bytes: MUTATION_INPUT_BYTES,
            output_bytes: 262144,
        }
    }
    fn ends_generation_on_success(&self) -> bool {
        self.name == "git_push"
    }
    fn authorize<'a>(
        &'a self,
        args: &'a Map<String, Value>,
        context: ToolContext,
    ) -> BoxFuture<'a, myagent_core::Result<()>> {
        Box::pin(async move {
            self.registry
                .authorize(self.name, args, &context)
                .await
                .map_err(Into::into)
        })
    }
    fn execute<'a>(
        &'a self,
        args: Map<String, Value>,
        context: ToolContext,
    ) -> BoxFuture<'a, myagent_core::Result<String>> {
        Box::pin(async move {
            self.registry
                .execute(self.name, args, context)
                .await
                .map(|v| v.to_string())
                .map_err(Into::into)
        })
    }
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub(crate) fn schema(name: &str) -> (&'static str, Value) {
    let path = json!({"type":"string","minLength":1,"maxLength":4096});
    let boolean = json!({"type":"boolean"});
    let remote = json!({"type":"string","minLength":1,"maxLength":64});
    match name {
        "write_file" => (
            "Atomically write a safe file after consent and checkpoint. Parent directories are created; overwrite defaults true; overwrite=false requires a new file; backup=true includes a non-overwritten .bak in the exact approval scope. Existing modes and exact pre-image bytes are preserved. Maximum 5 MiB/file and checkpoint budgets apply.",
            object(
                json!({"path":path,"content":{"type":"string"},"overwrite":boolean,"backup":boolean}),
                &["path", "content"],
            ),
        ),
        "edit_file" => (
            "Replace exactly one old_str occurrence, comparing LF/CRLF equivalently. New text uses the dominant EOL (first on ties); unaffected bytes and existing mode remain. Explicit occurrence errors, consent, checkpoint and /undo. No binary editing.",
            object(
                json!({"path":path,"old_str":{"type":"string","minLength":1},"new_str":{"type":"string"}}),
                &["path", "old_str"],
            ),
        ),
        "delete_file" => (
            "Delete a safe file, or a full safe directory with recursive=true. Always individual approval. Captures binary/untracked files and mode for /undo; never deletes workspace root or sensitive descendants.",
            object(json!({"path":path,"recursive":boolean}), &["path"]),
        ),
        "move_file" => (
            "Move a safe file/directory; overwrite defaults false. Exact approval scope includes both endpoints. Checkpoint covers the full affected trees and new parents; root, links and sensitive descendants are blocked.",
            object(
                json!({"from":path,"to":path,"overwrite":boolean}),
                &["from", "to"],
            ),
        ),
        "apply_patch" => (
            "Apply a unified diff of at most 500000 UTF-16 characters and 64 paths as ONE checkpoint. checkOnly validates without writes or a checkpoint. Rejects binary/symlink/rename/copy patches and names with spaces or escapes. Consent required; index is never modified.",
            object(
                json!({"patch":{"type":"string","maxLength":500000},"checkOnly":boolean}),
                &["patch"],
            ),
        ),
        "git_commit" => (
            "Individually approve a commit message (1-2000 characters) and 1-100 explicit safe files, with diff stat preview. Preserve unrelated staged entries; never push. Hooks, filters and signing disabled. Requires an existing user identity, checks staged names/key patterns; Git changes have no filesystem undo.",
            object(
                json!({"message":{"type":"string","minLength":1,"maxLength":2000},"files":{"type":"array","minItems":1,"maxItems":100,"items":path}}),
                &["message", "files"],
            ),
        ),
        "git_init" => (
            "Individually approve Git initialization on main, only outside any existing repository. Create requested files first; commit and push require separate approvals. Never initializes over existing .git or nested under a repository.",
            object(json!({}), &[]),
        ),
        "git_remote_add" => (
            "Individually approve an HTTPS remote on an allowed host (github.com by default), without credentials, query or fragment. Never replaces an existing different URL. Default remote: origin.",
            object(
                json!({"remote":remote,"url":{"type":"string","minLength":1,"maxLength":2048}}),
                &["url"],
            ),
        ),
        "git_push" => (
            "Individually approve exact HTTPS URL and current branch. main/master require a SECOND explicit M confirmation after Y; A is forbidden. No force/delete/mirror/tags. URL and branch are revalidated before network. Stop generation after success and await the user's GitHub release. Cannot be undone.",
            object(
                json!({"remote":remote,"branch":{"type":"string","minLength":1,"maxLength":200},"url":{"type":"string","maxLength":2048}}),
                &["branch"],
            ),
        ),
        _ => ("Unavailable", object(json!({}), &[])),
    }
}
fn validate(name: &str, args: &Map<String, Value>) -> Result<()> {
    if !MUTATION_NAMES.contains(&name) {
        return Err(Error::Unavailable);
    }
    if serde_json::to_vec(args)
        .map_err(|_| Error::InvalidInput)?
        .len()
        > MUTATION_INPUT_BYTES
    {
        return Err(Error::InputLimit);
    }
    crate::registry::validate_value(&Value::Object(args.clone()), &schema(name).1)?;
    if name.starts_with("git_")
        && args
            .get("remote")
            .and_then(Value::as_str)
            .is_some_and(|remote| {
                remote.is_empty()
                    || !remote.as_bytes()[0].is_ascii_alphanumeric()
                    || !remote
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            })
    {
        return Err(Error::GitTarget);
    }
    Ok(())
}
