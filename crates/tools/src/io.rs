use crate::{Error, Result, SafeWorkspace};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
pub(crate) struct Budget {
    pub cancel: CancellationToken,
    pub deadline: Instant,
}
impl Budget {
    pub fn new(cancel: CancellationToken, seconds: u64) -> Self {
        Self {
            cancel,
            deadline: Instant::now() + Duration::from_secs(seconds),
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }
}
pub(crate) fn units(s: &str) -> usize {
    s.encode_utf16().count()
}
pub(crate) fn cut(s: &str, max: usize) -> String {
    let mut units = 0;
    s.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}
pub(crate) fn read_text(
    ws: &SafeWorkspace,
    path: &str,
    max: usize,
    budget: &Budget,
) -> Result<(String, usize)> {
    budget.check()?;
    let safe = ws.resolve_safe_path(path, true)?;
    let meta = fs::symlink_metadata(&safe)?;
    if !meta.is_file() {
        return Err(Error::NotAFile);
    }
    if meta.len() > max as u64 {
        return Err(Error::FileTooLarge);
    }
    let mut f = File::open(&safe)?;
    crate::path::check_handle(&f)?;
    if f.metadata()?.len() > max as u64 {
        return Err(Error::FileTooLarge);
    }
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 32_768];
    loop {
        budget.check()?;
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > max {
            return Err(Error::FileTooLarge);
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    budget.check()?;
    if bytes.contains(&0) {
        return Err(Error::BinaryFile);
    }
    let len = bytes.len();
    Ok((String::from_utf8_lossy(&bytes).into_owned(), len))
}
pub(crate) struct Scan {
    pub files: Vec<String>,
    pub visited: usize,
    pub truncated: bool,
}
pub(crate) fn scan(
    ws: &SafeWorkspace,
    root: &Path,
    budget: &Budget,
    inspection: bool,
    ignored: impl Fn(&Path, bool) -> bool,
) -> Result<Scan> {
    let mut scan = Scan {
        files: vec![],
        visited: 0,
        truncated: false,
    };
    let mut stack = vec![(root.to_path_buf(), 0)];
    let (entries, maxfiles, maxdepth) = if inspection {
        (2000, 500, 12)
    } else {
        (20_000, 20_000, 256)
    };
    while let Some((path, depth)) = stack.pop() {
        budget.check()?;
        let meta = fs::symlink_metadata(&path)?;
        if ignored(&path, meta.is_dir()) {
            continue;
        }
        if meta.is_file() {
            scan.files.push(ws.relative(&path)?);
            continue;
        }
        for entry in fs::read_dir(&path)? {
            budget.check()?;
            scan.visited += 1;
            if scan.visited > entries || scan.files.len() >= maxfiles {
                scan.truncated = true;
                return Ok(scan);
            }
            let entry = entry?;
            let child = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if crate::is_sensitive_path(&name) {
                continue;
            }
            if inspection
                && [
                    "node_modules",
                    "dist",
                    ".dist",
                    "coverage",
                    ".next",
                    "vendor",
                    "target",
                    "__pycache__",
                ]
                .contains(&name.as_str())
            {
                continue;
            }
            let safe = match ws.resolve_safe_path(&ws.relative(&child)?, true) {
                Ok(s) => s,
                Err(Error::PathBlocked) => continue,
                Err(e) => return Err(e),
            };
            let info = fs::symlink_metadata(&safe)?;
            if ignored(&safe, info.is_dir()) {
                continue;
            }
            if info.is_dir() {
                if depth >= maxdepth {
                    scan.truncated = true;
                } else {
                    stack.push((safe, depth + 1));
                }
            } else if info.is_file() {
                scan.files.push(ws.relative(&safe)?);
            }
        }
    }
    scan.files.sort();
    Ok(scan)
}
pub(crate) fn bounded(mut value: serde_json::Value, max: usize) -> Result<serde_json::Value> {
    while units(&value.to_string()) > max {
        let map = value.as_object_mut().ok_or(Error::OutputLimit)?;
        let key = map
            .iter()
            .filter(|(_, v)| v.as_array().is_some_and(|a| !a.is_empty()))
            .max_by_key(|(_, v)| v.to_string().len())
            .map(|(k, _)| k.clone())
            .ok_or(Error::OutputLimit)?;
        let a = map
            .get_mut(&key)
            .and_then(|v| v.as_array_mut())
            .ok_or(Error::OutputLimit)?;
        a.truncate(a.len() / 2);
        map.insert("truncated".into(), true.into());
    }
    Ok(value)
}
