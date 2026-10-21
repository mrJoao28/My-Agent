use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
#[derive(Clone, Debug, Default)]
pub struct Input {
    text: String,
    cursor: usize,
}
impl Input {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = crate::safe_text(&text.into());
        self.cursor = self.text.len();
    }
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
    pub fn insert(&mut self, text: &str) -> bool {
        let clean = crate::safe_text(text);
        if self.text.encode_utf16().count() + clean.encode_utf16().count()
            > myagent_protocol::MAX_MESSAGE_LENGTH
        {
            return false;
        }
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        true
    }
    pub fn left(&mut self) {
        self.cursor = self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i);
    }
    pub fn right(&mut self) {
        if let Some(g) = self.text[self.cursor..].graphemes(true).next() {
            self.cursor += g.len();
        }
    }
    pub fn backspace(&mut self) {
        let end = self.cursor;
        self.left();
        self.text.drain(self.cursor..end);
    }
    pub fn delete(&mut self) {
        let start = self.cursor;
        self.right();
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }
    pub fn home(&mut self) {
        self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
    }
    pub fn end(&mut self) {
        self.cursor += self.text[self.cursor..]
            .find('\n')
            .unwrap_or(self.text.len() - self.cursor);
    }
    pub fn vertical(&mut self, down: bool) {
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        let col = self.text[start..self.cursor].width();
        let target = if down {
            self.text[start..].find('\n').map(|n| start + n + 1)
        } else if start > 0 {
            Some(self.text[..start - 1].rfind('\n').map_or(0, |n| n + 1))
        } else {
            None
        };
        if let Some(target) = target {
            let line = self.text[target..].split('\n').next().unwrap_or("");
            let mut width = 0;
            self.cursor = target;
            for g in line.graphemes(true) {
                if width + g.width() > col {
                    break;
                }
                width += g.width();
                self.cursor += g.len();
            }
        }
    }
    pub fn mention(&self) -> Option<&str> {
        let tail = self.text[..self.cursor]
            .rsplit(char::is_whitespace)
            .next()?;
        tail.strip_prefix('@')
    }
    pub fn replace_mention(&mut self, path: &str) {
        if let Some(query) = self.mention() {
            let start = self.cursor - query.len();
            self.text.replace_range(start..self.cursor, path);
            self.cursor = start + path.len();
        }
    }
    /// The renderer and cursor share one grapheme-aware wrapping calculation.
    pub fn visual(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(1);
        let mut lines = vec![String::new()];
        let (mut col, mut row) = (0, 0);
        let mut cursor = (0, 0);
        let mut full = false;
        for (index, g) in self.text.grapheme_indices(true) {
            if g == "\n" {
                if !full {
                    lines.push(String::new());
                    row += 1;
                }
                col = 0;
                full = false;
            } else {
                let w = g.width();
                if col + w > width && col > 0 {
                    lines.push(String::new());
                    row += 1;
                    col = 0;
                }
                if index == self.cursor {
                    cursor = (col, row);
                }
                lines[row].push_str(g);
                col += w;
                full = col >= width;
                if full {
                    lines.push(String::new());
                    row += 1;
                    col = 0;
                }
            }
            if index + g.len() == self.cursor {
                cursor = (col, row);
            }
        }
        (lines, cursor)
    }
    pub fn display_position(&self, width: usize) -> (usize, usize) {
        self.visual(width).1
    }
}
