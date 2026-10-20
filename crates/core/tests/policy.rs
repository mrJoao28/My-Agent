#![forbid(unsafe_code)]
use myagent_core::{
    AgentError, ModelMessage, ModelRole, Result, context, guard,
    instructions::{InstructionSource, MAX_INSTRUCTION_BYTES, ProjectInstructions},
};
use myagent_protocol::{ExecutionState, MessagePart};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
#[test]
fn tool_budget_parses_only_positive_integers_with_an_explicit_ceiling() {
    for value in [
        None,
        Some(""),
        Some("0"),
        Some("01"),
        Some("-1"),
        Some("1.0"),
        Some(" 4"),
        Some("201"),
        Some("999999999999999999999999"),
    ] {
        assert_eq!(guard::max_tool_calls(value), 40);
    }
    assert_eq!(guard::max_tool_calls(Some("200")), 200);
    assert_eq!(guard::max_tool_calls(Some("1")), 1);
    let mut g = guard::ToolGuard::new(40).unwrap();
    for i in 0..38 {
        assert!(!g.synthesis());
        g.register(
            "read_file",
            &serde_json::from_value(json!({"offset":i})).unwrap(),
        )
        .unwrap();
    }
    assert!(g.synthesis());
    assert!(guard::ToolGuard::new(0).is_err());
}
#[test]
fn canonical_repetitions_ignore_object_key_order_but_preserve_arrays() {
    let a: Map<String, Value> =
        serde_json::from_value(json!({"x":{"b":1,"a":2},"y":[1,2]})).unwrap();
    let b: Map<String, Value> = serde_json::from_str(r#"{"y":[1,2],"x":{"a":2,"b":1}}"#).unwrap();
    assert_eq!(guard::key("read_file", &a), guard::key("read_file", &b));
    let mut g = guard::ToolGuard::new(40).unwrap();
    for _ in 0..3 {
        g.register("read_file", &a).unwrap();
    }
    assert_eq!(
        g.register("read_file", &b).unwrap_err(),
        AgentError::ToolLoop
    );
}
#[test]
fn summary_only_attributes_writes_with_matching_successful_results() {
    let call = |id: &str, path: &str| MessagePart::ToolCall {
        id: id.into(),
        name: "write_file".into(),
        args: serde_json::from_value(json!({"path":path})).unwrap(),
        execution: Some(ExecutionState::Started),
    };
    let history = vec![
        ModelMessage {
            role: ModelRole::Assistant,
            content: vec![
                call("known", "known.rs"),
                call("missing", "unknown.rs"),
                call("failed", "failed.rs"),
            ],
            provider_metadata: None,
        },
        ModelMessage {
            role: ModelRole::Tool,
            content: vec![
                MessagePart::ToolResult {
                    id: "known".into(),
                    name: "write_file".into(),
                    result: "saved".into(),
                },
                MessagePart::ToolResult {
                    id: "failed".into(),
                    name: "write_file".into(),
                    result: "Tool failed: fixture".into(),
                },
            ],
            provider_metadata: None,
        },
    ];
    let summary = context::summarize(&history, 2000);
    let MessagePart::Text { text } = &summary.content[0] else {
        panic!("text summary")
    };
    let value: Value =
        serde_json::from_str(text.strip_prefix(context::SUMMARY_PREFIX).unwrap()).unwrap();
    assert_eq!(value["changed"], json!(["known.rs"]));
    assert!(!value["pending"].as_array().unwrap().is_empty());
}
#[test]
fn context_budget_rejects_large_current_user_without_silent_truncation() {
    let user = ModelMessage::text(ModelRole::User, "x".repeat(4096));
    assert_eq!(
        context::fit(vec![user], 256).unwrap_err(),
        AgentError::Context
    );
    let history = vec![
        ModelMessage::text(ModelRole::User, "old".repeat(1000)),
        ModelMessage::text(ModelRole::Assistant, "facts".repeat(500)),
        ModelMessage::text(ModelRole::User, "current request 🦀"),
    ];
    let (fit, truncated) = context::fit(history, 400).unwrap();
    assert!(truncated);
    assert_eq!(
        fit.last().unwrap().content,
        vec![MessagePart::Text {
            text: "current request 🦀".into()
        }]
    );
    assert!(context::estimate(&fit) <= 400);
    assert!(context::budget(1023, "", &[]).is_err());
}
#[derive(Default)]
struct Source {
    files: HashMap<String, String>,
    reads: Mutex<Vec<String>>,
}
impl InstructionSource for Source {
    fn read(&self, path: &Path) -> Result<Option<String>> {
        let path = path.to_string_lossy().replace('\\', "/");
        self.reads.lock().unwrap().push(path.clone());
        Ok(self.files.get(&path).cloned())
    }
}
#[test]
fn project_instructions_have_scopes_immutable_cache_and_byte_budget() {
    let source = Arc::new(Source {
        files: HashMap::from([
            ("AGENTS.md".into(), "root\r\npreserve".into()),
            ("src/AGENTS.md".into(), "scoped".into()),
            ("other/AGENTS.md".into(), "unrelated".into()),
        ]),
        reads: Mutex::new(vec![]),
    });
    let mut snapshot = ProjectInstructions::new(source.clone());
    let root = snapshot.for_paths(&[]).unwrap();
    assert!(root.contains("root"));
    assert!(!root.contains("scoped"));
    let note = snapshot.for_paths(&["src/file.rs".into()]).unwrap();
    assert!(note.contains("scoped"));
    assert!(!note.contains("root"));
    assert!(!note.contains("unrelated"));
    assert!(
        snapshot
            .for_paths(&["src/file.rs".into()])
            .unwrap()
            .is_empty()
    );
    snapshot
        .for_paths(&["../outside.rs".into(), ".env/secret".into()])
        .unwrap();
    assert_eq!(
        source
            .reads
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.as_str() == "src/AGENTS.md")
            .count(),
        1
    );
    let huge = Arc::new(Source {
        files: HashMap::from([("AGENTS.md".into(), "\\\"🦀".repeat(10000))]),
        reads: Mutex::new(vec![]),
    });
    let note = ProjectInstructions::new(huge).for_paths(&[]).unwrap();
    assert!(note.len() <= MAX_INSTRUCTION_BYTES);
    let json = note.split_once('\n').unwrap().1;
    assert_eq!(
        serde_json::from_str::<Value>(json).unwrap()["limited"],
        true
    );
}

#[test]
fn shortened_read_file_keeps_whole_lines_paging_and_pair_identity() {
    let page = json!({"path":"f.rs","content":"complete line 🦀\n".repeat(1000),"totalLines":2000,"truncated":true,"nextOffset":1005});
    let messages = vec![
        ModelMessage::text(ModelRole::User, "current"),
        ModelMessage {
            role: ModelRole::Assistant,
            content: vec![MessagePart::ToolCall {
                id: "c".into(),
                name: "read_file".into(),
                args: serde_json::from_value(json!({"path":"f.rs","offset":5})).unwrap(),
                execution: Some(ExecutionState::Started),
            }],
            provider_metadata: None,
        },
        ModelMessage {
            role: ModelRole::Tool,
            content: vec![MessagePart::ToolResult {
                id: "c".into(),
                name: "read_file".into(),
                result: page.to_string(),
            }],
            provider_metadata: None,
        },
    ];
    let (fit, truncated) = context::fit(messages, 600).unwrap();
    assert!(truncated);
    assert!(context::estimate(&fit) <= 600);
    let MessagePart::ToolResult { id, result, .. } = &fit[2].content[0] else {
        panic!("pair")
    };
    assert_eq!(id, "c");
    let page: Value = serde_json::from_str(result).unwrap();
    assert_eq!(page["contextTruncated"], true);
    let content = page["content"].as_str().unwrap();
    assert!(content.is_empty() || content.ends_with('\n'));
    assert_eq!(
        page["nextOffset"].as_u64().unwrap(),
        5 + content.lines().count() as u64
    );
    assert!(context::evidence(&fit, &Default::default()).is_empty());
}
#[test]
fn prompts_keep_six_sections_and_mode_specific_constraints() {
    for mode in [myagent_core::Mode::Plan, myagent_core::Mode::Build] {
        let prompt =
            myagent_core::prompts::system(mode, &["read_file".into()], "", 65536, false, 40);
        assert_eq!(prompt.lines().filter(|l| l.starts_with("## ")).count(), 6);
        assert!(prompt.contains("Ferramentas disponíveis: read_file."));
        assert!(!prompt.contains("write_file"));
        if mode == myagent_core::Mode::Plan {
            assert!(prompt.contains("Não modifique"));
            assert!(prompt.contains("critérios de aceite"));
        } else {
            assert!(prompt.contains("Não crie documentação automaticamente"));
            assert!(prompt.contains("sem rede"));
        }
    }
    assert_eq!(
        guard::configured_max_tool_calls(|name| {
            assert_eq!(name, "MYAGENT_MAX_TOOL_CALLS");
            Some("200".into())
        }),
        200
    );
}
#[test]
fn results_are_invalidated_by_writes_and_undo_notes() {
    let call = |id: &str, name: &str| MessagePart::ToolCall {
        id: id.into(),
        name: name.into(),
        args: Map::new(),
        execution: Some(ExecutionState::Started),
    };
    let result = |id: &str, name: &str| MessagePart::ToolResult {
        id: id.into(),
        name: name.into(),
        result: "confirmed".into(),
    };
    let mut history = vec![
        ModelMessage {
            role: ModelRole::Assistant,
            content: vec![call("r", "read_file")],
            provider_metadata: None,
        },
        ModelMessage {
            role: ModelRole::Tool,
            content: vec![result("r", "read_file")],
            provider_metadata: None,
        },
        ModelMessage {
            role: ModelRole::Assistant,
            content: vec![call("w", "write_file")],
            provider_metadata: None,
        },
        ModelMessage {
            role: ModelRole::Tool,
            content: vec![result("w", "write_file")],
            provider_metadata: None,
        },
    ];
    let evidence = context::evidence(&history, &Default::default());
    assert_eq!(evidence.len(), 1);
    assert!(evidence.keys().all(|k| !k.starts_with("read_file:")));
    history.push(ModelMessage::text(
        ModelRole::Assistant,
        "[UNDO] previous edit undone",
    ));
    assert!(context::evidence(&history, &Default::default()).is_empty());
}

#[test]
fn instruction_notes_have_a_total_budget_and_do_not_repeat_limit_warnings() {
    let source = Arc::new(Source {
        files: HashMap::from([("AGENTS.md".into(), "root".into())]),
        reads: Mutex::new(vec![]),
    });
    let mut notes = ProjectInstructions::new(source.clone());
    let mut total = notes.for_paths(&[]).unwrap().len();
    let mut warnings = 0;
    for n in 0..250 {
        let note = notes.for_paths(&[format!("d{n}/f.rs")]).unwrap();
        total += note.len();
        if note.contains("\"limited\":true") {
            warnings += 1;
        }
    }
    assert!(total <= MAX_INSTRUCTION_BYTES);
    assert_eq!(warnings, 1);
    assert!(source.reads.lock().unwrap().len() <= 128);
}
