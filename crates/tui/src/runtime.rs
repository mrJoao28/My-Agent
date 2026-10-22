use crate::{
    app::{Action, App, Screen},
    ports::{AccountView, Backend, ModelChoice, SessionView},
    preferences::Preferences,
    render,
};
use crossterm::event;
use myagent_core::{CancellationToken, Result};
use myagent_protocol::{Session, StreamEvent};
use std::{io, path::PathBuf, sync::Arc, time::Duration};
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};
enum UiEvent {
    Stream(StreamEvent),
    Loaded(Box<SessionView>, Screen, bool),
    Sessions(Vec<Session>),
    Models(Vec<ModelChoice>),
    Account(u64, Result<(AccountView, Vec<ModelChoice>)>),
    LoginUrl(u64, String),
    Finished {
        view: Option<Box<SessionView>>,
        error: Option<String>,
    },
    Error(String),
    Candidates {
        query: String,
        paths: Vec<String>,
    },
    Shutdown,
}
fn error_message(error: myagent_core::AgentError) -> String {
    match error {
        myagent_core::AgentError::PlanPermission => "Uso do plano indisponível. Escolha reautorizar em /chatgpt ou selecionar outro modelo em /models. Nenhuma troca automática foi feita.".into(),
        myagent_core::AgentError::Authentication => {
            "Chave ausente ou recusada. Selecione um modelo configurado em /models.".into()
        }
        myagent_core::AgentError::Cancelled => "Geração interrompida pelo usuário.".into(),
        other => other.to_string(),
    }
}
struct Driver<'a> {
    runtime: &'a Runtime,
    backend: Arc<dyn Backend>,
    sender: mpsc::Sender<UiEvent>,
    cancel: Option<CancellationToken>,
    account_epoch: u64,
    shutdown: CancellationToken,
    jobs: Vec<JoinHandle<()>>,
}
impl Driver<'_> {
    fn action(&mut self, action: Action, app: &mut App) -> bool {
        match action {
            Action::Quit => return false,
            Action::CancelLogin => {
                self.account_epoch = self.account_epoch.wrapping_add(1);
                if let Some(cancel) = self.cancel.take() {
                    cancel.cancel();
                }
            }
            Action::RefreshModels => {
                let backend = self.backend.clone();
                let sender = self.sender.clone();
                let token = self.shutdown.child_token();
                self.jobs.push(self.runtime.spawn(async move {
                    send_result(
                        &sender,
                        backend.refresh_models(token).await.map(UiEvent::Models),
                    )
                    .await;
                }));
            }
            Action::Account(op) => {
                self.account_epoch = self.account_epoch.wrapping_add(1);
                let epoch = self.account_epoch;
                let backend = self.backend.clone();
                let sender = self.sender.clone();
                let token = self.shutdown.child_token();
                self.cancel = Some(token.clone());
                self.jobs.push(self.runtime.spawn(async move {
                    let (urls, mut receiver) = mpsc::channel(1);
                    let task = {
                        let backend = backend.clone();
                        tokio::spawn(async move { backend.account(op, token, urls).await })
                    };
                    while let Some(url) = receiver.recv().await {
                        if sender.send(UiEvent::LoginUrl(epoch, url)).await.is_err() {
                            break;
                        }
                    }
                    let result = task.await.unwrap_or(Err(myagent_core::AgentError::Cleanup));
                    let _ = sender
                        .send(UiEvent::Account(
                            epoch,
                            result.map(|view| (view, backend.models())),
                        ))
                        .await;
                }));
            }
            Action::Cancel => {
                if let Some(cancel) = &self.cancel {
                    cancel.cancel();
                }
            }
            Action::Approve { id, decision } => {
                if !app
                    .session
                    .as_ref()
                    .is_some_and(|session| self.backend.approve(session, &id, decision))
                {
                    app.notify("A aprovação expirou ou já foi consumida.", true);
                }
            }
            Action::Submit(input) => {
                let backend = self.backend.clone();
                let sender = self.sender.clone();
                let cancel = self.shutdown.child_token();
                self.cancel = Some(cancel.clone());
                self.jobs.push(self.runtime.spawn(async move {
                    let (events, mut receiver) = mpsc::channel(64);
                    let worker = backend.clone();
                    let session = input.session.clone();
                    let token = cancel.clone();
                    let generate =
                        tokio::spawn(async move { worker.generate(input, token, events).await });
                    while let Some(event) = receiver.recv().await {
                        if sender.send(UiEvent::Stream(event)).await.is_err() {
                            cancel.cancel();
                            break;
                        }
                    }
                    let result = generate
                        .await
                        .unwrap_or(Err(myagent_core::AgentError::Cleanup));
                    let view = backend.load_session(session).await;
                    let error = result
                        .err()
                        .map(error_message)
                        .or_else(|| view.as_ref().err().cloned().map(error_message));
                    let _ = sender
                        .send(UiEvent::Finished {
                            view: view.ok().map(Box::new),
                            error,
                        })
                        .await;
                }));
            }
            Action::Sessions => {
                let backend = self.backend.clone();
                let sender = self.sender.clone();
                self.jobs.push(self.runtime.spawn(async move {
                    send_result(&sender, backend.sessions().await.map(UiEvent::Sessions)).await;
                }));
            }
            Action::NewSession | Action::LoadSession(_) => {
                let backend = self.backend.clone();
                let sender = self.sender.clone();
                if let Some(session) = &app.session {
                    self.backend.forget(session);
                }
                self.jobs.push(self.runtime.spawn(async move {
                    let result = match action {
                        Action::NewSession => backend
                            .new_session()
                            .await
                            .map(|view| UiEvent::Loaded(Box::new(view), Screen::NewSession, false)),
                        Action::LoadSession(id) => backend
                            .load_session(id)
                            .await
                            .map(|view| UiEvent::Loaded(Box::new(view), Screen::Session, true)),
                        _ => unreachable!(),
                    };
                    send_result(&sender, result).await;
                }));
            }
            Action::Undo(session) => {
                let backend = self.backend.clone();
                let sender = self.sender.clone();
                let token = self.shutdown.child_token();
                self.jobs.push(self.runtime.spawn(async move {
                    send_result(
                        &sender,
                        backend
                            .undo(session, token)
                            .await
                            .map(|view| UiEvent::Loaded(Box::new(view), Screen::Session, false)),
                    )
                    .await;
                }));
            }
            Action::SaveTheme(theme) => {
                let _ = theme;
            }
            Action::SuggestFiles(query) => {
                if self.jobs.len() >= 16 {
                    return true;
                }
                let backend = self.backend.clone();
                let sender = self.sender.clone();
                self.jobs.push(self.runtime.spawn(async move {
                    if let Ok(paths) = backend.file_candidates(query.clone()).await {
                        let _ = sender.send(UiEvent::Candidates { query, paths }).await;
                    }
                }));
            }
        }
        true
    }
}
async fn send_result(sender: &mpsc::Sender<UiEvent>, result: Result<UiEvent>) {
    let event = result.unwrap_or_else(|error| UiEvent::Error(error_message(error)));
    let _ = sender.send(event).await;
}
pub fn run(
    mut app: App,
    backend: Arc<dyn Backend>,
    preferences: PathBuf,
    runtime: &Runtime,
) -> io::Result<()> {
    let (_guard, mut terminal) = crate::terminal::TerminalGuard::enter()?;
    let (sender, mut receiver) = mpsc::channel(128);
    let shutdown = CancellationToken::new();
    let signal_sender = sender.clone();
    let signal = runtime.spawn(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
            if let Some(signal) = &mut term {
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=signal.recv()=>{}}
            } else {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = signal_sender.send(UiEvent::Shutdown).await;
    });
    let mut driver = Driver {
        runtime,
        backend,
        sender,
        cancel: None,
        account_epoch: 0,
        shutdown: shutdown.clone(),
        jobs: vec![],
    };
    let result = (|| {
        loop {
            for _ in 0..128 {
                let Ok(event) = receiver.try_recv() else {
                    break;
                };
                match event {
                    UiEvent::Stream(event) => app.stream(event),
                    UiEvent::Loaded(view, screen, resume) => app.load(*view, screen, resume),
                    UiEvent::Sessions(sessions) => app.sessions(sessions),
                    UiEvent::Account(epoch, result) => {
                        if epoch == driver.account_epoch {
                            driver.cancel = None;
                            app.loading = false;
                            match result {
                                Ok((view, models)) => {
                                    app.models = models;
                                    app.account_view(view);
                                }
                                Err(error) => app.notify(error_message(error), true),
                            }
                        }
                    }
                    UiEvent::LoginUrl(epoch, url) => {
                        if epoch == driver.account_epoch
                            && let Some(dialog) = &mut app.dialog
                            && dialog.kind == crate::dialog::DialogKind::ChatGpt
                        {
                            dialog.note = format!(
                                "Abra no navegador para continuar com ChatGPT:\n{url}\nEsc cancela o login."
                            );
                        }
                    }
                    UiEvent::Models(models) => {
                        app.models = models;
                        if let Some(dialog) = &mut app.dialog
                            && dialog.kind == crate::dialog::DialogKind::Models
                        {
                            dialog.choices = app
                                .models
                                .iter()
                                .map(|model| crate::dialog::Choice {
                                    id: model.id.clone(),
                                    label: model.label.clone(),
                                })
                                .collect();
                        }
                    }
                    UiEvent::Finished { view, error } => {
                        app.finish(view.map(|view| *view), error);
                        driver.cancel = None;
                    }
                    UiEvent::Error(error) => {
                        app.loading = false;
                        app.notify(error, true);
                    }
                    UiEvent::Candidates { query, paths } => {
                        if !app.busy() && app.input.mention() == Some(query.as_str()) {
                            app.mentions = paths.into_iter().take(8).collect();
                        }
                    }
                    UiEvent::Shutdown => {
                        app.quitting = true;
                        app.cancelling = true;
                        shutdown.cancel();
                    }
                }
            }
            if app.quitting && !app.busy() {
                break;
            }
            if !crate::terminal::TerminalGuard::is_active() {
                return Err(io::Error::other(
                    "A operação falhou; o terminal foi restaurado.",
                ));
            }
            terminal.draw(|frame| render::draw(frame, &mut app))?;
            if event::poll(Duration::from_millis(33))? {
                let actions = app.event(event::read()?);
                for action in actions {
                    if let Action::SaveTheme(theme) = action {
                        let path = preferences.clone();
                        let sender = driver.sender.clone();
                        driver.jobs.push(runtime.spawn(async move{let result=tokio::task::spawn_blocking(move||Preferences{theme}.save(&path)).await;
                            if !matches!(result,Ok(Ok(()))){let _=sender.send(UiEvent::Error("O tema mudou nesta sessão, mas não foi possível salvar preferences.json.".into())).await;}}));
                    } else if !driver.action(action, &mut app) {
                        return Ok(());
                    }
                }
            }
            app.tick();
            let mut pending = Vec::new();
            let mut failed = false;
            for job in driver.jobs.drain(..) {
                if job.is_finished() {
                    if runtime.block_on(job).is_err() {
                        failed = true;
                    }
                } else {
                    pending.push(job);
                }
            }
            driver.jobs = pending;
            if failed {
                return Err(io::Error::other(
                    "A operação foi interrompida por uma falha interna.",
                ));
            }
        }
        Ok(())
    })();
    shutdown.cancel();
    if let Some(cancel) = driver.cancel.take() {
        cancel.cancel();
    }
    signal.abort();
    drop(receiver);
    // Keep runtime/store alive until workers have completed their own cleanup.
    runtime.block_on(async {
        for job in driver.jobs {
            let _ = job.await;
        }
    });
    if let Some(session) = app.session {
        driver.backend.forget(&session);
    }
    result
}
