use crate::{
    Error, Result, SafeWorkspace,
    path::{check_handle, hardlinked, linked},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;
pub(crate) fn identifier() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::Internal)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn workspace_key(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s).to_owned();
    if cfg!(windows) {
        s.to_ascii_lowercase()
    } else {
        s
    }
}
type Locks = Mutex<HashMap<String, Weak<AsyncMutex<()>>>>;
static LOCKS: OnceLock<Locks> = OnceLock::new();
pub(crate) async fn lock_workspace(
    ws: &SafeWorkspace,
    cancel: &CancellationToken,
) -> Result<OwnedMutexGuard<()>> {
    let lock = {
        let mut locks = LOCKS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| Error::Internal)?;
        locks.retain(|_, l| l.strong_count() > 0);
        let key = workspace_key(ws.root());
        let lock = locks
            .get(&key)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| Arc::new(AsyncMutex::new(())));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    };
    tokio::select! {biased;_=cancel.cancelled()=>Err(Error::Cancelled),guard=lock.lock_owned()=>Ok(guard)}
}
pub(crate) fn mode(meta: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    }
    #[cfg(windows)]
    {
        if meta.permissions().readonly() {
            0o444
        } else {
            0o666
        }
    }
}
pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(mode & 0o222 == 0);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}
pub(crate) fn private_directory(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(Error::PathBlocked);
    }
    let mut cursor = PathBuf::new();
    for part in path.components() {
        cursor.push(part);
        if matches!(part, std::path::Component::Normal(_)) {
            match fs::create_dir(&cursor) {
                Ok(()) => set_mode(&cursor, 0o700)?,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
            let m = fs::symlink_metadata(&cursor)?;
            if !m.is_dir() || linked(&m) {
                return Err(Error::PathBlocked);
            }
        }
    }
    Ok(fs::canonicalize(path)?)
}
pub(crate) fn atomic_file(path: &Path, data: &[u8], mode: u32, no_clobber: bool) -> Result<()> {
    let parent = path.parent().ok_or(Error::PathBlocked)?;
    let tmp = parent.join(format!(".myagent-{}.tmp", identifier()?));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        file.sync_all()?;
        drop(file);
        if no_clobber {
            fs::hard_link(&tmp, path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Error::AlreadyExists
                } else {
                    e.into()
                }
            })?;
            fs::remove_file(&tmp)?;
        } else {
            fs::rename(&tmp, path)?;
        }
        #[cfg(windows)]
        set_mode(path, mode)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
pub(crate) fn open_metadata(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0200_0000 | 0x0020_0000);
    }
    Ok(options.open(path)?)
}
pub(crate) fn bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let m = fs::symlink_metadata(path)?;
    if linked(&m) || hardlinked(path, &m)? || !m.is_file() {
        return Err(Error::PathBlocked);
    }
    let file = File::open(path)?;
    check_handle(&file)?;
    if file.metadata()?.len() > limit as u64 {
        return Err(Error::CheckpointLimit);
    }
    let mut bytes = vec![];
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::CheckpointLimit);
    }
    Ok(bytes)
}
pub(crate) fn ensure_parents(ws: &SafeWorkspace, path: &Path) -> Result<()> {
    let parent = path.parent().ok_or(Error::PathBlocked)?;
    let relative = ws.relative(parent)?;
    let mut current = ws.root().to_path_buf();
    for part in relative.split('/').filter(|p| *p != ".") {
        current.push(part);
        ws.resolve_safe_path(&ws.relative(&current)?, false)?;
        match fs::create_dir(&current) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && current.is_dir() => {}
            Err(e) => return Err(e.into()),
        }
        ws.resolve_safe_path(&ws.relative(&current)?, true)?;
    }
    Ok(())
}
