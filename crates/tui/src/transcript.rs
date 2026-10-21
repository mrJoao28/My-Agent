use crate::preview::{self, CodePanel, CodeStatus};
use myagent_protocol::{Message, MessagePart, Mode, Role};
use serde_json::{Map, Value};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    User,
    Assistant,
    Error,
    Reasoning,
    Info,
}
impl Speaker {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "Você",
            Self::Assistant => "My-Agent",
            Self::Error => "Erro",
            Self::Reasoning => "Raciocínio",
            Self::Info => "Estado",
        }
    }
}
#[derive(Clone, Debug)]
pub enum Entry {
    Text {
        speaker: Speaker,
        text: String,
        mode: Option<Mode>,
    },
    Code(CodePanel),
    Tool {
        id: String,
        name: String,
        result: Option<String>,
    },
}
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    pub entries: Vec<Entry>,
}
impl Transcript {
    pub fn text(&mut self, speaker: Speaker, text: &str, delta: bool) {
        self.text_in_mode(speaker, text, delta, None);
    }
    pub fn text_in_mode(&mut self, speaker: Speaker, text: &str, delta: bool, mode: Option<Mode>) {
        let text = crate::safe_text(text);
        if delta
            && let Some(Entry::Text {
                speaker: old,
                text: prior,
                mode: prior_mode,
            }) = self.entries.last_mut()
            && *old == speaker
            && *prior_mode == mode
        {
            prior.push_str(&text);
            return;
        }
        self.entries.push(Entry::Text {
            speaker,
            text,
            mode,
        });
    }
    pub fn preview(&mut self, id: &str, name: &str, raw: &str, status: CodeStatus) {
        for panel in preview::panels(id, name, raw, status) {
            if let Some(Entry::Code(old))=self.entries.iter_mut().find(|entry|matches!(entry,Entry::Code(code) if code.call_id==id && code.path==panel.path)) { *old=panel; }
            else { self.entries.push(Entry::Code(panel)); }
        }
    }
    pub fn call(&mut self, id: &str, name: &str, args: &Map<String, Value>) {
        if matches!(name, "write_file" | "edit_file" | "apply_patch") {
            self.preview(
                id,
                name,
                &Value::Object(args.clone()).to_string(),
                CodeStatus::AwaitingApproval,
            );
        } else {
            self.entries.push(Entry::Tool {
                id: id.into(),
                name: crate::safe_text(name),
                result: None,
            });
        }
    }
    pub fn result(&mut self, id: &str, result: &str) {
        let status = if result.starts_with("Error:") || result.starts_with("Tool failed:") {
            CodeStatus::Failed
        } else if serde_json::from_str::<Value>(result)
            .is_ok_and(|v| v["applied"] == false && v["valid"] == true)
        {
            CodeStatus::Checked
        } else {
            CodeStatus::Applied
        };
        for entry in &mut self.entries {
            match entry {
                Entry::Code(code) if code.call_id == id => code.status = status.clone(),
                Entry::Tool {
                    id: call,
                    result: output,
                    ..
                } if call == id => *output = Some(preview::bounded(result).0),
                _ => {}
            }
        }
    }
    pub fn interrupted(&mut self) {
        for entry in &mut self.entries {
            if let Entry::Code(code) = entry
                && matches!(
                    code.status,
                    CodeStatus::Generating | CodeStatus::AwaitingApproval
                )
            {
                code.status = CodeStatus::Interrupted;
            }
        }
    }
    pub fn history(messages: &[Message]) -> Self {
        let mut transcript = Self::default();
        for message in messages {
            if message.role == Role::User {
                transcript.text(Speaker::User, &message.content, false);
                continue;
            }
            let start = transcript.entries.len();
            if message.parts.is_empty() && !message.content.is_empty() {
                transcript.text(
                    if message.role == Role::Error {
                        Speaker::Error
                    } else {
                        Speaker::Assistant
                    },
                    &message.content,
                    false,
                );
            }
            for part in &message.parts {
                match part {
                    MessagePart::Text { text } => {
                        transcript.text_in_mode(Speaker::Assistant, text, false, Some(message.mode))
                    }
                    MessagePart::Reasoning { text } => {
                        transcript.text_in_mode(Speaker::Reasoning, text, false, Some(message.mode))
                    }
                    MessagePart::ToolCall { id, name, args, .. } => transcript.call(id, name, args),
                    MessagePart::ToolResult { id, result, .. } => transcript.result(id, result),
                    MessagePart::ContextWarning { message, .. } => {
                        transcript.text(Speaker::Info, message, false)
                    }
                    MessagePart::ToolLimitWarning { limit } => transcript.text(
                        Speaker::Info,
                        &format!("Limite de {limit} chamadas"),
                        false,
                    ),
                }
            }
            for entry in &mut transcript.entries[start..] {
                if let Entry::Text { mode, .. } = entry {
                    *mode = Some(message.mode);
                }
            }
        }
        transcript.interrupted();
        transcript
    }
}
