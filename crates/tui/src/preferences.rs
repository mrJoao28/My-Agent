use crate::theme::Theme;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub theme: Theme,
}
pub fn check_path(path: &Path) -> io::Result<()> {
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(meta) => {
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let linked = meta.file_type().is_symlink();
                if linked && !myagent_platform::is_system_directory_alias(parent) {
                    return Err(io::Error::other("unsafe preferences path"));
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if meta.is_file() && meta.nlink() != 1 {
                        return Err(io::Error::other("hard-linked preferences"));
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
impl Preferences {
    pub fn load(path: &Path) -> io::Result<Self> {
        check_path(path)?;
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        if !file.metadata()?.is_file() || file.metadata()?.len() > 8192 {
            return Err(io::Error::other("invalid preferences file"));
        }
        let mut bytes = vec![];
        file.take(8193).read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes).map_err(|_| io::Error::other("invalid preferences JSON"))
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        check_path(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("invalid preferences path"))?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent)?;
        check_path(path)?;
        let temp: PathBuf = parent.join(format!(
            ".preferences-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos()
        ));
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut f = options.open(&temp)?;
            f.write_all(&serde_json::to_vec(self).map_err(io::Error::other)?)?;
            f.sync_all()?;
            drop(f);
            check_path(path)?;
            fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}
