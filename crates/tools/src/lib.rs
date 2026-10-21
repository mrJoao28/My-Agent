#![forbid(unsafe_code)]
//! Ferramentas de inspeção com autorização de workspace e limites nas bordas.
mod files;
mod git;
mod inspect;
mod instructions;
mod io;
mod path;
mod registry;
mod search;
mod source;
pub use instructions::FilesystemInstructions;
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
        } else {
            Self::ToolFailure(e.to_string())
        }
    }
}
