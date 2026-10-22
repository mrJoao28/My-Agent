//! Headless commands share the TUI backend, journal, leases and approvals.
use crate::backend::LocalBackend;
use clap::ArgMatches;
use myagent_core::{AgentError, CancellationToken, Result};
use myagent_protocol::{Mode, StreamEvent};
use myagent_store::StoreConfig;
use myagent_tui::{
    approval::Decision,
    ports::{AccountOperation, Backend, Submission},
};
use serde_json::json;
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::mpsc;

pub const SUCCESS: u8 = 0;
pub const FAILURE: u8 = 1;
pub const INVALID_ARGUMENTS: u8 = 2;
pub const DENIED: u8 = 3;
pub const CANCELLED: u8 = 130;
struct Context {
    backend: Arc<LocalBackend>,
    home: PathBuf,
    _temporary: Option<tempfile::TempDir>,
}
async fn context(args: &ArgMatches) -> Result<Context> {
    let temporary = if args.get_flag("demo") {
        Some(
            tempfile::Builder::new()
                .prefix("myagent-exec-")
                .tempdir()
                .map_err(|_| AgentError::Persistence)?,
        )
    } else {
        None
    };
    let (cwd, config) = if let Some(temp) = &temporary {
        let root = std::fs::canonicalize(temp.path()).map_err(|_| AgentError::Configuration)?;
        let cwd = root.join("workspace");
        std::fs::create_dir(&cwd).map_err(|_| AgentError::Persistence)?;
        (
            cwd,
            StoreConfig {
                home: root.join("state"),
                retention_days: 0,
            },
        )
    } else {
        (
            args.get_one::<String>("cwd")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir().map_err(|_| AgentError::Configuration)?),
            StoreConfig::from_env().map_err(|_| AgentError::Configuration)?,
        )
    };
    Ok(Context {
        home: config.home.clone(),
        backend: Arc::new(LocalBackend::open(&cwd, config, temporary.is_some()).await?),
        _temporary: temporary,
    })
}
pub fn run(args: &ArgMatches) -> u8 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return FAILURE,
    };
    runtime.block_on(async {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let signal = tokio::spawn(async move {
            #[cfg(unix)]
            {
                if let Ok(mut term) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                {
                    tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=term.recv()=>{} }
                } else {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            token.cancel();
        });
        let result = match args.subcommand() {
            Some(("exec", options)) => execute(args, options, cancel.clone()).await,
            Some(("doctor", options)) => doctor(args, options, cancel.clone()).await,
            Some(("chatgpt", options)) => accounts(args, options, cancel.clone()).await,
            _ => Ok(INVALID_ARGUMENTS),
        };
        signal.abort();
        match result {
            Ok(code) => code,
            Err(AgentError::Cancelled) => CANCELLED,
            Err(error) => {
                eprintln!("Falha: {error}");
                if cancel.is_cancelled() {
                    CANCELLED
                } else {
                    FAILURE
                }
            }
        }
    })
}
fn permits(policy: &str, name: &str, allow_push: bool, confirmation: bool) -> bool {
    !confirmation
        && (name != "git_push" || allow_push)
        && (policy == "all" || policy == "reads" && matches!(name, "web_fetch" | "web_search"))
}
async fn execute(args: &ArgMatches, options: &ArgMatches, cancel: CancellationToken) -> Result<u8> {
    let prompt = options
        .get_one::<String>("prompt")
        .ok_or(AgentError::Configuration)?;
    if prompt.trim().is_empty()
        || prompt.encode_utf16().count() > myagent_protocol::MAX_MESSAGE_LENGTH
    {
        return Ok(INVALID_ARGUMENTS);
    }
    let ctx = context(args).await?;
    let backend = ctx.backend.clone();
    let view = backend.new_session().await?;
    let model = args
        .get_one::<String>("model")
        .cloned()
        .unwrap_or(view.model);
    let mode = if options
        .get_one::<String>("mode")
        .is_some_and(|s| s == "build")
    {
        Mode::Build
    } else {
        Mode::Plan
    };
    let submission = Submission {
        session: view.session.id.clone(),
        model: model.clone(),
        mode,
        content: Some(prompt.clone()),
    };
    let (tx, mut rx) = mpsc::channel(64);
    let worker = backend.clone();
    let running = cancel.clone();
    let task = tokio::spawn(async move { worker.generate(submission, running, tx).await });
    let jsonl = options.get_flag("json");
    let policy = options
        .get_one::<String>("approve")
        .map(String::as_str)
        .unwrap_or("none");
    let mut denied = false;
    let mut done = false;
    let mut failed = false;
    let mut text = String::new();
    let mut calls = 0usize;
    while let Some(event) = rx.recv().await {
        if jsonl
            && writeln!(
                io::stdout(),
                "{}",
                serde_json::to_string(&event).map_err(|_| AgentError::Protocol)?
            )
            .is_err()
        {
            cancel.cancel();
        }
        match &event {
            StreamEvent::TextDelta { text: delta } => {
                if text.len() + delta.len() > 8 * 1024 * 1024 {
                    failed = true;
                    cancel.cancel();
                } else {
                    text.push_str(delta);
                }
            }
            StreamEvent::ToolCall { .. } => {
                calls += 1;
                text.clear();
            }
            StreamEvent::ApprovalRequired {
                approval_id,
                tool_name,
                confirmation,
                ..
            } => {
                let allowed = permits(
                    policy,
                    tool_name,
                    options.get_flag("allow-push"),
                    confirmation.is_some(),
                );
                denied |= !allowed;
                if !backend.approve(
                    &view.session.id,
                    approval_id,
                    if allowed {
                        Decision::Once
                    } else {
                        Decision::Deny
                    },
                ) {
                    failed = true;
                    cancel.cancel();
                }
            }
            StreamEvent::Done { .. } => done = true,
            StreamEvent::Error { .. } => failed = true,
            _ => {}
        }
    }
    let result = task.await.map_err(|_| AgentError::Cleanup)?;
    let code = if cancel.is_cancelled() {
        CANCELLED
    } else if denied {
        DENIED
    } else if result.is_err() || failed || !done {
        FAILURE
    } else {
        SUCCESS
    };
    let usage = backend.load_session(view.session.id.clone()).await?;
    backend.forget(&view.session.id);
    if jsonl {
        writeln!(io::stdout(), "{}", json!({"type":"exec-result", "exitCode":code,"status":match code {0=>"success",3=>"denied",130=>"cancelled",_=>"failure"},"sessionId":view.session.id,"model":model,"mode":mode,"text":text,"totalTokens":usage.measured_tokens,"toolCalls":calls})).map_err(|_| AgentError::Network)?;
    } else {
        if !text.is_empty() {
            println!("{}", myagent_tui::safe_text(&text));
        }
        if let Err(error) = result {
            eprintln!("Falha: {error}");
        }
    }
    Ok(code)
}
async fn doctor(args: &ArgMatches, options: &ArgMatches, cancel: CancellationToken) -> Result<u8> {
    // Opening/migrating app state is intentional; never runs project scripts.
    let result = context(args).await;
    let mut checks = vec![];
    let mut code = SUCCESS;
    match result {
        Ok(ctx) => {
            match tempfile::NamedTempFile::new_in(&ctx.home) {
                Ok(_) => checks.push(json!({"name":"state-write","status":"ok","detail":"Diretorio de estado gravavel."})),
                Err(_) => {
                    checks.push(json!({"name":"state-write","status":"error","detail":"Diretorio de estado nao gravavel."}));
                    code = FAILURE;
                }
            }
            checks.push(json!({"name":"runtime","status":"ok","detail":format!("My-Agent {} / Rust", env!("CARGO_PKG_VERSION"))}));
            checks.push(json!({"name":"sqlite","status":"ok","detail":"Estado aberto e migrations validadas."}));
            checks.push(json!({"name":"docker","status":if ctx.backend.sandbox_available {"ok"} else {"warning"},"detail":if ctx.backend.sandbox_available {"Sandbox disponivel."} else {"Docker/imagem indisponivel; ferramentas de processo desativadas."}}));
            let models: Vec<_> = ctx
                .backend
                .models()
                .into_iter()
                .map(|m| json!({"id":m.id,"configured":m.configured}))
                .collect();
            checks.push(json!({"name":"models","status":"ok","models":models}));
            let (tx, _rx) = mpsc::channel(1);
            match ctx
                .backend
                .account(AccountOperation::Status, cancel.clone(), tx)
                .await
            {
                Ok(view) => checks.push(json!({"name":"chatgpt","status":"ok","detail":view.note})),
                Err(_) => {
                    checks.push(json!({"name":"chatgpt","status":"error","detail":"Nao foi possivel validar o estado privado de contas."}));
                    code = FAILURE;
                }
            }
        }
        Err(_) => {
            checks.push(json!({"name":"configuration","status":"error","detail":"Falha ao abrir workspace, estado SQLite ou configuracao. Nenhum estado foi apagado."}));
            code = FAILURE;
        }
    }
    if cancel.is_cancelled() {
        code = CANCELLED;
    }
    if options.get_flag("json") {
        println!(
            "{}",
            json!({"schemaVersion":1,"checks":checks,"exitCode":code})
        );
    } else {
        for check in checks {
            println!(
                "{}: {} {}",
                check["name"].as_str().unwrap_or(""),
                check["status"].as_str().unwrap_or(""),
                check.get("detail").and_then(|v| v.as_str()).unwrap_or("")
            );
        }
    }
    Ok(code)
}
async fn accounts(
    args: &ArgMatches,
    options: &ArgMatches,
    cancel: CancellationToken,
) -> Result<u8> {
    let ctx = context(args).await?;
    let op = match options.subcommand() {
        Some(("login", login)) => {
            AccountOperation::Login(login.get_one::<String>("account").cloned())
        }
        Some(("use", item)) => AccountOperation::Activate(
            item.get_one::<String>("account")
                .ok_or(AgentError::Configuration)?
                .clone(),
        ),
        Some(("logout", item)) => AccountOperation::Logout(
            item.get_one::<String>("account")
                .ok_or(AgentError::Configuration)?
                .clone(),
        ),
        _ => AccountOperation::Status,
    };
    let (tx, mut rx) = mpsc::channel(1);
    let backend = ctx.backend.clone();
    let task = tokio::spawn(async move { backend.account(op, cancel, tx).await });
    while let Some(url) = rx.recv().await {
        println!("Continue com ChatGPT no navegador:\n{url}");
    }
    let view = task.await.map_err(|_| AgentError::Cleanup)??;
    println!("{}", myagent_tui::safe_text(&view.note));
    for (_, choice) in view.choices {
        println!("{}", myagent_tui::safe_text(&choice));
    }
    Ok(SUCCESS)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automation_never_confirms_protected_push_or_expands_read_scope() {
        assert!(!permits("all", "git_push", false, false));
        assert!(!permits("all", "git_push", true, true));
        assert!(!permits("reads", "write_file", false, false));
        assert!(permits("reads", "web_fetch", false, false));
        assert!(!permits("none", "web_fetch", false, false));
    }
}
