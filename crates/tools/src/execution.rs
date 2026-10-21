use crate::{Error, Result, SafeWorkspace, Snapshot, SnapshotOptions, io::cut};
use myagent_core::{BoxFuture, Effect, Mode, Tool, ToolContext, ToolSpec};
use myagent_sandbox::{Docker, DockerConfig, DockerDriver, DockerOutput, SandboxError};
use serde_json::{Map, Value, json};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ExecutionRegistry {
    workspace: SafeWorkspace,
    docker: Option<Docker>,
}
impl ExecutionRegistry {
    pub fn new(workspace: SafeWorkspace, docker: Option<Docker>) -> Self {
        Self { workspace, docker }
    }
    pub async fn detect(
        workspace: SafeWorkspace,
        config: DockerConfig,
        driver: Arc<dyn DockerDriver>,
        cancel: CancellationToken,
    ) -> Result<Self> {
        Ok(Self::new(
            workspace,
            Docker::detect(config, driver, cancel).await?,
        ))
    }
    pub fn available(&self) -> bool {
        self.docker.is_some()
    }
    pub fn tools(&self, mode: Mode) -> Vec<Arc<dyn Tool>> {
        if self.docker.is_none() || !matches!(mode, Mode::Plan | Mode::Build) {
            return vec![];
        }
        ["bash", "type_check", "test_run"]
            .into_iter()
            .filter(|name| mode == Mode::Build || *name != "bash")
            .map(|name| {
                Arc::new(ExecutionTool {
                    name,
                    registry: self.clone(),
                }) as Arc<dyn Tool>
            })
            .collect()
    }
    pub async fn call(
        &self,
        mode: Mode,
        name: &str,
        args: Value,
        ctx: ToolContext,
    ) -> Result<Value> {
        if !matches!(mode, Mode::Plan | Mode::Build) || (mode != Mode::Build && name == "bash") {
            return Err(Error::Denied);
        }
        self.execute(
            name,
            args.as_object().ok_or(Error::InvalidInput)?.clone(),
            ctx,
        )
        .await
    }
    async fn execute(
        &self,
        name: &str,
        args: Map<String, Value>,
        ctx: ToolContext,
    ) -> Result<Value> {
        validate(name, &args)?;
        let docker = self.docker.clone().ok_or(Error::Unavailable)?;
        let ws = self.workspace.clone();
        let name = name.to_owned();
        let token = ctx.cancellation.child_token();
        let _guard = token.clone().drop_guard();
        tokio::spawn(async move {
            let lock=crate::mutation_io::lock_workspace(&ws,&token).await?;
            let config=docker.config().clone(); let prepare_token=token.clone();
            let (snapshot,command)=tokio::task::spawn_blocking(move || {
                let b=crate::io::Budget::new(prepare_token.clone(),120); b.check()?;
                let command=prepare(&ws,&name,&args)?;
                let snapshot=Snapshot::build(&ws,SnapshotOptions{dependencies:config.dependencies},prepare_token)?;
                Ok::<_,Error>((snapshot,command))
            }).await.map_err(|_|Error::Internal)??;
            let result=docker.run(snapshot.path(),&command.script,Duration::from_millis(command.timeout),token).await;
            if matches!(result,Err(SandboxError::Cleanup)) {
                // Nenhuma remoção do mount enquanto um container puder continuar usando-o.
                let _retained=snapshot.retain();
                drop(lock); return Err(Error::Sandbox(SandboxError::Cleanup));
            }
            drop(snapshot); drop(lock);
            let timed_out=matches!(result,Err(SandboxError::Timeout));
            let output=match result {Ok(o)=>o,Err(SandboxError::Timeout)=>DockerOutput{stdout:String::new(),stderr:"Container timed out; partial output was discarded.".into(),exit_code:124},Err(e)=>return Err(e.into())};
            let stdout=cut(&output.stdout,20_000); let stderr=cut(&output.stderr,20_000);
            let truncated=stdout.len()!=output.stdout.len() || stderr.len()!=output.stderr.len();
            if command.kind=="type_check" {
                let raw=format!("{}\n{}",stdout,stderr);
                let diagnostics=diagnostics(&raw,command.runner=="cargo")?;
                return Ok(json!({"path":command.path,"runner":command.runner,"ok":output.exit_code==0 && !timed_out,"timedOut":timed_out,"diagnostics":diagnostics,
                    "errorCount":diagnostics.iter().filter(|d|d["severity"]=="error").count(),"warningCount":diagnostics.iter().filter(|d|d["severity"]=="warning").count(),
                    "rawOutput":cut(&raw,15_000),"truncated":truncated || crate::io::units(&raw)>15_000}));
            }
            Ok(json!({"runner":command.runner,"path":command.path,"files":command.targets,"stdout":stdout,"stderr":stderr,"exitCode":output.exit_code,
                "timedOut":timed_out,"truncated":truncated,"status":if timed_out {"timed_out"} else if output.exit_code==0 {"passed"} else {"failed"},
                "note":"Process result only; missing runners/dependencies and tmpfs exhaustion also fail. No test count inferred. No host changes are applied."}))
        }).await.map_err(|_|Error::Internal)?
    }
}
pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
struct Prepared {
    script: String,
    timeout: u64,
    kind: String,
    runner: String,
    path: String,
    targets: Vec<String>,
}
fn prepare(ws: &SafeWorkspace, name: &str, args: &Map<String, Value>) -> Result<Prepared> {
    if name == "bash" {
        return Ok(Prepared {
            script: args["command"]
                .as_str()
                .ok_or(Error::InvalidInput)?
                .trim()
                .into(),
            timeout: args
                .get("timeoutMs")
                .and_then(Value::as_u64)
                .unwrap_or(30_000),
            kind: name.into(),
            runner: "sh".into(),
            path: ".".into(),
            targets: vec![],
        });
    }
    let directory = ws.resolve_safe_path(
        args.get("path").and_then(Value::as_str).unwrap_or("."),
        true,
    )?;
    if !directory.is_dir() {
        return Err(Error::NotADirectory);
    }
    let relative = ws.relative(&directory)?;
    let runner = args
        .get("runner")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if directory.join("Cargo.toml").is_file() {
                "cargo"
            } else if name == "type_check" {
                "tsc"
            } else {
                "bun"
            }
        });
    if name == "type_check" {
        ws.resolve_safe_path(
            &ws.relative(&directory.join(if runner == "cargo" {
                "Cargo.toml"
            } else {
                "tsconfig.json"
            }))?,
            true,
        )?;
    }
    let mut targets = vec![];
    let mut cargo_targets = vec![];
    if let Some(files) = args.get("files").and_then(Value::as_array) {
        for file in files {
            let file = file.as_str().ok_or(Error::InvalidInput)?;
            let path = ws.resolve_safe_path(&format!("{relative}/{file}"), true)?;
            let child = path
                .strip_prefix(&directory)
                .map_err(|_| Error::PathBlocked)?;
            let target = child.to_str().ok_or(Error::PathBlocked)?.replace('\\', "/");
            if runner == "cargo" {
                let stem = target
                    .strip_prefix("tests/")
                    .and_then(|p| p.strip_suffix(".rs"))
                    .ok_or(Error::InvalidInput)?;
                if stem.is_empty()
                    || !stem
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Err(Error::InvalidInput);
                }
                cargo_targets.extend(["--test".to_owned(), stem.to_owned()]);
            }
            targets.push(format!("./{target}"));
        }
    }
    let mut invocation: Vec<String> = match (name, runner) {
        ("type_check", "cargo") => vec![
            "cargo",
            "check",
            "--offline",
            "--locked",
            "--message-format=json",
            "--target-dir",
            "/tmp/myagent-target",
        ],
        ("type_check", "tsc") => vec!["tsc", "--noEmit", "--pretty", "false"],
        ("test_run", "cargo") => vec![
            "cargo",
            "test",
            "--offline",
            "--locked",
            "--target-dir",
            "/tmp/myagent-target",
        ],
        ("test_run", "bun") => vec!["bun", "test"],
        ("test_run", "node") => vec!["node", "--test"],
        ("test_run", "vitest") => vec!["vitest", "run", "--maxWorkers=1"],
        ("test_run", "jest") => vec!["jest", "--runInBand", "--watch=false"],
        ("test_run", "pytest") => vec!["pytest", "-q", "-p", "no:cacheprovider"],
        _ => return Err(Error::InvalidInput),
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    invocation.extend(if runner == "cargo" {
        cargo_targets
    } else {
        targets.clone()
    });
    let mut script = String::from(
        r#"set -eu
testdir=$(mktemp -d /tmp/myagent-run.XXXXXX)
copy_tree() {
 mkdir -p "$2"
 for item in "$1"/* "$1"/.[!.]* "$1"/..?*; do
  [ -e "$item" ] || continue
  name=${item##*/}
  case "$name" in
   node_modules|.venv) ln -s "$item" "$2/$name";;
   *) if [ -d "$item" ]; then copy_tree "$item" "$2/$name"; else cp -p "$item" "$2/$name"; fi;;
  esac
 done
}
copy_tree /workspace "$testdir"
export HOME=/tmp PYTHONDONTWRITEBYTECODE=1
export CARGO_HOME=/tmp/myagent-cargo
mkdir -p "$CARGO_HOME"
if [ -d /usr/local/cargo/registry ]; then ln -s /usr/local/cargo/registry "$CARGO_HOME/registry"; fi
export PATH="$testdir/node_modules/.bin:$testdir/.venv/bin:$PATH"
cd "$testdir"/"#,
    );
    script.push_str(&shell_quote(&relative));
    script.push_str("\nexport PATH=\"$PWD/node_modules/.bin:$PWD/.venv/bin:$PATH\"\n");
    script.push_str(
        &invocation
            .iter()
            .map(|s| shell_quote(s))
            .collect::<Vec<_>>()
            .join(" "),
    );
    Ok(Prepared {
        script,
        timeout: args
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .unwrap_or(60_000),
        kind: name.into(),
        runner: runner.into(),
        path: relative,
        targets,
    })
}
fn diagnostics(raw: &str, cargo: bool) -> Result<Vec<Value>> {
    let mut diagnostics = vec![];
    if cargo {
        for line in raw.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && value["reason"] == "compiler-message"
            {
                let m = &value["message"];
                diagnostics.push(json!({"severity":m["level"],"message":m["message"],"code":m["code"]["code"],"spans":m["spans"]}));
            }
        }
    } else {
        let re = regex::Regex::new(r"^(.+?)\((\d+),(\d+)\): (error|warning) (TS\d+): (.+)$")
            .map_err(|_| Error::Internal)?;
        for line in raw.lines() {
            if let Some(m) = re.captures(line) {
                diagnostics.push(json!({"file":m[1],"line":m[2].parse::<u64>().map_err(|_|Error::InvalidInput)?,"column":m[3].parse::<u64>().map_err(|_|Error::InvalidInput)?,"severity":m[4],"code":m[5],"message":m[6]}));
            }
        }
    }
    Ok(diagnostics)
}
struct ExecutionTool {
    name: &'static str,
    registry: ExecutionRegistry,
}
impl Tool for ExecutionTool {
    fn cleanup_timeout(&self) -> Duration {
        // Docker removal has a 10s bound; leave time for CLI shutdown and
        // snapshot disposal before the core finishes the generation.
        Duration::from_secs(15)
    }
    fn spec(&self) -> ToolSpec {
        let (description, parameters) = schema(self.name);
        ToolSpec {
            name: self.name.into(),
            description: description.into(),
            parameters,
            effect: Effect::IsolatedProcess,
        }
    }
    fn authorize<'a>(
        &'a self,
        args: &'a Map<String, Value>,
        ctx: ToolContext,
    ) -> BoxFuture<'a, myagent_core::Result<()>> {
        Box::pin(async move {
            if ctx.cancellation.is_cancelled() {
                return Err(myagent_core::AgentError::Cancelled);
            }
            if !self.registry.available() {
                return Err(myagent_core::AgentError::ToolDenied);
            }
            validate(self.name, args).map_err(Into::into)
        })
    }
    fn execute<'a>(
        &'a self,
        args: Map<String, Value>,
        ctx: ToolContext,
    ) -> BoxFuture<'a, myagent_core::Result<String>> {
        Box::pin(async move {
            self.registry
                .execute(self.name, args, ctx)
                .await
                .map(|v| v.to_string())
                .map_err(Into::into)
        })
    }
}
fn schema(name: &str) -> (&'static str, Value) {
    let path = json!({"type":"string","maxLength":1000});
    let timeout = json!({"type":"integer","minimum":1000,"maximum":120000});
    let (description, properties, required) = match name {
        "bash" => (
            "Run a POSIX shell command ONLY in Docker, never on host; BUILD only. No network, root filesystem and filtered snapshot read-only, non-root, resource limits, disposable /tmp (64 MiB default). No approval required. Default timeout 30s/max 120s. At most 20000 UTF-16 units per stream; output overflow terminates execution.",
            json!({"command":{"type":"string","minLength":1,"maxLength":20000},"timeoutMs":{"type":"integer","minimum":1,"maximum":120000}}),
            vec!["command"],
        ),
        "type_check" => (
            "Check Rust (cargo check --offline --locked) or TypeScript (tsc --noEmit) ONLY in Docker. Runner auto-detected from Cargo.toml, or explicit cargo/tsc. Disposable /tmp copy; no host execution, network or approval; 60s. Toolchain and dependencies must exist in the image or opt-in snapshot.",
            json!({"path":path,"runner":{"type":"string","enum":["cargo","tsc"]}}),
            vec![],
        ),
        _ => (
            "Run Cargo/Bun/Node/Vitest/Jest/Pytest ONLY in Docker, on a disposable /tmp copy. Auto cargo for Cargo.toml, otherwise Bun. No approval or network; 60s default/max 120s, up to 10 files. Cargo files must be tests/<target>.rs. Dependencies remain read-only; none are installed. Exit status is not a count of executed tests.",
            json!({"path":path,"runner":{"type":"string","enum":["cargo","bun","node","vitest","jest","pytest"]},"files":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"string","minLength":1,"maxLength":1000}},"timeoutMs":timeout}),
            vec![],
        ),
    };
    (
        description,
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    )
}
fn validate(name: &str, args: &Map<String, Value>) -> Result<()> {
    if !["bash", "type_check", "test_run"].contains(&name) {
        return Err(Error::Unavailable);
    }
    crate::registry::validate_value(&Value::Object(args.clone()), &schema(name).1)?;
    if name == "bash"
        && args["command"]
            .as_str()
            .ok_or(Error::InvalidInput)?
            .trim()
            .is_empty()
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_support_windows_tsc_and_cargo_json() {
        let tsc=diagnostics("C:\\project\\a.ts(2,3): error TS2322: incompatible\r\nb.ts(4,5): warning TS1000: fixture",false).unwrap();
        assert_eq!(tsc.len(), 2);
        assert_eq!(tsc[0]["file"], "C:\\project\\a.ts");
        assert_eq!(tsc[0]["line"], 2);
        assert_eq!(tsc[1]["severity"], "warning");
        let cargo=diagnostics(r#"{"reason":"compiler-message","message":{"level":"error","message":"fixture","code":{"code":"E0001"},"spans":[]}}"#,true).unwrap();
        assert_eq!(cargo[0]["code"], "E0001");
    }
}
