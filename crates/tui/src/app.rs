use crate::{
    approval::{Approval, Decision},
    dialog::{Choice, Dialog, DialogKind},
    input::Input,
    ports::{AccountOperation, AccountView, ModelChoice, SessionView, Submission},
    preview::CodeStatus,
    theme::Theme,
    transcript::{Speaker, Transcript},
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use myagent_protocol::{MessageStatus, Mode, Role, Session, StreamEvent};
use ratatui::layout::Rect;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Home,
    NewSession,
    Session,
}
#[derive(Clone, Debug)]
pub enum Action {
    Submit(Submission),
    Cancel,
    Approve { id: String, decision: Decision },
    Sessions,
    RefreshModels,
    Account(AccountOperation),
    CancelLogin,
    NewSession,
    LoadSession(String),
    Undo(String),
    SaveTheme(Theme),
    SuggestFiles(String),
    Quit,
}
#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub ticks: u16,
}
pub struct App {
    pub screen: Screen,
    pub session: Option<String>,
    pub workspace: String,
    pub model: String,
    pub mode: Mode,
    pub running_mode: Option<Mode>,
    pub models: Vec<ModelChoice>,
    pub input: Input,
    pub transcript: Transcript,
    pub approval: Option<Approval>,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    pub theme: Theme,
    pub no_color: bool,
    pub active: bool,
    pub loading: bool,
    pub cancelling: bool,
    pub quitting: bool,
    pub tick: u64,
    pub measured: Option<u64>,
    pub live_tokens: Option<u32>,
    pub unreported: usize,
    pub status: String,
    pub scroll: usize,
    pub max_scroll: usize,
    pub follow: bool,
    pub menu_selected: usize,
    pub commands_dismissed: bool,
    pub mentions: Vec<String>,
    pub mention_query: Option<String>,
    pub approval_area: Rect,
    pub chat_area: Rect,
}
pub const COMMANDS: [(&str, &str); 8] = [
    ("/new", "Nova sessão"),
    ("/sessions", "Sessões"),
    ("/models", "Modelos"),
    ("/agents", "Modo PLAN/BUILD"),
    ("/theme", "Temas"),
    ("/undo", "Desfazer última alteração"),
    ("/chatgpt", "Conta ChatGPT"),
    ("/exit", "Sair"),
];
impl App {
    pub fn new(
        workspace: String,
        models: Vec<ModelChoice>,
        model: String,
        theme: Theme,
        no_color: bool,
    ) -> Self {
        Self {
            screen: Screen::Home,
            session: None,
            workspace,
            model,
            mode: Mode::Build,
            running_mode: None,
            models,
            input: Input::default(),
            transcript: Transcript::default(),
            approval: None,
            dialog: None,
            toast: None,
            theme,
            no_color,
            active: false,
            loading: false,
            cancelling: false,
            quitting: false,
            tick: 0,
            measured: None,
            live_tokens: None,
            unreported: 0,
            status: "Pronto".into(),
            scroll: 0,
            max_scroll: 0,
            follow: true,
            menu_selected: 0,
            commands_dismissed: false,
            mentions: vec![],
            mention_query: None,
            approval_area: Rect::default(),
            chat_area: Rect::default(),
        }
    }
    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            text: crate::safe_text(&text.into()),
            error,
            ticks: 140,
        });
    }
    pub fn account_view(&mut self, view: AccountView) {
        self.loading = false;
        if let Some(dialog) = &mut self.dialog
            && dialog.kind == DialogKind::ChatGpt
        {
            dialog.choices = view
                .choices
                .into_iter()
                .map(|(id, label)| Choice { id, label })
                .collect();
            dialog.note = view.note;
            dialog.selected = 0;
        }
    }
    pub fn tokens(&self) -> u64 {
        self.measured
            .unwrap_or(0)
            .saturating_add(u64::from(self.live_tokens.unwrap_or(0)))
    }
    pub fn busy(&self) -> bool {
        self.active || self.loading
    }
    pub fn load(&mut self, view: SessionView, screen: Screen, offer_resume: bool) {
        self.session = Some(view.session.id);
        self.workspace = view.session.cwd.unwrap_or(view.session.title);
        self.mode = view.mode;
        if self.models.iter().any(|model| model.id == view.model) {
            self.model = view.model;
        }
        self.transcript = Transcript::history(&view.messages);
        self.measured = view.measured_tokens;
        self.unreported = view.unreported;
        self.live_tokens = None;
        self.screen = screen;
        self.loading = false;
        self.scroll = 0;
        self.follow = true;
        self.status = "Pronto".into();
        if offer_resume
            && view.messages.last().is_some_and(|message| {
                message.role == Role::User
                    || message.role == Role::Assistant
                        && message.status == MessageStatus::Interrupted
            })
        {
            let mut dialog = Dialog::new(
                DialogKind::Resume,
                "Geração interrompida",
                vec![
                    Choice {
                        id: "resume".into(),
                        label: "Retomar explicitamente".into(),
                    },
                    Choice {
                        id: "cancel".into(),
                        label: "Manter histórico sem retomar".into(),
                    },
                ],
            );
            dialog.note =
                "Resultados anteriores serão preservados; efeitos iniciados não serão repetidos."
                    .into();
            self.dialog = Some(dialog);
        }
    }
    pub fn finish(&mut self, view: Option<SessionView>, error: Option<String>) {
        let partial = self
            .transcript
            .entries
            .iter()
            .filter_map(|entry| match entry {
                crate::transcript::Entry::Code(code) if code.status == CodeStatus::Generating => {
                    Some(code.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.active = false;
        self.loading = false;
        self.cancelling = false;
        self.running_mode = None;
        self.approval = None;
        let next_mode = self.mode;
        let next_model = self.model.clone();
        if let Some(view) = view {
            self.load(view, Screen::Session, false);
        } else if let Some(tokens) = self.live_tokens.take() {
            self.measured = Some(self.measured.unwrap_or(0).saturating_add(u64::from(tokens)));
        }
        self.mode = next_mode;
        self.model = next_model;
        if let Some(error) = error {
            for mut code in partial {
                code.status = CodeStatus::Interrupted;
                self.transcript
                    .entries
                    .push(crate::transcript::Entry::Code(code));
            }
            self.transcript.interrupted();
            self.notify(error, true);
            self.status = "Interrompido".into();
        } else {
            self.status = "Concluído".into();
        }
    }
    pub fn sessions(&mut self, sessions: Vec<Session>) {
        self.loading = false;
        if let Some(dialog) = &mut self.dialog
            && dialog.kind == DialogKind::Sessions
        {
            dialog.choices = sessions
                .into_iter()
                .map(|session| Choice {
                    id: session.id,
                    label: crate::safe_text(&session.title),
                })
                .collect();
            dialog.note = if dialog.choices.is_empty() {
                "Nenhuma sessão neste workspace".into()
            } else {
                String::new()
            };
        }
    }
    pub fn stream(&mut self, event: StreamEvent) {
        self.follow = true;
        match event {
            StreamEvent::GenerationStarted { .. } => {
                self.status = "Gerando".into();
            }
            StreamEvent::TextDelta { text } => {
                self.transcript
                    .text_in_mode(Speaker::Assistant, &text, true, self.running_mode)
            }
            StreamEvent::ReasoningDelta { text } => {
                self.transcript
                    .text_in_mode(Speaker::Reasoning, &text, true, self.running_mode)
            }
            StreamEvent::ToolPreview {
                tool_call_id,
                tool_name,
                arguments,
            } => self.transcript.preview(
                &tool_call_id,
                &tool_name,
                &arguments,
                CodeStatus::Generating,
            ),
            StreamEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
            } => self.transcript.call(&tool_call_id, &tool_name, &args),
            StreamEvent::ToolResult {
                tool_call_id,
                result,
                ..
            } => self.transcript.result(&tool_call_id, &result),
            StreamEvent::Usage { total_tokens } => self.live_tokens = Some(total_tokens),
            StreamEvent::ApprovalRequired {
                approval_id,
                tool_name,
                args,
                scope,
                allow_session,
                confirmation,
                ..
            } => {
                if !self.cancelling {
                    self.dialog = None;
                    self.approval = Some(Approval::new(
                        approval_id,
                        tool_name,
                        args,
                        scope,
                        allow_session.unwrap_or(false),
                        confirmation,
                    ));
                    self.status = "Aguardando aprovação".into();
                }
            }
            StreamEvent::ContextWarning { message, .. } => self.notify(message, false),
            StreamEvent::ToolLimitWarning { limit } => self.notify(
                format!("Limite de {limit} chamadas; preparando síntese"),
                false,
            ),
            StreamEvent::Done { .. } => {
                self.approval = None;
                self.status = "Sincronizando histórico".into();
            }
            StreamEvent::Error { message, .. } => {
                self.approval = None;
                self.transcript
                    .text_in_mode(Speaker::Error, &message, false, self.running_mode);
                self.status = "Interrompido".into();
            }
        }
    }
    pub fn command_menu_open(&self) -> bool {
        !self.commands_dismissed
            && self.input.text().starts_with('/')
            && !self.commands().is_empty()
    }
    pub fn commands(&self) -> Vec<(&'static str, &'static str)> {
        let q = self.input.text();
        COMMANDS
            .iter()
            .copied()
            .filter(|(name, _)| name.starts_with(q))
            .collect()
    }
    fn open(&mut self, kind: DialogKind) -> Vec<Action> {
        if self.busy()
            && matches!(
                kind,
                DialogKind::Models
                    | DialogKind::Sessions
                    | DialogKind::Agents
                    | DialogKind::ChatGpt
            )
        {
            self.notify(
                "Aguarde ou interrompa a geração antes de trocar a sessão/modelo.",
                false,
            );
            return vec![];
        }
        let (title, choices) = match kind {
            DialogKind::Models => (
                "Modelos",
                self.models
                    .iter()
                    .map(|model| Choice {
                        id: model.id.clone(),
                        label: format!(
                            "{}{}",
                            model.label,
                            if model.configured {
                                ""
                            } else {
                                " · chave ausente"
                            }
                        ),
                    })
                    .collect(),
            ),
            DialogKind::Themes => (
                "Temas",
                Theme::ALL
                    .into_iter()
                    .map(|theme| Choice {
                        id: theme.name().into(),
                        label: theme.name().into(),
                    })
                    .collect(),
            ),
            DialogKind::Agents => (
                "Modo do agente",
                vec![
                    Choice {
                        id: "PLAN".into(),
                        label: "PLAN · analisar sem alterar o projeto".into(),
                    },
                    Choice {
                        id: "BUILD".into(),
                        label: "BUILD · implementar com aprovações".into(),
                    },
                ],
            ),
            DialogKind::Sessions => ("Sessões", vec![]),
            DialogKind::ChatGpt => ("Conta ChatGPT", vec![]),
            DialogKind::Resume => ("Retomar", vec![]),
        };
        let mut dialog = Dialog::new(kind, title, choices);
        if kind == DialogKind::ChatGpt {
            dialog.note = "Carregando contas ChatGPT...".into();
            self.loading = true;
        }
        if kind == DialogKind::Sessions {
            dialog.note = "Carregando sessões...".into();
            self.loading = true;
        }
        self.dialog = Some(dialog);
        if kind == DialogKind::Sessions {
            vec![Action::Sessions]
        } else if kind == DialogKind::ChatGpt {
            vec![Action::Account(AccountOperation::Status)]
        } else if kind == DialogKind::Models {
            vec![Action::RefreshModels]
        } else {
            vec![]
        }
    }
    fn command(&mut self, command: &str) -> Vec<Action> {
        match command {
            "/exit" => {
                self.quitting = true;
                if self.active {
                    self.cancelling = true;
                    vec![Action::Cancel]
                } else {
                    vec![Action::Quit]
                }
            }
            "/theme" => self.open(DialogKind::Themes),
            "/models" => self.open(DialogKind::Models),
            "/sessions" => self.open(DialogKind::Sessions),
            "/agents" => self.open(DialogKind::Agents),
            "/chatgpt" => self.open(DialogKind::ChatGpt),
            "/new" if !self.busy() => {
                self.loading = true;
                vec![Action::NewSession]
            }
            "/undo" if !self.busy() => {
                if let Some(session) = &self.session {
                    self.loading = true;
                    vec![Action::Undo(session.clone())]
                } else {
                    vec![]
                }
            }
            "/new" | "/undo" => {
                self.notify("Interrompa a geração antes desta ação.", false);
                vec![]
            }
            _ => {
                self.notify("Comando desconhecido; digite / para listar.", true);
                vec![]
            }
        }
    }
    fn submit(&mut self, resume: bool) -> Vec<Action> {
        if self.busy() {
            return vec![];
        }
        let Some(session) = self.session.clone() else {
            self.notify("A sessão ainda não está pronta.", true);
            return vec![];
        };
        let content = if resume {
            None
        } else {
            if self.input.text().trim().is_empty() {
                return vec![];
            }
            Some(self.input.take())
        };
        let request = Submission {
            session,
            model: self.model.clone(),
            mode: self.mode,
            content: content.clone(),
        };
        if let Some(content) = content {
            self.transcript.text(Speaker::User, &content, false);
        }
        self.active = true;
        self.cancelling = false;
        self.running_mode = Some(self.mode);
        self.live_tokens = None;
        self.approval = None;
        self.mentions.clear();
        self.screen = Screen::Session;
        self.status = "Iniciando".into();
        self.follow = true;
        vec![Action::Submit(request)]
    }
    pub fn event(&mut self, event: Event) -> Vec<Action> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Paste(text)
                if self.dialog.is_none() && self.approval.is_none() && !self.busy() =>
            {
                self.commands_dismissed = false;
                self.menu_selected = 0;
                if !self.input.insert(&text) {
                    self.notify("Limite de 9000 caracteres atingido.", true);
                }
                self.suggest()
            }
            Event::Mouse(mouse) => {
                let delta = match mouse.kind {
                    MouseEventKind::ScrollUp => -3,
                    MouseEventKind::ScrollDown => 3,
                    _ => 0,
                };
                if let Some(dialog) = &mut self.dialog {
                    dialog.advance(delta);
                } else if let Some(approval) = &mut self.approval {
                    approval.scroll_by(delta);
                } else {
                    self.scroll = self
                        .scroll
                        .saturating_add_signed(delta)
                        .min(self.max_scroll);
                    self.follow = false;
                }
                vec![]
            }
            _ => vec![],
        }
    }
    fn key(&mut self, key: KeyEvent) -> Vec<Action> {
        // Windows may report AltGr as Ctrl+Alt. Printable characters from that
        // combination are text, never Ctrl shortcuts or approval decisions.
        let altgr = key
            .modifiers
            .contains(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let control = key.modifiers.contains(KeyModifiers::CONTROL) && !altgr;
        let interrupt = key.code == KeyCode::Esc || (control && key.code == KeyCode::Char('c'));
        if let Some(dialog) = &mut self.dialog {
            if interrupt {
                let login = dialog.kind == DialogKind::ChatGpt && self.loading;
                self.dialog = None;
                if login {
                    self.loading = false;
                    return vec![Action::CancelLogin];
                }
                return vec![];
            }
            if dialog.kind == DialogKind::ChatGpt && self.loading {
                return vec![];
            }
            match key.code {
                KeyCode::Up => dialog.advance(-1),
                KeyCode::Down | KeyCode::Tab => dialog.advance(1),
                KeyCode::PageUp => dialog.advance(-8),
                KeyCode::PageDown => dialog.advance(8),
                KeyCode::Backspace => {
                    dialog.query.pop();
                    dialog.selected = 0;
                }
                KeyCode::Char(c) if !control => {
                    dialog.query.push(c);
                    dialog.selected = 0;
                }
                KeyCode::Enter => {
                    let kind = dialog.kind;
                    let selected = dialog.selected();
                    self.dialog = None;
                    if let Some(id) = selected {
                        match kind {
                            DialogKind::Models => self.model = id,
                            DialogKind::Themes => {
                                if let Some(theme) =
                                    Theme::ALL.into_iter().find(|theme| theme.name() == id)
                                {
                                    self.theme = theme;
                                    return vec![Action::SaveTheme(theme)];
                                }
                            }
                            DialogKind::Agents => {
                                self.mode = if id == "PLAN" {
                                    Mode::Plan
                                } else {
                                    Mode::Build
                                }
                            }
                            DialogKind::Sessions => {
                                self.loading = true;
                                return vec![Action::LoadSession(id)];
                            }
                            DialogKind::ChatGpt => {
                                let op = if id == "login" {
                                    AccountOperation::Login(None)
                                } else if let Some(id) = id.strip_prefix("login:") {
                                    AccountOperation::Login(Some(id.into()))
                                } else if let Some(id) = id.strip_prefix("activate:") {
                                    AccountOperation::Activate(id.into())
                                } else if let Some(id) = id.strip_prefix("logout:") {
                                    AccountOperation::Logout(id.into())
                                } else if id == "models" {
                                    return self.open(DialogKind::Models);
                                } else {
                                    return vec![];
                                };
                                self.dialog =
                                    Some(Dialog::new(DialogKind::ChatGpt, "Conta ChatGPT", vec![]));
                                self.loading = true;
                                return vec![Action::Account(op)];
                            }
                            DialogKind::Resume if id == "resume" => return self.submit(true),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return vec![];
        }
        if let Some(approval) = &mut self.approval {
            if interrupt {
                self.cancelling = true;
                self.status = "Interrompendo e aguardando limpeza".into();
                self.approval = None;
                return vec![Action::Cancel];
            }
            match key.code {
                KeyCode::Up => approval.scroll_by(-1),
                KeyCode::Down => approval.scroll_by(1),
                KeyCode::PageUp => approval.scroll_by(-12),
                KeyCode::PageDown => approval.scroll_by(12),
                KeyCode::Home => approval.scroll = 0,
                KeyCode::End => approval.scroll = approval.max_scroll,
                KeyCode::Char(c) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
                    if let Some(decision) = approval.decision(c) {
                        let id = approval.id.clone();
                        self.approval = None;
                        self.status = "Decisão enviada".into();
                        return vec![Action::Approve { id, decision }];
                    }
                }
                _ => {}
            }
            return vec![];
        }
        if key.code == KeyCode::Esc && self.command_menu_open() && !self.busy() {
            self.commands_dismissed = true;
            return vec![];
        }
        if interrupt {
            if self.active {
                self.cancelling = true;
                self.status = "Interrompendo e aguardando limpeza".into();
                return vec![Action::Cancel];
            }
            if !self.input.text().is_empty() {
                self.input.take();
                self.mentions.clear();
                return vec![];
            }
            if control {
                self.quitting = true;
                return vec![Action::Quit];
            }
            return vec![];
        }
        if key.code == KeyCode::Tab {
            if !self.mentions.is_empty() {
                let path = self.mentions[0].clone();
                self.input.replace_mention(&path);
                self.mentions.clear();
                return vec![];
            }
            if self.command_menu_open() {
                if let Some((name, _)) = self.commands().get(self.menu_selected).copied() {
                    self.input.set(name);
                }
                return vec![];
            }
            self.mode = match self.mode {
                Mode::Plan => Mode::Build,
                _ => Mode::Plan,
            };
            return vec![];
        }
        if self.busy() {
            if key.code == KeyCode::PageUp {
                self.scroll = self.scroll.saturating_sub(12);
                self.follow = false;
            }
            if key.code == KeyCode::PageDown {
                self.scroll = (self.scroll + 12).min(self.max_scroll);
                self.follow = false;
            }
            return vec![];
        }
        let menu = self.command_menu_open();
        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.input.insert("\n");
            }
            KeyCode::Enter => {
                let exact = COMMANDS.iter().any(|(name, _)| *name == self.input.text());
                if exact {
                    let command = self.input.text().to_owned();
                    self.input.take();
                    self.menu_selected = 0;
                    return self.command(&command);
                }
                return self.submit(false);
            }
            KeyCode::Char('j') if control => {
                self.input.insert("\n");
            }
            KeyCode::Char('a') if control => self.input.home(),
            KeyCode::Char('e') if control => self.input.end(),
            KeyCode::Char(c)
                if c == '/'
                    || altgr
                    || (!control && !key.modifiers.contains(KeyModifiers::ALT)) =>
            {
                self.commands_dismissed = false;
                if !self.input.insert(&c.to_string()) {
                    self.notify("Limite de 9000 caracteres atingido.", true);
                }
                self.menu_selected = 0;
            }
            KeyCode::Backspace => {
                self.input.backspace();
                self.menu_selected = 0;
            }
            KeyCode::Delete => self.input.delete(),
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            KeyCode::Up if menu => self.menu_selected = self.menu_selected.saturating_sub(1),
            KeyCode::Down if menu => {
                self.menu_selected =
                    (self.menu_selected + 1).min(self.commands().len().saturating_sub(1))
            }
            KeyCode::Up => self.input.vertical(false),
            KeyCode::Down => self.input.vertical(true),
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(12);
                self.follow = false;
            }
            KeyCode::PageDown => {
                self.scroll = (self.scroll + 12).min(self.max_scroll);
                self.follow = false;
            }
            _ => {}
        }
        self.suggest()
    }
    fn suggest(&mut self) -> Vec<Action> {
        self.mentions.clear();
        self.mention_query = self.input.mention().map(str::to_owned);
        self.mention_query
            .clone()
            .map(|q| vec![Action::SuggestFiles(q)])
            .unwrap_or_default()
    }
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if let Some(toast) = &mut self.toast {
            toast.ticks = toast.ticks.saturating_sub(1);
            if toast.ticks == 0 {
                self.toast = None;
            }
        }
    }
}
