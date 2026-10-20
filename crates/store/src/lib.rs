#![forbid(unsafe_code)]

//! Persistência local em conexão SQLite com dono único.
//! Operações bloqueantes ficam na thread do store; consumidores async devem
//! fazer a ponte sem bloquear o executor. Drop do último handle libera leases.

mod config;
mod migrations;
mod records;
mod services;

pub use config::{
    CLEANUP_INTERVAL, DATABASE_FILENAME, DAY_MS, HEARTBEAT_INTERVAL, LEASE_TIMEOUT_MS,
    RETENTION_GRACE_MS, StoreConfig, Workspace, parse_retention_days,
};
pub use migrations::SCHEMA_VERSION;
pub use services::{RetentionReport, RetentionWarning};

use myagent_core::Clock;
use myagent_protocol::{ValidationError, validate_id};
use rusqlite::Connection;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SQLite operation failed")]
    Database(#[from] rusqlite::Error),
    #[error("filesystem operation failed")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("invalid home directory")]
    InvalidHome,
    #[error("invalid retention days; expected 0..365000")]
    InvalidRetention,
    #[error("unsafe database or workspace path")]
    UnsafePath,
    #[error("database belongs to another application")]
    ForeignDatabase,
    #[error("database schema is newer than this application")]
    FutureSchema,
    #[error("invalid migration sequence")]
    MigrationSequence,
    #[error("store worker stopped")]
    WorkerStopped,
    #[error("session not found in this workspace")]
    SessionNotFound,
    #[error("session is leased by another process")]
    SessionBusy,
    #[error("session lease lost; reopen explicitly before writing")]
    LeaseLost,
    #[error("assistant message is no longer in progress")]
    MessageNotInProgress,
    #[error("invalid tool call/result sequence")]
    ToolSequence,
    #[error("another generation is already in progress")]
    GenerationActive,
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| i64::try_from(time.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

pub(crate) struct State {
    pub connection: Connection,
    pub clock: Arc<dyn Clock>,
    pub owner: String,
    pub active: HashSet<String>,
    pub lost: HashSet<String>,
    pub retention_days: u32,
}
impl State {
    pub fn now(&self) -> Result<i64> {
        let now = self.clock.now_ms();
        if now < 0 || now.checked_add(LEASE_TIMEOUT_MS).is_none() {
            return Err(ValidationError::Field("clock").into());
        }
        Ok(now)
    }
}
type Job = Box<dyn FnOnce(&mut State) + Send + 'static>;

struct Worker {
    sender: Option<SyncSender<Job>>,
    join: Option<JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Clone)]
pub struct Store {
    worker: Arc<Worker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseSettings {
    pub journal_mode: String,
    pub busy_timeout_ms: u32,
    pub foreign_keys: bool,
    pub schema_version: u32,
}

impl Store {
    /// Não é chamada pela ajuda/versão da CLI.
    pub fn open_default() -> Result<Self> {
        Self::open(StoreConfig::from_env()?)
    }

    pub fn open(config: StoreConfig) -> Result<Self> {
        Self::open_with_clock(config, Arc::new(SystemClock))
    }

    pub fn open_with_clock(config: StoreConfig, clock: Arc<dyn Clock>) -> Result<Self> {
        if config.retention_days > 365_000 {
            return Err(Error::InvalidRetention);
        }
        let path = config::prepare_database(&config.database_path())?;
        Self::start(Some(path), config.retention_days, clock)
    }

    pub fn open_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = config::prepare_database(path.as_ref())?;
        Self::start(Some(path), 90, Arc::new(SystemClock))
    }

    pub fn in_memory() -> Result<Self> {
        Self::start(None, 90, Arc::new(SystemClock))
    }

    fn start(path: Option<PathBuf>, retention_days: u32, clock: Arc<dyn Clock>) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(64);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("myagent-store".into())
            .spawn(move || {
                let initialize = || -> Result<State> {
                    let mut connection = match &path {
                        Some(path) => Connection::open_with_flags(
                            path,
                            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
                        )?,
                        None => Connection::open_in_memory()?,
                    };
                    connection.busy_timeout(Duration::from_millis(5000))?;
                    connection.set_limit(
                        rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
                        (myagent_protocol::MAX_JSON_BYTES + 65536) as i32,
                    )?;
                    connection.pragma_update(None, "foreign_keys", true)?;
                    connection.pragma_update(None, "trusted_schema", false)?;
                    if path.is_some() {
                        let mode: String =
                            connection
                                .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
                        if mode != "wal" {
                            return Err(Error::UnsafePath);
                        }
                    }
                    migrations::migrate(&mut connection, migrations::MIGRATIONS)?;
                    let owner: String =
                        connection
                            .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
                    Ok(State {
                        connection,
                        clock,
                        owner,
                        active: HashSet::new(),
                        lost: HashSet::new(),
                        retention_days,
                    })
                };
                let mut state = match initialize() {
                    Ok(state) => {
                        if ready_tx.send(Ok(())).is_err() {
                            return;
                        }
                        state
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                // Mesmo worker: manutenção não sobrepõe escritas nem a si própria.
                let _ = state.cleanup();
                let mut heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
                let mut cleanup = Instant::now() + CLEANUP_INTERVAL;
                loop {
                    let now = Instant::now();
                    if now >= heartbeat {
                        if state.heartbeat().is_err() {
                            state.lost.extend(state.active.drain());
                        }
                        heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
                    }
                    if now >= cleanup {
                        let _ = state.cleanup();
                        cleanup = Instant::now() + CLEANUP_INTERVAL;
                    }
                    match receiver.recv_timeout(heartbeat.saturating_duration_since(Instant::now()))
                    {
                        Ok(job) => job(&mut state),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                let _ = state
                    .connection
                    .execute("DELETE FROM session_leases WHERE owner=?1", [&state.owner]);
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                worker: Arc::new(Worker {
                    sender: Some(sender),
                    join: Some(join),
                }),
            }),
            Ok(Err(error)) => {
                drop(sender);
                let _ = join.join();
                Err(error)
            }
            Err(_) => {
                drop(sender);
                let _ = join.join();
                Err(Error::WorkerStopped)
            }
        }
    }

    pub(crate) fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut State) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker
            .sender
            .as_ref()
            .ok_or(Error::WorkerStopped)?
            .send(Box::new(move |state| {
                let _ = tx.send(work(state));
            }))
            .map_err(|_| Error::WorkerStopped)?;
        rx.recv().map_err(|_| Error::WorkerStopped)?
    }

    pub fn settings(&self) -> Result<DatabaseSettings> {
        self.call(|state| {
            Ok(DatabaseSettings {
                journal_mode: state
                    .connection
                    .query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
                busy_timeout_ms: state
                    .connection
                    .query_row("PRAGMA busy_timeout", [], |r| r.get(0))?,
                foreign_keys: state
                    .connection
                    .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, u32>(0))?
                    == 1,
                schema_version: state
                    .connection
                    .query_row("PRAGMA user_version", [], |r| r.get(0))?,
            })
        })
    }

    /// Encerra este handle. O último handle espera a thread e libera seus leases.
    pub fn close(self) {
        drop(self);
    }

    pub(crate) fn checked_session_id(value: &str) -> Result<String> {
        validate_id(value)?;
        Ok(value.to_owned())
    }
}
