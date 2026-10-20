use myagent_core::{
    AgentError, Result,
    instructions::{InstructionSource, MAX_INSTRUCTION_BYTES},
};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct FilesystemInstructions {
    root: PathBuf,
}
fn check(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| AgentError::Configuration)?;
        let linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            linked || metadata.file_attributes() & 0x400 != 0
        };
        if linked {
            return Err(AgentError::Configuration);
        }
    }
    Ok(())
}
impl FilesystemInstructions {
    pub fn new(root: &Path) -> Result<Self> {
        check(root)?;
        let root = fs::canonicalize(root).map_err(|_| AgentError::Configuration)?;
        if !root.is_dir() {
            return Err(AgentError::Configuration);
        }
        Ok(Self { root })
    }
}
impl InstructionSource for FilesystemInstructions {
    fn read(&self, relative: &Path) -> Result<Option<String>> {
        if relative.is_absolute()
            || relative.file_name().and_then(|s| s.to_str()) != Some("AGENTS.md")
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(AgentError::Configuration);
        }
        for component in relative.components() {
            let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
            if value.starts_with(".env")
                || [
                    ".git",
                    ".myagent",
                    "node_modules",
                    "config.env",
                    "chatgpt-credentials.json",
                    "api-token",
                ]
                .contains(&value.as_str())
                || value.ends_with(".db")
                || value.ends_with(".log")
            {
                return Err(AgentError::Configuration);
            }
        }
        let path = self.root.join(relative);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(AgentError::Configuration),
            Ok(metadata) if !metadata.is_file() => return Err(AgentError::Configuration),
            _ => {}
        }
        check(&path)?;
        let file = File::open(&path).map_err(|_| AgentError::Configuration)?;
        let metadata = file.metadata().map_err(|_| AgentError::Configuration)?;
        if !metadata.is_file() {
            return Err(AgentError::Configuration);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err(AgentError::Configuration);
            }
        }
        #[cfg(windows)]
        {
            if winapi_util::file::information(&file)
                .map_err(|_| AgentError::Configuration)?
                .number_of_links()
                != 1
            {
                return Err(AgentError::Configuration);
            }
        }
        let mut bytes = Vec::new();
        file.take((MAX_INSTRUCTION_BYTES + 4) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| AgentError::Configuration)?;
        if bytes.contains(&0) {
            return Err(AgentError::Configuration);
        }
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error) if error.error_len().is_none() => {
                std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .map_err(|_| AgentError::Configuration)?
            }
            Err(_) => return Err(AgentError::Configuration),
        };
        Ok(Some(text.to_owned()))
    }
}
