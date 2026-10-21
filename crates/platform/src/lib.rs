#![deny(unsafe_code)]
#![cfg_attr(not(windows), forbid(unsafe_code))]
//! Consulta de metadados nativos. Justificativa e invariantes: docs/mutations.md.
use std::{fs::File, io};
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
