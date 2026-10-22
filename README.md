# My-Agent

Reescrita em Rust do My-Code-Agent, assistente de programação de terminal
originalmente desenvolvido em TypeScript/Bun. O objetivo é paridade funcional
com uma arquitetura idiomática em Rust, para Windows, Linux e macOS.

O repositório contém um workspace com nove crates: protocol, core, providers,
tools, platform, sandbox, store, tui e cli. O binário `myagent` abre uma TUI
Grafite com sessões, PLAN/BUILD, prévias de código, aprovações e `/undo`.
As bibliotecas incluem contratos, SQLite local e o núcleo de geração
com providers Anthropic, OpenAI, Google e Ollama, além de 17 ferramentas
locais de inspeção e nove ferramentas de alteração com aprovação e checkpoints.
As bibliotecas também oferecem dois adapters web com aprovação por origem e
três comandos exclusivamente em Docker. Contas ChatGPT usam OAuth separado
das chaves OpenAI, com catálogo da conta ativa e escolha explícita do modelo.
O tipo Mode prevê evolução, mas somente Plan e Build estão declarados.

A arquitetura está aceita no [ADR 0001](docs/adr/0001-arquitetura.md): canais
internos, armazenamento separado e futura importação explícita, SQLite bundled
e tree-sitter com compilador C no build. As dependências do M1 e os contratos
do store estão descritos em [Persistência M1](docs/persistence.md).
Providers, configuração de modelos, limites e testes simulados estão em
[Gerações M2](docs/generations.md).
Ferramentas, limites e proteções estão em [Inspeção M3](docs/tools.md).
Consentimento, checkpoints e desfazer estão em [Alterações M4](docs/mutations.md).
Isolamento, runners e proteção SSRF estão em [Sandbox e web M5](docs/sandbox-web.md).
Uso, teclado, temas e validação da interface estão em [TUI Grafite M6](docs/tui.md).
As escolhas e alternativas estão justificadas no ADR.
Consulte a [matriz de paridade](docs/PARITY.md) para comportamentos e testes a portar.

## Instalação global

Após a publicação autorizada no npm:

```sh
npm i -g @j2008/myagent
myagent --version
myagent doctor
myagent
```

Requer Node.js >= 22.14.0; mantenha as dependências opcionais habilitadas.
O Node apenas inicia o binário Rust correspondente: Windows x64, Linux
x64/arm64 ou macOS x64/arm64. Não há postinstall, compilação ou download
adicional na instalação. Rust e Bun não são necessários para executar.
Docker permanece opcional para as ferramentas isoladas.

O workflow e o checklist de publicação, Trusted Publishers, tarballs e
recuperação de publicação parcial estão em [Distribuição npm M8](docs/npm-release.md).
O suporte configurado na matriz não comprova execução local em cada sistema;
consulte a evidência efetivamente executada em [PARITY](docs/PARITY.md).

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
cargo run --locked -p myagent-cli --bin myagent -- --demo
```

`--demo` demonstra os painéis de código, aprovações e undo em um projeto
temporário, sem chave de API. Para usar no projeto atual, execute o mesmo
comando sem `--demo`; `--new` inicia nova sessão. Configure modelos/providers
conforme [Gerações M2](docs/generations.md).

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
