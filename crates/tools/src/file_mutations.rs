use crate::registry::{flag, text};
use crate::{
    Error, Result, SafeWorkspace,
    checkpoints::CHECKPOINT_FILE_BYTES,
    io::{Budget, cut},
    mutation_io::*,
};
use serde_json::{Map, Value, json};
use std::{fs, path::PathBuf};
pub(crate) enum Action {
    Write {
        path: String,
        data: Vec<u8>,
        mode: u32,
        no_clobber: bool,
    },
    Delete(String),
    Move {
        from: String,
        to: String,
    },
    Mode {
        path: String,
        mode: u32,
    },
}
pub(crate) struct FilePlan {
    pub paths: Vec<String>,
    pub actions: Vec<Action>,
    pub result: Value,
    pub preview: String,
    pub addition: usize,
    pub backup: bool,
}
fn target(ws: &SafeWorkspace, path: &str, must_exist: bool) -> Result<(PathBuf, String)> {
    let safe = ws.resolve_safe_path(path, must_exist)?;
    let rel = ws.relative(&safe)?;
    if rel == "." {
        return Err(Error::PathBlocked);
    }
    Ok((safe, rel))
}
fn read(ws: &SafeWorkspace, path: &str) -> Result<(Vec<u8>, u32)> {
    let (safe, _) = target(ws, path, true)?;
    let info = fs::symlink_metadata(&safe)?;
    if !info.is_file() {
        return Err(Error::NotAFile);
    }
    Ok((bounded_file(&safe, CHECKPOINT_FILE_BYTES)?, mode(&info)))
}
pub(crate) fn prepare(
    ws: &SafeWorkspace,
    name: &str,
    a: &Map<String, Value>,
    b: &Budget,
) -> Result<FilePlan> {
    b.check()?;
    match name {
        "write_file" => {
            let (safe, path) = target(ws, text(a, "path", ""), false)?;
            let data = text(a, "content", "").as_bytes().to_vec();
            if data.len() > CHECKPOINT_FILE_BYTES {
                return Err(Error::CheckpointLimit);
            }
            let exists = safe.try_exists()?;
            if exists && !flag(a, "overwrite", true) {
                return Err(Error::AlreadyExists);
            }
            let mut mode = 0o644;
            let mut actions = vec![];
            let mut paths = vec![path.clone()];
            if exists {
                let (old, previous_mode) = read(ws, &path)?;
                mode = previous_mode;
                if flag(a, "backup", false) {
                    let (_, backup) = target(ws, &format!("{path}.bak"), false)?;
                    if ws.root().join(&backup).try_exists()? {
                        return Err(Error::AlreadyExists);
                    }
                    paths.push(backup.clone());
                    actions.push(Action::Write {
                        path: backup,
                        data: old,
                        mode,
                        no_clobber: true,
                    });
                }
            }
            actions.push(Action::Write {
                path: path.clone(),
                data: data.clone(),
                mode,
                no_clobber: !flag(a, "overwrite", true),
            });
            let preview = format!(
                "Write {path}: {} bytes\n{}",
                data.len(),
                String::from_utf8_lossy(&data)
            );
            Ok(FilePlan {
                paths,
                actions,
                result: json!({"path":path,"bytesWritten":data.len(),"overwritten":exists,"backupPath":if exists&&flag(a,"backup",false){Some(format!("{path}.bak"))}else{None}}),
                preview,
                addition: data.len(),
                backup: flag(a, "backup", false),
            })
        }
        "edit_file" => {
            let (_, path) = target(ws, text(a, "path", ""), true)?;
            let (raw, mode) = read(ws, &path)?;
            if raw.contains(&0) {
                return Err(Error::BinaryFile);
            }
            let old = String::from_utf8(raw).map_err(|_| Error::BinaryFile)?;
            let updated = replace_unique(&old, text(a, "old_str", ""), text(a, "new_str", ""))?;
            if updated.len() > CHECKPOINT_FILE_BYTES {
                return Err(Error::CheckpointLimit);
            }
            let preview = format!(
                "--- {path}\n+++ {path}\n-{}\n+{}",
                text(a, "old_str", ""),
                text(a, "new_str", "")
            );
            Ok(FilePlan {
                paths: vec![path.clone()],
                actions: vec![Action::Write {
                    path: path.clone(),
                    data: updated.into_bytes(),
                    mode,
                    no_clobber: false,
                }],
                result: json!({"path":path,"replaced":true}),
                preview,
                addition: text(a, "new_str", "").len(),
                backup: false,
            })
        }
        "delete_file" => {
            let (safe, path) = target(ws, text(a, "path", ""), true)?;
            if safe.is_dir() && !flag(a, "recursive", false) {
                return Err(Error::InvalidInput);
            }
            Ok(FilePlan {
                paths: vec![path.clone()],
                actions: vec![Action::Delete(path.clone())],
                result: json!({"path":path,"deleted":true}),
                preview: format!("Delete {path}; recursive={}", flag(a, "recursive", false)),
                addition: 0,
                backup: false,
            })
        }
        "move_file" => {
            let (_, from) = target(ws, text(a, "from", ""), true)?;
            let (to_safe, to) = target(ws, text(a, "to", ""), false)?;
            if from == to || to.starts_with(&format!("{from}/")) {
                return Err(Error::PathBlocked);
            }
            if to_safe.try_exists()? && !flag(a, "overwrite", false) {
                return Err(Error::AlreadyExists);
            }
            Ok(FilePlan {
                paths: vec![from.clone(), to.clone()],
                actions: vec![Action::Move {
                    from: from.clone(),
                    to: to.clone(),
                }],
                result: json!({"from":from,"to":to}),
                preview: format!(
                    "Move {from} -> {to}; overwrite={}",
                    flag(a, "overwrite", false)
                ),
                addition: 0,
                backup: false,
            })
        }
        "apply_patch" => patch(ws, a, b),
        _ => Err(Error::Unavailable),
    }
}
pub(crate) fn execute(
    ws: &SafeWorkspace,
    plan: &FilePlan,
    b: &Budget,
    check: impl Fn() -> Result<()>,
) -> Result<Value> {
    for action in &plan.actions {
        b.check()?;
        check()?;
        match action {
            Action::Write {
                path,
                data,
                mode,
                no_clobber,
            } => {
                let (safe, _) = target(ws, path, false)?;
                ensure_parents(ws, &safe)?;
                target(ws, path, false)?;
                atomic_file(&safe, data, *mode, *no_clobber)?;
            }
            Action::Delete(path) => crate::checkpoints::remove_safe_tree_with(ws, path, &|| {
                b.check()?;
                check()
            })?,
            Action::Move { from, to } => {
                let (source, _) = target(ws, from, true)?;
                let (destination, _) = target(ws, to, false)?;
                ensure_parents(ws, &destination)?;
                target(ws, from, true)?;
                target(ws, to, false)?;
                fs::rename(source, destination)?;
            }
            Action::Mode { path, mode } => {
                let (safe, _) = target(ws, path, true)?;
                set_mode(&safe, *mode)?;
            }
        }
    }
    Ok(plan.result.clone())
}
pub(crate) fn preview(value: &str) -> (String, bool) {
    let lines: Vec<_> = value.split('\n').take(1000).collect();
    let page = lines.join("\n");
    let shortened = cut(&page, 80_000);
    let truncated = shortened.len() < value.len();
    (shortened, truncated)
}
fn eol(content: &str) -> &'static str {
    let mut crlf = 0;
    let mut lf = 0;
    let mut first = "";
    for (i, b) in content.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            let current = if i > 0 && content.as_bytes()[i - 1] == b'\r' {
                crlf += 1;
                "\r\n"
            } else {
                lf += 1;
                "\n"
            };
            if first.is_empty() {
                first = current;
            }
        }
    }
    if crlf > lf {
        "\r\n"
    } else if lf > crlf || first.is_empty() {
        "\n"
    } else {
        first
    }
}
fn original_offset(content: &str, offset: usize) -> usize {
    let mut original = 0;
    let mut normalized = 0;
    let raw = content.as_bytes();
    while normalized < offset {
        if raw.get(original) == Some(&b'\r') && raw.get(original + 1) == Some(&b'\n') {
            original += 1;
        }
        original += 1;
        normalized += 1;
    }
    original
}
pub(crate) fn replace_unique(content: &str, old: &str, new: &str) -> Result<String> {
    let normalized = content.replace("\r\n", "\n");
    let old = old.replace("\r\n", "\n");
    let occurrences = if old.is_empty() {
        0
    } else {
        normalized.match_indices(&old).count()
    };
    if occurrences != 1 {
        return Err(Error::Occurrences(occurrences));
    }
    let at = normalized.find(&old).ok_or(Error::Occurrences(0))?;
    let start = original_offset(content, at);
    let end = original_offset(content, at + old.len());
    let replacement = new.replace("\r\n", "\n").replace('\n', eol(content));
    Ok(format!(
        "{}{}{}",
        &content[..start],
        replacement,
        &content[end..]
    ))
}
struct PatchFile {
    old: Option<String>,
    new: Option<String>,
    mode: Option<u32>,
    hunks: Vec<Hunk>,
}
struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<(u8, String)>,
}
fn patch_path(ws: &SafeWorkspace, value: &str, prefix: &str) -> Result<Option<String>> {
    let value = value
        .split('\t')
        .next()
        .unwrap_or(value)
        .trim_end_matches('\r');
    if value == "/dev/null" {
        return Ok(None);
    }
    if value.chars().any(char::is_whitespace) || value.contains('\\') || !value.starts_with(prefix)
    {
        return Err(Error::Patch);
    }
    Ok(Some(target(ws, &value[2..], false)?.1))
}
fn patch(ws: &SafeWorkspace, a: &Map<String, Value>, b: &Budget) -> Result<FilePlan> {
    let raw = text(a, "patch", "");
    let lines: Vec<_> = raw.split('\n').collect();
    let mut files: Vec<PatchFile> = vec![];
    let mut i = 0;
    let header = regex::Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?:.*)$")
        .map_err(|_| Error::Internal)?;
    while i < lines.len() {
        b.check()?;
        let line = lines[i].trim_end_matches('\r');
        if line.starts_with("diff --git ") {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() != 4 {
                return Err(Error::Patch);
            }
            let old = patch_path(ws, parts[2], "a/")?;
            let new = patch_path(ws, parts[3], "b/")?;
            if old != new {
                return Err(Error::Patch);
            }
            files.push(PatchFile {
                old,
                new,
                mode: None,
                hunks: vec![],
            });
            i += 1;
        } else if let Some(path) = line.strip_prefix("--- ") {
            let old = patch_path(ws, path, "a/")?;
            i += 1;
            let next = lines.get(i).ok_or(Error::Patch)?.trim_end_matches('\r');
            let new = patch_path(ws, next.strip_prefix("+++ ").ok_or(Error::Patch)?, "b/")?;
            if old.is_none() && new.is_none() || old.is_some() && new.is_some() && old != new {
                return Err(Error::Patch);
            }
            if let Some(last) = files.last_mut().filter(|f| {
                f.hunks.is_empty()
                    && (f.old == old || old.is_none())
                    && (f.new == new || new.is_none())
            }) {
                last.old = old;
                last.new = new;
            } else {
                files.push(PatchFile {
                    old,
                    new,
                    mode: None,
                    hunks: vec![],
                });
            }
            i += 1;
        } else if line.starts_with("@@ ") {
            let matched = header.captures(line).ok_or(Error::Patch)?;
            let n = |at: usize, default: usize| -> Result<usize> {
                matched
                    .get(at)
                    .map(|m| m.as_str().parse().map_err(|_| Error::Patch))
                    .unwrap_or(Ok(default))
            };
            let mut hunk = Hunk {
                old_start: n(1, 0)?,
                old_count: n(2, 1)?,
                new_start: n(3, 0)?,
                new_count: n(4, 1)?,
                lines: vec![],
            };
            i += 1;
            let mut old = 0;
            let mut new = 0;
            while old < hunk.old_count || new < hunk.new_count {
                let line = *lines.get(i).ok_or(Error::Patch)?;
                let tag = *line.as_bytes().first().ok_or(Error::Patch)?;
                if !matches!(tag, b' ' | b'+' | b'-') {
                    return Err(Error::Patch);
                }
                old += usize::from(tag != b'+');
                new += usize::from(tag != b'-');
                if old > hunk.old_count || new > hunk.new_count {
                    return Err(Error::Patch);
                }
                hunk.lines.push((tag, format!("{}\n", &line[1..])));
                i += 1;
                if lines
                    .get(i)
                    .is_some_and(|s| s.starts_with("\\ No newline at end of file"))
                {
                    hunk.lines.last_mut().ok_or(Error::Patch)?.1.pop();
                    i += 1;
                }
            }
            files.last_mut().ok_or(Error::Patch)?.hunks.push(hunk);
        } else if [
            "old mode ",
            "new mode ",
            "new file mode ",
            "deleted file mode ",
        ]
        .iter()
        .any(|s| line.starts_with(s))
        {
            let mode = line.split_whitespace().last().ok_or(Error::Patch)?;
            if mode.len() != 6 || !mode.starts_with("100") {
                return Err(Error::Patch);
            }
            let mode = u32::from_str_radix(&mode[3..], 8).map_err(|_| Error::Patch)?;
            if line.starts_with("new mode ") || line.starts_with("new file mode ") {
                files.last_mut().ok_or(Error::Patch)?.mode = Some(mode);
            }
            i += 1;
        } else if line.starts_with("index ") {
            if line.contains("120000") || line.contains("160000") {
                return Err(Error::Patch);
            }
            i += 1;
        } else if line.is_empty() {
            i += 1;
        } else {
            return Err(Error::Patch);
        }
    }
    if files.is_empty() {
        return Err(Error::Patch);
    }
    let mut paths = vec![];
    let mut actions = vec![];
    for file in files {
        let path = file
            .new
            .as_ref()
            .or(file.old.as_ref())
            .ok_or(Error::Patch)?
            .clone();
        if paths.contains(&path) {
            return Err(Error::Patch);
        }
        paths.push(path.clone());
        let (old, mode) = if let Some(old) = &file.old {
            read(ws, old)?
        } else {
            if ws.root().join(&path).try_exists()? {
                return Err(Error::AlreadyExists);
            }
            (vec![], 0o644)
        };
        if old.contains(&0) {
            return Err(Error::BinaryFile);
        }
        let source = String::from_utf8(old).map_err(|_| Error::BinaryFile)?;
        let source: Vec<_> = source.split_inclusive('\n').collect();
        let mut output = String::new();
        let mut cursor = 0;
        let mut output_lines = 0;
        for hunk in &file.hunks {
            let at = if hunk.old_count == 0 {
                hunk.old_start
            } else {
                hunk.old_start.checked_sub(1).ok_or(Error::Patch)?
            };
            if at < cursor || at > source.len() {
                return Err(Error::Patch);
            }
            for line in &source[cursor..at] {
                output.push_str(line);
                output_lines += 1;
            }
            cursor = at;
            let new_at = if hunk.new_count == 0 {
                hunk.new_start
            } else {
                hunk.new_start.checked_sub(1).ok_or(Error::Patch)?
            };
            if output_lines != new_at {
                return Err(Error::Patch);
            }
            for (tag, line) in &hunk.lines {
                if *tag != b'+' {
                    if source.get(cursor).copied() != Some(line.as_str()) {
                        return Err(Error::Patch);
                    }
                    cursor += 1;
                }
                if *tag != b'-' {
                    output.push_str(line);
                    output_lines += 1;
                }
            }
        }
        for line in &source[cursor..] {
            output.push_str(line);
        }
        if output.len() > CHECKPOINT_FILE_BYTES {
            return Err(Error::CheckpointLimit);
        }
        if file.new.is_none() {
            if !output.is_empty() {
                return Err(Error::Patch);
            }
            actions.push(Action::Delete(path));
        } else if file.hunks.is_empty() {
            if let Some(mode) = file.mode {
                actions.push(Action::Mode { path, mode });
            } else {
                return Err(Error::Patch);
            }
        } else {
            actions.push(Action::Write {
                path,
                data: output.into_bytes(),
                mode: file.mode.unwrap_or(mode),
                no_clobber: file.old.is_none(),
            });
        }
    }
    if paths.len() > 64 {
        return Err(Error::CheckpointLimit);
    }
    let check = flag(a, "checkOnly", false);
    if check {
        actions.clear();
    }
    Ok(FilePlan {
        paths,
        actions,
        result: json!({"applied":!check,"valid":true}),
        preview: raw.into(),
        addition: raw.len(),
        backup: false,
    })
}
