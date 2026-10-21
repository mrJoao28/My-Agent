use crate::{
    Error, Result, SafeWorkspace,
    io::{Budget, cut, read_text, scan, units},
    registry::{flag, number, text},
};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use regex::{Regex, RegexBuilder};
use serde_json::{Map, Value, json};
use std::path::Path;
pub(crate) fn literal_glob(pattern: &str) -> Result<Regex> {
    let chars: Vec<_> = pattern.chars().collect();
    let mut s = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if chars.get(i + 1) == Some(&'*') {
                    i += 1;
                    if chars.get(i + 1) == Some(&'/') {
                        i += 1;
                        s.push_str("(?:.*/)?");
                    } else {
                        s.push_str(".*");
                    }
                } else {
                    s.push_str("[^/]*");
                }
            }
            '?' => s.push_str("[^/]"),
            c => s.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    s.push('$');
    RegexBuilder::new(&s)
        .size_limit(2 * 1024 * 1024)
        .build()
        .map_err(|_| Error::InvalidPattern)
}
fn load_ignore(ws: &SafeWorkspace, b: &Budget) -> Result<Gitignore> {
    let content = match read_text(ws, ".gitignore", 65_536, b) {
        Ok((s, _)) => s,
        Err(Error::NotFound) => String::new(),
        Err(e) => return Err(e),
    };
    let mut builder = GitignoreBuilder::new(ws.root());
    let mut count = 0;
    for line in content.trim_start_matches('\u{feff}').lines() {
        b.check()?;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        count += 1;
        if count > 2000 {
            return Err(Error::IgnoreTooLarge);
        }
        builder
            .add_line(None, &git_pattern(line))
            .map_err(|_| Error::InvalidPattern)?;
    }
    builder.build().map_err(|_| Error::InvalidPattern)
}
fn git_pattern(line: &str) -> String {
    let mut result = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            result.push(c);
            if let Some(escaped) = chars.next() {
                result.push(escaped);
            }
        } else {
            if "{}()".contains(c) {
                result.push('\\');
            }
            result.push(c);
        }
    }
    result
}
fn ignored(ws: &SafeWorkspace, ignore: &Gitignore, path: &Path, directory: bool) -> bool {
    let Ok(rel) = ws.relative(path) else {
        return true;
    };
    if rel == "." {
        return false;
    }
    let parts: Vec<_> = rel.split('/').collect();
    let mut prefix = ws.root().to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        if ["node_modules", ".git", "dist", "build", ".next"].contains(part) {
            return true;
        }
        prefix.push(part);
        if ignore
            .matched(&prefix, index + 1 < parts.len() || directory)
            .is_ignore()
        {
            return true;
        }
    }
    false
}
pub(crate) fn untracked(
    ws: &SafeWorkspace,
    tracked: &[String],
    b: &Budget,
) -> Result<(Vec<String>, bool)> {
    let ignore = load_ignore(ws, b)?;
    let scan = scan(ws, ws.root(), b, false, |p, d| ignored(ws, &ignore, p, d))?;
    Ok((
        scan.files
            .into_iter()
            .filter(|p| !tracked.contains(p))
            .collect(),
        scan.truncated,
    ))
}
pub(crate) fn execute(
    ws: &SafeWorkspace,
    name: &str,
    a: &Map<String, Value>,
    b: &Budget,
) -> Result<Value> {
    let root = ws.resolve_safe_path(text(a, "path", "."), true)?;
    let ignore = load_ignore(ws, b)?;
    let scan = scan(ws, &root, b, false, |p, d| ignored(ws, &ignore, p, d))?;
    let pattern = text(a, "pattern", "");
    if name == "glob" {
        if !root.is_dir() {
            return Err(Error::NotADirectory);
        }
        let matcher = literal_glob(pattern)?;
        let mut matches = vec![];
        let mut truncated = scan.truncated;
        for file in scan.files {
            b.check()?;
            let p = ws.root().join(&file);
            let rel = p
                .strip_prefix(&root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            if matcher.is_match(&rel) {
                if matches.len() == 500 {
                    truncated = true;
                    break;
                }
                matches.push(rel);
            }
        }
        return Ok(json!({"pattern":pattern,"matches":matches,"truncated":truncated}));
    }
    let case = flag(a, "caseSensitive", false);
    let literal = flag(a, "literal", false);
    let only = flag(a, "files_only", false);
    let context = number(a, "context", 0);
    let needle = if literal {
        regex::escape(pattern)
    } else {
        pattern.to_owned()
    };
    let regex = RegexBuilder::new(&needle)
        .case_insensitive(!case)
        .size_limit(2 * 1024 * 1024)
        .dfa_size_limit(2 * 1024 * 1024)
        .nest_limit(128)
        .build()
        .map_err(|_| Error::InvalidPattern)?;
    let include = if a.contains_key("include") {
        Some(
            globset::GlobBuilder::new(text(a, "include", ""))
                .literal_separator(true)
                .build()
                .map_err(|_| Error::InvalidPattern)?
                .compile_matcher(),
        )
    } else {
        None
    };
    let mut matches = vec![];
    let mut files = vec![];
    let mut chars = 0;
    let mut bytes = 0;
    let mut truncated = scan.truncated;
    'files: for file in scan.files {
        b.check()?;
        let p = ws.root().join(&file);
        let rel = if root.is_file() {
            p.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            p.strip_prefix(&root)
                .map_err(|_| Error::PathBlocked)?
                .to_string_lossy()
                .replace('\\', "/")
        };
        if let Some(ref matcher) = include {
            let candidate = if text(a, "include", "").contains('/') {
                rel.as_str()
            } else {
                rel.rsplit('/').next().unwrap_or(&rel)
            };
            if !matcher.is_match(candidate) {
                continue;
            }
        }
        let size = std::fs::symlink_metadata(&p)?.len();
        if size > 1_000_000 {
            continue;
        }
        if bytes + size as usize > 20_000_000 {
            truncated = true;
            break;
        }
        let (raw, len) = match read_text(ws, &file, 1_000_000.min(20_000_000 - bytes), b) {
            Ok(r) => r,
            Err(Error::BinaryFile) => continue,
            Err(Error::FileTooLarge) => {
                truncated = true;
                break;
            }
            Err(e) => return Err(e),
        };
        bytes += len;
        let lines: Vec<_> = raw
            .split('\n')
            .map(|s| s.strip_suffix('\r').unwrap_or(s))
            .collect();
        for (index, line) in lines.iter().enumerate() {
            b.check()?;
            if !regex.is_match(line) {
                continue;
            }
            if only {
                let cost = units(&json!(file).to_string());
                if chars + cost > 60_000 {
                    truncated = true;
                    break 'files;
                }
                files.push(file.clone());
                chars += cost;
                break;
            }
            let mut item = json!({"file":file,"line":index+1,"text":cut(line.trim(),300)});
            if context > 0 {
                let entry = |i: usize| json!({"line":i+1,"text":cut(lines[i],300)});
                item["before"] = (index.saturating_sub(context)..index)
                    .map(entry)
                    .collect::<Vec<_>>()
                    .into();
                item["after"] = (index + 1..(index + 1 + context).min(lines.len()))
                    .map(entry)
                    .collect::<Vec<_>>()
                    .into();
            }
            let cost = units(&item.to_string());
            if chars + cost > 60_000 {
                truncated = true;
                break 'files;
            }
            chars += cost;
            matches.push(item);
            if matches.len() >= 200 {
                truncated = true;
                break 'files;
            }
        }
        if files.len() >= 200 {
            truncated = true;
            break;
        }
    }
    let mut value = json!({"pattern":pattern,"matches":matches,"truncated":truncated});
    if only {
        value["files"] = files.into();
    }
    Ok(value)
}
