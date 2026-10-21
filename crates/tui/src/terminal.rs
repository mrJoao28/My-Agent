use crossterm::{
    cursor::{Hide, Show},
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, IsTerminal, Stdout},
    sync::{
        Once,
        atomic::{AtomicBool, Ordering},
    },
};
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PANIC_HOOK: Once = Once::new();
pub type NativeTerminal = Terminal<CrosstermBackend<Stdout>>;
pub struct TerminalGuard;
fn restore() {
    if ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            Show,
            LeaveAlternateScreen
        );
    }
}
impl TerminalGuard {
    pub fn is_active() -> bool {
        ACTIVE.load(Ordering::SeqCst)
    }
    pub fn enter() -> io::Result<(Self, NativeTerminal)> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(io::Error::other("A TUI requer um terminal interativo."));
        }
        PANIC_HOOK.call_once(|| {
            let prior = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                restore();
                prior(info);
            }));
        });
        if ACTIVE.swap(true, Ordering::SeqCst) {
            return Err(io::Error::other("terminal already active"));
        }
        let guard = Self;
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            Hide
        )?;
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok((guard, terminal))
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}
