use crate::{Error, Result};
use std::{
    fs::{self, File, Metadata},
    path::{Path, PathBuf},
};

pub fn is_sensitive_path(path: &str) -> bool {
    path.split(['/', '\\']).any(|part| {
        let n = part.to_lowercase();
        [
            ".git",
            ".aws",
            ".ssh",
            ".gnupg",
            ".azure",
            ".kube",
            ".myagent",
            ".docker",
            ".codex",
            ".claude",
            ".config",
            ".git-credentials",
            "api-token",
            ".npmrc",
            ".pypirc",
            ".netrc",
            "credentials",
            "credentials.json",
            "secrets",
            "id_rsa",
            "id_dsa",
            "id_ecdsa",
            "id_ed25519",
            "config.env",
            "chatgpt-credentials.json",
        ]
        .contains(&n.as_str())
            || (n.starts_with(".env") && n != ".env.example")
            || [".pem", ".key", ".p12", ".pfx", ".db", ".log"]
                .iter()
                .any(|s| n.ends_with(s))
            || n.split(['.', '_', '-'])
                .any(|s| matches!(s, "credential" | "credentials" | "secret" | "secrets"))
    })
}
pub(crate) fn linked(meta: &Metadata) -> bool {
    let result = meta.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        result || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        result
    }
}
pub(crate) fn hardlinked(path: &Path, meta: &Metadata) -> Result<bool> {
    if !meta.is_file() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Ok(meta.nlink() != 1)
    }
    #[cfg(windows)]
    {
        let file = File::open(path)?;
        Ok(winapi_util::file::information(file)?.number_of_links() != 1)
    }
}
pub(crate) fn check_handle(file: &File) -> Result<()> {
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(Error::NotAFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(Error::PathBlocked);
        }
    }
    #[cfg(windows)]
    if winapi_util::file::information(file)?.number_of_links() != 1 {
        return Err(Error::PathBlocked);
    }
    Ok(())
}
fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    s.strip_prefix("//?/")
        .unwrap_or(&s)
        .trim_end_matches('/')
        .to_owned()
}
#[derive(Clone, Debug)]
pub struct SafeWorkspace {
    root: PathBuf,
}
impl SafeWorkspace {
    /// Autoriza uma pasta existente sob uma das raízes confiáveis, sem aliases.
    pub fn authorize(requested: &Path, roots: &[PathBuf]) -> Result<Self> {
        if !requested.is_absolute() {
            return Err(Error::PathBlocked);
        }
        for root in roots {
            let authorized = Self::new(root)?;
            if authorized.relative(requested).is_ok() {
                let target = authorized.resolve_safe_path(&requested.to_string_lossy(), true)?;
                return Self::new(&target);
            }
        }
        Err(Error::PathBlocked)
    }
    pub fn new(root: &Path) -> Result<Self> {
        if !root.is_absolute() || is_sensitive_path(&root.to_string_lossy()) {
            return Err(Error::PathBlocked);
        }
        let meta = fs::symlink_metadata(root)?;
        if linked(&meta) || !meta.is_dir() {
            return Err(Error::PathBlocked);
        }
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn relative(&self, path: &Path) -> Result<String> {
        let p = display_path(path);
        let r = display_path(&self.root);
        let equal = if cfg!(windows) {
            p.eq_ignore_ascii_case(&r)
        } else {
            p == r
        };
        if equal {
            return Ok(".".into());
        }
        let prefix = format!("{r}/");
        let inside = if cfg!(windows) {
            p.get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(&prefix))
        } else {
            p.starts_with(&prefix)
        };
        if !inside {
            return Err(Error::PathBlocked);
        }
        Ok(p[prefix.len()..].to_owned())
    }
    pub fn resolve_safe_path(&self, target: &str, must_exist: bool) -> Result<PathBuf> {
        let root_meta = fs::symlink_metadata(&self.root)?;
        if linked(&root_meta)
            || !root_meta.is_dir()
            || self.relative(&fs::canonicalize(&self.root)?)? != "."
        {
            return Err(Error::PathBlocked);
        }
        #[cfg(windows)]
        let target = target
            .strip_prefix("\\\\?\\")
            .filter(|s| {
                s.as_bytes().get(1) == Some(&b':')
                    && s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            })
            .unwrap_or(target);
        if target.len() > 32_768
            || target.contains('\0')
            || target.starts_with(['\\'])
            || target.starts_with("//")
        {
            return Err(Error::PathBlocked);
        }
        let portable = target.replace('\\', "/");
        let relative = if Path::new(target).is_absolute() {
            self.relative(Path::new(target))?
        } else {
            portable
        };
        let mut parts = Vec::new();
        for p in relative.split('/') {
            match p {
                "" | "." => continue,
                ".." => {
                    if parts.pop().is_none() {
                        return Err(Error::PathBlocked);
                    }
                    continue;
                }
                _ => {}
            }
            let stem = p.split('.').next().unwrap_or("").to_ascii_lowercase();
            if p.contains(':')
                || p.ends_with(['.', ' '])
                || is_sensitive_path(p)
                || ["con", "prn", "aux", "nul"].contains(&stem.as_str())
                || (stem.len() == 4
                    && (stem.starts_with("com") || stem.starts_with("lpt"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
            {
                return Err(Error::PathBlocked);
            }
            parts.push(p);
        }
        let target = parts.iter().fold(self.root.clone(), |p, s| p.join(s));
        let mut cursor = self.root.clone();
        let mut missing = false;
        for part in parts {
            cursor.push(part);
            if missing {
                continue;
            }
            match fs::symlink_metadata(&cursor) {
                Ok(meta) => {
                    if linked(&meta) || hardlinked(&cursor, &meta)? {
                        return Err(Error::PathBlocked);
                    }
                    if cursor != target && !meta.is_dir() {
                        return Err(Error::PathBlocked);
                    }
                    let canonical = fs::canonicalize(&cursor)?;
                    let rel = self.relative(&canonical)?;
                    if is_sensitive_path(&rel) {
                        return Err(Error::PathBlocked);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && !must_exist => missing = true,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(target)
    }
}
