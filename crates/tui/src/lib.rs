#![forbid(unsafe_code)]
//! Terminal UI; models and reducers remain independent of real terminal I/O.
pub mod input;
pub mod preferences;
pub mod theme;

/// Never allow untrusted terminal control sequences or bidi formatting into cells.
pub fn safe_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter(|c| {
            (!c.is_control() || *c == '\n' || *c == '\t')
                && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect::<String>()
        .replace('\t', "    ")
}
pub mod app;
pub mod approval;
pub mod dialog;
pub mod links;
pub mod ports;
pub mod preview;
pub mod render;
pub mod runtime;
pub mod terminal;
pub mod transcript;
