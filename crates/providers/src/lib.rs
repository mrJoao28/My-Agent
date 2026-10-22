#![forbid(unsafe_code)]
//! Codecs HTTP/SSE; nenhum SDK pode executar ferramentas.
pub mod chatgpt;
mod http;
pub mod registry;
pub mod sse;
mod wire;
pub use http::{HttpProvider, ProviderEndpoints, StreamLimits};
pub use registry::{Model, ModelRegistry, Pricing, ProviderKind};

#[cfg(test)]
mod tests;
