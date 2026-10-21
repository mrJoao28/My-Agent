# My-Agent

Reescrita em Rust do My-Code-Agent, assistente de programação de terminal
originalmente desenvolvido em TypeScript/Bun. O objetivo é paridade funcional
com uma arquitetura idiomática em Rust, para Windows, Linux e macOS.

O repositório contém um workspace com nove crates: protocol, core, providers,
tools, platform, sandbox, store, tui e cli. O binário `myagent` oferece apenas ajuda e
versão. As bibliotecas incluem contratos, SQLite local e o núcleo de geração
com providers Anthropic, OpenAI, Google e Ollama, além de 17 ferramentas
locais de inspeção e nove ferramentas de alteração com aprovação e checkpoints.
Conversa e diálogos na CLI/TUI, web e execução de processos do projeto
continuam para os próximos milestones.
O tipo Mode prevê evolução, mas somente Plan e Build estão declarados.

A arquitetura está aceita no [ADR 0001](docs/adr/0001-arquitetura.md): canais
internos, armazenamento separado e futura importação explícita, SQLite bundled
e tree-sitter com compilador C no build. As dependências do M1 e os contratos
do store estão descritos em [Persistência M1](docs/persistence.md).
Providers, configuração de modelos, limites e testes simulados estão em
[Gerações M2](docs/generations.md).
Ferramentas, limites e proteções estão em [Inspeção M3](docs/tools.md).
Consentimento, checkpoints e desfazer estão em [Alterações M4](docs/mutations.md).
As escolhas e alternativas estão justificadas no ADR.
Consulte a [matriz de paridade](docs/PARITY.md) para comportamentos e testes a portar.

## Desenvolvimento

Instale Rust com rustup e as ferramentas de compilação da sua plataforma
(no Windows/MSVC, o linker das Build Tools do Visual Studio). A versão está
fixada em `rust-toolchain.toml`: Rust 1.99.1, stable em 19/10/2026, conforme o
[anúncio oficial](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/).

```sh
cargo fmt --all --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo run --locked -p myagent-cli --bin myagent -- --help
cargo run --locked -p myagent-cli --bin myagent -- --version
```

O CI está configurado para executar esses gates em Windows, Linux e macOS.
Uma execução local em Windows não comprova os outros dois sistemas. Os
resultados dos testes de contratos, persistência, providers simulados e
orquestração ficam registrados em docs/PARITY.md; os outros sistemas aguardam CI.
Ajuda/versão não abrem banco, configuração ou rede.
Docker e GitHub CLI são opcionais para preparar o repositório.

## Contribuição e segurança

Siga [AGENTS.md](AGENTS.md), [CONTRIBUTING.md](CONTRIBUTING.md) e
[SECURITY.md](SECURITY.md). O legado em `C:\Users\joaof\My-Code-Agent` é somente
leitura. Não copie dependências, artefatos, bancos ou configurações privadas.
Referências locais ficam em `reference/`, ignorado pelo Git.

Licença: [MIT](LICENSE).
