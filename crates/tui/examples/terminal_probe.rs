//! PTY probe for normal and panic restoration, without configuration or providers.
fn main() -> std::io::Result<()> {
    let (_guard, mut terminal) = myagent_tui::terminal::TerminalGuard::enter()?;
    terminal.draw(|frame| {
        frame.render_widget(
            ratatui::widgets::Paragraph::new("terminal probe"),
            frame.area(),
        );
    })?;
    if std::env::args().any(|arg| arg == "--panic") {
        panic!("intentional restoration probe");
    }
    Ok(())
}
