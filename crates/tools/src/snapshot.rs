use crate::{
    Error, Result, SafeWorkspace,
    io::Budget,
    mutation_io::{mode, set_mode},
    path::{check_handle, hardlinked, linked},
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

pub const SNAPSHOT_FILE_LIMIT: usize = 5 * 1024 * 1024;
pub const SNAPSHOT_TOTAL_LIMIT: usize = 200 * 1024 * 1024;
pub const SNAPSHOT_ENTRY_LIMIT: usize = 20_000;
#[derive(Clone, Copy, Debug, Default)]
pub struct SnapshotOptions {
    pub dependencies: bool,
}
/// Diretório temporário exclusivo, fora do workspace; nunca reutiliza dados após reinício.
pub struct Snapshot {
    directory: tempfile::TempDir,
    pub files: usize,
    pub bytes: usize,
}
impl Snapshot {
    pub fn path(&self) -> &Path {
        self.directory.path()
    }
    pub(crate) fn retain(self) -> PathBuf {
        self.directory.keep()
    }
    pub fn build(
        ws: &SafeWorkspace,
        options: SnapshotOptions,
        cancel: CancellationToken,
    ) -> Result<Self> {
        Self::build_with_limits(
            ws,
            options,
            cancel,
            SNAPSHOT_TOTAL_LIMIT,
            SNAPSHOT_ENTRY_LIMIT,
        )
    }
    fn build_with_limits(
        ws: &SafeWorkspace,
        options: SnapshotOptions,
        cancel: CancellationToken,
        max_bytes: usize,
        max_entries: usize,
    ) -> Result<Self> {
        let b = Budget::new(cancel, 120);
        b.check()?;
        let directory = tempfile::Builder::new()
            .prefix("myagent-snapshot-")
            .tempdir()?;
        let canonical = fs::canonicalize(directory.path())?;
        if canonical.starts_with(ws.root()) {
            return Err(Error::PathBlocked);
        }
        set_mode(directory.path(), 0o755)?;
        let mut snapshot = Self {
            directory,
            files: 0,
            bytes: 0,
        };
        let mut stack = vec![(ws.root().to_path_buf(), false, 0usize)];
        let mut visited = 0;
        let mut shims = vec![];
        while let Some((dir, dependency, depth)) = stack.pop() {
            b.check()?;
            if depth > 64 {
                return Err(Error::SnapshotLimit);
            }
            ws.resolve_safe_path(&ws.relative(&dir)?, true)?;
            for item in fs::read_dir(&dir)? {
                b.check()?;
                visited += 1;
                if visited > max_entries {
                    return Err(Error::SnapshotLimit);
                }
                let item = item?;
                let name = item
                    .file_name()
                    .to_str()
                    .ok_or(Error::PathBlocked)?
                    .to_owned();
                let relative = ws.relative(&item.path())?;
                let dep = dependency || matches!(name.as_str(), "node_modules" | ".venv");
                if excluded(&relative, dep) || (dep && !options.dependencies) {
                    continue;
                }
                let meta = fs::symlink_metadata(item.path())?;
                if linked(&meta) {
                    // Exceção materializada: .bin -> arquivo regular seguro incluído na cópia.
                    if dep
                        && dir.file_name().is_some_and(|n| n == ".bin")
                        && meta.file_type().is_symlink()
                    {
                        let target = fs::read_link(item.path())?;
                        let resolved = normalize_bin_target(ws, &dir, &target);
                        if let Ok(target) = resolved {
                            shims.push((relative, target));
                        }
                    }
                    continue;
                }
                if hardlinked(&item.path(), &meta)? {
                    continue;
                }
                let safe = ws.resolve_safe_path(&relative, true)?;
                let dest = snapshot.path().join(&relative);
                if meta.is_dir() {
                    fs::create_dir_all(&dest)?;
                    set_mode(&dest, 0o755)?;
                    stack.push((safe, dep, depth + 1));
                } else if meta.is_file() && meta.len() <= SNAPSHOT_FILE_LIMIT as u64 {
                    if snapshot.bytes + meta.len() as usize > max_bytes {
                        return Err(Error::SnapshotLimit);
                    }
                    let mut source = File::open(&safe)?;
                    check_handle(&source)?;
                    let before = source.metadata()?;
                    if before.len() != meta.len() || before.modified()? != meta.modified()? {
                        return Err(Error::SnapshotChanged);
                    }
                    let ctime = myagent_platform::change_time(&source)?;
                    let mut output = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&dest)?;
                    let mut copied = 0;
                    let mut buffer = [0u8; 32_768];
                    loop {
                        b.check()?;
                        let n = source.read(&mut buffer)?;
                        if n == 0 {
                            break;
                        }
                        copied += n;
                        if copied > SNAPSHOT_FILE_LIMIT
                            || copied > before.len() as usize
                            || snapshot.bytes + n > max_bytes
                        {
                            return Err(Error::SnapshotLimit);
                        }
                        snapshot.bytes += n;
                        output.write_all(&buffer[..n])?;
                    }
                    let after = source.metadata()?;
                    if copied != before.len() as usize
                        || after.len() != before.len()
                        || after.modified()? != before.modified()?
                        || myagent_platform::change_time(&source)? != ctime
                    {
                        return Err(Error::SnapshotChanged);
                    }
                    drop(output);
                    let executable =
                        mode(&meta) & 0o111 != 0 || relative.contains("node_modules/.bin/");
                    set_mode(&dest, if executable { 0o755 } else { 0o644 })?;
                    snapshot.files += 1;
                }
            }
        }
        for (relative, target) in shims {
            b.check()?;
            let target_rel = ws.relative(&target)?;
            if !snapshot.path().join(&target_rel).is_file() {
                continue;
            }
            set_mode(&snapshot.path().join(&target_rel), 0o755)?;
            let script = format!(
                "#!/bin/sh\nexec {} \"$@\"\n",
                crate::execution::shell_quote(&format!("/workspace/{target_rel}"))
            );
            if snapshot.bytes + script.len() > max_bytes {
                return Err(Error::SnapshotLimit);
            }
            let dest = snapshot.path().join(relative);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&dest)?;
            file.write_all(script.as_bytes())?;
            drop(file);
            set_mode(&dest, 0o755)?;
            snapshot.bytes += script.len();
            snapshot.files += 1;
        }
        Ok(snapshot)
    }
}
fn normalize_bin_target(ws: &SafeWorkspace, directory: &Path, target: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in directory.join(target).components() {
        match component {
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return Err(Error::PathBlocked);
                }
            }
            std::path::Component::CurDir => {}
            part => normalized.push(part),
        }
    }
    ws.resolve_safe_path(&ws.relative(&normalized)?, true)
}
fn excluded(path: &str, dependency: bool) -> bool {
    path.split('/').any(|name| {
        let n = name.to_ascii_lowercase();
        n.starts_with(".env")
            || crate::is_sensitive_path(name)
            || [".cargo-cache", ".agents", ".venv-cache"].contains(&n.as_str())
            || n.ends_with(".sqlite3")
            || n.ends_with(".sqlite")
            || [
                ".db-wal",
                ".db-shm",
                ".db-journal",
                ".sqlite3-wal",
                ".sqlite3-shm",
                ".sqlite3-journal",
                ".sqlite-wal",
                ".sqlite-shm",
            ]
            .iter()
            .any(|suffix| n.ends_with(suffix))
            || (!dependency
                && [
                    "dist",
                    ".dist",
                    "build",
                    "out",
                    "target",
                    ".next",
                    ".nuxt",
                    ".cache",
                    ".turbo",
                    "coverage",
                    "__pycache__",
                ]
                .contains(&n.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filtering_limits_and_dependencies_are_checked_before_copy() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("work");
        fs::create_dir(&root).unwrap();
        for p in ["target", "node_modules/pkg", "empty"] {
            fs::create_dir_all(root.join(p)).unwrap();
        }
        fs::write(root.join("a"), b"a\r\nb\0").unwrap();
        fs::write(root.join("target/omit"), b"build").unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), b"module").unwrap();
        let huge = File::create(root.join("large")).unwrap();
        huge.set_len((SNAPSHOT_FILE_LIMIT + 1) as u64).unwrap();
        drop(huge);
        let ws = SafeWorkspace::new(&root).unwrap();
        let s = Snapshot::build(&ws, SnapshotOptions::default(), CancellationToken::new()).unwrap();
        assert_eq!(fs::read(s.path().join("a")).unwrap(), b"a\r\nb\0");
        assert!(!s.path().join("target").exists());
        assert!(!s.path().join("node_modules").exists());
        assert!(!s.path().join("large").exists());
        assert!(
            Snapshot::build_with_limits(
                &ws,
                SnapshotOptions::default(),
                CancellationToken::new(),
                3,
                100
            )
            .is_err()
        );
        assert!(
            Snapshot::build_with_limits(
                &ws,
                SnapshotOptions::default(),
                CancellationToken::new(),
                100,
                1
            )
            .is_err()
        );
        let dep = Snapshot::build(
            &ws,
            SnapshotOptions { dependencies: true },
            CancellationToken::new(),
        )
        .unwrap();
        assert!(dep.path().join("node_modules/pkg/index.js").exists());
        for n in [
            ".env",
            ".env.example",
            "config.env",
            ".git/config",
            "chatgpt-credentials.json",
            "x.db",
            "x.log",
            ".ssh/key",
            ".cargo-cache/a",
        ] {
            assert!(excluded(n, false), "{n}");
        }
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(
            Snapshot::build(&ws, SnapshotOptions::default(), token),
            Err(Error::Cancelled)
        ));
    }
    #[test]
    fn links_never_copy_outside_or_duplicate_file_data() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("work");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a"), b"data").unwrap();
        fs::hard_link(root.join("a"), root.join("b")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(temp.path(), root.join("alias")).unwrap();
        #[cfg(windows)]
        {
            let mut c = std::process::Command::new("cmd");
            c.args(["/c", "mklink", "/J"])
                .arg(root.join("alias"))
                .arg(temp.path());
            assert!(c.output().unwrap().status.success());
        }
        let ws = SafeWorkspace::new(&root).unwrap();
        let s = Snapshot::build(&ws, SnapshotOptions::default(), CancellationToken::new()).unwrap();
        assert_eq!(s.files, 0);
        assert!(!s.path().join("alias").exists());
    }
    #[cfg(unix)]
    #[test]
    fn dependency_bin_shims_keep_relative_imports_without_following_links() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("work");
        fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(
            root.join("node_modules/pkg/cli.js"),
            b"#!/usr/bin/env node\nimport './helper.js';\n",
        )
        .unwrap();
        fs::write(root.join("node_modules/pkg/helper.js"), b"fixture").unwrap();
        std::os::unix::fs::symlink("../pkg/cli.js", root.join("node_modules/.bin/fixture"))
            .unwrap();
        std::os::unix::fs::symlink(
            "../../../../outside",
            root.join("node_modules/.bin/external"),
        )
        .unwrap();
        let ws = SafeWorkspace::new(&root).unwrap();
        let s = Snapshot::build(
            &ws,
            SnapshotOptions { dependencies: true },
            CancellationToken::new(),
        )
        .unwrap();
        let shim = s.path().join("node_modules/.bin/fixture");
        assert!(
            !fs::symlink_metadata(&shim)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::read_to_string(shim)
                .unwrap()
                .contains("/workspace/node_modules/pkg/cli.js")
        );
        assert!(!s.path().join("node_modules/.bin/external").exists());
        assert_ne!(
            mode(&fs::metadata(s.path().join("node_modules/pkg/cli.js")).unwrap()) & 0o111,
            0
        );
    }
}
