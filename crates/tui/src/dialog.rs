#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Models,
    Sessions,
    Themes,
    Agents,
    ChatGpt,
    Resume,
}
#[derive(Clone, Debug)]
pub struct Choice {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug)]
pub struct Dialog {
    pub kind: DialogKind,
    pub title: String,
    pub query: String,
    pub selected: usize,
    pub choices: Vec<Choice>,
    pub note: String,
    pub login: Option<LoginPrompt>,
}
/// An ephemeral authorization URL is never included in Debug/log output.
#[derive(Clone)]
pub struct LoginPrompt {
    url: String,
    pub message: String,
    pub scroll: usize,
    pub max_scroll: usize,
}
impl std::fmt::Debug for LoginPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoginPrompt { url: [REDACTED] }")
    }
}
impl LoginPrompt {
    pub fn new(url: String) -> Self {
        Self {
            url,
            message: "Abrindo o navegador padrão...".into(),
            scroll: 0,
            max_scroll: 0,
        }
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll);
    }
}
impl Dialog {
    pub fn new(kind: DialogKind, title: &str, choices: Vec<Choice>) -> Self {
        Self {
            kind,
            title: title.into(),
            query: String::new(),
            selected: 0,
            choices,
            note: String::new(),
            login: None,
        }
    }
    pub fn filtered(&self) -> Vec<&Choice> {
        let q = self.query.to_lowercase();
        self.choices
            .iter()
            .filter(|choice| choice.label.to_lowercase().contains(&q))
            .collect()
    }
    pub fn selected(&self) -> Option<String> {
        self.filtered().get(self.selected).map(|c| c.id.clone())
    }
    pub fn advance(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.filtered().len().saturating_sub(1));
    }
}
