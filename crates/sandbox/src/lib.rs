#![forbid(unsafe_code)]
//! Primitivas de supervisão. Criar containers e autorizar execução ficam no adapter Docker.
mod docker;
mod process;
pub use docker::{
    DEFAULT_SANDBOX_IMAGE, Docker, DockerConfig, DockerDriver, DockerFuture, DockerOutput,
    NativeDocker, SandboxError,
};
pub use process::{ProcessError, ProcessOutput, supervise_child};
