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
