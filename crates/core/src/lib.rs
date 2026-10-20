#![forbid(unsafe_code)]

//! Portas mínimas do núcleo, sem dependência de adapters concretos.

pub use myagent_protocol::Mode;

/// Relógio injetável para regras de persistência e testes determinísticos.
pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> i64;
}
