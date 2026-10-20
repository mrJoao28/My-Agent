# My-Agent

Reescrita em Rust do My-Code-Agent, assistente de programação de terminal
originalmente desenvolvido em TypeScript/Bun. O objetivo é paridade funcional
com uma arquitetura idiomática em Rust, para Windows, Linux e macOS.

Este repositório está no bootstrap: há somente um crate-base vazio, sem
dependências externas. Ainda não existe CLI, TUI, provider ou ferramenta
implementada em Rust. Consulte a [matriz de paridade](docs/PARITY.md) para o
inventário de comportamentos, evidências no legado e testes a portar.

## Desenvolvimento

Instale Rust com rustup e as ferramentas de compilação da sua plataforma
(no Windows/MSVC, o linker das Build Tools do Visual Studio). A versão está
fixada em `rust-toolchain.toml`: Rust 1.99.1, stable em 19/10/2026, conforme o
[anúncio oficial](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/).

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

O CI está configurado para executar esses gates em Windows, Linux e macOS.
Uma execução local em Windows não comprova os outros dois sistemas. Nesta fase,
`cargo test` valida apenas a compilação do crate vazio; não há testes funcionais.
Docker e GitHub CLI são opcionais para preparar o repositório.

## Contribuição e segurança

Siga [AGENTS.md](AGENTS.md), [CONTRIBUTING.md](CONTRIBUTING.md) e
[SECURITY.md](SECURITY.md). O legado em `C:\Users\joaof\My-Code-Agent` é somente
leitura. Não copie dependências, artefatos, bancos ou configurações privadas.
Referências locais ficam em `reference/`, ignorado pelo Git.

Licença: [MIT](LICENSE).
