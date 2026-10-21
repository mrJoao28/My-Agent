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
