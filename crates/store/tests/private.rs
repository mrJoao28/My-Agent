use myagent_core::CancellationToken;
use myagent_store::private::{PrivateStore, check_private};

#[test]
fn protected_json_is_atomic_and_invalid_state_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrivateStore::new(dir.path());
    let guard = store.lock(&CancellationToken::new()).unwrap();
    assert_eq!(guard.read::<u64>().unwrap(), 0);
    guard.write(&41u64).unwrap();
    guard.write(&42u64).unwrap();
    assert_eq!(guard.read::<u64>().unwrap(), 42);
    let root = dir.path().join("oauth-rust");
    let path = root.join("accounts.json");
    check_private(&root).unwrap();
    check_private(&path).unwrap();
    std::fs::write(&path, "invalid fictitious state").unwrap();
    assert!(guard.read::<u64>().is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "invalid fictitious state"
    );
    let alias = dir.path().join("alias.json");
    std::fs::hard_link(&path, alias).unwrap();
    assert!(guard.read::<u64>().is_err());
    assert!(guard.write(&0u64).is_err());
}
#[test]
fn lock_wait_is_cancellable_and_releases_after_drop() {
    let dir = tempfile::tempdir().unwrap();
    let store = PrivateStore::new(dir.path());
    let guard = store.lock(&CancellationToken::new()).unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(store.lock(&cancelled).is_err());
    drop(guard);
    assert!(store.lock(&CancellationToken::new()).is_ok());
}
#[cfg(unix)]
#[test]
fn broad_permissions_and_symlinked_roots_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let store = PrivateStore::new(dir.path());
    drop(store.lock(&CancellationToken::new()).unwrap());
    std::fs::set_permissions(
        dir.path().join("oauth-rust"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(store.lock(&CancellationToken::new()).is_err());
    let other = tempfile::tempdir().unwrap();
    symlink(
        dir.path().join("oauth-rust"),
        other.path().join("oauth-rust"),
    )
    .unwrap();
    assert!(
        PrivateStore::new(other.path())
            .lock(&CancellationToken::new())
            .is_err()
    );
}
#[cfg(windows)]
#[test]
fn broad_windows_acl_is_rejected_before_credentials_are_read() {
    use windows_permissions::{
        LocalBox, SecurityDescriptor,
        constants::{SeObjectType, SecurityInformation},
        wrappers::SetNamedSecurityInfo,
    };
    let dir = tempfile::tempdir().unwrap();
    let guard = PrivateStore::new(dir.path())
        .lock(&CancellationToken::new())
        .unwrap();
    guard.write(&1u64).unwrap();
    let path = dir.path().join("oauth-rust/accounts.json");
    let sd: LocalBox<SecurityDescriptor> = "D:P(A;;FA;;;WD)".parse().unwrap();
    SetNamedSecurityInfo(
        path.as_os_str(),
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::from_bits_truncate(0x80000004),
        None,
        None,
        sd.dacl(),
        None,
    )
    .unwrap();
    assert!(guard.read::<u64>().is_err());
}
