#![forbid(unsafe_code)]

//! Contratos de dados do My-Agent, sem I/O.

/// Modo do agente. Novos modos exigem política explícita antes de executar tools.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Mode {
    /// Planejamento sem ferramentas que alterem o projeto.
    Plan,
    /// Implementação sujeita à política de ferramentas e aprovações.
    Build,
}
