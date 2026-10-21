use myagent_tui::{
    input::Input,
    preferences::Preferences,
    theme::{Theme, format_tokens, token_ratio},
};
#[test]
fn unicode_editing_and_paste_keep_graphemes_and_input_limit() {
    let mut input = Input::default();
    assert!(input.insert("a🦀e\u{301}"));
    input.backspace();
    assert_eq!(input.text(), "a🦀");
    input.left();
    input.delete();
    assert_eq!(input.text(), "a");
    assert!(!input.insert(&"x".repeat(9000)));
    assert!(input.insert("\n中文"));
    input.home();
    assert_eq!(input.display_position(20), (0, 1));
    input.set("@src/ma");
    input.replace_mention("src/main.rs");
    assert_eq!(input.text(), "@src/main.rs");
    input.set("\u{1b}[31mtest\u{202e}");
    assert!(!input.text().contains('\u{1b}'));
    assert!(!input.text().contains('\u{202e}'));
}
#[test]
fn token_scale_is_ten_million_and_no_color_uses_terminal_defaults() {
    assert_eq!(token_ratio(2_400_000), 0.24);
    assert_eq!(token_ratio(u64::MAX), 1.0);
    assert_eq!(format_tokens(2_400_000), "2.4M");
    assert_eq!(
        Theme::Grafite.palette(true).fg,
        ratatui::style::Color::Reset
    );
}
#[test]
fn theme_preferences_round_trip_and_invalid_json_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preferences.json");
    Preferences {
        theme: Theme::Paper,
    }
    .save(&path)
    .unwrap();
    assert_eq!(Preferences::load(&path).unwrap().theme, Theme::Paper);
    std::fs::write(&path, "broken").unwrap();
    assert!(Preferences::load(&path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
}
#[cfg(unix)]
#[test]
fn preferences_refuse_arbitrary_links() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("prefs");
    std::fs::write(&file, "{}").unwrap();
    let alias = dir.path().join("preferences.json");
    std::os::unix::fs::symlink(&file, &alias).unwrap();
    assert!(Preferences::load(&alias).is_err());
    assert!(Preferences::default().save(&alias).is_err());
}

#[test]
fn cursor_and_visual_lines_share_exact_width_and_unicode_wrapping() {
    let mut input = Input::default();
    for (text, expected, position) in [
        ("abcd", vec!["abcd", ""], (0, 1)),
        ("abcd\nx", vec!["abcd", "x"], (1, 1)),
        ("abcd\n\nx", vec!["abcd", "", "x"], (1, 2)),
        ("abc🦀x", vec!["abc", "🦀x"], (3, 1)),
    ] {
        input.set(text);
        let (lines, cursor) = input.visual(4);
        assert_eq!(lines, expected);
        assert_eq!(cursor, position);
    }
}
