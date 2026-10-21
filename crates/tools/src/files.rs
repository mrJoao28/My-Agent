use crate::{
    Error, Result, SafeWorkspace,
    io::{Budget, cut, read_text, units},
    registry::{flag, number, text},
};
use serde_json::{Map, Value, json};
use std::fs;
pub(crate) fn read_file(ws: &SafeWorkspace, a: &Map<String, Value>, b: &Budget) -> Result<Value> {
    let path = text(a, "path", "");
    let (raw, _) = read_text(ws, path, 5 * 1024 * 1024, b)?;
    let lines: Vec<_> = if raw.is_empty() {
        vec![]
    } else {
        raw.split('\n')
            .map(|s| s.strip_suffix('\r').unwrap_or(s))
            .collect()
    };
    let start = number(a, "offset", 0).min(lines.len());
    let end = start
        .saturating_add(number(a, "limit", lines.len()))
        .min(lines.len());
    let mut selected = vec![];
    let mut chars = 0;
    for (i, line) in lines.iter().enumerate().take(end).skip(start) {
        b.check()?;
        let line = if flag(a, "numbered", true) {
            format!("{}: {line}", i + 1)
        } else {
            (*line).to_owned()
        };
        let cost = units(&line) + usize::from(!selected.is_empty());
        if chars + cost > 10_000 {
            if selected.is_empty() {
                return Err(Error::LineTooLong);
            }
            break;
        }
        selected.push(line);
        chars += cost;
    }
    let next = start + selected.len();
    let truncated = next < lines.len();
    let mut value = json!({"path":ws.relative(&ws.resolve_safe_path(path,true)?)?,"content":selected.join("\n"),"totalLines":lines.len(),"truncated":truncated});
    if truncated {
        value["nextOffset"] = next.into();
        value["continuation"] = format!(
            "Continue com offset={next} e limit={}.",
            number(a, "limit", selected.len().max(1))
        )
        .into();
    }
    Ok(value)
}
pub(crate) fn read_files(ws: &SafeWorkspace, a: &Map<String, Value>, b: &Budget) -> Result<Value> {
    let mut used = 0;
    let mut bytes = 0;
    let mut chars = 0;
    let mut results = vec![];
    for file in a["files"].as_array().ok_or(Error::InvalidInput)? {
        b.check()?;
        let args = file.as_object().ok_or(Error::InvalidInput)?;
        let path = text(args, "path", "");
        if used >= 4 * 1_048_576 || chars >= 30_000 {
            results.push(json!({"path":path,"error":"BATCH_LIMIT"}));
            continue;
        }
        let allowance = (4 * 1_048_576 - used).min(1_048_576);
        match read_text(ws, path, allowance, b) {
            Ok((raw, len)) => {
                bytes += len;
                used += len;
                let lines: Vec<_> = raw.split('\n').collect();
                let start = number(args, "offset", 0).min(lines.len());
                let end = start
                    .saturating_add(number(args, "limit", lines.len()))
                    .min(lines.len());
                let selected = lines[start..end].join("\n");
                let content = cut(&selected, (30_000 - chars).min(10_000));
                chars += units(&content);
                results.push(json!({"path":ws.relative(&ws.resolve_safe_path(path,true)?)?,"truncated":content.len()<selected.len(),"content":content,"totalLines":lines.len()}));
            }
            Err(e) => {
                b.check()?;
                used += allowance;
                results.push(json!({"path":path,"error":e.to_string()}));
            }
        }
    }
    let truncated = results
        .iter()
        .any(|r| r["truncated"] == true || r["error"] == "BATCH_LIMIT");
    Ok(json!({"files":results,"bytesRead":bytes,"truncated":truncated}))
}
pub(crate) fn list_directory(
    ws: &SafeWorkspace,
    a: &Map<String, Value>,
    b: &Budget,
) -> Result<Value> {
    b.check()?;
    let safe = ws.resolve_safe_path(text(a, "path", "."), true)?;
    if !safe.is_dir() {
        return Err(Error::NotADirectory);
    }
    let offset = number(a, "offset", 0);
    let limit = number(a, "limit", 500);
    let mut entries = vec![];
    let mut count = 0;
    let mut truncated = false;
    for (visited, entry) in fs::read_dir(&safe)?.enumerate() {
        b.check()?;
        if visited >= 20_000 {
            truncated = true;
            break;
        }
        let entry = entry?;
        let child = entry.path();
        let rel = ws.relative(&child)?;
        if ws.resolve_safe_path(&rel, true).is_err() {
            continue;
        }
        let meta = fs::symlink_metadata(&child)?;
        if !meta.is_dir() && !meta.is_file() {
            continue;
        }
        count += 1;
        if count <= offset {
            continue;
        }
        if entries.len() == limit {
            truncated = true;
            break;
        }
        entries.push(json!({"name":entry.file_name().to_string_lossy(),"type":if meta.is_dir(){"directory"}else{"file"}}));
    }
    entries.sort_by(|a, b| {
        a["type"]
            .as_str()
            .cmp(&b["type"].as_str())
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    Ok(
        json!({"path":ws.relative(&safe)?,"nextOffset":if truncated&&!entries.is_empty(){Some(offset+entries.len())}else{None},"entries":entries,"truncated":truncated}),
    )
}
