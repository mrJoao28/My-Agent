use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Grafite,
    Nightfox,
    Paper,
}
impl Theme {
    pub const ALL: [Self; 3] = [Self::Grafite, Self::Nightfox, Self::Paper];
    pub fn name(self) -> &'static str {
        match self {
            Self::Grafite => "Grafite",
            Self::Nightfox => "Nightfox",
            Self::Paper => "Papel",
        }
    }
    pub fn palette(self, no_color: bool) -> Palette {
        if no_color {
            return Palette {
                bg: Color::Reset,
                fg: Color::Reset,
                muted: Color::Reset,
                border: Color::Reset,
                plan: Color::Reset,
                build: Color::Reset,
                good: Color::Reset,
                bad: Color::Reset,
            };
        }
        match self {
            Self::Grafite => Palette {
                bg: Color::Rgb(24, 24, 24),
                fg: Color::Rgb(225, 225, 225),
                muted: Color::Rgb(150, 154, 163),
                border: Color::Rgb(92, 98, 109),
                plan: Color::Rgb(95, 194, 232),
                build: Color::Rgb(255, 190, 50),
                good: Color::Rgb(120, 202, 151),
                bad: Color::Rgb(236, 125, 125),
            },
            Self::Nightfox => Palette {
                bg: Color::Rgb(25, 33, 48),
                fg: Color::Rgb(205, 214, 230),
                muted: Color::Rgb(143, 155, 179),
                border: Color::Rgb(82, 101, 128),
                plan: Color::Rgb(129, 178, 223),
                build: Color::Rgb(236, 176, 113),
                good: Color::Rgb(165, 196, 139),
                bad: Color::Rgb(215, 131, 143),
            },
            Self::Paper => Palette {
                bg: Color::Rgb(245, 243, 237),
                fg: Color::Rgb(40, 43, 51),
                muted: Color::Rgb(94, 101, 113),
                border: Color::Rgb(155, 156, 159),
                plan: Color::Rgb(16, 101, 160),
                build: Color::Rgb(155, 89, 0),
                good: Color::Rgb(32, 112, 62),
                bad: Color::Rgb(163, 45, 51),
            },
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub border: Color,
    pub plan: Color,
    pub build: Color,
    pub good: Color,
    pub bad: Color,
}
impl Palette {
    pub fn base(self) -> Style {
        Style::default().bg(self.bg).fg(self.fg)
    }
}
pub const TOKEN_SCALE: u64 = 10_000_000;
pub fn token_ratio(tokens: u64) -> f64 {
    tokens.min(TOKEN_SCALE) as f64 / TOKEN_SCALE as f64
}
pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}
