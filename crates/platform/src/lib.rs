#![deny(unsafe_code)]
#![cfg_attr(not(windows), forbid(unsafe_code))]
//! Consulta de metadados nativos. Justificativa e invariantes: docs/mutations.md.
use std::{fs::File, io};

/// macOS exposes these two system temporary roots through fixed aliases.
/// No workspace-created symlink, junction or arbitrary target is trusted.
pub fn is_system_directory_alias(path: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let expected = match path.to_str() {
            Some("/var") => "/private/var",
            Some("/tmp") => "/private/tmp",
            _ => return false,
        };
        std::fs::read_link(path).is_ok_and(|target| {
            // Apple's root aliases may store a relative target (private/var).
            // Resolve only against their fixed parent, without canonicalizing
            // arbitrary user-controlled links into trusted paths.
            std::path::Path::new("/").join(target) == std::path::Path::new(expected)
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

#[cfg(unix)]
pub fn change_time(file: &File) -> io::Result<i128> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata()?;
    Ok(i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec()))
}
#[cfg(windows)]
pub fn change_time(file: &File) -> io::Result<i128> {
    windows::change_time(file)
}
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandleEx,
    };
    pub(super) fn change_time(file: &File) -> io::Result<i128> {
        let mut basic = FILE_BASIC_INFO {
            CreationTime: 0,
            LastAccessTime: 0,
            LastWriteTime: 0,
            ChangeTime: 0,
            FileAttributes: 0,
        };
        // SAFETY: file keeps its valid owned handle alive for this synchronous call.
        // basic is initialized, aligned, exclusively borrowed, and exactly the
        // FILE_BASIC_INFO layout required by FileBasicInfo. The pointer cannot
        // escape; the byte size is the matching struct size, representable in u32.
        let success = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(i128::from(basic.ChangeTime) * 100)
    }
}

#[cfg(test)]
mod path_tests {
    #[test]
    fn arbitrary_paths_are_never_system_aliases() {
        for path in [
            "/project/var",
            "/project/tmp",
            "/var/project",
            "/private/var",
            "var",
            "/",
        ] {
            assert!(!super::is_system_directory_alias(std::path::Path::new(
                path
            )));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_temporary_roots_match_only_their_fixed_targets() {
        for (path, target) in [("/var", "/private/var"), ("/tmp", "/private/tmp")] {
            assert_eq!(
                std::path::Path::new("/").join(std::fs::read_link(path).unwrap()),
                std::path::Path::new(target)
            );
            assert!(super::is_system_directory_alias(std::path::Path::new(path)));
        }
    }
}
