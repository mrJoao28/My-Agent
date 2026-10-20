use crate::{Error, Result};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

pub const DATABASE_FILENAME: &str = "myagent-rust.sqlite3";
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const LEASE_TIMEOUT_MS: i64 = 30_000;
pub const RETENTION_GRACE_MS: i64 = 24 * 60 * 60 * 1000;
pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);
pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Debug)]
pub struct StoreConfig {
    pub home: PathBuf,
    pub retention_days: u32,
}

impl StoreConfig {
    /// Apenas resolve valores; não abre nem enumera o diretório global.
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|name| std::env::var_os(name), &std::env::current_dir()?)
    }

    /// Injeção de configuração sem alterar o ambiente global nos testes.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<OsString>, cwd: &Path) -> Result<Self> {
        let configured = lookup("MYAGENT_HOME")
            .filter(|s| !s.is_empty())
            .or_else(|| lookup("MYAGENT_CONFIG_DIR").filter(|s| !s.is_empty()));
        let home = match configured {
            Some(path) => PathBuf::from(path),
            None => {
                let profile = if cfg!(windows) {
                    lookup("USERPROFILE")
                        .filter(|s| !s.is_empty())
                        .or_else(|| lookup("HOME"))
                } else {
                    lookup("HOME")
                }
                .filter(|s| !s.is_empty())
                .ok_or(Error::InvalidHome)?;
                PathBuf::from(profile).join(".myagent")
            }
        };
        let home = if home.is_absolute() {
            home
        } else {
            cwd.join(home)
        };
        let retention = lookup("MYAGENT_SESSION_RETENTION_DAYS");
        let retention_days = match retention {
            None => 90,
            Some(value) => parse_retention_days(value.to_str().ok_or(Error::InvalidRetention)?)?,
        };
        Ok(Self {
            home,
            retention_days,
        })
    }

    pub fn database_path(&self) -> PathBuf {
        self.home.join(DATABASE_FILENAME)
    }
}

pub fn parse_retention_days(value: &str) -> Result<u32> {
    if value.is_empty() {
        return Ok(90);
    }
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::InvalidRetention);
    }
    let days: u32 = value.parse().map_err(|_| Error::InvalidRetention)?;
    if days > 365_000 {
        return Err(Error::InvalidRetention);
    }
    Ok(days)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn check_file(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if is_link(metadata) {
        return Err(Error::UnsafePath);
    }
    if metadata.is_file() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err(Error::UnsafePath);
            }
        }
        #[cfg(windows)]
        {
            let file = fs::File::open(path)?;
            if winapi_util::file::information(&file)?.number_of_links() != 1 {
                return Err(Error::UnsafePath);
            }
        }
        #[cfg(not(windows))]
        let _ = path;
    }
    Ok(())
}

/// Não segue links/junctions. Não elimina corridas com outro processo local.
pub(crate) fn check_path(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if ancestor != path && !metadata.is_dir() {
                    return Err(Error::UnsafePath);
                }
                check_file(ancestor, &metadata)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) fn prepare_database(path: &Path) -> Result<PathBuf> {
    // Recusa inclusive o nome legado sem consultar ou abrir esse arquivo.
    if path.file_name().and_then(|s| s.to_str()) != Some(DATABASE_FILENAME) {
        return Err(Error::UnsafePath);
    }
    if path.components().any(|part| {
        part.as_os_str()
            .to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("myagent.db"))
    }) {
        return Err(Error::UnsafePath);
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    check_path(&absolute)?;
    let parent = absolute.parent().ok_or(Error::InvalidHome)?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent)?;
    check_path(&absolute)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = absolute.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = PathBuf::from(name);
        check_path(&sidecar)?;
        if let Ok(metadata) = fs::symlink_metadata(&sidecar)
            && !metadata.is_file()
        {
            return Err(Error::UnsafePath);
        }
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&absolute) {
        Ok(file) => drop(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&absolute)?;
            if !metadata.is_file() {
                return Err(Error::UnsafePath);
            }
            check_file(&absolute, &metadata)?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(absolute)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub(crate) key: String,
    pub(crate) path: String,
}
impl Workspace {
    pub fn resolve(path: impl AsRef<Path>) -> Result<Self> {
        let input = path.as_ref();
        let absolute = if input.is_absolute() {
            input.to_owned()
        } else {
            std::env::current_dir()?.join(input)
        };
        if !fs::symlink_metadata(&absolute)?.is_dir() {
            return Err(Error::UnsafePath);
        }
        check_path(&absolute)?;
        let canonical = fs::canonicalize(&absolute)?;
        if !canonical.is_dir() {
            return Err(Error::UnsafePath);
        }
        let path = canonical.to_str().ok_or(Error::InvalidHome)?.to_owned();
        let key = normalize_key(&path, cfg!(windows));
        Ok(Self { key, path })
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn path(&self) -> &str {
        &self.path
    }
}

fn normalize_key(path: &str, windows: bool) -> String {
    if windows {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_and_home_precedence_are_validated_without_touching_profiles() {
        for input in ["-1", "1.5", "NaN", "365001", " ", "99999999999999999999"] {
            assert!(parse_retention_days(input).is_err());
        }
        assert_eq!(parse_retention_days("").unwrap(), 90);
        assert_eq!(parse_retention_days("0").unwrap(), 0);
        let cwd = std::env::current_dir().unwrap();
        let config = StoreConfig::from_lookup(
            |key| match key {
                "MYAGENT_HOME" => Some("fixture-new".into()),
                "MYAGENT_CONFIG_DIR" => Some("fixture-legacy".into()),
                "MYAGENT_SESSION_RETENTION_DAYS" => Some("0".into()),
                _ => None,
            },
            &cwd,
        )
        .unwrap();
        assert_eq!(config.home, cwd.join("fixture-new"));
        assert_eq!(config.retention_days, 0);
        let config = StoreConfig::from_lookup(
            |key| match key {
                "MYAGENT_CONFIG_DIR" => Some("fixture-alias".into()),
                _ => None,
            },
            &cwd,
        )
        .unwrap();
        assert_eq!(config.home, cwd.join("fixture-alias"));
        let config = StoreConfig::from_lookup(
            |key| match key {
                "HOME" | "USERPROFILE" => Some(cwd.as_os_str().to_owned()),
                _ => None,
            },
            &cwd,
        )
        .unwrap();
        assert_eq!(config.home, cwd.join(".myagent"));
        assert_eq!(
            normalize_key("C:\\Fixture\\PROJECT", true),
            "c:\\fixture\\project"
        );
        assert_eq!(normalize_key("/Fixture/PROJECT", false), "/Fixture/PROJECT");
    }
}
