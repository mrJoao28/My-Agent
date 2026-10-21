use crate::{
    context,
    guard::{self, ToolGuard},
    instructions::{InstructionSource, ProjectInstructions},
    ports::blocking,
    *,
};
use myagent_protocol::{
    ExecutionState, MessagePart, MessageStatus, StreamEvent, Validate, validate_id,
    validate_user_content,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};

#[derive(Clone)]
pub struct GenerationInput {
    pub session_id: String,
    pub model: String,
    pub mode: Mode,
    pub content: Option<String>,
    pub resume: bool,
    pub context_window: u32,
    pub context_estimated: bool,
    pub max_tool_calls: u32,
    pub cancellation: CancellationToken,
}
#[derive(Clone, Debug)]
pub struct GenerationResult {
    pub message_id: String,
    pub total_tokens: Option<u32>,
    pub tool_calls: u32,
}
#[derive(Clone)]
pub struct Agent {
    provider: Arc<dyn Provider>,
    journal: SharedJournal,
    tools: Arc<HashMap<String, Arc<dyn Tool>>>,
    instructions: Option<Arc<dyn InstructionSource>>,
}
struct Reservation(String);
static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
impl Reservation {
    fn acquire(id: &str) -> Result<Self> {
        if !ACTIVE
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| AgentError::Busy)?
            .insert(id.into())
        {
            return Err(AgentError::Busy);
        }
        Ok(Self(id.into()))
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(active) = ACTIVE.get()
            && let Ok(mut active) = active.lock()
        {
            active.remove(&self.0);
        }
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
pub(crate) fn mutating_name(name: &str) -> bool {
    matches!(
        name,
        "write_file"
            | "edit_file"
            | "delete_file"
            | "move_file"
            | "apply_patch"
            | "git_commit"
            | "git_init"
            | "git_switch"
            | "git_remote_add"
            | "git_push"
            | "git_sync"
            | "bash"
    )
}
impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        journal: SharedJournal,
        tools: Vec<Arc<dyn Tool>>,
        instructions: Option<Arc<dyn InstructionSource>>,
    ) -> Result<Self> {
        let mut registry = HashMap::new();
        for tool in tools {
            let spec = tool.spec();
            validate_id(&spec.name).map_err(|_| AgentError::Configuration)?;
            if spec.description.len() > 4096
                || !spec.parameters.is_object()
                || serde_json::to_vec(&spec.parameters)
                    .map_err(|_| AgentError::Configuration)?
                    .len()
                    > 65536
                || registry.insert(spec.name, tool).is_some()
            {
                return Err(AgentError::Configuration);
            }
        }
        Ok(Self {
            provider,
            journal,
            tools: Arc::new(registry),
            instructions,
        })
    }
    /// O worker sobrevive ao abandono do consumidor para persistir INTERRUPTED.
    pub async fn generate(
        &self,
        mut input: GenerationInput,
        events: mpsc::Sender<StreamEvent>,
    ) -> Result<GenerationResult> {
        let token = input.cancellation.child_token();
        input.cancellation = token.clone();
        let _cancel = CancelOnDrop(token);
        let agent = self.clone();
        tokio::spawn(async move { agent.drive(input, events).await })
            .await
            .map_err(|_| AgentError::Persistence)?
    }
    async fn drive(
        &self,
        input: GenerationInput,
        events: mpsc::Sender<StreamEvent>,
    ) -> Result<GenerationResult> {
        validate_id(&input.session_id).map_err(|_| AgentError::Configuration)?;
        if input.model.is_empty()
            || input.model.len() > 200
            || input.resume == input.content.is_some()
        {
            return Err(AgentError::Configuration);
        }
        if let Some(content) = &input.content {
            validate_user_content(content).map_err(|_| AgentError::Configuration)?;
        }
        let _reservation = Reservation::acquire(&input.session_id)?;
        let mut guard = ToolGuard::new(input.max_tool_calls)?;
        if input.cancellation.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let start = Instant::now();
        let deadline = start + guard::GENERATION_TIMEOUT;
        let mut tools: Vec<_> = self
            .tools
            .values()
            .filter_map(|tool| {
                let spec = tool.spec();
                let allowed = match input.mode {
                    Mode::Plan => spec.effect != Effect::Write && !mutating_name(&spec.name),
                    Mode::Build => true,
                    _ => false,
                };
                let essential = [
                    "read_file",
                    "read_files",
                    "list_directory",
                    "glob",
                    "grep",
                    "edit_file",
                    "write_file",
                    "apply_patch",
                    "git_status",
                    "git_diff",
                    "git_commit",
                    "bash",
                ];
                (allowed
                    && (input.context_window >= 32768 || essential.contains(&spec.name.as_str())))
                .then_some(spec)
            })
            .collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        let instructions = self
            .instructions
            .as_ref()
            .map(|source| Arc::new(Mutex::new(ProjectInstructions::new(source.clone()))));
        let root = if let Some(snapshot) = &instructions {
            let snapshot = snapshot.clone();
            blocking(move || {
                snapshot
                    .lock()
                    .map_err(|_| AgentError::Configuration)?
                    .for_paths(&[])
            })
            .await?
        } else {
            String::new()
        };
        let system = prompts::system(
            input.mode,
            &tools.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
            &root,
            input.context_window,
            input.context_estimated,
            input.max_tool_calls,
        );
        let schemas=tools.iter().map(|s|serde_json::json!({"name":s.name,"description":s.description,"parameters":s.parameters})).collect::<Vec<_>>();
        let (limit, output) = context::budget(input.context_window, &system, &schemas)?;
        if let Some(content) = &input.content {
            context::fit(vec![ModelMessage::text(ModelRole::User, content)], limit)?;
        }
        let journal = self.journal.clone();
        let prepare = PrepareGeneration {
            session_id: input.session_id.clone(),
            model: input.model.clone(),
            mode: input.mode,
            content: input.content.clone(),
            resume: input.resume,
        };
        let state = blocking(move || journal.prepare(prepare)).await?;
        guard.restore(&state.previous);
        let mut usage = state.message.total_tokens;
        let token = input.cancellation.clone();
        let tasks = Arc::new(Mutex::new(Vec::new()));
        let lease_lost = Arc::new(AtomicBool::new(false));
        let journal = self.journal.clone();
        let session = input.session_id.clone();
        let watch_cancel = token.clone();
        let watch_lost = lease_lost.clone();
        let monitor = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            tick.tick().await;
            loop {
                tokio::select! {biased;
                    _=watch_cancel.cancelled()=>break,
                    _=tick.tick()=>{
                        let journal=journal.clone();let session=session.clone();
                        if blocking(move||journal.check_lease(&session)).await.is_err(){
                            watch_lost.store(true,Ordering::Release);watch_cancel.cancel();break;
                        }
                    }
                }
            }
        });
        tasks
            .lock()
            .map_err(|_| AgentError::Persistence)?
            .push(monitor);

        let result = tokio::select! {biased;
            _=token.cancelled()=>Err(if lease_lost.load(Ordering::Acquire){AgentError::LeaseLost}else{AgentError::Cancelled}),
            _=tokio::time::sleep_until(deadline)=>{token.cancel();Err(AgentError::Timeout)},
            result=self.run(&input,&state,&events,&mut guard,&mut usage,RunContext{root,system,tools,limit,output,instructions,tasks:tasks.clone()})=>result,
        };
        token.cancel();
        settle_tasks(tasks).await?;
        let duration = u32::try_from(start.elapsed().as_secs()).unwrap_or(u32::MAX);
        let journal = self.journal.clone();
        let id = state.message.id.clone();
        let tokens = usage;
        let status = if result.is_ok() {
            MessageStatus::Complete
        } else {
            MessageStatus::Interrupted
        };
        token.cancel(); // fecha rede/processos ainda associados à geração.
        let result = match blocking(move || journal.finish(&id, status, duration, tokens)).await {
            Ok(()) => result,
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => {
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    events.send(StreamEvent::Done {
                        message_id: state.message.id.clone(),
                        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                    }),
                )
                .await;
                Ok(GenerationResult {
                    message_id: state.message.id,
                    total_tokens: usage,
                    tool_calls: guard.used,
                })
            }
            Err(error) => {
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    events.send(StreamEvent::Error {
                        kind: if error == AgentError::Context {
                            Some(myagent_protocol::StreamErrorKind::Context)
                        } else {
                            None
                        },
                        message: error.to_string(),
                        message_id: Some(state.message.id),
                        parts: None,
                    }),
                )
                .await;
                Err(error)
            }
        }
    }
    async fn run(
        &self,
        input: &GenerationInput,
        state: &GenerationState,
        events: &mpsc::Sender<StreamEvent>,
        guard: &mut ToolGuard,
        usage: &mut Option<u32>,
        mut plan: RunContext,
    ) -> Result<()> {
        emit(
            events,
            StreamEvent::GenerationStarted {
                message_id: state.message.id.clone(),
            },
            &input.cancellation,
        )
        .await?;
        if input.context_window < 32768 {
            self.context_warning(input,state,events,&plan,"Perfil compacto: somente ferramentas essenciais; janela e orçamento estimados explicitamente.").await?;
        }
        let mut history = state.history.clone();
        let mut warned = false;
        loop {
            if input.cancellation.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            let journal = self.journal.clone();
            let session = input.session_id.clone();
            blocking(move || journal.check_lease(&session)).await?;
            if guard.synthesis() {
                plan.tools.clear();
                if !warned {
                    warned = true;
                    self.append(
                        &state.message.id,
                        MessagePart::ToolLimitWarning {
                            limit: guard.limit as u16,
                        },
                    )
                    .await?;
                    emit(
                        events,
                        StreamEvent::ToolLimitWarning {
                            limit: guard.limit as u16,
                        },
                        &input.cancellation,
                    )
                    .await?;
                    plan.system = prompts::system(
                        input.mode,
                        &[],
                        &plan.root,
                        input.context_window,
                        input.context_estimated,
                        guard.limit,
                    );
                    plan.system.push('\n');
                    plan.system.push_str(prompts::SYNTHESIS);
                    plan.limit = plan
                        .limit
                        .min(context::budget(input.context_window, &plan.system, &[])?.0);
                }
            }
            let (bounded, truncated) = context::fit(history, plan.limit)?;
            if truncated {
                self.context_warning(input,state,events,&plan,"Contexto anterior compactado ou abreviado. Histórico completo permanece salvo; não repetir efeitos para reconstruí-lo.").await?;
            }
            let estimated = context::estimate(&bounded);
            let journal = self.journal.clone();
            let session = input.session_id.clone();
            let limit = plan.limit;
            history =
                blocking(move || journal.context(&session, bounded, limit, estimated, truncated))
                    .await?;
            let writes = self
                .tools
                .values()
                .filter_map(|tool| {
                    let spec = tool.spec();
                    (spec.effect == Effect::Write).then_some(spec.name)
                })
                .collect();
            let mut cached = context::evidence(&history, &writes);
            let request = ProviderRequest {
                model: input.model.clone(),
                system: plan.system.clone(),
                messages: history.clone(),
                tools: plan.tools.clone(),
                max_output_tokens: plan.output,
            };
            let (sender, mut receiver) = mpsc::channel(32);
            let provider = self.provider.clone();
            let cancel = input.cancellation.clone();
            let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let result = provider.stream(request, cancel, sender).await;
                let _ = finished_tx.send(result);
            });
            plan.tasks
                .lock()
                .map_err(|_| AgentError::Persistence)?
                .push(task);
            let mut calls = Vec::new();
            let mut parts = Vec::new();
            let mut metadata = None;
            let base = *usage;
            let mut done = false;
            while let Some(event) = receiver.recv().await {
                if done {
                    return Err(AgentError::Protocol);
                }
                let reasoning = matches!(&event, ProviderEvent::Reasoning(_));
                match event {
                    ProviderEvent::Text(text) | ProviderEvent::Reasoning(text) => {
                        let part = if reasoning {
                            MessagePart::Reasoning { text: text.clone() }
                        } else {
                            MessagePart::Text { text: text.clone() }
                        };
                        part.validate().map_err(|_| AgentError::Limit)?;
                        self.append(&state.message.id, part.clone()).await?;
                        context::push_part(&mut parts, part);
                        emit(
                            events,
                            if reasoning {
                                StreamEvent::ReasoningDelta { text }
                            } else {
                                StreamEvent::TextDelta { text }
                            },
                            &input.cancellation,
                        )
                        .await?;
                    }
                    ProviderEvent::ToolCall(call) => {
                        if calls.len() >= guard.limit.saturating_sub(guard.used) as usize {
                            return Err(AgentError::Limit);
                        }
                        if plan.tools.iter().all(|s| s.name != call.name) {
                            return Err(AgentError::ToolDenied);
                        }
                        if calls.iter().any(|old: &ToolCall| old.id == call.id) {
                            return Err(AgentError::Protocol);
                        }
                        let part = MessagePart::ToolCall {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            args: call.args.clone(),
                            execution: Some(ExecutionState::Pending),
                        };
                        part.validate().map_err(|_| AgentError::Protocol)?;
                        self.append(&state.message.id, part.clone()).await?;
                        parts.push(part);
                        emit(
                            events,
                            StreamEvent::ToolCall {
                                tool_call_id: call.id.clone(),
                                tool_name: call.name.clone(),
                                args: call.args.clone(),
                            },
                            &input.cancellation,
                        )
                        .await?;
                        calls.push(call);
                    }
                    ProviderEvent::Usage(reported) => {
                        if reported > i32::MAX as u32 {
                            return Err(AgentError::Protocol);
                        }
                        *usage = Some(
                            base.unwrap_or(0)
                                .saturating_add(reported)
                                .min(i32::MAX as u32),
                        );
                        let journal = self.journal.clone();
                        let id = state.message.id.clone();
                        let total = *usage;
                        blocking(move || journal.usage(&id, total)).await?;
                        emit(
                            events,
                            StreamEvent::Usage {
                                total_tokens: usage.unwrap_or(0),
                            },
                            &input.cancellation,
                        )
                        .await?;
                    }
                    ProviderEvent::Metadata(value) => metadata = Some(value),
                    ProviderEvent::Done => done = true,
                }
            }
            // O join é obrigatório: canal fechado/EOF sem conclusão não é sucesso.
            finished_rx.await.map_err(|_| AgentError::Network)??;
            if !done {
                return Err(AgentError::Protocol);
            }
            if parts.is_empty() && !calls.is_empty() {
                return Err(AgentError::Protocol);
            }
            history.push(ModelMessage {
                role: ModelRole::Assistant,
                content: parts,
                provider_metadata: metadata,
            });
            if calls.is_empty() {
                return Ok(());
            }
            // A resposta final fica no journal; o contexto contém o snapshot enviado.
            // Em tool-use, signatures e pares pendentes são salvos antes dos efeitos.
            let (snapshot, truncated) = context::fit(history.clone(), plan.limit)?;
            let journal = self.journal.clone();
            let session = input.session_id.clone();
            let limit = plan.limit;
            history = blocking(move || {
                let estimate = context::estimate(&snapshot);
                journal.context(&session, snapshot, limit, estimate, truncated)
            })
            .await?;
            let mut results = Vec::new();
            for call in calls {
                guard.register(&call.name, &call.args)?;
                let tool = self.tools.get(&call.name).ok_or(AgentError::ToolDenied)?;
                let limits = tool.limits();
                if serde_json::to_vec(&call.args)
                    .map_err(|_| AgentError::Protocol)?
                    .len()
                    > limits.input_bytes
                {
                    return Err(AgentError::Limit);
                }
                let context = ToolContext {
                    session_id: input.session_id.clone(),
                    cancellation: input.cancellation.clone(),
                };
                let fingerprint = guard::key(&call.name, &call.args);
                let replay = tool.spec().effect == Effect::Write || mutating_name(&call.name);
                if replay && state.previous.iter().any(|p|matches!(p,MessagePart::ToolCall{name,args,execution,..} if name==&call.name && guard::key(name,args)==fingerprint && *execution!=Some(ExecutionState::Pending))) {return Err(AgentError::Replay);}
                let result = if cached.contains_key(&fingerprint) {
                    "Error: resultado já disponível no contexto; não repita esta chamada."
                        .to_owned()
                } else {
                    tool.authorize(&call.args, context.clone()).await?;
                    let mut note = if let Some(snapshot) = &plan.instructions {
                        let snapshot = snapshot.clone();
                        let paths = instructions::tool_paths(&call.name, &call.args);
                        blocking(move || {
                            snapshot
                                .lock()
                                .map_err(|_| AgentError::Configuration)?
                                .for_paths(&paths)
                        })
                        .await?
                    } else {
                        String::new()
                    };
                    let journal = self.journal.clone();
                    let id = state.message.id.clone();
                    let call_id = call.id.clone();
                    blocking(move || journal.started(&id, &call_id)).await?;
                    if input.cancellation.is_cancelled() {
                        return Err(AgentError::Cancelled);
                    }
                    let executing = tool.clone();
                    let args = call.args.clone();
                    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
                    let task = tokio::spawn(async move {
                        let result = executing.execute(args, context).await;
                        let _ = result_tx.send(result);
                    });
                    plan.tasks
                        .lock()
                        .map_err(|_| AgentError::Persistence)?
                        .push(task);
                    let result = result_rx.await.map_err(|_| AgentError::Tool)?;
                    let mut output = match result {
                        Ok(value) => value,
                        Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                        Err(AgentError::ToolFailure(code)) => format!("Tool failed: {code}"),
                        Err(_) => "Tool failed: execution failed".into(),
                    };
                    if output.len() > limits.output_bytes {
                        return Err(AgentError::Limit);
                    }
                    if matches!(
                        call.name.as_str(),
                        "grep" | "glob" | "symbol_search" | "project_inspect"
                    ) && let Some(snapshot) = &plan.instructions
                    {
                        let paths = instructions::result_paths(&call.name, &call.args, &output);
                        let snapshot = snapshot.clone();
                        let extra = blocking(move || {
                            snapshot
                                .lock()
                                .map_err(|_| AgentError::Configuration)?
                                .for_paths(&paths)
                        })
                        .await?;
                        note.push_str(&extra);
                    }
                    if !note.is_empty() {
                        output.push('\n');
                        output.push_str(&note);
                    }
                    if !output.starts_with("Tool failed:") {
                        cached.insert(fingerprint.clone(), output.clone());
                    }
                    if replay {
                        cached.retain(|key, _| key == &fingerprint);
                    }
                    output
                };
                let part = MessagePart::ToolResult {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    result: result.clone(),
                };
                self.append(&state.message.id, part.clone()).await?;
                emit(
                    events,
                    StreamEvent::ToolResult {
                        tool_call_id: call.id,
                        tool_name: call.name,
                        result,
                    },
                    &input.cancellation,
                )
                .await?;
                results.push(part);
            }
            history.push(ModelMessage {
                role: ModelRole::Tool,
                content: results,
                provider_metadata: None,
            });
        }
    }
    async fn context_warning(
        &self,
        input: &GenerationInput,
        state: &GenerationState,
        events: &mpsc::Sender<StreamEvent>,
        plan: &RunContext,
        message: &str,
    ) -> Result<()> {
        self.append(
            &state.message.id,
            MessagePart::ContextWarning {
                context_window: input.context_window,
                estimated: input.context_estimated,
                tool_count: plan.tools.len() as u32,
                message: message.into(),
            },
        )
        .await?;
        emit(
            events,
            StreamEvent::ContextWarning {
                context_window: input.context_window,
                estimated: input.context_estimated,
                tool_count: plan.tools.len() as u32,
                message: message.into(),
            },
            &input.cancellation,
        )
        .await
    }
    async fn append(&self, id: &str, part: MessagePart) -> Result<()> {
        let journal = self.journal.clone();
        let id = id.to_owned();
        blocking(move || journal.append(&id, part)).await
    }
}
struct RunContext {
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    root: String,
    system: String,
    tools: Vec<ToolSpec>,
    limit: u32,
    output: u32,
    instructions: Option<Arc<Mutex<ProjectInstructions>>>,
}
async fn emit(
    events: &mpsc::Sender<StreamEvent>,
    event: StreamEvent,
    cancel: &CancellationToken,
) -> Result<()> {
    tokio::select! {biased;_=cancel.cancelled()=>Err(AgentError::Cancelled),result=events.send(event)=>result.map_err(|_|AgentError::Cancelled)}
}

async fn settle_tasks(pool: Arc<Mutex<Vec<JoinHandle<()>>>>) -> Result<()> {
    let tasks = {
        let mut tasks = pool.lock().map_err(|_| AgentError::Persistence)?;
        std::mem::take(&mut *tasks)
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    for mut task in tasks {
        if tokio::time::timeout_at(deadline, &mut task).await.is_err() {
            task.abort();
            let _ = task.await;
        }
    }
    Ok(())
}
