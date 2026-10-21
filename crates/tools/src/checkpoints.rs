use crate::{Error, Result, SafeWorkspace, io::Budget, mutation_io::*};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
pub const CHECKPOINT_FILE_BYTES: usize = 5 * 1024 * 1024;
pub const CHECKPOINT_STEP_BYTES: usize = 20 * 1024 * 1024;
pub const CHECKPOINT_SESSION_BYTES: usize = 64 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 48 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    kind: String,
    mode: u32,
    hash: String,
    dev: u64,
    ino: u64,
    mtime: i128,
    ctime: i128,
    data: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tree {
    path: String,
    entries: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    id: String,
    workspace: String,
    tool: String,
    state: String,
    before: Vec<Tree>,
    after: Option<Vec<Tree>>,
}
#[derive(Clone, Debug)]
pub struct CheckpointStore {
    base: PathBuf,
}
pub(crate) struct Step<'a> {
    pub tool: &'a str,
    pub paths: &'a [String],
    pub approved: &'a str,
    pub addition: usize,
    pub backup: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub checkpoint_id: String,
    pub paths: Vec<String>,
    pub message: String,
}
fn stamp(file: &File) -> Result<(u64, u64, i128, i128)> {
    let meta = file.metadata()?;
    let modified = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidCheckpoint)?
        .as_nanos() as i128;
    #[cfg(unix)]
    let (dev, ino) = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(windows)]
    let (dev, ino) = {
        let info = winapi_util::file::information(file)?;
        (info.volume_serial_number(), info.file_index())
    };
    Ok((dev, ino, modified, myagent_platform::change_time(file)?))
}
pub(crate) fn capture(
    ws: &SafeWorkspace,
    paths: &[String],
    data: bool,
    b: &Budget,
) -> Result<Vec<Tree>> {
    let mut count = 0;
    let mut bytes = 0;
    let mut trees = vec![];
    for path in paths {
        let mut tree = Tree {
            path: path.clone(),
            entries: vec![],
        };
        let root = ws.resolve_safe_path(path, false)?;
        let mut stack = vec![(root.clone(), 0)];
        while let Some((path, depth)) = stack.pop() {
            b.check()?;
            count += 1;
            if count > 1000 || depth > 32 {
                return Err(Error::CheckpointLimit);
            }
            let path = ws.resolve_safe_path(&ws.relative(&path)?, false)?;
            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            if !meta.is_dir() && !meta.is_file() {
                return Err(Error::PathBlocked);
            }
            let rel = path
                .strip_prefix(&root)
                .map_err(|_| Error::PathBlocked)?
                .to_string_lossy()
                .replace('\\', "/");
            let mut file = open_metadata(&path)?;
            let before = stamp(&file)?;
            let mut entry = Entry {
                path: rel,
                kind: if meta.is_dir() { "directory" } else { "file" }.into(),
                mode: mode(&meta),
                hash: String::new(),
                dev: before.0,
                ino: before.1,
                mtime: before.2,
                ctime: before.3,
                data: None,
            };
            if meta.is_file() {
                crate::path::check_handle(&file)?;
                if meta.len() > CHECKPOINT_FILE_BYTES as u64 {
                    return Err(Error::CheckpointLimit);
                }
                let mut raw = vec![];
                let mut buffer = [0u8; 32_768];
                loop {
                    b.check()?;
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    if raw.len() + n > CHECKPOINT_FILE_BYTES || bytes + n > CHECKPOINT_STEP_BYTES {
                        return Err(Error::CheckpointLimit);
                    }
                    raw.extend_from_slice(&buffer[..n]);
                    bytes += n;
                }
                if stamp(&file)? != before || file.metadata()?.len() != raw.len() as u64 {
                    return Err(Error::StaleApproval);
                }
                entry.hash = hash(&raw);
                if data {
                    entry.data = Some(STANDARD.encode(&raw));
                }
            } else {
                let mut children = vec![];
                for child in fs::read_dir(&path)? {
                    b.check()?;
                    if children.len() + count >= 1000 {
                        return Err(Error::CheckpointLimit);
                    }
                    children.push(child?.path());
                }
                children.sort();
                for child in children.into_iter().rev() {
                    stack.push((child, depth + 1));
                }
            }
            tree.entries.push(entry);
        }
        trees.push(tree);
    }
    Ok(trees)
}
pub(crate) fn fingerprint(trees: &[Tree]) -> Result<String> {
    let mut value = trees.to_vec();
    for tree in &mut value {
        for entry in &mut tree.entries {
            entry.data = None;
        }
    }
    Ok(hash(
        &serde_json::to_vec(&value).map_err(|_| Error::InvalidCheckpoint)?,
    ))
}
fn semantic(tree: &Tree) -> Result<String> {
    let values: Vec<_> = tree
        .entries
        .iter()
        .map(|e| (&e.path, &e.kind, e.mode, &e.hash))
        .collect();
    Ok(hash(
        &serde_json::to_vec(&values).map_err(|_| Error::InvalidCheckpoint)?,
    ))
}
pub(crate) fn expanded_paths(ws: &SafeWorkspace, requested: &[String]) -> Result<Vec<String>> {
    if requested.len() > 64 {
        return Err(Error::CheckpointLimit);
    }
    let mut result = vec![];
    for path in requested {
        let safe = ws.resolve_safe_path(path, false)?;
        let relative = ws.relative(&safe)?;
        if relative == "." || relative.split('/').count() > 32 {
            return Err(Error::PathBlocked);
        }
        let mut cursor = ws.root().to_path_buf();
        for part in relative.split('/') {
            cursor.push(part);
            match fs::symlink_metadata(&cursor) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(e.into()),
            }
        }
        result.push(ws.relative(&cursor)?);
    }
    result.sort();
    result.dedup();
    let all = result.clone();
    result.retain(|p| {
        !all.iter()
            .any(|other| other != p && p.starts_with(&format!("{other}/")))
    });
    Ok(result)
}
impl CheckpointStore {
    /// Apenas configura o destino; não lê/cria o home real.
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }
    pub fn from_home(home: &Path) -> Self {
        Self::new(home.join("checkpoints"))
    }
    pub fn base(&self) -> &Path {
        &self.base
    }
    fn directory(&self, ws: &SafeWorkspace, session: &str) -> Result<PathBuf> {
        if session.is_empty()
            || session.len() > 128
            || !session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(Error::InvalidInput);
        }
        if !self.base.is_absolute()
            || ws.relative(&self.base).is_ok()
            || self.base.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            return Err(Error::PathBlocked);
        }
        let base = private_directory(&self.base)?;
        if ws.relative(&base).is_ok() {
            return Err(Error::PathBlocked);
        }
        private_directory(&base.join(session))
    }
    fn files(&self, directory: &Path) -> Result<Vec<PathBuf>> {
        let mut files = vec![];
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if valid_record_name(&name) {
                let path = entry.path();
                let meta = fs::symlink_metadata(&path)?;
                if !meta.is_file()
                    || crate::path::linked(&meta)
                    || crate::path::hardlinked(&path, &meta)?
                {
                    return Err(Error::InvalidCheckpoint);
                }
                files.push(path);
            } else if name.starts_with(".myagent-") && name.ends_with(".tmp") {
                return Err(Error::IncompleteCheckpoint);
            } else {
                return Err(Error::InvalidCheckpoint);
            }
        }
        files.sort();
        Ok(files)
    }
    fn load(&self, path: &Path) -> Result<Record> {
        let raw = bounded_file(path, MAX_RECORD_BYTES)?;
        let record: Record = serde_json::from_slice(&raw).map_err(|_| Error::InvalidCheckpoint)?;
        if path.file_stem().and_then(|s| s.to_str()) != Some(&record.id)
            || record.before.len() > 64
            || record
                .after
                .as_ref()
                .is_some_and(|t| t.len() != record.before.len())
        {
            return Err(Error::InvalidCheckpoint);
        }
        Ok(record)
    }
    fn save(&self, path: &Path, record: &Record) -> Result<()> {
        private_directory(path.parent().ok_or(Error::InvalidCheckpoint)?)?;
        let raw = serde_json::to_vec(record).map_err(|_| Error::InvalidCheckpoint)?;
        if raw.len() > MAX_RECORD_BYTES {
            return Err(Error::CheckpointLimit);
        }
        atomic_file(path, &raw, 0o600, false)
    }
    pub(crate) fn run<T>(
        &self,
        ws: &SafeWorkspace,
        session: &str,
        step: Step<'_>,
        b: &Budget,
        execute: impl FnOnce() -> Result<T>,
    ) -> Result<(T, Option<String>)> {
        let Step {
            tool,
            paths,
            approved,
            addition,
            backup,
        } = step;
        let before = capture(ws, paths, true, b)?;
        if fingerprint(&before)? != approved {
            return Err(Error::StaleApproval);
        }
        let mut bytes = 0;
        let mut count = 0;
        for tree in &before {
            for entry in &tree.entries {
                count += 1;
                if let Some(data) = &entry.data {
                    let size = STANDARD
                        .decode(data)
                        .map_err(|_| Error::InvalidCheckpoint)?
                        .len();
                    bytes += size;
                    if matches!(tool, "edit_file" | "apply_patch")
                        && size + addition > CHECKPOINT_FILE_BYTES
                    {
                        return Err(Error::CheckpointLimit);
                    }
                }
            }
        }
        if addition > CHECKPOINT_FILE_BYTES
            || bytes + addition * if backup { 2 } else { 1 } > CHECKPOINT_STEP_BYTES
            || count + paths.iter().map(|p| p.split('/').count()).sum::<usize>() > 1000
        {
            return Err(Error::CheckpointLimit);
        }
        let directory = self.directory(ws, session)?;
        let mut files = self.files(&directory)?;
        let reservation = serde_json::to_vec(&before)
            .map_err(|_| Error::InvalidCheckpoint)?
            .len()
            * 2
            + 8 * 1024 * 1024;
        if reservation > CHECKPOINT_SESSION_BYTES {
            return Err(Error::CheckpointLimit);
        }
        let mut total = 0;
        for path in &files {
            let m = fs::symlink_metadata(path)?;
            if crate::path::linked(&m) || crate::path::hardlinked(path, &m)? {
                return Err(Error::PathBlocked);
            }
            total += m.len() as usize;
        }
        while files.len() >= 20 || total + reservation > CHECKPOINT_SESSION_BYTES {
            let oldest = files.first().ok_or(Error::CheckpointLimit)?;
            if self.load(oldest)?.state != "complete" {
                return Err(Error::IncompleteCheckpoint);
            }
            total -= fs::symlink_metadata(oldest)?.len() as usize;
            fs::remove_file(oldest)?;
            files.remove(0);
        }
        static LAST: Mutex<u128> = Mutex::new(0);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Internal)?
            .as_millis();
        let stamp = {
            let mut last = LAST.lock().map_err(|_| Error::Internal)?;
            *last = now.max(*last + 1);
            *last
        };
        let id = format!("{stamp:020}-{}", identifier()?);
        let filename = directory.join(format!("{id}.json"));
        let mut record = Record {
            id: id.clone(),
            workspace: workspace_key(ws.root()),
            tool: tool.into(),
            state: "pending".into(),
            before,
            after: None,
        };
        self.save(&filename, &record)?;
        if let Err(error) = (|| {
            b.check()?;
            if fingerprint(&capture(ws, paths, false, b)?)? != approved {
                return Err(Error::StaleApproval);
            }
            Ok(())
        })() {
            fs::remove_file(filename)?;
            return Err(error);
        }
        let result = execute();
        let after = capture(
            ws,
            paths,
            false,
            &Budget::new(tokio_util::sync::CancellationToken::new(), 300),
        )
        .map_err(|_| Error::IncompleteCheckpoint)?;
        if fingerprint(&after)? == approved {
            fs::remove_file(filename)?;
            result.map(|value| (value, None))
        } else {
            record.state = "complete".into();
            record.after = Some(after);
            self.save(&filename, &record)
                .map_err(|_| Error::IncompleteCheckpoint)?;
            result.map(|value| (value, Some(id)))
        }
    }
    /// Chamador deve manter o lock de workspace e a exclusão da geração/lease.
    pub(crate) fn undo_locked(
        &self,
        ws: &SafeWorkspace,
        session: &str,
        b: &Budget,
        check: impl Fn() -> Result<()>,
        note: impl FnOnce(&UndoResult) -> Result<()>,
    ) -> Result<UndoResult> {
        let directory = self.directory(ws, session)?;
        let files = self.files(&directory)?;
        let latest = files.last().ok_or(Error::NoCheckpoint)?;
        let mut record = self.load(latest)?;
        if record.workspace != workspace_key(ws.root()) || record.state != "complete" {
            return Err(Error::IncompleteCheckpoint);
        }
        let after = record.after.as_ref().ok_or(Error::IncompleteCheckpoint)?;
        let paths: Vec<_> = record.before.iter().map(|t| t.path.clone()).collect();
        let expanded = expanded_paths(ws, &paths)?;
        if expanded != paths {
            return Err(Error::InvalidCheckpoint);
        }
        let current = capture(ws, &paths, false, b)?;
        let conflicts: Vec<_> = current
            .iter()
            .zip(after)
            .filter(|(a, b)| a != b)
            .map(|(t, _)| t.path.clone())
            .collect();
        if !conflicts.is_empty() {
            return Err(Error::Conflict { paths: conflicts });
        }
        let mut bytes = 0;
        let mut count = 0;
        let mut decoded = vec![];
        for tree in &record.before {
            let root = ws.resolve_safe_path(&tree.path, false)?;
            let mut tree_data = vec![];
            let mut seen = std::collections::HashSet::new();
            let mut directories = std::collections::HashSet::new();
            if !tree.entries.is_empty() && !tree.entries[0].path.is_empty() {
                return Err(Error::InvalidCheckpoint);
            }
            for entry in &tree.entries {
                count += 1;
                if count > 1000
                    || entry.mode > 0o777
                    || entry.path.split('/').count() > 32
                    || !seen.insert(&entry.path)
                {
                    return Err(Error::InvalidCheckpoint);
                }
                if !entry.path.is_empty() {
                    let parent = entry.path.rsplit_once('/').map(|p| p.0).unwrap_or("");
                    if !directories.contains(parent) {
                        return Err(Error::InvalidCheckpoint);
                    }
                }
                let joined = root.join(&entry.path);
                if !joined.starts_with(&root)
                    || Path::new(&entry.path)
                        .components()
                        .any(|c| !matches!(c, std::path::Component::Normal(_)))
                        && !entry.path.is_empty()
                {
                    return Err(Error::InvalidCheckpoint);
                }
                ws.resolve_safe_path(&ws.relative(&joined)?, false)?;
                if entry.kind == "file" {
                    let data = STANDARD
                        .decode(entry.data.as_ref().ok_or(Error::InvalidCheckpoint)?)
                        .map_err(|_| Error::InvalidCheckpoint)?;
                    bytes += data.len();
                    if hash(&data) != entry.hash
                        || data.len() > CHECKPOINT_FILE_BYTES
                        || bytes > CHECKPOINT_STEP_BYTES
                    {
                        return Err(Error::InvalidCheckpoint);
                    }
                    tree_data.push(Some(data));
                } else if entry.kind == "directory" {
                    directories.insert(entry.path.clone());
                    tree_data.push(None);
                } else {
                    return Err(Error::InvalidCheckpoint);
                }
            }
            decoded.push(tree_data);
        }
        // Commit the incomplete marker before any destructive restoration.
        record.state = "restoring".into();
        self.save(latest, &record)?;
        for (tree, data) in record.before.iter().zip(decoded) {
            check()?;
            let root = ws.resolve_safe_path(&tree.path, false)?;
            remove_safe_tree_with(ws, &tree.path, &check)?;
            for (entry, data) in tree.entries.iter().zip(data) {
                check()?;
                let path = if entry.path.is_empty() {
                    root.clone()
                } else {
                    root.join(&entry.path)
                };
                ensure_parents(ws, &path)?;
                if let Some(bytes) = data {
                    atomic_file(&path, &bytes, entry.mode, false)?;
                } else {
                    fs::create_dir(&path)?;
                }
            }
            for entry in tree.entries.iter().rev().filter(|e| e.kind == "directory") {
                check()?;
                set_mode(&root.join(&entry.path), entry.mode)?;
            }
        }
        // Refresh earlier fingerprints only when semantic state matches exactly.
        if let Some(previous) = files.iter().rev().nth(1) {
            let mut previous_record = self.load(previous)?;
            if previous_record.workspace == record.workspace
                && previous_record.state == "complete"
                && let Some(old) = &mut previous_record.after
            {
                let previous_paths = old.iter().map(|t| t.path.clone()).collect::<Vec<_>>();
                let actual = capture(
                    ws,
                    &previous_paths,
                    false,
                    &Budget::new(tokio_util::sync::CancellationToken::new(), 300),
                )?;
                for (old, actual) in old.iter_mut().zip(actual) {
                    if semantic(old)? == semantic(&actual)? {
                        *old = actual;
                    }
                }
                self.save(previous, &previous_record)?;
            }
        }
        let result = UndoResult {
            checkpoint_id: record.id,
            paths: paths.clone(),
            message: format!(
                "[UNDO] Restored {}: {}. Reread affected files before continuing.",
                record.tool,
                paths.join(", ")
            ),
        };
        note(&result)?;
        fs::remove_file(latest)?;
        Ok(result)
    }
}
fn valid_record_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".json") else {
        return false;
    };
    let Some((time, id)) = stem.split_once('-') else {
        return false;
    };
    time.len() == 20
        && time.bytes().all(|b| b.is_ascii_digit())
        && id.len() == 32
        && id.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn remove_safe_tree_with(
    ws: &SafeWorkspace,
    path: &str,
    check: &impl Fn() -> Result<()>,
) -> Result<()> {
    check()?;
    let safe = ws.resolve_safe_path(path, false)?;
    if ws.relative(&safe)? == "." {
        return Err(Error::PathBlocked);
    }
    let info = match fs::symlink_metadata(&safe) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if info.is_dir() {
        for entry in fs::read_dir(&safe)? {
            let child = entry?.path();
            remove_safe_tree_with(ws, &ws.relative(&child)?, check)?;
        }
        fs::remove_dir(safe)?;
    } else {
        #[cfg(windows)]
        if info.permissions().readonly() {
            set_mode(&safe, mode(&info) | 0o200)?;
        }
        fs::remove_file(safe)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;
    fn fixture() -> (tempfile::TempDir, SafeWorkspace, CheckpointStore) {
        let dir = tempfile::Builder::new()
            .prefix("myagent checkpoint unit ")
            .tempdir()
            .unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let root = base.join("workspace");
        fs::create_dir(&root).unwrap();
        (
            dir,
            SafeWorkspace::new(&root).unwrap(),
            CheckpointStore::new(base.join("checkpoints")),
        )
    }
    #[test]
    fn failure_after_partial_effect_keeps_a_complete_undoable_step() {
        let (_dir, ws, store) = fixture();
        fs::write(ws.root().join("a"), b"before").unwrap();
        let paths = vec!["a".to_owned()];
        let b = Budget::new(CancellationToken::new(), 300);
        let before = fingerprint(&capture(&ws, &paths, false, &b).unwrap()).unwrap();
        let result: Result<((), Option<String>)> = store.run(
            &ws,
            "session",
            Step {
                tool: "write_file",
                paths: &paths,
                approved: &before,
                addition: 5,
                backup: false,
            },
            &b,
            || {
                fs::write(ws.root().join("a"), b"after")?;
                Err(Error::Io)
            },
        );
        assert!(result.is_err());
        let files = store
            .files(&store.directory(&ws, "session").unwrap())
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(store.load(&files[0]).unwrap().state, "complete");
        store
            .undo_locked(&ws, "session", &b, || Ok(()), |_| Ok(()))
            .unwrap();
        assert_eq!(fs::read(ws.root().join("a")).unwrap(), b"before");
    }
    #[test]
    fn restore_io_failure_preserves_preimages_and_refuses_automatic_retry() {
        let (_dir, ws, store) = fixture();
        fs::write(ws.root().join("a"), b"before").unwrap();
        let paths = vec!["a".to_owned()];
        let b = Budget::new(CancellationToken::new(), 300);
        let before = fingerprint(&capture(&ws, &paths, false, &b).unwrap()).unwrap();
        store
            .run(
                &ws,
                "session",
                Step {
                    tool: "write_file",
                    paths: &paths,
                    approved: &before,
                    addition: 5,
                    backup: false,
                },
                &b,
                || {
                    fs::write(ws.root().join("a"), b"after")?;
                    Ok(())
                },
            )
            .unwrap();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let result = store.undo_locked(
            &ws,
            "session",
            &b,
            || {
                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2 {
                    Err(Error::Io)
                } else {
                    Ok(())
                }
            },
            |_| Ok(()),
        );
        assert!(result.is_err());
        let files = store
            .files(&store.directory(&ws, "session").unwrap())
            .unwrap();
        let record = store.load(&files[0]).unwrap();
        assert_eq!(record.state, "restoring");
        assert_eq!(
            STANDARD
                .decode(record.before[0].entries[0].data.as_ref().unwrap())
                .unwrap(),
            b"before"
        );
        assert!(matches!(
            store.undo_locked(&ws, "session", &b, || Ok(()), |_| Ok(())),
            Err(Error::IncompleteCheckpoint)
        ));
    }
    #[test]
    fn incomplete_oldest_step_is_never_evicted_for_a_new_write() {
        let (_dir, ws, store) = fixture();
        let directory = store.directory(&ws, "session").unwrap();
        for n in 0..20 {
            let id = format!("{n:020}-{}", "a".repeat(32));
            let record = Record {
                id: id.clone(),
                workspace: workspace_key(ws.root()),
                tool: "write_file".into(),
                state: if n == 0 { "pending" } else { "complete" }.into(),
                before: vec![],
                after: None,
            };
            store
                .save(&directory.join(format!("{id}.json")), &record)
                .unwrap();
        }
        let b = Budget::new(CancellationToken::new(), 300);
        let paths = vec!["a".into()];
        let before = fingerprint(&capture(&ws, &paths, false, &b).unwrap()).unwrap();
        let result = store.run(
            &ws,
            "session",
            Step {
                tool: "write_file",
                paths: &paths,
                approved: &before,
                addition: 1,
                backup: false,
            },
            &b,
            || {
                fs::write(ws.root().join("a"), b"x")?;
                Ok(())
            },
        );
        assert!(matches!(result, Err(Error::IncompleteCheckpoint)));
        assert!(!ws.root().join("a").exists());
        assert_eq!(store.files(&directory).unwrap().len(), 20);
    }
    #[test]
    fn capture_enforces_total_bytes_entries_and_depth() {
        let (_dir, ws, _store) = fixture();
        let b = Budget::new(CancellationToken::new(), 300);
        fs::create_dir(ws.root().join("many")).unwrap();
        for n in 0..1001 {
            fs::write(ws.root().join(format!("many/{n}")), b"").unwrap();
        }
        assert!(matches!(
            capture(&ws, &["many".into()], false, &b),
            Err(Error::CheckpointLimit)
        ));
        fs::create_dir(ws.root().join("large")).unwrap();
        for n in 0..5 {
            fs::write(
                ws.root().join(format!("large/{n}")),
                vec![0; CHECKPOINT_FILE_BYTES],
            )
            .unwrap();
        }
        assert!(matches!(
            capture(&ws, &["large".into()], false, &b),
            Err(Error::CheckpointLimit)
        ));
        let path = ws.root().join("deep").join(vec!["d"; 33].join("/"));
        fs::create_dir_all(path).unwrap();
        assert!(matches!(
            capture(&ws, &["deep".into()], false, &b),
            Err(Error::CheckpointLimit)
        ));
    }
}
