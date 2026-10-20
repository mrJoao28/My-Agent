use myagent_protocol::{Message, Role};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenUsageSummary {
    pub measured_tokens: Option<u64>,
    pub unreported_messages: usize,
}
/// Soma apenas medições reais; ausência permanece desconhecida, não vira zero.
pub fn token_usage(messages: &[Message]) -> TokenUsageSummary {
    let mut measured = None;
    let mut unreported = 0;
    for message in messages
        .iter()
        .filter(|m| matches!(m.role, Role::Assistant | Role::Error))
    {
        if let Some(tokens) = message.total_tokens {
            measured = Some(measured.unwrap_or(0u64).saturating_add(u64::from(tokens)));
        } else {
            unreported += 1;
        }
    }
    TokenUsageSummary {
        measured_tokens: measured,
        unreported_messages: unreported,
    }
}
