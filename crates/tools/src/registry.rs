use crate::{
    Error, Result, SafeWorkspace,
    io::{Budget, bounded},
};
use myagent_core::{AgentError, BoxFuture, Effect, Mode, Tool, ToolContext, ToolLimits, ToolSpec};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub const TOOL_NAMES: &[&str] = &[
    "read_file",
    "read_files",
    "list_directory",
    "glob",
    "grep",
    "file_outline",
    "symbol_search",
    "project_inspect",
    "environment_check",
    "test_discover",
    "git_status",
    "git_diff",
    "git_log",
    "git_show",
    "git_blame",
    "diff_summary",
    "todo_write",
];
type Todos = Arc<Mutex<HashMap<String, Value>>>;
#[derive(Clone)]
pub struct ToolRegistry {
    mutations: Option<crate::MutationRegistry>,
    execution: Option<crate::ExecutionRegistry>,
    web: Option<crate::WebRegistry>,
    workspace: SafeWorkspace,
    todos: Todos,
    sandbox_configured: bool,
}
impl ToolRegistry {
    pub fn new(workspace: SafeWorkspace, sandbox_configured: bool) -> Self {
        Self {
            mutations: None,
            execution: None,
            web: None,
            workspace,
            todos: Arc::new(Mutex::new(HashMap::new())),
            sandbox_configured,
        }
    }
    pub fn tools(&self, mode: Mode) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = TOOL_NAMES
            .iter()
            .map(|name| {
                Arc::new(Builtin {
                    name,
                    registry: self.clone(),
                }) as Arc<dyn Tool>
            })
            .filter(|tool| match mode {
                Mode::Plan => matches!(tool.spec().effect, Effect::Read | Effect::SessionState),
                Mode::Build => true,
                _ => false,
            })
            .collect();
        if let Some(mutations) = &self.mutations {
            tools.extend(mutations.tools(mode));
        }
        if let Some(execution) = &self.execution {
            tools.extend(execution.tools(mode));
        }
        if let Some(web) = &self.web {
            tools.extend(web.tools(mode));
        }
        tools
    }
    pub fn with_mutations(mut self, mutations: crate::MutationRegistry) -> Self {
        self.mutations = Some(mutations);
        self
    }
    pub fn with_execution(mut self, execution: crate::ExecutionRegistry) -> Self {
        self.sandbox_configured = execution.available();
        self.execution = Some(execution);
        self
    }
    pub fn with_web(mut self, web: crate::WebRegistry) -> Self {
        self.web = Some(web);
        self
    }
    pub fn session_todos(&self, session: &str) -> Result<Value> {
        Ok(self
            .todos
            .lock()
            .map_err(|_| Error::Internal)?
            .get(session)
            .cloned()
            .unwrap_or_else(|| json!([])))
    }
    pub fn clear_session(&self, session: &str) -> Result<()> {
        if let Some(web) = &self.web {
            web.forget_session(session);
        }
        if let Some(mutations) = &self.mutations {
            mutations.forget_session(session);
        }
        self.todos
            .lock()
            .map_err(|_| Error::Internal)?
            .remove(session);
        Ok(())
    }
    pub async fn call(&self, name: &str, args: Value, context: ToolContext) -> Result<Value> {
        let args = args.as_object().ok_or(Error::InvalidInput)?.clone();
        validate(name, &args)?;
        if context.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let token = context.cancellation.child_token();
        let _guard = token.clone().drop_guard();
        if name.starts_with("git_") || name == "diff_summary" {
            return crate::git::execute(&self.workspace, name, &args, token).await;
        }
        let this = self.clone();
        let name = name.to_owned();
        let seconds = if matches!(name.as_str(), "file_outline" | "symbol_search") {
            30
        } else {
            15
        };
        let budget = Budget::new(token, seconds);
        tokio::task::spawn_blocking(move || {
            budget.check()?;
            let value=match name.as_str() {
                "read_file"=>crate::files::read_file(&this.workspace,&args,&budget)?,
                "read_files"=>crate::files::read_files(&this.workspace,&args,&budget)?,
                "list_directory"=>crate::files::list_directory(&this.workspace,&args,&budget)?,
                "glob"|"grep"=>crate::search::execute(&this.workspace,&name,&args,&budget)?,
                "file_outline"|"symbol_search"=>crate::source::execute(&this.workspace,&name,&args,&budget)?,
                "project_inspect"|"test_discover"=>crate::inspect::execute(&this.workspace,&name,&args,&budget)?,
                "environment_check"=>{
                    this.workspace.resolve_safe_path(".",true)?;
                    json!({"server":{"platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"myagent":env!("CARGO_PKG_VERSION")},"workspaceAccessible":true,"sandbox":{"configured":this.sandbox_configured,"verified":this.execution.as_ref().is_some_and(|e|e.available()),"network":false,"hostExecution":false,"dependenciesVerified":false,"nextStep":if this.sandbox_configured {"Verify the sandbox; host runtime does not prove image dependencies."} else {"Configure MYAGENT_SANDBOX_IMAGE before executing processes."}}})
                },
                "todo_write"=>{
                    static NEXT: std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
                    let todos=args["todos"].as_array().ok_or(Error::InvalidInput)?;
                    let items:Vec<_>=todos.iter().map(|t|json!({"id":format!("{}-{}",std::process::id(),NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed)),"content":t["content"],"status":t["status"]})).collect();
                    let result=json!({"todos":items,"pending":items.iter().filter(|t|t["status"]=="pending").count(),"inProgress":items.iter().filter(|t|t["status"]=="in_progress").count(),"completed":items.iter().filter(|t|t["status"]=="completed").count()});
                    if result.to_string().len()>ToolLimits::default().output_bytes {return Err(Error::OutputLimit);}
                    this.todos.lock().map_err(|_|Error::Internal)?.insert(context.session_id,Value::Array(items)); result
                },
                _=>return Err(Error::Unavailable),
            };
            budget.check()?;
            if value.to_string().len()>ToolLimits::default().output_bytes {return Err(Error::OutputLimit);}
            Ok(value)
        }).await.map_err(|_|Error::Internal)?
    }
}
struct Builtin {
    name: &'static str,
    registry: ToolRegistry,
}
impl Tool for Builtin {
    fn spec(&self) -> ToolSpec {
        let (description, parameters) = schema(self.name);
        ToolSpec {
            name: self.name.into(),
            description: description.into(),
            parameters,
            effect: if self.name == "todo_write" {
                Effect::SessionState
            } else {
                Effect::Read
            },
        }
    }
    fn authorize<'a>(
        &'a self,
        args: &'a Map<String, Value>,
        context: ToolContext,
    ) -> BoxFuture<'a, myagent_core::Result<()>> {
        Box::pin(async move {
            if context.cancellation.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            validate(self.name, args).map_err(Into::into)
        })
    }
    fn execute<'a>(
        &'a self,
        args: Map<String, Value>,
        context: ToolContext,
    ) -> BoxFuture<'a, myagent_core::Result<String>> {
        Box::pin(async move {
            self.registry
                .call(self.name, Value::Object(args), context)
                .await
                .map(|v| v.to_string())
                .map_err(Into::into)
        })
    }
}
fn string(max: usize) -> Value {
    json!({"type":"string","maxLength":max})
}
fn integer(min: u64, max: u64) -> Value {
    json!({"type":"integer","minimum":min,"maximum":max})
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn schema(name: &str) -> (&'static str, Value) {
    let path = string(1000);
    let boolean = json!({"type":"boolean"});
    match name {
        "read_file" => (
            "Read text files up to 5 MiB in complete-line pages of 10000 UTF-16 characters including numbering. offset is zero-based; numbered defaults true. nextOffset continues without gaps; an oversized line raises LINE_TOO_LONG. Number prefixes are not file content.",
            object(
                json!({"path":path,"offset":integer(0,9_007_199_254_740_991),"limit":integer(1,9_007_199_254_740_991),"numbered":boolean}),
                &["path"],
            ),
        ),
        "read_files" => (
            "Read 1-8 safe files: 1 MiB each, 4 MiB batch, 10000 characters/file and 30000 content characters. Zero-based lines, individual sanitized errors and explicit abbreviation.",
            object(
                json!({"files":{"type":"array","minItems":1,"maxItems":8,"items":object(json!({"path":path,"offset":integer(0,9_007_199_254_740_991),"limit":integer(1,2000)}),&["path"])}}),
                &["files"],
            ),
        ),
        "list_directory" => (
            "List up to 500 safe entries per page, examining up to 20000 entries; nextOffset requires an unchanged directory. Excludes sensitive names and links.",
            object(
                json!({"path":path,"offset":integer(0,20000),"limit":integer(1,500)}),
                &[],
            ),
        ),
        "glob" => (
            "Find up to 500 safe file paths using literal metacharacters and *, **, ?. Relative to path, up to 20000 entries; root .gitignore and fixed exclusions apply even under a subdirectory.",
            object(json!({"path":path,"pattern":string(1000)}), &["pattern"]),
        ),
        "grep" => (
            "Search safe text using regex or literal=true. At most 200 matches, 60000 output characters, 20 MB read, 1 MB/file, 20000 entries and 15 seconds. include is relative to path, or basename without slash; context 0-20, snippets 300 characters; files_only returns unique paths. Root gitignore applies. Unsupported regex constructs return INVALID_PATTERN.",
            object(
                json!({"path":path,"pattern":{"type":"string","minLength":1,"maxLength":1000},"caseSensitive":boolean,"literal":boolean,"include":{"type":"string","minLength":1,"maxLength":1000},"context":integer(0,20),"files_only":boolean}),
                &["pattern"],
            ),
        ),
        "file_outline" => (
            "List up to 200 JS/TS/JSX/TSX declarations using a cancellable syntax parser: 256 KiB, 30 seconds, 30000 output characters. No type or reference resolution; does not execute the project.",
            object(json!({"path":path}), &["path"]),
        ),
        "symbol_search" => (
            "Find declaration names by literal substring using JS/TS AST: 100 files, 2 MiB total, 256 KiB/file, 100 matches, 30 seconds; incomplete results explicit. Does not find all references.",
            object(
                json!({"path":path,"query":{"type":"string","minLength":1,"maxLength":200},"caseSensitive":boolean}),
                &["query"],
            ),
        ),
        "project_inspect" => (
            "Static inspection: 2000 entries, 500 paths, depth 12, 20 manifests. Returns script/dependency names, 50 configs/tests, 30 lockfiles, 100 paths; 15 seconds and 30000 output characters. Never executes project configuration.",
            object(json!({"path":path}), &[]),
        ),
        "test_discover" => (
            "Static test conventions for JS/TS, Python and Go: 100 tests, 20 manifests, 30 configurations, 2000 entries, depth 12; 15 seconds, 30000 output characters. Does not execute tests or enumerate dynamic cases.",
            object(json!({"path":path}), &[]),
        ),
        "environment_check" => (
            "Passive server runtime and declared sandbox configuration only. Does not execute commands, reveal environment values or verify Docker/image dependencies.",
            object(json!({}), &[]),
        ),
        "git_status" => (
            "Repository branch, ahead/behind and staged/unstaged states. Protected Git executor, no project execution. Sensitive paths are excluded.",
            object(json!({}), &[]),
        ),
        "git_diff" => (
            "Working-tree or staged diff of up to 1000 safe currently tracked files. At most 20000 characters; no external diff, textconv, submodules or renames.",
            object(json!({"path":path,"staged":boolean}), &[]),
        ),
        "git_log" => (
            "Recent commits with hash/date/author/subject, default 20, maximum 100. Restricted to up to 1000 safe currently tracked paths.",
            object(json!({"path":path,"limit":integer(1,100)}), &[]),
        ),
        "git_show" => (
            "Commit metadata and patch, 20000 characters, up to 1000 safe currently tracked paths. revision: hex hash or HEAD/HEAD~N (N up to 999); no historical paths absent from current index, external diff, textconv or renames.",
            object(json!({"path":path,"revision":string(40)}), &[]),
        ),
        "git_blame" => (
            "Commit authorship for a safe tracked file: one-based startLine, default 100/max 200 lines, 1 MiB historical file, 20000 output characters. No textconv; rejects blame.ignoreRevsFile and external mailmaps.",
            object(
                json!({"path":path,"revision":string(40),"startLine":integer(1,1000000),"limit":integer(1,200)}),
                &["path"],
            ),
        ),
        "diff_summary" => (
            "Numeric safe tracked working-tree/staged diff. Count up to 1000 files, return 100; additions/deletions/binary totals, 30000 output characters. No untracked, renames, external diff or textconv.",
            object(json!({"path":path,"staged":boolean}), &[]),
        ),
        "todo_write" => (
            "Replace the complete in-memory todo list for this session. pending, in_progress or completed. No project files are modified; state disappears when the registry closes. Input 128 KiB/output 256 KiB.",
            object(
                json!({"todos":{"type":"array","items":object(json!({"content":{"type":"string"},"status":{"enum":["pending","in_progress","completed"]}}),&["content","status"])}}),
                &["todos"],
            ),
        ),
        _ => ("Unavailable tool", object(json!({}), &[])),
    }
}
pub(crate) fn validate_value(value: &Value, schema: &Value) -> Result<()> {
    if let Some(choices) = schema.get("enum").and_then(Value::as_array)
        && !choices.contains(value)
    {
        return Err(Error::InvalidInput);
    }
    match schema["type"].as_str() {
        Some("object") => {
            let map = value.as_object().ok_or(Error::InvalidInput)?;
            let props = schema["properties"]
                .as_object()
                .ok_or(Error::InvalidInput)?;
            for key in schema["required"].as_array().ok_or(Error::InvalidInput)? {
                if !map.contains_key(key.as_str().ok_or(Error::InvalidInput)?) {
                    return Err(Error::InvalidInput);
                }
            }
            for (k, v) in map {
                validate_value(v, props.get(k).ok_or(Error::InvalidInput)?)?;
            }
        }
        Some("string") => {
            let s = value.as_str().ok_or(Error::InvalidInput)?;
            let len = crate::io::units(s) as u64;
            if len < schema["minLength"].as_u64().unwrap_or(0)
                || len > schema["maxLength"].as_u64().unwrap_or(u64::MAX)
                || s.contains('\0')
            {
                return Err(Error::InvalidInput);
            }
        }
        Some("integer") => {
            let n = value.as_u64().ok_or(Error::InvalidInput)?;
            if n < schema["minimum"].as_u64().unwrap_or(0)
                || n > schema["maximum"].as_u64().unwrap_or(u64::MAX)
            {
                return Err(Error::InvalidInput);
            }
        }
        Some("boolean") => {
            if !value.is_boolean() {
                return Err(Error::InvalidInput);
            }
        }
        Some("array") => {
            let a = value.as_array().ok_or(Error::InvalidInput)?;
            if (a.len() as u64) < schema["minItems"].as_u64().unwrap_or(0)
                || a.len() as u64 > schema["maxItems"].as_u64().unwrap_or(u64::MAX)
            {
                return Err(Error::InvalidInput);
            }
            for v in a {
                validate_value(v, &schema["items"])?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn validate(name: &str, args: &Map<String, Value>) -> Result<()> {
    if !TOOL_NAMES.contains(&name) {
        return Err(Error::Unavailable);
    }
    if serde_json::to_vec(args)
        .map_err(|_| Error::InvalidInput)?
        .len()
        > ToolLimits::default().input_bytes
    {
        return Err(Error::InputLimit);
    }
    validate_value(&Value::Object(args.clone()), &schema(name).1)?;
    if name == "symbol_search" && text(args, "query", "").trim().is_empty() {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
pub(crate) fn text<'a>(a: &'a Map<String, Value>, k: &str, default: &'a str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or(default)
}
pub(crate) fn number(a: &Map<String, Value>, k: &str, default: usize) -> usize {
    a.get(k)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(default)
}
pub(crate) fn flag(a: &Map<String, Value>, k: &str, default: bool) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(default)
}
pub(crate) fn limit_output(value: Value) -> Result<Value> {
    bounded(value, 30000)
}
