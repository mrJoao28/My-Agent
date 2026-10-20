#![forbid(unsafe_code)]
//! Primitivas de supervisão. Criar containers e autorizar execução ficam no adapter Docker.
mod process;
pub use process::{ProcessError, ProcessOutput, supervise_child};
