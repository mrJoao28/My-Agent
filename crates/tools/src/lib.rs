#![forbid(unsafe_code)]
//! Ferramentas de inspeção com autorização de workspace e limites nas bordas.
mod approval;
mod checkpoints;
mod execution;
mod execution_approval;
mod file_mutations;
mod files;
mod git;
mod git_mutations;
mod inspect;
mod instructions;
mod io;
mod mutation_io;
mod mutations;
mod snapshot;
mod web;
pub use execution::ExecutionRegistry;
pub use snapshot::{Snapshot, SnapshotOptions};
pub use web::{
    NativeWebTransport, SystemResolver, WebClient, WebRegistry, WebRequest, WebResolver,
    WebResponse, WebTransport, is_public_ip,
};
mod path;
mod registry;
mod search;
mod source;
pub use approval::{ApprovalDecision, ApprovalGrant, ApprovalManager};
pub use checkpoints::{
    CHECKPOINT_FILE_BYTES, CHECKPOINT_SESSION_BYTES, CHECKPOINT_STEP_BYTES, CheckpointStore,
    UndoResult,
};
pub use git_mutations::{GitSettings, NativePush, PushRequest, PushTransport, validate_git_url};
pub use instructions::FilesystemInstructions;
pub use mutations::{MUTATION_NAMES, MutationRegistry};
pub use path::{SafeWorkspace, is_sensitive_path};
pub use registry::{TOOL_NAMES, ToolRegistry, schema};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("PATH_BLOCKED")]
    PathBlocked,
    #[error("NOT_FOUND")]
    NotFound,
    #[error("NOT_A_FILE")]
    NotAFile,
    #[error("NOT_A_DIRECTORY")]
    NotADirectory,
    #[error("FILE_TOO_LARGE")]
    FileTooLarge,
    #[error("BINARY_FILE")]
    BinaryFile,
    #[error("LINE_TOO_LONG")]
    LineTooLong,
    #[error("INVALID_INPUT")]
    InvalidInput,
    #[error("INPUT_LIMIT")]
    InputLimit,
    #[error("OUTPUT_LIMIT")]
    OutputLimit,
    #[error("INVALID_PATTERN")]
    InvalidPattern,
    #[error("GITIGNORE_TOO_LARGE")]
    IgnoreTooLarge,
    #[error("UNSUPPORTED_LANGUAGE")]
    UnsupportedLanguage,
    #[error("SOURCE_PARSE_FAILED")]
    Parse,
    #[error("INVALID_PACKAGE_JSON")]
    Package,
    #[error("TOOL_CANCELLED")]
    Cancelled,
    #[error("TOOL_TIMEOUT")]
    Timeout,
    #[error("TOOL_UNAVAILABLE")]
    Unavailable,
    #[error("SANDBOX_ERROR: {0}")]
    Sandbox(#[from] myagent_sandbox::SandboxError),
    #[error("SNAPSHOT_LIMIT")]
    SnapshotLimit,
    #[error("SNAPSHOT_CHANGED")]
    SnapshotChanged,
    #[error("SSRF_BLOCKED")]
    Ssrf,
    #[error("CROSS_ORIGIN_REDIRECT")]
    CrossOrigin,
    #[error("WEB_REQUEST_FAILED")]
    Web,
    #[error("TAVILY_API_KEY_REQUIRED")]
    SearchKey,
    #[error("UNSAFE_GIT_METADATA_OR_CONFIG")]
    GitUnsafe,
    #[error("GIT_FAILED")]
    Git,
    #[error("INVALID_REVISION")]
    Revision,
    #[error("FILE_NOT_TRACKED")]
    NotTracked,
    #[error("LINE_OUT_OF_RANGE")]
    LineRange,
    #[error("READ_FAILED")]
    Io,
    #[error("INTERNAL_ERROR")]
    Internal,
    #[error("APPROVAL_DENIED")]
    Denied,
    #[error("APPROVAL_EXPIRED")]
    ApprovalExpired,
    #[error("APPROVAL_SCOPE_LIMIT")]
    ScopeLimit,
    #[error("STALE_APPROVAL")]
    StaleApproval,
    #[error("CHECKPOINT_LIMIT")]
    CheckpointLimit,
    #[error("INVALID_CHECKPOINT")]
    InvalidCheckpoint,
    #[error("INCOMPLETE_CHECKPOINT")]
    IncompleteCheckpoint,
    #[error("NO_CHECKPOINT")]
    NoCheckpoint,
    #[error("UNDO_CONFLICT")]
    Conflict { paths: Vec<String> },
    #[error("ALREADY_EXISTS")]
    AlreadyExists,
    #[error("OLD_STRING_OCCURRENCES: {0}; expected exactly 1")]
    Occurrences(usize),
    #[error("INVALID_PATCH")]
    Patch,
    #[error("SESSION_OR_LEASE_INVALID")]
    Session,
    #[error("GENERATION_ACTIVE")]
    GenerationActive,
    #[error("GIT_IDENTITY_REQUIRED")]
    GitIdentity,
    #[error("GIT_URL_OR_BRANCH_REJECTED")]
    GitTarget,
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Io
        }
    }
}
impl From<Error> for myagent_core::AgentError {
    fn from(e: Error) -> Self {
        if matches!(e, Error::Cancelled) {
            Self::Cancelled
        } else if matches!(e, Error::Sandbox(myagent_sandbox::SandboxError::Cleanup)) {
            Self::Cleanup
        } else {
            Self::ToolFailure(e.to_string())
        }
    }
}
