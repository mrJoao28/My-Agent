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
        r"C:\Users\fixture\My-Agent".into(),
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
    app.transcript.text_in_mode(
        Speaker::Assistant,
        "Validação concluída. Preparando os testes.",
        false,
        Some(myagent_protocol::Mode::Build),
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

#[test]
fn workspace_labels_are_portable_and_do_not_modify_identity() {
    use myagent_tui::render::workspace_name;
    for (path, name) in [
        (r"C:\Users\fixture\My-Agent", "My-Agent"),
        ("/home/fixture/My-Agent/", "My-Agent"),
        ("/Users/fixture/My-Agent", "My-Agent"),
        ("C:\\", "C:"),
        ("/", "/"),
        (r"\\server\share\", "share"),
        (r"\\?\C:\Users\fixture\My-Agent", "My-Agent"),
    ] {
        assert_eq!(workspace_name(path), name);
        let mut app = app();
        app.workspace = path.into();
        let text = capture(&mut app, 100, 24);
        assert!(text.lines().next().unwrap().contains(name));
        assert_eq!(app.workspace, path);
    }
}

#[test]
fn multiline_geometry_and_cursor_are_stable_during_resize_and_navigation() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    for (w, h) in [(45, 13), (80, 24), (120, 36)] {
        let mut app = app();
        let empty = capture(&mut app, w, h);
        let baseline = app.chat_area;
        app.event(Event::Paste("🦀中文/example\n".repeat(50)));
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        for code in [
            KeyCode::Left,
            KeyCode::Up,
            KeyCode::Home,
            KeyCode::Down,
            KeyCode::End,
        ] {
            app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            assert_eq!(app.chat_area, baseline);
            let (x, y) = ratatui::backend::Backend::get_cursor_position(terminal.backend_mut())
                .map(|p| (p.x, p.y))
                .unwrap();
            assert!(x > 0 && x < w - 1);
            assert!(y > baseline.bottom() && y < h - 5);
        }
        let filled = capture(&mut app, w, h);
        for marker in ["Mensagem", "Tokens:", "Tab modo"] {
            assert_eq!(
                empty.lines().position(|l| l.contains(marker)),
                filled.lines().position(|l| l.contains(marker))
            );
        }
    }
}

#[test]
fn response_borders_retain_mode_colors_and_no_color_labels() {
    use myagent_protocol::Mode;
    for no_color in [false, true] {
        let mut app = app();
        app.no_color = no_color;
        app.transcript
            .text_in_mode(Speaker::Assistant, "planned", false, Some(Mode::Plan));
        app.transcript
            .text_in_mode(Speaker::Assistant, "built", false, Some(Mode::Build));
        app.mode = Mode::Plan;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let palette = app.theme.palette(no_color);
        let mut found = 0;
        for y in 2..app.chat_area.bottom() {
            let line: String = (0..80).map(|x| buffer[(x, y)].symbol()).collect();
            for (label, color) in [
                ("My-Agent · PLAN", palette.plan),
                ("My-Agent · BUILD", palette.build),
            ] {
                if line.contains(label) {
                    assert_eq!(buffer[(0, y)].fg, color);
                    found += 1;
                }
            }
        }
        assert_eq!(found, 2);
    }
}

#[test]
fn resizing_preserves_input_and_keeps_suggestions_above_the_editor() {
    let mut app = app();
    app.input.set("/");
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    for (w, h) in [(100, 30), (45, 13), (80, 24), (120, 36)] {
        terminal.backend_mut().resize(w, h);
        terminal
            .resize(ratatui::layout::Rect::new(0, 0, w, h))
            .unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let y = app.chat_area.bottom() + 1;
        assert_eq!(terminal.backend().buffer()[(2, y)].symbol(), "/");
        assert_eq!(app.input.text(), "/");
    }
}
