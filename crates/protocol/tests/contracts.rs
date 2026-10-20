#![forbid(unsafe_code)]

use myagent_protocol::*;
use serde_json::{Value, json};

fn round_trip<T>(value: &T) -> T
where
    T: Validate + serde::Serialize + serde::de::DeserializeOwned,
{
    decode_json(&encode_json(value).unwrap()).unwrap()
}
fn message() -> Message {
    Message {
        id: "message-1".into(),
        session_id: "session-1".into(),
        generation_id: Some("message-1".into()),
        role: Role::Assistant,
        status: MessageStatus::InProgress,
        model: "fixture:model".into(),
        mode: Mode::Build,
        content: "olá\r\n".into(),
        parts: vec![MessagePart::Text {
            text: "olá\r\n".into(),
        }],
        duration: None,
        total_tokens: None,
        created_at: 1000,
    }
}

#[test]
fn persisted_contracts_round_trip_without_losing_eol_unicode_or_metadata() {
    let original = message();
    assert_eq!(round_trip(&original), original);
    let encoded: Value = serde_json::from_slice(&encode_json(&original).unwrap()).unwrap();
    assert_eq!(encoded["status"], "IN_PROGRESS");
    assert_eq!(encoded["mode"], "BUILD");
    assert_eq!(encoded["role"], "ASSISTANT");
    assert_eq!(encoded["sessionId"], "session-1");
    let session = Session {
        id: "session-1".into(),
        user_id: "local-user".into(),
        title: "C:\\fixture com espaços".into(),
        cwd: Some("C:\\fixture com espaços".into()),
        workspace_path: "c:\\fixture com espaços".into(),
        retention_warned_at: Some(1000),
        created_at: 0,
        updated_at: 1000,
    };
    assert_eq!(round_trip(&session), session);
    let lease = SessionLease {
        session_id: "session-1".into(),
        owner: "owner-1".into(),
        expires_at: 31000,
    };
    assert_eq!(round_trip(&lease), lease);
    let context = SessionContext {
        session_id: "session-1".into(),
        messages: vec![
            json!({"role":"assistant","content":[{"type":"reasoning","text":"fixture","providerOptions":{"fixture":{"opaque":true}}}]}),
        ],
        token_limit: 16384,
        estimated_tokens: 10,
        truncated: true,
        updated_at: 1000,
    };
    assert_eq!(round_trip(&context), context);
    for role in [Role::User, Role::Assistant, Role::Error] {
        let encoded = serde_json::to_string(&role).unwrap();
        assert_eq!(serde_json::from_str::<Role>(&encoded).unwrap(), role);
    }
    for status in [
        MessageStatus::Complete,
        MessageStatus::InProgress,
        MessageStatus::Interrupted,
    ] {
        let encoded = serde_json::to_string(&status).unwrap();
        assert_eq!(
            serde_json::from_str::<MessageStatus>(&encoded).unwrap(),
            status
        );
    }
}

#[test]
fn every_part_and_stream_event_has_a_stable_tag_and_field_names() {
    let parts = [
        json!({"type":"text","text":"text"}),
        json!({"type":"reasoning","text":"reasoning"}),
        json!({"type":"tool-call","id":"call-1","name":"read_file","args":{"path":"a.txt"},"execution":"started"}),
        json!({"type":"tool-result","id":"call-1","name":"read_file","result":"saved"}),
        json!({"type":"context-warning","contextWindow":16384,"estimated":true,"toolCount":3,"message":"fixture"}),
        json!({"type":"tool-limit-warning","limit":40}),
    ];
    for value in parts {
        let part = decode_json::<MessagePart>(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&encode_json(&part).unwrap()).unwrap(),
            value
        );
    }
    let events = [
        json!({"type":"generation-started","messageId":"m"}),
        json!({"type":"text-delta","text":"á"}),
        json!({"type":"reasoning-delta","text":"fixture"}),
        json!({"type":"tool-call","toolCallId":"c","toolName":"read_file","args":{}}),
        json!({"type":"tool-result","toolCallId":"c","toolName":"read_file","result":"saved"}),
        json!({"type":"usage","totalTokens":12}),
        json!({"type":"approval-required","approvalId":"a","toolName":"write_file","args":{},"confirmation":"main","scope":"fixture","allowSession":false,"toolCallId":"c"}),
        json!({"type":"context-warning","contextWindow":1024,"estimated":false,"toolCount":0,"message":"fixture"}),
        json!({"type":"tool-limit-warning","limit":200}),
        json!({"type":"done","messageId":"m","durationMs":1000}),
        json!({"type":"error","kind":"context","message":"fixture","messageId":"m","parts":[{"type":"text","text":"partial"}]}),
    ];
    for value in events {
        let event = decode_json::<StreamEvent>(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&encode_json(&event).unwrap()).unwrap(),
            value
        );
    }
}

#[test]
fn boundaries_reject_unknown_modes_invalid_ids_counts_shapes_and_oversized_input() {
    assert!(serde_json::from_str::<Mode>("\"LEARN\"").is_err());
    assert!(serde_json::from_str::<Role>("\"ADMIN\"").is_err());
    for value in [
        json!({"type":"usage","totalTokens":-1}),
        json!({"type":"usage","totalTokens":2147483648_u64}),
        json!({"type":"tool-limit-warning","limit":0}),
        json!({"type":"tool-limit-warning","limit":201}),
        json!({"type":"context-warning","contextWindow":0,"estimated":false,"toolCount":0,"message":""}),
        json!({"type":"tool-call","toolCallId":"","toolName":"read_file","args":{}}),
        json!({"type":"tool-call","toolCallId":"c","toolName":"read_file","args":[]}),
        json!({"type":"done","messageId":"m","durationMs":-1}),
        json!({"type":"done","messageId":"m","durationMs":1,"unexpected":true}),
    ] {
        assert!(
            decode_json::<StreamEvent>(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{value}"
        );
    }
    assert!(decode_json::<StreamEvent>(b"{").is_err());
    assert!(decode_json::<StreamEvent>(&vec![b' '; MAX_JSON_BYTES + 1]).is_err());
    let mut invalid = message();
    invalid.total_tokens = Some(u32::MAX);
    assert!(encode_json(&invalid).is_err());
    invalid = message();
    invalid.created_at = -1;
    assert!(encode_json(&invalid).is_err());
    invalid = message();
    invalid.parts = vec![
        MessagePart::Text {
            text: String::new()
        };
        MAX_PARTS + 1
    ];
    assert!(encode_json(&invalid).is_err());
}

#[test]
fn user_length_matches_legacy_utf16_and_context_envelopes_are_validated() {
    assert!(validate_user_content("   ").is_err());
    assert!(validate_user_content(&"a".repeat(9000)).is_ok());
    assert!(validate_user_content(&"a".repeat(9001)).is_err());
    assert!(validate_user_content(&"🦀".repeat(4500)).is_ok());
    assert!(validate_user_content(&"🦀".repeat(4501)).is_err());
    let mut user = message();
    user.role = Role::User;
    assert!(user.validate().is_err());
    user.status = MessageStatus::Complete;
    assert!(user.validate().is_ok());
    let mut context = SessionContext {
        session_id: "s".into(),
        messages: vec![json!({"role":"assistant","content":"ok"})],
        token_limit: 1024,
        estimated_tokens: 10,
        truncated: false,
        updated_at: 0,
    };
    assert!(context.validate().is_ok());
    context.estimated_tokens = 1025;
    assert!(context.validate().is_err());
    context.estimated_tokens = 0;
    context.messages = vec![json!({"role":"admin","content":"bad"})];
    assert!(context.validate().is_err());
    context.messages = vec![json!({"role":"user","content":7})];
    assert!(context.validate().is_err());
    context.messages = vec![json!({"role":"assistant","content":[{"text":"missing type"}]})];
    assert!(context.validate().is_err());
}

#[test]
fn encoding_stops_at_the_byte_budget_and_json_depth_is_bounded() {
    let mut value = message();
    value.content = "a".repeat(MAX_JSON_BYTES);
    // Cada campo isolado cabe; o envelope serializado não cabe.
    assert!(encode_json(&value).is_err());
    let mut nested = json!("leaf");
    for _ in 0..66 {
        nested = json!([nested]);
    }
    let part = MessagePart::ToolCall {
        id: "c".into(),
        name: "read_file".into(),
        args: [("nested".into(), nested)].into_iter().collect(),
        execution: None,
    };
    assert!(part.validate().is_err());
}
