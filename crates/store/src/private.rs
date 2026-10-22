//! Protected JSON with a stable OS lock. Tokens never enter session SQLite.
use myagent_core::CancellationToken;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct PrivateStore {
    root: PathBuf,
}
pub struct PrivateGuard {
    root: PathBuf,
    _lock: File,
}
const LIMIT: u64 = 2 * 1024 * 1024;

impl PrivateStore {
    pub fn new(home: &Path) -> Self {
        Self {
            root: home.join("oauth-rust"),
        }
    }
    pub fn lock(&self, cancel: &CancellationToken) -> io::Result<PrivateGuard> {
        crate::config::check_path(&self.root)
            .map_err(|_| io::Error::other("unsafe credential path"))?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        let existed = self.root.exists();
        match builder.create(&self.root) {
            Ok(()) if !existed => protect(&self.root)?,
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        check_private(&self.root)?;
        let path = self.root.join("accounts.lock");
        crate::config::check_path(&path).map_err(|_| io::Error::other("unsafe credential lock"))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = match options.open(&path) {
            Ok(file) => {
                protect(&path)?;
                file
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                check_private(&path)?;
                OpenOptions::new().read(true).write(true).open(&path)?
            }
            Err(e) => return Err(e),
        };
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if cancel.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "credential operation cancelled",
                ));
            }
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return Err(io::Error::other("credential lock unavailable")),
            }
        }
        Ok(PrivateGuard {
            root: self.root.clone(),
            _lock: lock,
        })
    }
}
impl PrivateGuard {
    pub fn read<T: DeserializeOwned + Default>(&self) -> io::Result<T> {
        let path = self.root.join("accounts.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(T::default()),
            Err(e) => return Err(e),
            Ok(_) => check_private(&path)?,
        }
        let file = File::open(&path)?;
        if !file.metadata()?.is_file() || file.metadata()?.len() > LIMIT {
            return Err(io::Error::other("invalid credential file"));
        }
        let mut bytes = vec![];
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes)
            .map_err(|_| io::Error::other("invalid credential state; preserved"))
    }
    pub fn write<T: Serialize>(&self, value: &T) -> io::Result<()> {
        let bytes =
            serde_json::to_vec(value).map_err(|_| io::Error::other("invalid credential state"))?;
        if bytes.len() > LIMIT as usize {
            return Err(io::Error::other("credential state too large"));
        }
        check_private(&self.root)?;
        let path = self.root.join("accounts.json");
        crate::config::check_path(&path).map_err(|_| io::Error::other("unsafe credential path"))?;
        if path.exists() {
            check_private(&path)?;
        }
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        protect(temp.path())?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(&path)
            .map_err(|_| io::Error::other("credential update failed"))?;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}
pub fn check_private(path: &Path) -> io::Result<()> {
    crate::config::check_path(path).map_err(|_| io::Error::other("unsafe credential path"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if fs::metadata(path)?.permissions().mode() & 0o077 != 0
            || fs::metadata(path)?.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(io::Error::other(
                "credential permissions must be owner-only",
            ));
        }
    }
    #[cfg(windows)]
    windows_private(path, false)?;
    Ok(())
}
fn protect(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if path.is_dir() { 0o700 } else { 0o600 }),
        )?;
    }
    #[cfg(windows)]
    windows_private(path, true)?;
    Ok(())
}
#[cfg(windows)]
fn windows_private(path: &Path, set: bool) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_permissions::{
        LocalBox, SecurityDescriptor,
        constants::{SeObjectType, SecurityInformation},
        wrappers,
    };
    static SID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let sid = if let Some(sid) = SID.get() {
        sid.clone()
    } else {
        let system = std::env::var_os("SystemRoot")
            .ok_or_else(|| io::Error::other("system directory unavailable"))?;
        let output = std::process::Command::new(PathBuf::from(system).join("System32/whoami.exe"))
            .args(["/user", "/fo", "csv", "/nh"])
            .creation_flags(0x08000000)
            .output()?;
        let sid = String::from_utf8(output.stdout)
            .map_err(|_| io::Error::other("invalid user SID"))?
            .trim()
            .rsplit(',')
            .next()
            .unwrap_or("")
            .trim_matches('"')
            .to_owned();
        if !output.status.success()
            || !sid.starts_with("S-1-5-")
            || !sid
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'S' || b == b'-')
        {
            return Err(io::Error::other("user SID unavailable"));
        }
        let _ = SID.set(sid.clone());
        sid
    };
    // OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION.
    let info = SecurityInformation::from_bits_truncate(5);
    if set {
        let sd: LocalBox<SecurityDescriptor> = format!("O:{sid}D:P(A;;FA;;;{sid})")
            .parse()
            .map_err(io::Error::other)?;
        wrappers::SetNamedSecurityInfo(
            path.as_os_str(),
            SeObjectType::SE_FILE_OBJECT,
            SecurityInformation::from_bits_truncate(0x80000005),
            sd.owner(),
            None,
            sd.dacl(),
            None,
        )
        .map_err(|e| io::Error::other(format!("set private ACL: {e}")))?;
    }
    let sd = wrappers::GetNamedSecurityInfo(path.as_os_str(), SeObjectType::SE_FILE_OBJECT, info)
        .map_err(|e| io::Error::other(format!("read private ACL: {e}")))?;
    validate_windows_descriptor(&sd, &sid)
}

#[cfg(windows)]
fn validate_windows_descriptor(
    sd: &windows_permissions::SecurityDescriptor,
    sid: &str,
) -> io::Result<()> {
    use windows_permissions::{
        LocalBox, Sid,
        constants::{AccessRights, AceType, SecurityInformation},
        wrappers,
    };
    // Check protection before accessing the ACL: a NULL DACL grants everyone
    // access, and the dependency's dacl() accessor cannot represent it safely.
    let info = SecurityInformation::from_bits_truncate(5);
    let sddl = wrappers::ConvertSecurityDescriptorToStringSecurityDescriptor(sd, info)
        .map_err(|e| io::Error::other(format!("encode private ACL: {e}")))?;
    let flags = sddl
        .to_str()
        .and_then(|text| text.split_once("D:"))
        .and_then(|(_, dacl)| dacl.split_once('('))
        .map(|(flags, _)| flags);
    if !matches!(flags, Some("P" | "PAI")) {
        return Err(io::Error::other("credential ACL must be protected"));
    }
    let expected: LocalBox<Sid> = sid.parse().map_err(io::Error::other)?;
    // Compare SID values, not their SDDL spelling: Windows can emit aliases
    // such as SY/LS/NS/LA instead of the numeric SID returned by whoami.
    let acl = sd
        .dacl()
        .ok_or_else(|| io::Error::other("missing private ACL"))?;
    let ace = acl
        .get_ace(0)
        .ok_or_else(|| io::Error::other("empty private ACL"))?;
    if sd.owner() != Some(&*expected)
        || acl.len() != 1
        || ace.ace_type() != AceType::ACCESS_ALLOWED_ACE_TYPE
        || !ace.flags().is_empty()
        || ace.mask() != AccessRights::FileAllAccess
        || ace.sid() != Some(&*expected)
    {
        return Err(io::Error::other("credential ACL must be owner-only"));
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::validate_windows_descriptor;
    use windows_permissions::{
        LocalBox, SecurityDescriptor, constants::SecurityInformation, wrappers,
    };

    #[test]
    fn numeric_sids_and_sddl_aliases_have_the_same_private_permissions() {
        for sid in ["S-1-5-18", "S-1-5-19", "S-1-5-20", "S-1-5-32-544"] {
            for flags in ["P", "PAI"] {
                let original = format!("O:{sid}D:{flags}(A;;FA;;;{sid})");
                let sd: LocalBox<SecurityDescriptor> = original.parse().unwrap();
                let canonical = wrappers::ConvertSecurityDescriptorToStringSecurityDescriptor(
                    &sd,
                    SecurityInformation::from_bits_truncate(5),
                )
                .unwrap();
                assert_ne!(
                    canonical.to_str().unwrap(),
                    original,
                    "fixture must reproduce SDDL alias normalization"
                );
                validate_windows_descriptor(&sd, sid).unwrap();
            }
        }
        let administrator: LocalBox<SecurityDescriptor> = "O:LAD:P(A;;FA;;;LA)".parse().unwrap();
        let numeric = administrator.owner().unwrap().to_string();
        validate_windows_descriptor(&administrator, &numeric).unwrap();
    }

    #[test]
    fn aliases_do_not_relax_owner_trustee_rights_or_inheritance_checks() {
        for text in [
            "O:LSD:P(A;;FA;;;SY)",
            "O:SYD:P(A;;FA;;;LS)",
            "O:SYD:P(A;;FA;;;SY)(A;;FR;;;WD)",
            "O:SYD:(A;;FA;;;SY)",
            "O:SYD:P",
            "O:SYD:NO_ACCESS_CONTROL",
            "O:SYD:P(A;;FR;;;SY)",
            "O:SYD:P(D;;FA;;;SY)",
            "O:SYD:P(A;OI;FA;;;SY)",
        ] {
            let sd: LocalBox<SecurityDescriptor> = text.parse().unwrap();
            assert!(
                validate_windows_descriptor(&sd, "S-1-5-18").is_err(),
                "accepted unsafe fixture: {text}"
            );
        }
    }
}
