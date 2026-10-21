use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use myagent_protocol::Mode;
use myagent_tui::{
    app::{Action, App},
    approval::{Approval, Decision},
    dialog::{Choice, Dialog, DialogKind},
    preview::{self, CodeStatus},
    theme::Theme,
};
use serde_json::json;
fn app() -> App {
    let mut app = App::new(
        "fixture".into(),
        vec![],
        "fixture".into(),
        Theme::Grafite,
        false,
    );
    app.session = Some("session".into());
    app
}
fn key(app: &mut App, code: KeyCode) -> Vec<Action> {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}
fn approval(tool: &str, allow: bool) -> Approval {
    Approval::new(
        "approval".into(),
        tool.into(),
        json!({"path":"file.rs","preview":"- old\n+ new"})
            .as_object()
            .unwrap()
            .clone(),
        None,
        allow,
        None,
    )
}
#[test]
fn enter_tab_and_background_shortcuts_never_approve_or_edit() {
    let mut app = app();
    app.active = true;
    app.input.set("must remain");
    app.approval = Some(approval("edit_file", true));
    for code in [
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::Backspace,
        KeyCode::Char('/'),
    ] {
        assert!(key(&mut app, code).is_empty());
    }
    assert_eq!(app.input.text(), "must remain");
    assert_eq!(app.mode, Mode::Build);
    assert!(matches!(
        key(&mut app, KeyCode::Char('y')).as_slice(),
        [Action::Approve {
            decision: Decision::Once,
            ..
        }]
    ));
}
#[test]
fn individual_approvals_and_main_confirmation_reject_wrong_keys() {
    for tool in [
        "delete_file",
        "git_commit",
        "git_init",
        "git_remote_add",
        "git_push",
    ] {
        assert_eq!(approval(tool, true).decision('a'), None);
    }
    let mut approval = approval("git_push", false);
    approval.confirmation = Some("main".into());
    assert_eq!(approval.decision('y'), None);
    assert_eq!(approval.decision('m'), Some(Decision::ConfirmMain));
}
#[test]
fn dialog_has_priority_over_approval_and_generation_then_escape_cancels() {
    let mut app = app();
    app.active = true;
    app.approval = Some(approval("edit_file", true));
    app.dialog = Some(Dialog::new(
        DialogKind::Themes,
        "Temas",
        vec![Choice {
            id: "Papel".into(),
            label: "Papel".into(),
        }],
    ));
    assert!(key(&mut app, KeyCode::Esc).is_empty());
    assert!(app.approval.is_some());
    assert!(app.active);
    assert!(matches!(
        key(&mut app, KeyCode::Esc).as_slice(),
        [Action::Cancel]
    ));
    assert!(app.cancelling);
}
#[test]
fn mode_changes_affect_next_request_and_input_is_blocked_while_running() {
    let mut app = app();
    app.input.set("request");
    let actions = key(&mut app, KeyCode::Enter);
    assert!(matches!(actions.as_slice(),[Action::Submit(input)] if input.mode==Mode::Build));
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.mode, Mode::Plan);
    assert_eq!(app.running_mode, Some(Mode::Build));
    key(&mut app, KeyCode::Char('x'));
    assert!(app.input.text().is_empty());
}
#[test]
fn undo_is_local_command_and_never_sent_to_provider() {
    let mut app = app();
    app.input.set("/undo");
    assert!(matches!(
        key(&mut app, KeyCode::Enter).as_slice(),
        [Action::Undo(_)]
    ));
}
#[test]
fn partial_unicode_json_creates_safe_code_panels_without_marking_writes() {
    let raw = r#"{"path":"src/a.rs","content":"a\n\uD83E\uDD80"#;
    let panels = preview::panels("id", "write_file", raw, CodeStatus::Generating);
    assert_eq!(panels.len(), 1);
    assert_eq!(panels[0].code, "a\n🦀");
    assert_eq!(panels[0].status, CodeStatus::Generating);
    assert!(
        preview::panels(
            "id",
            "write_file",
            r#"{"path":"src/a"#,
            CodeStatus::Generating
        )
        .is_empty()
    );
    let bounded = preview::bounded(&"line\n".repeat(1500));
    assert!(bounded.1);
    assert!(bounded.0.lines().count() <= 1000);
    let approval = Approval::new(
        "id".into(),
        "web_fetch".into(),
        json!({"url":format!("https://example.com/{}","x".repeat(3000))})
            .as_object()
            .unwrap()
            .clone(),
        None,
        true,
        None,
    );
    assert!(approval.lines.join("\n").contains(&"x".repeat(3000)));
}

#[test]
fn approval_scroll_and_mouse_stay_inside_the_top_layer() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let mut app = app();
    let mut panel = approval("edit_file", true);
    panel.max_scroll = 80;
    app.approval = Some(panel);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::PageDown);
    assert_eq!(app.approval.as_ref().unwrap().scroll, 13);
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 2,
        row: 3,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(app.approval.as_ref().unwrap().scroll, 16);
    assert_eq!(app.scroll, 0);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.approval.as_ref().unwrap().scroll, 80);
    key(&mut app, KeyCode::PageUp);
    key(&mut app, KeyCode::Up);
    assert_eq!(app.approval.as_ref().unwrap().scroll, 67);
    key(&mut app, KeyCode::Home);
    assert_eq!(app.approval.as_ref().unwrap().scroll, 0);
    assert!(matches!(
        key(&mut app, KeyCode::Char('n')).as_slice(),
        [Action::Approve {
            decision: Decision::Deny,
            ..
        }]
    ));
}

#[test]
fn code_panels_remain_previews_until_the_matching_tool_result() {
    use myagent_tui::transcript::{Entry, Transcript};
    let mut transcript = Transcript::default();
    let raw = r#"{"path":"one.rs","content":"pub fn one() {}"}"#;
    transcript.preview("one", "write_file", raw, CodeStatus::Generating);
    let first = |transcript: &Transcript| match &transcript.entries[0] {
        Entry::Code(code) => code.status.clone(),
        _ => panic!("code expected"),
    };
    assert_eq!(first(&transcript), CodeStatus::Generating);
    let args = serde_json::from_str(raw).unwrap();
    transcript.call("one", "write_file", &args);
    assert_eq!(first(&transcript), CodeStatus::AwaitingApproval);
    transcript.preview(
        "two",
        "write_file",
        r#"{"path":"two.rs","content":"unfinished"#,
        CodeStatus::Generating,
    );
    transcript.result("other", "{}");
    assert_eq!(first(&transcript), CodeStatus::AwaitingApproval);
    transcript.result("one", r#"{"bytesWritten":15}"#);
    assert_eq!(first(&transcript), CodeStatus::Applied);
    transcript.interrupted();
    assert!(transcript.entries.iter().any(|entry| matches!(entry, Entry::Code(code) if code.path == "two.rs" && code.status == CodeStatus::Interrupted)));
    assert_eq!(first(&transcript), CodeStatus::Applied);
}

#[test]
fn preview_character_limit_and_action_endpoints_are_explicit() {
    let (text, truncated) = preview::bounded(&"🦀".repeat(45_000));
    assert!(truncated);
    assert!(text.encode_utf16().count() <= 80_000);
    for (tool, args, expected) in [
        (
            "bash",
            json!({"command":"printf 'fixture'"}),
            vec!["printf 'fixture'"],
        ),
        (
            "move_file",
            json!({"from":"src/old.rs","to":"src/new.rs"}),
            vec!["src/old.rs", "src/new.rs"],
        ),
        (
            "delete_file",
            json!({"path":"src/old.rs"}),
            vec!["src/old.rs"],
        ),
        (
            "write_file",
            json!({"path":"new.rs","content":"Olá"}),
            vec!["new.rs", "4 bytes", "Olá"],
        ),
    ] {
        let panel = Approval::new(
            "id".into(),
            tool.into(),
            args.as_object().unwrap().clone(),
            None,
            true,
            None,
        );
        let text = panel.lines.join("\n");
        for expected in expected {
            assert!(text.contains(expected), "{tool}: missing {expected}");
        }
    }
}

#[test]
fn slash_keyboard_modifiers_and_paste_preserve_literal_text() {
    for modifiers in [
        KeyModifiers::NONE,
        KeyModifiers::SHIFT,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
    ] {
        let mut app = app();
        app.event(Event::Key(KeyEvent::new(KeyCode::Char('/'), modifiers)));
        assert_eq!(app.input.text(), "/");
        assert!(app.command_menu_open());
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.input.text(), "/");
        assert!(!app.command_menu_open());
        app.event(Event::Paste("https://example.com/a/b\r\n/tmp/a".into()));
        assert_eq!(app.input.text(), "/https://example.com/a/b\n/tmp/a");
    }
    for text in [
        "/",
        "/tmp/a",
        "/new project",
        "/new\nsecond line",
        "https://example.com/a/b",
        "first\n/next",
    ] {
        for paste in [false, true] {
            let mut app = app();
            if paste {
                app.event(Event::Paste(text.into()));
            } else {
                for c in text.chars() {
                    if c == '\n' {
                        app.event(Event::Key(KeyEvent::new(
                            KeyCode::Char('j'),
                            KeyModifiers::CONTROL,
                        )));
                    } else {
                        key(&mut app, KeyCode::Char(c));
                    }
                }
            }
            assert_eq!(app.input.text(), text);
            let actions = key(&mut app, KeyCode::Enter);
            assert!(
                matches!(actions.as_slice(), [Action::Submit(input)] if input.content.as_deref() == Some(text))
            );
        }
    }
}

#[test]
fn suggestions_complete_explicitly_and_altgr_never_approves() {
    let mut app = app();
    app.input.set("/mod");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.input.text(), "/models");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.dialog.as_ref().unwrap().kind, DialogKind::Models);
    app.dialog = None;
    app.approval = Some(approval("write_file", true));
    for c in ['y', 'a', 'n', 'c'] {
        assert!(
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )))
            .is_empty()
        );
        assert!(app.approval.is_some());
    }
}

#[test]
fn streaming_keeps_generation_mode_after_next_mode_changes() {
    use myagent_protocol::StreamEvent;
    use myagent_tui::transcript::Entry;
    let mut app = app();
    app.mode = Mode::Plan;
    app.input.set("request");
    key(&mut app, KeyCode::Enter);
    app.stream(StreamEvent::TextDelta {
        text: "first".into(),
    });
    key(&mut app, KeyCode::Tab);
    app.stream(StreamEvent::TextDelta {
        text: " second".into(),
    });
    assert_eq!(app.mode, Mode::Build);
    assert!(
        matches!(app.transcript.entries.last(), Some(Entry::Text { mode: Some(Mode::Plan), text, .. }) if text == "first second")
    );
}

#[test]
fn persisted_messages_keep_modes_and_message_boundaries() {
    use myagent_protocol::{Message, MessagePart, MessageStatus, Role};
    use myagent_tui::transcript::{Entry, Transcript};
    let messages = [Mode::Plan, Mode::Build, Mode::Build].map(|mode| Message {
        id: "fixture".into(),
        session_id: "session".into(),
        generation_id: None,
        role: Role::Assistant,
        status: MessageStatus::Complete,
        model: "fixture".into(),
        mode,
        content: "reply".into(),
        parts: vec![MessagePart::Text {
            text: "reply".into(),
        }],
        duration: None,
        total_tokens: None,
        created_at: 0,
    });
    let transcript = Transcript::history(&messages);
    assert_eq!(transcript.entries.len(), 3);
    for (entry, message) in transcript.entries.iter().zip(&messages) {
        assert!(
            matches!(entry, Entry::Text { mode: Some(mode), text, .. } if *mode == message.mode && text == "reply")
        );
    }
    let mut plain = messages[0].clone();
    plain.parts.clear();
    assert!(matches!(
        &Transcript::history(&[plain]).entries[0],
        Entry::Text {
            mode: Some(Mode::Plan),
            ..
        }
    ));
}
