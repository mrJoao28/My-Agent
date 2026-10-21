//! Partial JSON is display-only. Completed ToolCall remains authoritative.
use std::collections::BTreeMap;
pub const MAX_PREVIEW_LINES: usize = 1000;
pub const MAX_PREVIEW_UNITS: usize = 80_000;
pub fn bounded(text: &str) -> (String, bool) {
    let clean = crate::safe_text(text);
    let mut units = 0;
    let mut lines = 1;
    let mut out = String::new();
    for c in clean.chars() {
        if units + c.len_utf16() > MAX_PREVIEW_UNITS || (c == '\n' && lines >= MAX_PREVIEW_LINES) {
            break;
        }
        units += c.len_utf16();
        if c == '\n' {
            lines += 1;
        }
        out.push(c);
    }
    let truncated = out.len() < clean.len();
    (out, truncated)
}
fn quoted(s: &str) -> Option<(String, usize, bool)> {
    if !s.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    while i < s.len() {
        let c = s[i..].chars().next()?;
        if c == '"' {
            return Some((out, i + 1, true));
        }
        if c == '\\' {
            let Some(&escape) = s.as_bytes().get(i + 1) else {
                break;
            };
            i += 2;
            match escape {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'u' => {
                    let Some(hex) = s.get(i..i + 4) else { break };
                    let mut cp = u32::from_str_radix(hex, 16).ok()?;
                    i += 4;
                    if (0xd800..=0xdbff).contains(&cp) {
                        if s.get(i..i + 2) != Some("\\u") {
                            break;
                        }
                        let Some(hex) = s.get(i + 2..i + 6) else {
                            break;
                        };
                        let low = u32::from_str_radix(hex, 16).ok()?;
                        if !(0xdc00..=0xdfff).contains(&low) {
                            return None;
                        }
                        cp = 0x10000 + ((cp - 0xd800) << 10) + (low - 0xdc00);
                        i += 6;
                    }
                    out.push(char::from_u32(cp)?);
                }
                _ => return None,
            }
        } else {
            if c.is_control() {
                return None;
            }
            out.push(c);
            i += c.len_utf8();
        }
    }
    Some((out, i, false))
}
pub fn fields(raw: &str) -> BTreeMap<String, (String, bool)> {
    let mut out = BTreeMap::new();
    if raw.len() > 65536 {
        return out;
    }
    let mut rest = raw.trim_start();
    let Some(s) = rest.strip_prefix('{') else {
        return out;
    };
    rest = s;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        let Some((key, n, true)) = quoted(rest) else {
            break;
        };
        rest = rest[n..].trim_start();
        let Some(s) = rest.strip_prefix(':') else {
            break;
        };
        rest = s.trim_start();
        if rest.starts_with('"') {
            let Some((value, n, complete)) = quoted(rest) else {
                break;
            };
            out.insert(key, (value, complete));
            if !complete {
                break;
            }
            rest = &rest[n..];
        } else {
            let mut stream =
                serde_json::Deserializer::from_str(rest).into_iter::<serde_json::Value>();
            if !matches!(stream.next(), Some(Ok(_))) {
                break;
            }
            rest = &rest[stream.byte_offset()..];
        }
    }
    out
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeStatus {
    Generating,
    AwaitingApproval,
    Applied,
    Checked,
    Failed,
    Interrupted,
}
impl CodeStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Generating => "Gerando prévia · não salvo",
            Self::AwaitingApproval => "Aguardando aprovação",
            Self::Applied => "Alteração aplicada",
            Self::Checked => "Validado · não aplicado",
            Self::Failed => "Não aplicado / falha",
            Self::Interrupted => "Interrompido · confira o estado",
        }
    }
}
#[derive(Clone, Debug)]
pub struct CodePanel {
    pub call_id: String,
    pub path: String,
    pub code: String,
    pub truncated: bool,
    pub status: CodeStatus,
}
pub fn panels(id: &str, name: &str, raw: &str, status: CodeStatus) -> Vec<CodePanel> {
    let values = fields(raw);
    let value = |key: &str| values.get(key).map(|(s, _)| s.as_str()).unwrap_or("");
    let mut drafts = vec![];
    if name == "apply_patch" {
        let patch = value("patch");
        let mut path = String::new();
        let mut body = String::new();
        for line in patch.lines() {
            if let Some(p) = line.strip_prefix("--- a/") {
                if !path.is_empty() {
                    drafts.push((std::mem::take(&mut path), std::mem::take(&mut body)));
                }
                path = p.split('\t').next().unwrap_or(p).to_owned();
            } else if let Some(p) = line.strip_prefix("+++ b/") {
                path = p.split('\t').next().unwrap_or(p).to_owned();
            }
            body.push_str(line);
            body.push('\n');
        }
        if !path.is_empty() {
            drafts.push((path, body));
        }
    } else if matches!(name, "write_file" | "edit_file")
        && values.get("path").is_some_and(|(_, complete)| *complete)
    {
        let code = if name == "edit_file" {
            value("old_str")
                .lines()
                .map(|line| format!("- {line}\n"))
                .chain(value("new_str").lines().map(|line| format!("+ {line}\n")))
                .collect()
        } else {
            value("content").to_owned()
        };
        drafts.push((value("path").to_owned(), code));
    }
    drafts
        .into_iter()
        .take(64)
        .map(|(path, code)| {
            let (code, truncated) = bounded(&code);
            CodePanel {
                call_id: id.into(),
                path: crate::safe_text(&path),
                code,
                truncated,
                status: status.clone(),
            }
        })
        .collect()
}
