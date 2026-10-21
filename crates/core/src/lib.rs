#![forbid(unsafe_code)]

//! Orquestração independente de adapters concretos.
mod agent;
pub mod context;
pub mod guard;
pub mod instructions;
pub mod ports;
pub mod prompts;
pub use agent::{Agent, GenerationInput, GenerationResult, SessionActionGuard};
pub use myagent_protocol::Mode;
pub use ports::*;
pub use tokio_util::sync::CancellationToken;
pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> i64;
}

mod tokens;
pub use tokens::{TokenUsageSummary, token_usage};
