use crate::registry::ProviderKind;
use myagent_core::{AgentError, ModelRole, ProviderEvent, ProviderRequest, Result, ToolCall};
use myagent_protocol::{MessagePart, validate_id};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
pub(crate) fn request(kind: ProviderKind, r: &ProviderRequest) -> Result<Value> {
    if r.model.is_empty()
        || r.model.starts_with("chatgpt:")
        || r.model.len() > 200
        || r.max_output_tokens == 0
    {
        return Err(AgentError::Configuration);
    }
    let mut google_ids = std::collections::HashSet::new();
    for message in &r.messages {
        if let Some(parts) = message
            .provider_metadata
            .as_ref()
            .and_then(|v| v.pointer("/google/parts"))
            .and_then(Value::as_array)
        {
            for part in parts {
                if let Some(id) = part["functionCall"]["id"].as_str() {
                    google_ids.insert(id.to_owned());
                }
            }
        } else {
            for part in &message.content {
                if let MessagePart::ToolCall { id, .. } = part {
                    google_ids.insert(id.clone());
                }
            }
        }
    }
    let mut messages = Vec::new();
    for message in &r.messages {
        match kind {
            ProviderKind::Anthropic => {
                let raw = message
                    .provider_metadata
                    .as_ref()
                    .and_then(|m| m.pointer("/anthropic/blocks"))
                    .and_then(Value::as_array);
                let content = if let Some(raw) = raw {
                    raw.clone()
                } else {
                    message
                        .content
                        .iter()
                        .filter_map(|p| match p {
                            MessagePart::Text { text } => Some(json!({"type":"text","text":text})),
                            MessagePart::ToolCall { id, name, args, .. } => {
                                Some(json!({"type":"tool_use","id":id,"name":name,"input":args}))
                            }
                            MessagePart::ToolResult { id, result, .. } => Some(
                                json!({"type":"tool_result","tool_use_id":id,"content":result}),
                            ),
                            _ => None,
                        })
                        .collect()
                };
                if !content.is_empty() {
                    messages.push(json!({"role":if message.role==ModelRole::Assistant{"assistant"}else{"user"},"content":content}));
                }
            }
            ProviderKind::Openai | ProviderKind::Chatgpt => {
                if let Some(raw) = message
                    .provider_metadata
                    .as_ref()
                    .and_then(|m| m.pointer("/openai/output"))
                    .and_then(Value::as_array)
                {
                    messages.extend(raw.clone());
                    continue;
                }
                for part in &message.content {
                    match part {
                        MessagePart::Text{text}=>messages.push(json!({"role":if message.role==ModelRole::Assistant{"assistant"}else{"user"},"content":text})),
                        MessagePart::ToolCall{id,name,args,..}=>messages.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":Value::Object(args.clone()).to_string()})),
                        MessagePart::ToolResult{id,result,..}=>messages.push(json!({"type":"function_call_output","call_id":id,"output":result})),
                        _=>{}
                    }
                }
            }
            ProviderKind::Google => {
                let raw = message
                    .provider_metadata
                    .as_ref()
                    .and_then(|m| m.pointer("/google/parts"))
                    .and_then(Value::as_array);
                let content = if let Some(raw) = raw {
                    raw.clone()
                } else {
                    message
                        .content
                        .iter()
                        .filter_map(|p| match p {
                            MessagePart::Text { text } => Some(json!({"text":text})),
                            MessagePart::ToolCall { id, name, args, .. } => {
                                Some(json!({"functionCall":{"id":id,"name":name,"args":args}}))
                            }
                            MessagePart::ToolResult { id, name, result } => {
                                let mut response =
                                    json!({"name":name,"response":{"result":result}});
                                if google_ids.contains(id) {
                                    response["id"] = json!(id);
                                }
                                Some(json!({"functionResponse":response}))
                            }
                            _ => None,
                        })
                        .collect()
                };
                if !content.is_empty() {
                    messages.push(json!({"role":if message.role==ModelRole::Assistant{"model"}else{"user"},"parts":content}));
                }
            }
            ProviderKind::Ollama => {
                let mut text = String::new();
                let mut calls = Vec::new();
                for part in &message.content {
                    match part {
                    MessagePart::Text{text:delta}=>text.push_str(delta),
                    MessagePart::ToolCall{id,name,args,..}=>calls.push(json!({"id":id,"type":"function","function":{"name":name,"arguments":Value::Object(args.clone()).to_string()}})),
                    MessagePart::ToolResult{id,result,..}=>messages.push(json!({"role":"tool","tool_call_id":id,"content":result})),
                    _=>{}
                }
                }
                if message.role != ModelRole::Tool {
                    let mut m = json!({"role":if message.role==ModelRole::Assistant{"assistant"}else{"user"},"content":text});
                    if !calls.is_empty() {
                        m["tool_calls"] = json!(calls);
                    }
                    messages.push(m);
                }
            }
        }
    }
    let definitions:Vec<Value>=r.tools.iter().map(|t| match kind{
        ProviderKind::Anthropic=>json!({"name":t.name,"description":t.description,"input_schema":t.parameters}),
        ProviderKind::Openai | ProviderKind::Chatgpt=>json!({"type":"function","name":t.name,"description":t.description,"parameters":t.parameters,"strict":false}),
        ProviderKind::Google=>json!({"name":t.name,"description":t.description,"parameters":t.parameters}),
        ProviderKind::Ollama=>json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.parameters}})
    }).collect();
    let mut body = match kind {
        ProviderKind::Anthropic => {
            json!({"model":r.model,"system":r.system,"messages":messages,"stream":true,"max_tokens":r.max_output_tokens})
        }
        ProviderKind::Openai | ProviderKind::Chatgpt => {
            json!({"model":r.model,"instructions":r.system,"input":messages,"stream":true,"store":false,"max_output_tokens":r.max_output_tokens,"include":["reasoning.encrypted_content"]})
        }
        ProviderKind::Google => {
            json!({"contents":messages,"systemInstruction":{"parts":[{"text":r.system}]},"generationConfig":{"maxOutputTokens":r.max_output_tokens}})
        }
        ProviderKind::Ollama => {
            messages.insert(0, json!({"role":"system","content":r.system}));
            json!({"model":r.model,"messages":messages,"stream":true,"stream_options":{"include_usage":true},"max_tokens":r.max_output_tokens})
        }
    };
    if !definitions.is_empty() {
        body["tools"] = if kind == ProviderKind::Google {
            json!([{"functionDeclarations":definitions}])
        } else {
            json!(definitions)
        };
    } else {
        match kind {
            ProviderKind::Openai | ProviderKind::Chatgpt | ProviderKind::Ollama => {
                body["tool_choice"] = json!("none")
            }
            ProviderKind::Google => {
                body["toolConfig"] = json!({"functionCallingConfig":{"mode":"NONE"}})
            }
            _ => {}
        }
    }
    if kind == ProviderKind::Anthropic
        && matches!(r.model.as_str(), "claude-opus-4-6" | "claude-sonnet-4-6")
        && r.max_output_tokens > 10000
    {
        body["thinking"] = json!({"type":"enabled","budget_tokens":10000});
    }
    if kind == ProviderKind::Openai && r.model == "gpt-5.4" {
        body["reasoning"] = json!({"summary":"detailed"});
    }
    if kind == ProviderKind::Google && r.model == "gemini-3-pro" {
        body["generationConfig"]["thinkingConfig"] = json!({"thinkingBudget":8192});
    }
    Ok(body)
}
#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}
pub(crate) struct WireParser {
    kind: ProviderKind,
    nonce: u64,
    pub done: bool,
    finish_seen: bool,
    calls: BTreeMap<u64, PartialCall>,
    emitted: HashMap<String, String>,
    raw: BTreeMap<u64, Value>,
    google_parts: Vec<Value>,
    input: Option<u32>,
    output: Option<u32>,
    usage: Option<u32>,
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(AgentError::Protocol)
}
fn count(value: &Value) -> Result<u32> {
    value
        .as_u64()
        .filter(|n| *n <= i32::MAX as u64)
        .map(|n| n as u32)
        .ok_or(AgentError::Protocol)
}
impl WireParser {
    pub fn new(kind: ProviderKind, nonce: u64) -> Self {
        Self {
            kind,
            nonce,
            done: false,
            finish_seen: false,
            calls: BTreeMap::new(),
            emitted: HashMap::new(),
            raw: BTreeMap::new(),
            google_parts: Vec::new(),
            input: None,
            output: None,
            usage: None,
        }
    }
    fn call(&mut self, index: u64) -> Result<&mut PartialCall> {
        if index > 10000 || (!self.calls.contains_key(&index) && self.calls.len() >= 200) {
            return Err(AgentError::Limit);
        }
        Ok(self.calls.entry(index).or_default())
    }
    fn emit_call(&mut self, index: u64, events: &mut Vec<ProviderEvent>) -> Result<()> {
        let c = self.calls.get(&index).ok_or(AgentError::Protocol)?;
        validate_id(&c.id).map_err(|_| AgentError::Protocol)?;
        validate_id(&c.name).map_err(|_| AgentError::Protocol)?;
        if c.args.len() > 65536 {
            return Err(AgentError::Limit);
        }
        let args: Map<String, Value> =
            serde_json::from_str(if c.args.is_empty() { "{}" } else { &c.args })
                .map_err(|_| AgentError::Protocol)?;
        let fingerprint = myagent_core::guard::key(&c.name, &args);
        if let Some(old) = self.emitted.get(&c.id) {
            if old != &fingerprint {
                return Err(AgentError::Protocol);
            }
        } else {
            self.emit_preview(index, events)?;
            self.emitted.insert(c.id.clone(), fingerprint);
            events.push(ProviderEvent::ToolCall(ToolCall {
                id: c.id.clone(),
                name: c.name.clone(),
                args,
            }));
        }
        Ok(())
    }
    fn emit_preview(&self, index: u64, events: &mut Vec<ProviderEvent>) -> Result<()> {
        let c = self.calls.get(&index).ok_or(AgentError::Protocol)?;
        if !matches!(c.name.as_str(), "write_file" | "edit_file" | "apply_patch") || c.id.is_empty()
        {
            return Ok(());
        }
        if c.args.len() > 65536 {
            return Err(AgentError::Limit);
        }
        events.push(ProviderEvent::ToolPreview {
            id: c.id.clone(),
            name: c.name.clone(),
            arguments: c.args.clone(),
        });
        Ok(())
    }
    fn finish(&mut self, events: &mut Vec<ProviderEvent>) -> Result<()> {
        if self.done {
            return Err(AgentError::Protocol);
        }
        let indices: Vec<_> = self.calls.keys().copied().collect();
        for index in indices {
            self.emit_call(index, events)?;
        }
        if let Some(total) = self.usage.or_else(|| {
            if self.input.is_some() || self.output.is_some() {
                Some(
                    self.input
                        .unwrap_or(0)
                        .saturating_add(self.output.unwrap_or(0))
                        .min(i32::MAX as u32),
                )
            } else {
                None
            }
        }) {
            events.push(ProviderEvent::Usage(total));
        }
        match self.kind {
            ProviderKind::Anthropic if !self.raw.is_empty() => {
                events.push(ProviderEvent::Metadata(
                    json!({"anthropic":{"blocks":self.raw.values().collect::<Vec<_>>()}}),
                ))
            }
            ProviderKind::Openai | ProviderKind::Chatgpt if !self.raw.is_empty() => {
                events.push(ProviderEvent::Metadata(
                    json!({"openai":{"output":self.raw.values().collect::<Vec<_>>()}}),
                ))
            }
            ProviderKind::Google if !self.google_parts.is_empty() => events.push(
                ProviderEvent::Metadata(json!({"google":{"parts":self.google_parts}})),
            ),
            _ => {}
        }
        events.push(ProviderEvent::Done);
        self.done = true;
        Ok(())
    }
    pub fn feed(&mut self, data: &str) -> Result<Vec<ProviderEvent>> {
        if self.done {
            return Err(AgentError::Protocol);
        }
        let mut events = Vec::new();
        if data == "[DONE]" {
            if self.kind != ProviderKind::Ollama || !self.finish_seen {
                return Err(AgentError::Protocol);
            }
            self.finish(&mut events)?;
            return Ok(events);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| AgentError::Protocol)?;
        if value.get("error").is_some() {
            return Err(AgentError::Protocol);
        }
        match self.kind {
            ProviderKind::Openai | ProviderKind::Chatgpt => self.openai(&value, &mut events)?,
            ProviderKind::Anthropic => self.anthropic(&value, &mut events)?,
            ProviderKind::Google => self.google(&value, &mut events)?,
            ProviderKind::Ollama => self.ollama(&value, &mut events)?,
        }
        Ok(events)
    }
    pub fn eof(&mut self) -> Result<Vec<ProviderEvent>> {
        if self.done {
            return Ok(vec![]);
        }
        if self.kind == ProviderKind::Google && self.finish_seen {
            let mut events = Vec::new();
            self.finish(&mut events)?;
            return Ok(events);
        }
        Err(AgentError::Protocol)
    }
    fn openai(&mut self, v: &Value, e: &mut Vec<ProviderEvent>) -> Result<()> {
        match text(v, "type")? {
            "response.output_text.delta" | "response.refusal.delta" => {
                e.push(ProviderEvent::Text(text(v, "delta")?.to_owned()))
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                e.push(ProviderEvent::Reasoning(text(v, "delta")?.to_owned()))
            }
            "response.output_item.added" | "response.output_item.done" => {
                let index = v["output_index"].as_u64().ok_or(AgentError::Protocol)?;
                let item = &v["item"];
                if item["type"] == "function_call" {
                    let c = self.call(index)?;
                    let id = text(item, "call_id")?;
                    let name = text(item, "name")?;
                    if (!c.id.is_empty() && c.id != id) || (!c.name.is_empty() && c.name != name) {
                        return Err(AgentError::Protocol);
                    }
                    c.id = id.into();
                    c.name = name.into();
                    if v["type"] == "response.output_item.done" {
                        let args = text(item, "arguments")?;
                        if args.len() > 65536 {
                            return Err(AgentError::Limit);
                        }
                        c.args = args.into();
                        self.emit_call(index, e)?;
                    } else if let Some(args) = item["arguments"].as_str() {
                        c.args = args.into();
                    }
                }
                if v["type"] == "response.output_item.done" {
                    self.raw.insert(index, item.clone());
                }
            }
            "response.function_call_arguments.delta" => {
                let index = v["output_index"].as_u64().ok_or(AgentError::Protocol)?;
                let delta = text(v, "delta")?;
                let c = self.call(index)?;
                if c.args.len() + delta.len() > 65536 {
                    return Err(AgentError::Limit);
                }
                c.args.push_str(delta);
                self.emit_preview(index, e)?;
            }
            "response.completed" => {
                if v["response"]["status"] != "completed" {
                    return Err(AgentError::Protocol);
                }
                if let Some(output) = v["response"]["output"].as_array() {
                    for (index, item) in output.iter().enumerate() {
                        self.raw.insert(index as u64, item.clone());
                        if item["type"] == "function_call" {
                            let c = self.call(index as u64)?;
                            c.id = text(item, "call_id")?.into();
                            c.name = text(item, "name")?.into();
                            c.args = text(item, "arguments")?.into();
                        }
                    }
                }
                let u = &v["response"]["usage"];
                if !u.is_null() {
                    self.usage = usage(u, "input_tokens", "output_tokens", "total_tokens")?;
                }
                self.finish(e)?;
            }
            "response.failed" | "response.incomplete" | "response.cancelled" | "error" => {
                return Err(AgentError::Protocol);
            }
            _ => {}
        }
        Ok(())
    }
    fn anthropic(&mut self, v: &Value, e: &mut Vec<ProviderEvent>) -> Result<()> {
        match text(v, "type")? {
            "message_start" => {
                let u = &v["message"]["usage"];
                if !u.is_null() {
                    let mut total = None;
                    for key in [
                        "input_tokens",
                        "cache_creation_input_tokens",
                        "cache_read_input_tokens",
                    ] {
                        if let Some(n) = u.get(key) {
                            total = Some(
                                total
                                    .unwrap_or(0u32)
                                    .saturating_add(count(n)?)
                                    .min(i32::MAX as u32),
                            );
                        }
                    }
                    self.input = total;
                    if let Some(n) = u.get("output_tokens") {
                        self.output = Some(count(n)?);
                    }
                }
            }
            "content_block_start" => {
                let index = v["index"]
                    .as_u64()
                    .filter(|n| *n < 10000)
                    .ok_or(AgentError::Protocol)?;
                let block = &v["content_block"];
                self.raw.insert(index, block.clone());
                if let Some(text) = block["text"].as_str().filter(|s| !s.is_empty()) {
                    e.push(ProviderEvent::Text(text.into()));
                }
                if let Some(text) = block["thinking"].as_str().filter(|s| !s.is_empty()) {
                    e.push(ProviderEvent::Reasoning(text.into()));
                }
                if block["type"] == "tool_use" {
                    let c = self.call(index)?;
                    c.id = text(block, "id")?.into();
                    c.name = text(block, "name")?.into();
                    if block["input"].as_object().is_some_and(|a| !a.is_empty()) {
                        c.args = block["input"].to_string();
                    }
                }
            }
            "content_block_delta" => {
                let index = v["index"].as_u64().ok_or(AgentError::Protocol)?;
                let delta = &v["delta"];
                match text(delta, "type")? {
                    "text_delta" | "thinking_delta" | "signature_delta" => {
                        let (field, event) = match text(delta, "type")? {
                            "text_delta" => ("text", 0),
                            "thinking_delta" => ("thinking", 1),
                            _ => ("signature", 2),
                        };
                        let text = text(delta, field)?;
                        let block = self.raw.get_mut(&index).ok_or(AgentError::Protocol)?;
                        let mut value = block[field].as_str().unwrap_or_default().to_owned();
                        if value.len() + text.len() > 1024 * 1024 {
                            return Err(AgentError::Limit);
                        }
                        value.push_str(text);
                        block[field] = json!(value);
                        if event == 0 {
                            e.push(ProviderEvent::Text(text.into()));
                        } else if event == 1 {
                            e.push(ProviderEvent::Reasoning(text.into()));
                        }
                    }
                    "input_json_delta" => {
                        let part = text(delta, "partial_json")?;
                        let c = self.call(index)?;
                        if c.args.len() + part.len() > 65536 {
                            return Err(AgentError::Limit);
                        }
                        c.args.push_str(part);
                        self.emit_preview(index, e)?;
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = v["index"].as_u64().ok_or(AgentError::Protocol)?;
                if self.calls.contains_key(&index) {
                    self.emit_call(index, e)?;
                    let c = &self.calls[&index];
                    let input: Value =
                        serde_json::from_str(if c.args.is_empty() { "{}" } else { &c.args })
                            .map_err(|_| AgentError::Protocol)?;
                    self.raw.get_mut(&index).ok_or(AgentError::Protocol)?["input"] = input;
                }
            }
            "message_delta" => {
                if let Some(reason) = v["delta"]["stop_reason"].as_str() {
                    if !matches!(reason, "end_turn" | "tool_use" | "stop_sequence") {
                        return Err(AgentError::Protocol);
                    }
                    self.finish_seen = true;
                }
                if let Some(n) = v["usage"].get("output_tokens") {
                    self.output = Some(count(n)?);
                }
            }
            "message_stop" => {
                if !self.finish_seen {
                    return Err(AgentError::Protocol);
                }
                self.finish(e)?;
            }
            "error" => return Err(AgentError::Protocol),
            _ => {}
        }
        Ok(())
    }
    fn ollama(&mut self, v: &Value, e: &mut Vec<ProviderEvent>) -> Result<()> {
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            self.usage = usage(u, "prompt_tokens", "completion_tokens", "total_tokens")?;
        }
        let Some(choices) = v["choices"].as_array() else {
            return Err(AgentError::Protocol);
        };
        if choices.len() > 1 {
            return Err(AgentError::Protocol);
        }
        if let Some(choice) = choices.first() {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                return Err(AgentError::Protocol);
            }
            let delta = &choice["delta"];
            if let Some(text) = delta["content"].as_str() {
                e.push(ProviderEvent::Text(text.into()));
            }
            if let Some(text) = delta["reasoning_content"]
                .as_str()
                .or_else(|| delta["reasoning"].as_str())
            {
                e.push(ProviderEvent::Reasoning(text.into()));
            }
            if let Some(calls) = delta["tool_calls"].as_array() {
                for call in calls {
                    let index = call["index"].as_u64().ok_or(AgentError::Protocol)?;
                    let generated = format!("ollama_{}_{}", self.nonce, index);
                    let c = self.call(index)?;
                    if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
                        if !c.id.is_empty() && c.id != id {
                            return Err(AgentError::Protocol);
                        }
                        c.id = id.into();
                    }
                    if c.id.is_empty() {
                        c.id = generated;
                    }
                    if let Some(name) = call["function"]["name"].as_str()
                        && c.name != name
                    {
                        c.name.push_str(name);
                    }
                    if let Some(args) = call["function"]["arguments"].as_str() {
                        if c.args.len() + args.len() > 65536 {
                            return Err(AgentError::Limit);
                        }
                        c.args.push_str(args);
                    }
                    self.emit_preview(index, e)?;
                }
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                if !matches!(reason, "stop" | "tool_calls") {
                    return Err(AgentError::Protocol);
                }
                self.finish_seen = true;
            }
        }
        Ok(())
    }
    fn google(&mut self, v: &Value, e: &mut Vec<ProviderEvent>) -> Result<()> {
        if v["promptFeedback"].get("blockReason").is_some() {
            return Err(AgentError::Protocol);
        }
        if let Some(u) = v.get("usageMetadata") {
            self.usage = usage(
                u,
                "promptTokenCount",
                "candidatesTokenCount",
                "totalTokenCount",
            )?;
        }
        if let Some(candidates) = v["candidates"].as_array() {
            if candidates.len() > 1 {
                return Err(AgentError::Protocol);
            }
            if let Some(c) = candidates.first() {
                if let Some(parts) = c["content"]["parts"].as_array() {
                    for part in parts {
                        if let Some(text) = part["text"].as_str() {
                            e.push(if part["thought"] == true {
                                ProviderEvent::Reasoning(text.into())
                            } else {
                                ProviderEvent::Text(text.into())
                            });
                        }
                        if let Some(call) = part.get("functionCall") {
                            let index = self.calls.len() as u64;
                            let id = call["id"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| format!("google_{}_{}", self.nonce, index));
                            let c = self.call(index)?;
                            c.id = id;
                            c.name = text(call, "name")?.into();
                            c.args = call
                                .get("args")
                                .cloned()
                                .unwrap_or_else(|| json!({}))
                                .to_string();
                            self.emit_call(index, e)?;
                        }
                        self.google_parts.push(part.clone());
                    }
                }
                if let Some(reason) = c["finishReason"].as_str() {
                    if reason != "STOP" {
                        return Err(AgentError::Protocol);
                    }
                    self.finish_seen = true;
                }
            }
        }
        Ok(())
    }
}
fn usage(value: &Value, input: &str, output: &str, total: &str) -> Result<Option<u32>> {
    if let Some(value) = value.get(total).filter(|v| !v.is_null()) {
        return Ok(Some(count(value)?));
    }
    let a = value
        .get(input)
        .filter(|v| !v.is_null())
        .map(count)
        .transpose()?;
    let b = value
        .get(output)
        .filter(|v| !v.is_null())
        .map(count)
        .transpose()?;
    Ok(if a.is_some() || b.is_some() {
        Some(
            a.unwrap_or(0)
                .saturating_add(b.unwrap_or(0))
                .min(i32::MAX as u32),
        )
    } else {
        None
    })
}
