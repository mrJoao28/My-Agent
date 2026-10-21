#![forbid(unsafe_code)]

use myagent_core::Clock;
use myagent_protocol::{ExecutionState, MessagePart, MessageStatus, Mode, Role};
use myagent_store::*;
use serde_json::json;
use std::{
    fs,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct TestClock(AtomicI64);

#[test]
fn system_temporary_paths_work_without_canonicalizing_the_fixture() {
    let root = tempfile::tempdir().unwrap();
    let workspace = Workspace::resolve(root.path()).unwrap();
    let store = Store::open(StoreConfig {
        home: root.path().join("state"),
        retention_days: 0,
    })
    .unwrap();
    let session = store.create_session(&workspace).unwrap();
    assert!(
        store
            .load_session(&workspace, &session.id)
            .unwrap()
            .is_some()
    );
}
impl TestClock {
    fn set(&self, value: i64) {
        self.0.store(value, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
struct Fixture {
    store: Store,
    clock: Arc<TestClock>,
    workspace: Workspace,
    home: std::path::PathBuf,
    _root: TempDir,
}
impl Fixture {
    fn new(retention_days: u32) -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let root = tempfile::Builder::new()
            .prefix("myagent rust fixture ")
            .tempdir_in(base)
            .unwrap();
        let workspace_path = root.path().join("workspace with spaces");
        fs::create_dir(&workspace_path).unwrap();
        let workspace = Workspace::resolve(workspace_path).unwrap();
        let home = root.path().join("private").join("nested");
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::open_with_clock(
            StoreConfig {
                home: home.clone(),
                retention_days,
            },
            clock.clone(),
        )
        .unwrap();
        Self {
            store,
            clock,
            workspace,
            home,
            _root: root,
        }
    }
    fn second(&self) -> Store {
        Store::open_with_clock(
            StoreConfig {
                home: self.home.clone(),
                retention_days: 0,
            },
            self.clock.clone(),
        )
        .unwrap()
    }
}

#[test]
fn migrations_settings_workspace_isolation_and_pagination_survive_reopen() {
    let f = Fixture::new(90);
    let settings = f.store.settings().unwrap();
    assert_eq!(settings.journal_mode, "wal");
    assert_eq!(settings.busy_timeout_ms, 5000);
    assert!(settings.foreign_keys);
    assert_eq!(settings.schema_version, SCHEMA_VERSION);
    let first = f.store.create_session(&f.workspace).unwrap();
    f.clock.set(2000);
    let second = f.store.create_session(&f.workspace).unwrap();
    let other_path = f._root.path().join("another workspace");
    fs::create_dir(&other_path).unwrap();
    let other = Workspace::resolve(other_path).unwrap();
    assert!(f.store.load_session(&other, &first.id).unwrap().is_none());
    let sessions = f.store.list_sessions(&f.workspace, 1, 0).unwrap();
    assert_eq!(sessions[0].id, second.id);
    assert_eq!(
        f.store.list_sessions(&f.workspace, 1, 1).unwrap()[0].id,
        first.id
    );
    assert!(f.store.list_sessions(&f.workspace, 0, 0).is_err());
    assert!(f.store.list_sessions(&other, 100, 0).unwrap().is_empty());
    assert!(
        f.store
            .load_session(&f.workspace, "missing")
            .unwrap()
            .is_none()
    );
    let reader = f.second();
    assert_eq!(reader.list_sessions(&f.workspace, 100, 0).unwrap().len(), 2);
    assert!(f.home.join(DATABASE_FILENAME).exists());
}

#[test]
fn assistant_journal_is_incremental_durable_and_preserves_tool_ids() {
    let f = Fixture::new(90);
    let s = f.store.create_session(&f.workspace).unwrap();
    let user = f
        .store
        .add_user_message(&f.workspace, &s.id, "fixture", Mode::Build, "  inspect  ")
        .unwrap();
    assert_eq!(user.role, Role::User);
    assert_eq!(user.content, "inspect");
    let assistant = f
        .store
        .begin_assistant(&f.workspace, &s.id, "fixture", Mode::Build)
        .unwrap();
    assert_eq!(assistant.status, MessageStatus::InProgress);
    assert!(matches!(
        f.store
            .begin_assistant(&f.workspace, &s.id, "fixture", Mode::Build),
        Err(Error::GenerationActive)
    ));
    f.store
        .append_part(
            &f.workspace,
            &assistant.id,
            MessagePart::Text {
                text: "olá\r\n".into(),
            },
        )
        .unwrap();
    f.store
        .append_part(
            &f.workspace,
            &assistant.id,
            MessagePart::Reasoning {
                text: "internal".into(),
            },
        )
        .unwrap();
    f.store
        .append_part(
            &f.workspace,
            &assistant.id,
            MessagePart::ToolCall {
                id: "c".into(),
                name: "read_file".into(),
                args: [("path".into(), json!("a.txt"))].into_iter().collect(),
                execution: Some(ExecutionState::Pending),
            },
        )
        .unwrap();
    let reader = f.second();
    let history = reader.list_messages(&f.workspace, &s.id, 100, 0).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].content, "olá\r\n");
    assert_eq!(history[1].status, MessageStatus::InProgress);
    assert!(matches!(
        history[1].parts[2],
        MessagePart::ToolCall {
            execution: Some(ExecutionState::Pending),
            ..
        }
    ));
    f.store
        .mark_tool_started(&f.workspace, &assistant.id, "c")
        .unwrap();
    assert!(
        f.store
            .finish_assistant(
                &f.workspace,
                &assistant.id,
                MessageStatus::Complete,
                None,
                None
            )
            .is_err()
    );
    let result = MessagePart::ToolResult {
        id: "c".into(),
        name: "read_file".into(),
        result: "persistent result".into(),
    };
    f.store
        .append_part(&f.workspace, &assistant.id, result.clone())
        .unwrap();
    f.store
        .append_part(&f.workspace, &assistant.id, result)
        .unwrap();
    let saved = f
        .store
        .finish_assistant(
            &f.workspace,
            &assistant.id,
            MessageStatus::Complete,
            Some(2),
            Some(17),
        )
        .unwrap();
    assert_eq!(saved.parts.len(), 4);
    assert_eq!(saved.duration, Some(2));
    assert_eq!(saved.total_tokens, Some(17));
    assert!(
        f.store
            .append_part(
                &f.workspace,
                &assistant.id,
                MessagePart::Text {
                    text: "late".into()
                }
            )
            .is_err()
    );
    let persisted = reader.list_messages(&f.workspace, &s.id, 100, 0).unwrap();
    assert_eq!(persisted[1], saved);
}

#[test]
fn partials_and_context_persist_across_connections_without_replay_or_cross_session_leaks() {
    let f = Fixture::new(90);
    let s = f.store.create_session(&f.workspace).unwrap();
    let other = f.store.create_session(&f.workspace).unwrap();
    let assistant = f
        .store
        .begin_assistant(&f.workspace, &s.id, "fixture", Mode::Plan)
        .unwrap();
    f.store
        .append_part(
            &f.workspace,
            &assistant.id,
            MessagePart::Text {
                text: "partial".into(),
            },
        )
        .unwrap();
    let interrupted = f
        .store
        .finish_assistant(
            &f.workspace,
            &assistant.id,
            MessageStatus::Interrupted,
            None,
            None,
        )
        .unwrap();
    assert_eq!(interrupted.content, "partial");
    assert_eq!(interrupted.total_tokens, None);
    let mut context = f.store.load_context(&f.workspace, &s.id).unwrap();
    assert_eq!(context.token_limit, 16384);
    context.messages = vec![
        json!({"role":"assistant","content":[{"type":"text","text":"persistent evidence","providerMetadata":{"fixture":true}}]}),
    ];
    context.estimated_tokens = 20;
    context.truncated = true;
    let saved = f.store.save_context(&f.workspace, context.clone()).unwrap();
    let reader = f.second();
    assert_eq!(reader.load_context(&f.workspace, &s.id).unwrap(), saved);
    assert!(
        reader
            .load_context(&f.workspace, &other.id)
            .unwrap()
            .messages
            .is_empty()
    );
    context.estimated_tokens = context.token_limit + 1;
    assert!(f.store.save_context(&f.workspace, context).is_err());
    assert_eq!(reader.load_context(&f.workspace, &s.id).unwrap(), saved);
}

#[test]
fn validation_failures_leave_no_partial_messages_or_activity_changes() {
    let f = Fixture::new(90);
    let s = f.store.create_session(&f.workspace).unwrap();
    assert!(
        f.store
            .add_user_message(&f.workspace, &s.id, "fixture", Mode::Plan, "")
            .is_err()
    );
    assert!(
        f.store
            .begin_assistant(&f.workspace, &s.id, "", Mode::Plan)
            .is_err()
    );
    assert!(
        f.store
            .list_messages(&f.workspace, &s.id, 100, 0)
            .unwrap()
            .is_empty()
    );
    let assistant = f
        .store
        .begin_assistant(&f.workspace, &s.id, "fixture", Mode::Plan)
        .unwrap();
    assert!(
        f.store
            .append_part(
                &f.workspace,
                &assistant.id,
                MessagePart::ToolResult {
                    id: "orphan".into(),
                    name: "read_file".into(),
                    result: "invalid".into()
                }
            )
            .is_err()
    );
    assert!(
        f.store
            .finish_assistant(
                &f.workspace,
                &assistant.id,
                MessageStatus::Complete,
                None,
                Some(u32::MAX)
            )
            .is_err()
    );
    let message = &f.store.list_messages(&f.workspace, &s.id, 100, 0).unwrap()[0];
    assert_eq!(message.status, MessageStatus::InProgress);
    assert!(message.parts.is_empty());
}

#[test]
fn leases_expire_reject_other_owners_and_old_owner_cannot_release_or_write() {
    let f = Fixture::new(90);
    let s = f.store.create_session(&f.workspace).unwrap();
    let second = f.second();
    assert!(matches!(
        second.load_session(&f.workspace, &s.id),
        Err(Error::SessionBusy)
    ));
    let before = f.store.session_lease(&f.workspace, &s.id).unwrap().unwrap();
    f.clock.set(2000);
    f.store.heartbeat().unwrap();
    assert_eq!(
        f.store
            .session_lease(&f.workspace, &s.id)
            .unwrap()
            .unwrap()
            .expires_at,
        32000
    );
    f.clock.set(32000);
    let acquired = second.acquire_lease(&f.workspace, &s.id).unwrap();
    assert_ne!(acquired.owner, before.owner);
    f.store.heartbeat().unwrap();
    assert!(f.store.lost_sessions().unwrap().contains(&s.id));
    assert!(matches!(
        f.store
            .add_user_message(&f.workspace, &s.id, "fixture", Mode::Plan, "no"),
        Err(Error::LeaseLost)
    ));
    f.store.release_lease(&f.workspace, &s.id).unwrap();
    assert_eq!(
        second
            .session_lease(&f.workspace, &s.id)
            .unwrap()
            .unwrap()
            .owner,
        acquired.owner
    );
    second.close();
    assert!(
        f.store
            .session_lease(&f.workspace, &s.id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn background_heartbeat_renews_after_five_seconds_and_close_releases() {
    assert_eq!(HEARTBEAT_INTERVAL, Duration::from_secs(5));
    assert_eq!(LEASE_TIMEOUT_MS, 30000);
    let f = Fixture::new(90);
    let s = f.store.create_session(&f.workspace).unwrap();
    let first = f
        .store
        .session_lease(&f.workspace, &s.id)
        .unwrap()
        .unwrap()
        .expires_at;
    f.clock.set(2000);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let expiry = f
            .store
            .session_lease(&f.workspace, &s.id)
            .unwrap()
            .unwrap()
            .expires_at;
        if expiry > first {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "heartbeat did not renew the lease"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn retention_warns_preserves_activity_waits_24h_and_opening_cancels_deletion() {
    let f = Fixture::new(90);
    let a = f.store.create_session(&f.workspace).unwrap();
    let b = f.store.create_session(&f.workspace).unwrap();
    f.store.release_lease(&f.workspace, &a.id).unwrap();
    f.store.release_lease(&f.workspace, &b.id).unwrap();
    let now = 1000 + 90 * DAY_MS;
    f.clock.set(now);
    let warning = f.store.cleanup().unwrap();
    assert_eq!(warning.deleted, 0);
    assert_eq!(warning.warned.len(), 2);
    assert!(
        warning
            .warned
            .iter()
            .all(|w| w.delete_after == now + RETENTION_GRACE_MS)
    );
    let listed = f.store.list_sessions(&f.workspace, 100, 0).unwrap();
    assert!(
        listed
            .iter()
            .all(|s| s.updated_at == 1000 && s.retention_warned_at == Some(now))
    );
    f.clock.set(now + RETENTION_GRACE_MS - 1);
    assert_eq!(f.store.cleanup().unwrap().deleted, 0);
    let opened = f.store.load_session(&f.workspace, &a.id).unwrap().unwrap();
    assert!(opened.retention_warned_at.is_none());
    f.clock.set(now + RETENTION_GRACE_MS);
    assert_eq!(f.store.cleanup().unwrap().deleted, 1);
    let remaining = f.store.list_sessions(&f.workspace, 100, 0).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, a.id);
    assert!(f.store.load_session(&f.workspace, &b.id).unwrap().is_none());
}

#[test]
fn retention_disabled_and_active_generations_or_leases_are_never_warned_or_deleted() {
    let f = Fixture::new(90);
    let generation = f.store.create_session(&f.workspace).unwrap();
    f.store
        .begin_assistant(&f.workspace, &generation.id, "fixture", Mode::Build)
        .unwrap();
    let leased = f.store.create_session(&f.workspace).unwrap();
    f.clock.set(1000 + 200 * DAY_MS);
    f.store.acquire_lease(&f.workspace, &leased.id).unwrap();
    assert_eq!(f.store.cleanup().unwrap(), RetentionReport::default());
    assert_eq!(
        f.store.list_sessions(&f.workspace, 100, 0).unwrap().len(),
        2
    );
    let disabled = Fixture::new(0);
    let s = disabled.store.create_session(&disabled.workspace).unwrap();
    disabled
        .store
        .release_lease(&disabled.workspace, &s.id)
        .unwrap();
    disabled.clock.set(1000 + 200 * DAY_MS);
    assert_eq!(
        disabled.store.cleanup().unwrap(),
        RetentionReport::default()
    );
    assert!(
        disabled
            .store
            .list_sessions(&disabled.workspace, 100, 0)
            .unwrap()[0]
            .retention_warned_at
            .is_none()
    );
}

#[test]
fn legacy_filename_links_and_hard_links_are_rejected_before_sqlite_open() {
    let f = Fixture::new(0);
    assert!(matches!(
        Store::open_file(f._root.path().join("myagent.db")),
        Err(Error::UnsafePath)
    ));
    assert!(!f._root.path().join("myagent.db").exists());
    let alias_home = f._root.path().join("hardlink");
    fs::create_dir(&alias_home).unwrap();
    fs::hard_link(
        f.home.join(DATABASE_FILENAME),
        alias_home.join(DATABASE_FILENAME),
    )
    .unwrap();
    assert!(matches!(
        Store::open_file(alias_home.join(DATABASE_FILENAME)),
        Err(Error::UnsafePath)
    ));
    fs::remove_file(alias_home.join(DATABASE_FILENAME)).unwrap();
}

#[test]
fn recent_messages_override_stale_session_activity_and_warnings_survive_reopen() {
    let f = Fixture::new(90);
    let recent = f.store.create_session(&f.workspace).unwrap();
    let stale = f.store.create_session(&f.workspace).unwrap();
    let now = 1000 + 91 * DAY_MS;
    f.clock.set(now);
    f.store.acquire_lease(&f.workspace, &recent.id).unwrap();
    f.store
        .add_user_message(&f.workspace, &recent.id, "fixture", Mode::Plan, "recent")
        .unwrap();
    f.store.release_lease(&f.workspace, &recent.id).unwrap();
    f.store.release_lease(&f.workspace, &stale.id).unwrap();
    // Simula metadados antigos; o critério das mensagens é independente.
    let db = rusqlite::Connection::open(f.home.join(DATABASE_FILENAME)).unwrap();
    db.busy_timeout(Duration::from_millis(5000)).unwrap();
    db.execute("UPDATE sessions SET updated_at=1000", [])
        .unwrap();
    drop(db);
    let reopened = Store::open_with_clock(
        StoreConfig {
            home: f.home.clone(),
            retention_days: 90,
        },
        f.clock.clone(),
    )
    .unwrap();
    let sessions = reopened.list_sessions(&f.workspace, 100, 0).unwrap();
    assert!(
        sessions
            .iter()
            .find(|s| s.id == recent.id)
            .unwrap()
            .retention_warned_at
            .is_none()
    );
    assert_eq!(
        sessions
            .iter()
            .find(|s| s.id == stale.id)
            .unwrap()
            .retention_warned_at,
        Some(now)
    );
    f.clock.set(now + RETENTION_GRACE_MS);
    assert_eq!(reopened.cleanup().unwrap().deleted, 1);
    assert!(matches!(
        reopened.load_context(&f.workspace, &stale.id),
        Err(Error::SessionNotFound)
    ));
    assert!(
        reopened
            .load_session(&f.workspace, &recent.id)
            .unwrap()
            .is_some()
    );
}

#[cfg(unix)]
#[test]
fn symlinked_database_and_ancestors_are_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new(0);
    let alias = f._root.path().join("linked-home");
    symlink(&f.home, &alias).unwrap();
    assert!(matches!(
        Store::open_file(alias.join(DATABASE_FILENAME)),
        Err(Error::UnsafePath)
    ));
    assert!(matches!(Workspace::resolve(&alias), Err(Error::UnsafePath)));
}

#[cfg(windows)]
#[test]
fn junctioned_database_directory_is_rejected() {
    let f = Fixture::new(0);
    let alias = f._root.path().join("junction-home");
    let output = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&alias)
        .arg(&f.home)
        .output()
        .unwrap();
    assert!(output.status.success(), "failed to create fixture junction");
    assert!(matches!(
        Store::open_file(alias.join(DATABASE_FILENAME)),
        Err(Error::UnsafePath)
    ));
    fs::remove_dir(alias).unwrap();
}

#[test]
fn independent_processes_observe_the_same_lease() {
    if let Some(home) = std::env::var_os("MYAGENT_TEST_STORE_CHILD") {
        let workspace =
            Workspace::resolve(std::env::var_os("MYAGENT_TEST_WORKSPACE").unwrap()).unwrap();
        let id = std::env::var("MYAGENT_TEST_SESSION").unwrap();
        let store = Store::open(StoreConfig {
            home: home.into(),
            retention_days: 0,
        })
        .unwrap();
        assert!(matches!(
            store.load_session(&workspace, &id),
            Err(Error::SessionBusy)
        ));
        return;
    }
    // Usa relógio real para que outro processo interprete o mesmo expires_at.
    let f = Fixture::new(0);
    let parent = Store::open(StoreConfig {
        home: f.home.clone(),
        retention_days: 0,
    })
    .unwrap();
    let s = parent.create_session(&f.workspace).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "independent_processes_observe_the_same_lease",
            "--nocapture",
        ])
        .env("MYAGENT_TEST_STORE_CHILD", &f.home)
        .env("MYAGENT_TEST_WORKSPACE", f.workspace.path())
        .env("MYAGENT_TEST_SESSION", &s.id)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child lease assertion failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    parent.close();
    let next = Store::open(StoreConfig {
        home: f.home.clone(),
        retention_days: 0,
    })
    .unwrap();
    assert!(next.load_session(&f.workspace, &s.id).unwrap().is_some());
}
