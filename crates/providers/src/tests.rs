use super::*;
use crate::wire::{WireParser, request};
use myagent_core::{
    AgentError, CancellationToken, Effect, ModelMessage, ModelRole, Provider, ProviderEvent,
    ProviderRequest, ToolSpec,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
fn req() -> ProviderRequest {
    ProviderRequest {
        model: "fixture".into(),
        system: "fixture system".into(),
        messages: vec![ModelMessage::text(ModelRole::User, "hello")],
        tools: vec![ToolSpec {
            name: "read_file".into(),
            description: "fixture".into(),
            parameters: json!({"type":"object","properties":{}}),
            effect: Effect::Read,
        }],
        max_output_tokens: 16384,
    }
}
fn parse(kind: ProviderKind, values: Vec<Value>) -> myagent_core::Result<Vec<ProviderEvent>> {
    let mut parser = WireParser::new(kind, 1);
    let mut events = Vec::new();
    for value in values {
        events.extend(parser.feed(&value.to_string())?);
    }
    events.extend(parser.eof()?);
    Ok(events)
}
#[test]
fn registry_preserves_catalog_overrides_and_key_name_contract() {
    let registry = ModelRegistry::default();
    assert_eq!(registry.models().len(), 13);
    assert_eq!(
        registry
            .find(ModelRegistry::DEFAULT_MODEL)
            .unwrap()
            .api_key_env()
            .as_deref(),
        Some("MYAGENT_ANTHROPIC_CLAUDE_OPUS_4_6_API_KEY")
    );
    assert!(registry.find("llama3.1").unwrap().api_key_env().is_none());
    let custom=ModelRegistry::with_custom_json(br#"[{"id":"gpt-5.4","provider":"openai","apiKeyEnv":"MY_CUSTOM_KEY"},{"id":"  unit/a--b  ","provider":"google","contextWindow":2048}]"#).unwrap();
    assert_eq!(
        custom.find("gpt-5.4").unwrap().context_window,
        Some(1050000)
    );
    assert_eq!(
        custom.find("gpt-5.4").unwrap().api_key_env().as_deref(),
        Some("MY_CUSTOM_KEY")
    );
    assert_eq!(
        custom.find("unit/a--b").unwrap().api_key_env().as_deref(),
        Some("MYAGENT_GOOGLE_UNIT_A_B_API_KEY")
    );
    for invalid in [
        json!([{"id":"x","provider":"google","apiKeyEnv":"PATH"}]),
        json!([{"id":"x","provider":"google","apiKeyEnv":"NODE_OPTIONS"}]),
        json!([{"id":"x","provider":"google","contextWindow":1023}]),
        json!([{"id":"x","provider":"google","pricing":{"inputUsdPerMillionTokens":-1,"outputUsdPerMillionTokens":0}}]),
        json!([{"id":"chatgpt:x","provider":"openai"}]),
        json!([{"id":"x","provider":"chatgpt"}]),
        json!([{"id":"x","provider":"openai","unknown":true}]),
        json!([{"id":"x","provider":"openai"},{"id":"x","provider":"google"}]),
    ] {
        assert!(ModelRegistry::with_custom_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}
#[test]
fn models_file_is_explicit_bounded_and_invalid_json_is_preserved() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("models.json");
    std::fs::write(&file, b"invalid fixture").unwrap();
    assert!(ModelRegistry::load(&file).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"invalid fixture");
    assert!(ModelRegistry::load(&root.path().join("config.env")).is_err());
    std::fs::write(&file, b"[]").unwrap();
    assert_eq!(ModelRegistry::load(&file).unwrap().models().len(), 13);
}

#[cfg(unix)]
#[test]
fn model_paths_reject_user_symlinks_and_hardlinks() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let file = real.join("models.json");
    std::fs::write(&file, b"[]").unwrap();
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    assert!(ModelRegistry::load(&alias.join("models.json")).is_err());
    let linked_file = root.path().join("models.json");
    std::os::unix::fs::symlink(&file, &linked_file).unwrap();
    assert!(ModelRegistry::load(&linked_file).is_err());
    std::fs::remove_file(&linked_file).unwrap();
    std::fs::hard_link(&file, &linked_file).unwrap();
    assert!(ModelRegistry::load(&linked_file).is_err());
}
#[test]
fn sse_fragments_utf8_crlf_comments_and_multiline_data() {
    let bytes =
        ": ping\r\nevent: delta\r\ndata: {\"text\":\"ação 🦀\",\r\ndata: \"ok\":true}\r\n\r\n"
            .as_bytes();
    for split in 0..=bytes.len() {
        let mut decoder = sse::SseDecoder::new(1024, 4096);
        let mut frames = decoder.push(&bytes[..split]).unwrap();
        frames.extend(decoder.push(&bytes[split..]).unwrap());
        decoder.finish().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, "delta");
        assert_eq!(
            serde_json::from_str::<Value>(&frames[0].data).unwrap()["text"],
            "ação 🦀"
        );
    }
    let mut decoder = sse::SseDecoder::new(8, 16);
    assert!(decoder.push(b"data: 123456789").is_err());
    let mut decoder = sse::SseDecoder::new(8, 16);
    assert!(decoder.push(&[b' '; 17]).is_err());
    let mut decoder = sse::SseDecoder::new(1024, 4096);
    decoder.push(b"data: incomplete").unwrap();
    assert!(decoder.finish().is_err());
    let mut decoder = sse::SseDecoder::new(1024, 4096);
    assert!(decoder.push(b"data: \xff\n\n").is_err());
}
#[test]
fn native_anthropic_reasoning_tools_signatures_and_usage() {
    let events=parse(ProviderKind::Anthropic,vec![
        json!({"type":"message_start","message":{"usage":{"input_tokens":3,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reason"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"fictitious-signature"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_1","name":"read_file","input":{}}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"file.rs\"}"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
        json!({"type":"message_stop"}),
    ]).unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e,ProviderEvent::Reasoning(t) if t=="reason"))
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ProviderEvent::ToolCall(_)))
            .count(),
        1
    );
    assert!(events.iter().any(|e| matches!(e, ProviderEvent::Usage(8))));
    let metadata = events
        .iter()
        .find_map(|e| {
            if let ProviderEvent::Metadata(m) = e {
                Some(m.clone())
            } else {
                None
            }
        })
        .unwrap();
    let mut r = req();
    r.messages = vec![ModelMessage {
        role: ModelRole::Assistant,
        content: vec![],
        provider_metadata: Some(metadata),
    }];
    let body = request(ProviderKind::Anthropic, &r).unwrap();
    assert_eq!(
        body["messages"][0]["content"][0]["signature"],
        "fictitious-signature"
    );
    assert_eq!(
        body["messages"][0]["content"][1]["input"]["path"],
        "file.rs"
    );
}
#[test]
fn native_openai_responses_assembles_calls_and_replays_encrypted_reasoning() {
    let output = json!([
        {"type":"reasoning","id":"rs_1","encrypted_content":"fictitious-encrypted","summary":[]},
        {"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"f.rs\"}"}
    ]);
    let events=parse(ProviderKind::Openai,vec![
        json!({"type":"response.reasoning_summary_text.delta","delta":"thinking"}),
        json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":""}}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"path\":"}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"\"f.rs\"}"}),
        json!({"type":"response.output_item.done","output_index":1,"item":output[1]}),
        json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5},"output":output}}),
    ]).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ProviderEvent::ToolCall(_)))
            .count(),
        1
    );
    assert!(events.iter().any(|e| matches!(e, ProviderEvent::Usage(5))));
    let metadata = events
        .iter()
        .find_map(|e| {
            if let ProviderEvent::Metadata(m) = e {
                Some(m.clone())
            } else {
                None
            }
        })
        .unwrap();
    let mut r = req();
    r.messages = vec![ModelMessage {
        role: ModelRole::Assistant,
        content: vec![],
        provider_metadata: Some(metadata),
    }];
    assert_eq!(
        request(ProviderKind::Openai, &r).unwrap()["input"][0]["encrypted_content"],
        "fictitious-encrypted"
    );
}
#[test]
fn native_google_preserves_thought_signatures_and_uses_terminal_reason() {
    let events=parse(ProviderKind::Google,vec![
        json!({"candidates":[{"content":{"parts":[{"text":"reason","thought":true}]}}]}),
        json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"read_file","args":{"path":"file.rs"}},"thoughtSignature":"fictitious-signature"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":3,"totalTokenCount":8}}),
    ]).unwrap();
    assert!(events.iter().any(|e| matches!(e, ProviderEvent::Usage(8))));
    assert!(
        events
            .iter()
            .any(|e| matches!(e,ProviderEvent::ToolCall(call) if call.name=="read_file"))
    );
    let metadata = events
        .iter()
        .find_map(|e| {
            if let ProviderEvent::Metadata(m) = e {
                Some(m.clone())
            } else {
                None
            }
        })
        .unwrap();
    let mut r = req();
    r.messages = vec![ModelMessage {
        role: ModelRole::Assistant,
        content: vec![],
        provider_metadata: Some(metadata),
    }];
    assert_eq!(
        request(ProviderKind::Google, &r).unwrap()["contents"][0]["parts"][1]["thoughtSignature"],
        "fictitious-signature"
    );
}
#[test]
fn native_ollama_handles_interleaved_arguments_and_requires_done() {
    let mut p = WireParser::new(ProviderKind::Ollama, 7);
    let mut events = Vec::new();
    for v in [
        json!({"choices":[{"index":0,"delta":{"reasoning_content":"reason","content":"text","tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":"}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"f.rs\"}"}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}),
    ] {
        events.extend(p.feed(&v.to_string()).unwrap());
    }
    assert!(!p.done);
    events.extend(p.feed("[DONE]").unwrap());
    p.eof().unwrap();
    assert!(events.iter().any(
        |e| matches!(e,ProviderEvent::ToolCall(c) if c.id=="call_1" && c.args["path"]=="f.rs")
    ));
    assert!(events.iter().any(|e| matches!(e, ProviderEvent::Usage(3))));
}
#[test]
fn incomplete_refused_and_malformed_streams_fail_closed() {
    for kind in [
        ProviderKind::Anthropic,
        ProviderKind::Openai,
        ProviderKind::Google,
        ProviderKind::Ollama,
    ] {
        assert!(WireParser::new(kind, 1).eof().is_err());
        assert!(WireParser::new(kind, 1).feed("{").is_err());
        assert!(
            WireParser::new(kind, 1)
                .feed(r#"{"error":{"message":"fictitious secret that must not be printed"}}"#)
                .is_err()
        );
    }
    assert!(
        parse(
            ProviderKind::Openai,
            vec![json!({"type":"response.incomplete"})]
        )
        .is_err()
    );
    assert!(
        parse(
            ProviderKind::Google,
            vec![json!({"candidates":[{"finishReason":"MAX_TOKENS"}]})]
        )
        .is_err()
    );
    assert!(
        parse(
            ProviderKind::Anthropic,
            vec![json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}})]
        )
        .is_err()
    );
    assert!(
        parse(
            ProviderKind::Ollama,
            vec![json!({"choices":[{"finish_reason":"length"}]})]
        )
        .is_err()
    );
}
async fn fixture_server(
    body: String,
    status: u16,
    content_type: &str,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let ct = content_type.to_owned();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let n = socket.read(&mut buffer).await.unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..n]);
            assert!(bytes.len() < 128 * 1024);
            if let Some(index) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..index]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= index + 4 + length {
                    break;
                }
            }
        }
        let header = format!(
            "HTTP/1.1 {status} fixture\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(header.as_bytes()).await;
        for chunk in body.as_bytes().chunks(7) {
            if socket.write_all(chunk).await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
        String::from_utf8(bytes).unwrap()
    });
    (base, task)
}
fn endpoints(base: &str) -> ProviderEndpoints {
    ProviderEndpoints {
        anthropic: format!("{base}/v1/messages"),
        openai: format!("{base}/v1/responses"),
        google: format!("{base}/v1beta/models/"),
        ollama: format!("{base}/v1"),
    }
}
#[tokio::test]
async fn all_adapters_use_loopback_http_and_emit_text_usage_done() {
    for (kind, body, path) in [
        (
            ProviderKind::Anthropic,
            concat!(
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"á\"}}\n\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            ),
            "/v1/messages",
        ),
        (
            ProviderKind::Openai,
            concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"á\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"total_tokens\":3}}}\n\n"
            ),
            "/v1/responses",
        ),
        (
            ProviderKind::Google,
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"á\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"totalTokenCount\":3}}\n\n",
            "/v1beta/models/fixture:streamGenerateContent?alt=sse",
        ),
        (
            ProviderKind::Ollama,
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"á\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"total_tokens\":3}}\n\n",
                "data: [DONE]\n\n"
            ),
            "/v1/chat/completions",
        ),
    ] {
        let (base, server) = fixture_server(body.into(), 200, "text/event-stream").await;
        let provider = HttpProvider::new(
            kind,
            "fictitious-test-key",
            endpoints(&base),
            StreamLimits::default(),
        )
        .unwrap();
        assert!(!format!("{provider:?}").contains("fictitious-test-key"));
        let (tx, mut rx) = mpsc::channel(32);
        provider
            .stream(req(), CancellationToken::new(), tx)
            .await
            .unwrap();
        let request = server.await.unwrap();
        assert!(request.starts_with(&format!("POST {path} HTTP/1.1")));
        assert!(
            !request
                .lines()
                .next()
                .unwrap()
                .contains("fictitious-test-key")
        );
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e,ProviderEvent::Text(t) if t=="á"))
        );
        assert!(events.iter().any(|e| matches!(e, ProviderEvent::Usage(3))));
        assert!(matches!(events.last(), Some(ProviderEvent::Done)));
    }
}
#[tokio::test]
async fn http_rejects_status_content_type_oversized_and_truncated_bodies() {
    for (body, status, ct, expected) in [
        (
            "secret fixture".to_owned(),
            401,
            "text/plain",
            AgentError::Authentication,
        ),
        (
            "secret fixture".to_owned(),
            500,
            "text/plain",
            AgentError::Network,
        ),
        (
            "<html>fixture</html>".to_owned(),
            200,
            "text/html",
            AgentError::Protocol,
        ),
        ("x".repeat(500), 200, "text/event-stream", AgentError::Limit),
        (
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n".into(),
            200,
            "text/event-stream",
            AgentError::Protocol,
        ),
    ] {
        let (base, server) = fixture_server(body, status, ct).await;
        let provider = HttpProvider::new(
            ProviderKind::Openai,
            "fictitious-key",
            endpoints(&base),
            StreamLimits {
                frame_bytes: 128,
                response_bytes: 256,
            },
        )
        .unwrap();
        let (tx, _rx) = mpsc::channel(32);
        let error = provider
            .stream(req(), CancellationToken::new(), tx)
            .await
            .unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("secret"));
        server.await.unwrap();
    }
}
#[tokio::test]
async fn cancelling_network_closes_the_pending_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0; 4096];
        while !bytes.windows(4).any(|p| p == b"\r\n\r\n") {
            let n = socket.read(&mut buf).await.unwrap();
            bytes.extend_from_slice(&buf[..n]);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let _ = ready_tx.send(());
        let mut b = [0; 4096];
        // Drena eventual corpo restante antes de observar fechamento.
        loop {
            if socket.read(&mut b).await.unwrap_or(0) == 0 {
                break;
            }
        }
    });
    let provider = HttpProvider::new(
        ProviderKind::Openai,
        "fictitious-key",
        endpoints(&base),
        StreamLimits::default(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let request_cancel = cancel.clone();
    let (tx, _rx) = mpsc::channel(32);
    let task = tokio::spawn(async move { provider.stream(req(), request_cancel, tx).await });
    ready_rx.await.unwrap();
    cancel.cancel();
    assert_eq!(task.await.unwrap().unwrap_err(), AgentError::Cancelled);
    tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
#[test]
fn resolver_uses_per_model_variables_and_ollama_defaults_without_environment_access() {
    let registry = ModelRegistry::default();
    let cloud = registry.find("gpt-5.4").unwrap();
    assert!(matches!(
        HttpProvider::resolve(cloud, |_| None),
        Err(AgentError::Authentication)
    ));
    let provider = HttpProvider::resolve(cloud, |name| {
        if name == "MYAGENT_OPENAI_GPT_5_4_API_KEY" {
            Some("fictitious-key".into())
        } else {
            None
        }
    });
    assert!(provider.is_ok());
    assert!(HttpProvider::resolve(registry.find("llama3.1").unwrap(), |_| None).is_ok());
    assert!(
        HttpProvider::resolve(registry.find("llama3.1").unwrap(), |name| {
            if name == "OLLAMA_BASE_URL" {
                Some("file:///fixture".into())
            } else {
                None
            }
        })
        .is_err()
    );
}

#[test]
fn window_resolution_distinguishes_override_catalog_family_hint_and_estimation() {
    let r=ModelRegistry::with_custom_json(br#"[{"id":"gpt-5.4-2026-10-01","provider":"openai"},{"id":"custom","provider":"google"},{"id":"local","provider":"ollama"},{"id":"override","provider":"google","contextWindow":8192}]"#).unwrap();
    assert_eq!(r.resolve_window("gpt-5.4", None).unwrap().source, "catalog");
    assert_eq!(
        r.resolve_window("gpt-5.4-2026-10-01", None).unwrap().source,
        "family"
    );
    assert_eq!(
        r.resolve_window("override", Some(16384)).unwrap().tokens,
        8192
    );
    assert_eq!(
        r.resolve_window("custom", Some(32768)).unwrap().source,
        "provider"
    );
    assert_eq!(r.resolve_window("custom", None).unwrap().tokens, 65536);
    assert!(r.resolve_window("custom", None).unwrap().estimated);
    assert_eq!(r.resolve_window("local", None).unwrap().tokens, 16384);
    assert!(r.resolve_window("custom", Some(1023)).is_err());
}
#[test]
fn changed_final_arguments_cannot_disagree_with_a_streamed_call() {
    let mut p = WireParser::new(ProviderKind::Openai, 1);
    p.feed(&json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","call_id":"c","name":"read_file","arguments":"{\"path\":\"safe.rs\"}"}}).to_string()).unwrap();
    let altered = json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"function_call","call_id":"c","name":"write_file","arguments":"{}"}]}});
    assert_eq!(
        p.feed(&altered.to_string()).unwrap_err(),
        AgentError::Protocol
    );
}
#[test]
fn sse_accepts_bom_and_bare_cr_delimiters() {
    let mut decoder = sse::SseDecoder::new(1024, 4096);
    let mut frames = vec![];
    for byte in "\u{feff}data: {\"text\":\"é\"}\r\r".as_bytes() {
        frames.extend(decoder.push(&[*byte]).unwrap());
    }
    decoder.finish().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&frames[0].data).unwrap()["text"],
        "é"
    );
}

#[test]
fn relative_models_path_loads_custom_models_instead_of_silently_using_defaults() {
    let cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir_in(&cwd).unwrap();
    std::fs::write(
        root.path().join("models.json"),
        br#"[{"id":"fixture-relative","provider":"ollama"}]"#,
    )
    .unwrap();
    let relative = root.path().strip_prefix(&cwd).unwrap().join("models.json");
    assert!(
        ModelRegistry::load(&relative)
            .unwrap()
            .find("fixture-relative")
            .is_some()
    );
}
