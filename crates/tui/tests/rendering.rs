use myagent_tui::{
    app::{App, Screen},
    approval::Approval,
    dialog::{Choice, Dialog, DialogKind},
    preview::CodeStatus,
    render::draw,
    theme::Theme,
    transcript::Speaker,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
fn app() -> App {
    let mut app = App::new(
        "my-agent / feat/m6-tui".into(),
        vec![],
        "Ollama · local".into(),
        Theme::Grafite,
        false,
    );
    app.session = Some("fixture".into());
    app.measured = Some(2_400_000);
    app
}
fn capture(app: &mut App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn grafite_code_panels_header_and_single_ten_million_bar() {
    let mut app = app();
    app.screen = Screen::Session;
    app.transcript.text(
        Speaker::Assistant,
        "Validação concluída. Preparando os testes.",
        false,
    );
    app.transcript.preview("first","write_file",r##"{"path":"src/validation.rs","content":"pub fn validate_value(value: i32) -> bool {\n    value >= 0\n}"}"##,CodeStatus::Applied);
    app.transcript.preview("second","write_file",r##"{"path":"tests/validation.rs","content":"#[test]\nfn zero_is_valid() {\n    assert!(validate_value(0));"##,CodeStatus::Generating);
    let text = capture(&mut app, 120, 36);
    insta::assert_snapshot!("grafite_session", text);
    assert_eq!(text.matches("/ 10M").count(), 1);
    assert!(text.lines().next().unwrap().contains("BUILD"));
    assert!(!text.lines().next().unwrap().contains("Tab"));
}
#[test]
fn approvals_show_diff_scope_keyboard_and_truncation_warning() {
    let mut app = app();
    app.active = true;
    app.approval=Some(Approval::new("id".into(),"edit_file".into(),json!({"path":"src/a.rs","preview":"--- src/a.rs\n+++ src/a.rs\n- old\n+ new","previewTruncated":true}).as_object().unwrap().clone(),Some("Paths: src/a.rs".into()),true,None));
    insta::assert_snapshot!("grafite_approval", capture(&mut app, 110, 25));
}
#[test]
fn home_new_and_modal_views_are_stable() {
    let mut app = app();
    insta::assert_snapshot!("grafite_home", capture(&mut app, 100, 24));
    app.screen = Screen::NewSession;
    insta::assert_snapshot!("grafite_new_session", capture(&mut app, 100, 24));
    app.dialog = Some(Dialog::new(
        DialogKind::Themes,
        "Temas",
        vec![
            Choice {
                id: "Grafite".into(),
                label: "Grafite".into(),
            },
            Choice {
                id: "Papel".into(),
                label: "Papel".into(),
            },
        ],
    ));
    insta::assert_snapshot!("grafite_theme_dialog", capture(&mut app, 100, 24));
}
#[test]
fn small_resized_and_no_color_terminals_remain_valid() {
    for (w, h) in [(1, 1), (20, 5), (45, 13), (80, 24), (160, 45)] {
        let mut app = app();
        app.no_color = true;
        app.input.set("🦀中文\nmultiline");
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        for cell in terminal.backend().buffer().content() {
            assert_eq!(cell.fg, ratatui::style::Color::Reset);
            assert_eq!(cell.bg, ratatui::style::Color::Reset);
        }
    }
}
