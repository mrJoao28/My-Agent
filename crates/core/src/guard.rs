use crate::{AgentError, Result};
use serde_json::{Map, Value};
use std::collections::HashMap;
pub const DEFAULT_MAX_TOOL_CALLS: u32 = 40;
pub const MAX_TOOL_CALLS: u32 = 200;
pub const MAX_IDENTICAL_REPEATS: u32 = 3;
pub const GENERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
pub fn max_tool_calls(value: Option<&str>) -> u32 {
    value
        .filter(|s| !s.starts_with('0') && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| (1..=MAX_TOOL_CALLS).contains(n))
        .unwrap_or(DEFAULT_MAX_TOOL_CALLS)
}
pub fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|k| format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical(&map[k])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        _ => value.to_string(),
    }
}
pub fn key(name: &str, args: &Map<String, Value>) -> String {
    format!("{name}:{}", canonical(&Value::Object(args.clone())))
}
pub struct ToolGuard {
    pub used: u32,
    pub limit: u32,
    counts: HashMap<String, u32>,
}
impl ToolGuard {
    pub fn new(limit: u32) -> Result<Self> {
        if !(1..=MAX_TOOL_CALLS).contains(&limit) {
            return Err(AgentError::Configuration);
        }
        Ok(Self {
            used: 0,
            limit,
            counts: HashMap::new(),
        })
    }
    pub fn synthesis(&self) -> bool {
        self.limit.saturating_sub(self.used) <= 2
    }
    pub fn register(&mut self, name: &str, args: &Map<String, Value>) -> Result<()> {
        if self.used >= self.limit {
            return Err(AgentError::Limit);
        }
        self.used += 1;
        let count = self.counts.entry(key(name, args)).or_default();
        *count += 1;
        if *count > MAX_IDENTICAL_REPEATS {
            return Err(AgentError::ToolLoop);
        }
        Ok(())
    }
}

pub fn configured_max_tool_calls(lookup: impl Fn(&str) -> Option<String>) -> u32 {
    max_tool_calls(lookup("MYAGENT_MAX_TOOL_CALLS").as_deref())
}

impl ToolGuard {
    pub fn restore(&mut self, parts: &[myagent_protocol::MessagePart]) {
        for part in parts {
            if let myagent_protocol::MessagePart::ToolCall { name, args, .. } = part {
                self.used = self.used.saturating_add(1);
                let count = self.counts.entry(key(name, args)).or_default();
                *count = count.saturating_add(1);
            }
        }
        self.used = self.used.min(self.limit);
    }
}
