use crate::preview::bounded;
use serde_json::{Map, Value};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Once,
    Session,
    Deny,
    ConfirmMain,
}
#[derive(Clone, Debug)]
pub struct Approval {
    pub id: String,
    pub tool: String,
    pub scope: String,
    pub lines: Vec<String>,
    pub truncated: bool,
    pub allow_session: bool,
    pub confirmation: Option<String>,
    pub scroll: usize,
    pub max_scroll: usize,
}
impl Approval {
    pub fn new(
        id: String,
        tool: String,
        args: Map<String, Value>,
        scope: Option<String>,
        allow_session: bool,
        confirmation: Option<String>,
    ) -> Self {
        let mut display = String::new();
        for key in [
            "path", "from", "to", "url", "command", "branch", "remote", "message", "files",
        ] {
            if let Some(value) = args.get(key) {
                display.push_str(&format!(
                    "{key}: {}\n",
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string())
                ));
            }
        }
        if let Some(preview) = args.get("preview").and_then(Value::as_str) {
            display.push_str(preview);
        }
        // Adapters may retain content; never omit write size or preview.
        if let Some(content) = args.get("content").and_then(Value::as_str) {
            display.push_str(&format!("\n{} bytes\n{content}", content.len()));
        }
        let (display, limited) = bounded(&display);
        let individual = matches!(
            tool.as_str(),
            "delete_file" | "git_commit" | "git_init" | "git_remote_add" | "git_push"
        );
        Self {
            id,
            tool: crate::safe_text(&tool),
            scope: crate::safe_text(&scope.unwrap_or_default()),
            lines: display.lines().map(str::to_owned).collect(),
            truncated: limited || args.get("previewTruncated") == Some(&Value::Bool(true)),
            allow_session: allow_session && !individual && confirmation.is_none(),
            confirmation,
            scroll: 0,
            max_scroll: 0,
        }
    }
    pub fn decision(&self, key: char) -> Option<Decision> {
        match key.to_ascii_lowercase() {
            'n' => Some(Decision::Deny),
            'm' if self.confirmation.is_some() => Some(Decision::ConfirmMain),
            'y' if self.confirmation.is_none() => Some(Decision::Once),
            'a' if self.allow_session => Some(Decision::Session),
            _ => None,
        }
    }
    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll);
    }
}
