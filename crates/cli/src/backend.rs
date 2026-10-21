use myagent_core::{
    Agent, AgentError, BoxFuture, CancellationToken, GenerationInput, Result, SharedJournal,
    ToolContext, guard, token_usage,
};
use myagent_protocol::{Message, Mode, Session, StreamEvent};
use myagent_providers::{HttpProvider, ModelRegistry, ProviderKind};
use myagent_sandbox::{DockerConfig, NativeDocker};
use myagent_store::{Store, StoreConfig, StoreJournal, Workspace};
use myagent_tools::{
    ApprovalDecision, ApprovalManager, CheckpointStore, ExecutionRegistry, FilesystemInstructions,
    GitSettings, MutationRegistry, SafeWorkspace, ToolRegistry, WebClient, WebRegistry,
};
use myagent_tui::{
    approval::Decision,
    ports::{Backend, ModelChoice, SessionView, Submission},
};
use serde_json::json;
use std::{path::Path, sync::Arc};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct LocalBackend {
    store: Store,
    workspace: Workspace,
    models: ModelRegistry,
    tools: ToolRegistry,
    mutations: MutationRegistry,
    approvals: ApprovalManager,
    journal: SharedJournal,
    demo: bool,
    pub sandbox_available: bool,
}
fn persistence(error: myagent_store::Error) -> AgentError {
    match error {
        myagent_store::Error::SessionBusy | myagent_store::Error::GenerationActive => {
            AgentError::Busy
        }
        myagent_store::Error::LeaseLost => AgentError::LeaseLost,
        _ => AgentError::Persistence,
    }
}
impl LocalBackend {
    pub async fn open(path: &Path, config: StoreConfig, demo: bool) -> Result<Self> {
        let workspace = Workspace::resolve(path).map_err(persistence)?;
        let safe = if demo {
            SafeWorkspace::new(Path::new(workspace.path()))?
        } else if let Some(roots) = std::env::var_os("MYAGENT_WORKSPACE_ROOTS") {
            let roots = std::env::split_paths(&roots).collect::<Vec<_>>();
            SafeWorkspace::authorize(Path::new(workspace.path()), &roots)?
        } else {
            SafeWorkspace::new(Path::new(workspace.path()))?
        };
        let models = if demo {
            ModelRegistry::default()
        } else {
            ModelRegistry::load(&config.home.join("models.json"))?
        };
        let checkpoints = CheckpointStore::from_home(&config.home);
        let store = Store::open(config).map_err(persistence)?;
        store
            .recover_orphaned_generations(&workspace)
            .map_err(persistence)?;
        let journal: SharedJournal = Arc::new(StoreJournal::new(store.clone(), workspace.clone()));
        let approvals = ApprovalManager::default();
        let mutations = MutationRegistry::new(
            safe.clone(),
            checkpoints,
            approvals.clone(),
            journal.clone(),
            if demo {
                GitSettings::default()
            } else {
                GitSettings::from_environment()
            },
        );
        let execution = if demo {
            ExecutionRegistry::new(safe.clone(), None)
        } else {
            ExecutionRegistry::detect(
                safe.clone(),
                DockerConfig::from_lookup(|name| std::env::var(name).ok())
                    .map_err(|_| AgentError::Configuration)?,
                Arc::new(NativeDocker),
                CancellationToken::new(),
            )
            .await?
        }
        .with_approvals(approvals.clone());
        let sandbox_available = execution.available();
        let web = WebRegistry::new(
            WebClient::from_lookup(|name| if demo { None } else { std::env::var(name).ok() }),
            approvals.clone(),
        );
        let tools = ToolRegistry::new(safe, sandbox_available)
            .with_mutations(mutations.clone())
            .with_execution(execution)
            .with_web(web);
        Ok(Self {
            store,
            workspace,
            models,
            tools,
            mutations,
            approvals,
            journal,
            demo,
            sandbox_available,
        })
    }
    pub fn workspace(&self) -> &str {
        self.workspace.path()
    }
    pub fn valid_model(&self, id: &str) -> bool {
        self.models.find(id).is_some()
    }
    pub async fn initial(&self, fresh: bool) -> Result<SessionView> {
        if !fresh && let Some(session) = self.sessions().await?.first() {
            return self.load_session(session.id.clone()).await;
        }
        self.new_session().await
    }
    fn view(&self, id: &str) -> Result<SessionView> {
        let session = self
            .store
            .load_session(&self.workspace, id)
            .map_err(persistence)?
            .ok_or(AgentError::Persistence)?;
        let mut messages: Vec<Message> = Vec::new();
        let mut offset = 0;
        let mut measured: Option<u64> = None;
        let mut unreported = 0;
        // Read bounded pages and retain only the most recent messages for drawing.
        // Usage is summed over the entire persisted session, independently of the viewport.
        loop {
            let page = self
                .store
                .list_messages(&self.workspace, id, 500, offset)
                .map_err(persistence)?;
            let count = page.len();
            let usage = token_usage(&page);
            if let Some(tokens) = usage.measured_tokens {
                measured = Some(measured.unwrap_or(0).saturating_add(tokens));
            }
            unreported += usage.unreported_messages;
            messages.extend(page);
            if messages.len() > 500 {
                messages.drain(..messages.len() - 500);
            }
            if count < 500 {
                break;
            }
            offset = offset.checked_add(500).ok_or(AgentError::Limit)?;
        }
        let model = messages
            .iter()
            .rev()
            .map(|message| &message.model)
            .find(|model| self.valid_model(model))
            .cloned()
            .unwrap_or_else(|| ModelRegistry::DEFAULT_MODEL.into());
        let mode = messages
            .iter()
            .rev()
            .map(|message| message.mode)
            .next()
            .unwrap_or(Mode::Build);
        Ok(SessionView {
            session,
            messages,
            model,
            mode,
            measured_tokens: measured,
            unreported,
        })
    }
}
impl Backend for LocalBackend {
    fn models(&self) -> Vec<ModelChoice> {
        self.models
            .models()
            .iter()
            .map(|model| ModelChoice {
                id: model.id.clone(),
                label: model
                    .display_name
                    .clone()
                    .unwrap_or_else(|| model.id.clone()),
                configured: self.demo
                    || model.provider == ProviderKind::Ollama
                    || model
                        .api_key_env()
                        .and_then(|name| std::env::var(name).ok())
                        .is_some_and(|key| !key.trim().is_empty()),
            })
            .collect()
    }
    fn sessions(&self) -> BoxFuture<'_, Result<Vec<Session>>> {
        let this = self.clone();
        Box::pin(myagent_core::blocking(move || {
            this.store
                .list_sessions(&this.workspace, 500, 0)
                .map_err(persistence)
        }))
    }
    fn new_session(&self) -> BoxFuture<'_, Result<SessionView>> {
        let this = self.clone();
        Box::pin(myagent_core::blocking(move || {
            let session = this
                .store
                .create_session(&this.workspace)
                .map_err(persistence)?;
            this.view(&session.id)
        }))
    }
    fn load_session(&self, id: String) -> BoxFuture<'_, Result<SessionView>> {
        let this = self.clone();
        Box::pin(myagent_core::blocking(move || this.view(&id)))
    }
    fn generate(
        &self,
        input: Submission,
        cancel: CancellationToken,
        events: mpsc::Sender<StreamEvent>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let model = self
                .models
                .find(&input.model)
                .ok_or(AgentError::Configuration)?;
            let provider: Arc<dyn myagent_core::Provider> = if self.demo {
                Arc::new(crate::demo::DemoProvider)
            } else {
                Arc::new(HttpProvider::resolve(model, |name| {
                    std::env::var(name).ok()
                })?)
            };
            let window = self.models.resolve_window(&input.model, None)?;
            let agent = Agent::new(
                provider,
                self.journal.clone(),
                self.tools.tools(Mode::Build),
                Some(Arc::new(FilesystemInstructions::new(Path::new(
                    self.workspace.path(),
                ))?)),
            )?;
            agent
                .generate(
                    GenerationInput {
                        session_id: input.session,
                        model: input.model,
                        mode: input.mode,
                        resume: input.content.is_none(),
                        content: input.content,
                        context_window: window.tokens,
                        context_estimated: window.estimated,
                        max_tool_calls: guard::configured_max_tool_calls(|name| {
                            if self.demo {
                                None
                            } else {
                                std::env::var(name).ok()
                            }
                        }),
                        cancellation: cancel,
                    },
                    events,
                )
                .await
                .map(|_| ())
        })
    }
    fn undo(
        &self,
        session: String,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<SessionView>> {
        Box::pin(async move {
            self.mutations
                .try_local_command(&session, "/undo", cancel)
                .await?;
            self.load_session(session).await
        })
    }
    fn approve(&self, session: &str, id: &str, choice: Decision) -> bool {
        self.approvals.decide(
            session,
            id,
            match choice {
                Decision::Once => ApprovalDecision::Once,
                Decision::Session => ApprovalDecision::Session,
                Decision::Deny => ApprovalDecision::Deny,
                Decision::ConfirmMain => ApprovalDecision::ConfirmMain,
            },
        )
    }
    fn forget(&self, session: &str) {
        let _ = self.tools.clear_session(session);
        self.approvals.forget(session);
        let _ = self.store.release_lease(&self.workspace, session);
    }
    fn file_candidates(&self, prefix: String) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async move {
            let result = self
                .tools
                .call(
                    "glob",
                    json!({"pattern":"**/*"}),
                    ToolContext {
                        session_id: "file_completion".into(),
                        tool_call_id: "file_completion".into(),
                        cancellation: CancellationToken::new(),
                        events: None,
                    },
                )
                .await?;
            Ok(result["matches"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|path| path.as_str())
                .filter(|path| path.contains(&prefix))
                .take(8)
                .map(str::to_owned)
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> (tempfile::TempDir, LocalBackend) {
        let temp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        let root = base.join("workspace");
        std::fs::create_dir(&root).unwrap();
        let backend = LocalBackend::open(
            &root,
            StoreConfig {
                home: base.join("state"),
                retention_days: 0,
            },
            true,
        )
        .await
        .unwrap();
        (temp, backend)
    }
    fn submission(session: String, mode: Mode) -> Submission {
        Submission {
            session,
            model: ModelRegistry::DEFAULT_MODEL.into(),
            mode,
            content: Some("demonstrar arquivos".into()),
        }
    }
    #[tokio::test]
    async fn demo_uses_real_approvals_journal_token_totals_and_local_undo() {
        let (_temp, backend) = fixture().await;
        let view = backend.new_session().await.unwrap();
        let session = view.session.id;
        let worker = backend.clone();
        let input = submission(session.clone(), Mode::Build);
        let (sender, mut receiver) = mpsc::channel(64);
        let job = tokio::spawn(async move {
            worker
                .generate(input, CancellationToken::new(), sender)
                .await
        });
        let mut approvals = 0;
        let mut previews = 0;
        while let Some(event) = receiver.recv().await {
            match event {
                StreamEvent::ToolPreview { .. } => previews += 1,
                StreamEvent::ApprovalRequired {
                    approval_id, args, ..
                } => {
                    let path = args["path"].as_str().unwrap();
                    assert!(!Path::new(backend.workspace()).join(path).exists());
                    assert!(!backend.approve("other_session", &approval_id, Decision::Once));
                    assert!(backend.approve(&session, &approval_id, Decision::Once));
                    approvals += 1;
                }
                _ => {}
            }
        }
        job.await.unwrap().unwrap();
        assert_eq!(approvals, 2);
        assert!(previews > 2);
        let view = backend.load_session(session.clone()).await.unwrap();
        assert_eq!(view.measured_tokens, Some(3600));
        assert_eq!(view.unreported, 0);
        assert_eq!(
            backend.file_candidates("demo".into()).await.unwrap().len(),
            2
        );
        assert!(
            Path::new(backend.workspace())
                .join("tests/demo_test.rs")
                .exists()
        );
        backend
            .undo(session.clone(), CancellationToken::new())
            .await
            .unwrap();
        assert!(
            !Path::new(backend.workspace())
                .join("tests/demo_test.rs")
                .exists()
        );
        assert!(Path::new(backend.workspace()).join("demo.rs").exists());
        let other_root = Path::new(backend.workspace())
            .parent()
            .unwrap()
            .join("other");
        std::fs::create_dir(&other_root).unwrap();
        let other = Workspace::resolve(other_root).unwrap();
        assert!(
            backend
                .store
                .list_sessions(&other, 10, 0)
                .unwrap()
                .is_empty()
        );
        backend.forget(&session);
    }
    #[tokio::test]
    async fn cancelling_approval_persists_interruption_without_creating_file() {
        let (_temp, backend) = fixture().await;
        let session = backend.new_session().await.unwrap().session.id;
        let worker = backend.clone();
        let input = submission(session.clone(), Mode::Build);
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (sender, mut receiver) = mpsc::channel(64);
        let job = tokio::spawn(async move { worker.generate(input, token, sender).await });
        while let Some(event) = receiver.recv().await {
            if matches!(event, StreamEvent::ApprovalRequired { .. }) {
                cancel.cancel();
            }
        }
        assert_eq!(job.await.unwrap().unwrap_err(), AgentError::Cancelled);
        assert!(!Path::new(backend.workspace()).join("demo.rs").exists());
        let view = backend.load_session(session.clone()).await.unwrap();
        assert_eq!(
            view.messages.last().unwrap().status,
            myagent_protocol::MessageStatus::Interrupted
        );
        backend.forget(&session);
    }
    #[tokio::test]
    async fn plan_demo_never_offers_or_executes_mutations() {
        let (_temp, backend) = fixture().await;
        let session = backend.new_session().await.unwrap().session.id;
        let (sender, mut receiver) = mpsc::channel(64);
        let input = submission(session.clone(), Mode::Plan);
        let (result, ()) = tokio::join!(
            backend.generate(input, CancellationToken::new(), sender),
            async {
                while let Some(event) = receiver.recv().await {
                    assert!(!matches!(
                        event,
                        StreamEvent::ToolCall { .. } | StreamEvent::ApprovalRequired { .. }
                    ));
                }
            }
        );
        result.unwrap();
        assert_eq!(
            backend.load_session(session.clone()).await.unwrap().mode,
            Mode::Plan
        );
        assert!(!Path::new(backend.workspace()).join("demo.rs").exists());
        backend.forget(&session);
    }
}
