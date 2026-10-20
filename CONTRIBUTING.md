# Como contribuir

Leia [AGENTS.md](AGENTS.md) antes de trabalhar. Documentação e explicações são em
português; código, identificadores e commits são em inglês. A licença MIT mantém
seu texto jurídico original em inglês.

## Escopo e dependências

Implemente um milestone por vez, em passos verificáveis. Confirme comportamentos
no código do legado, que é somente leitura, e atualize [docs/PARITY.md](docs/PARITY.md).
Justifique cada crate por uma necessidade concreta. O bootstrap não usa crates externos.
Prefira `#![forbid(unsafe_code)]`; qualquer exceção exige justificativa documentada.

## Verificação

Antes de fechar um milestone, execute:

```sh
cargo fmt --all --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

O CI deve executar os mesmos gates em Windows, Linux e macOS. Cubra CRLF,
caminhos, junctions, cancelamento e limites quando afetados. Testes ignorados
não contam como aprovados. Diferencie bloqueios de ambiente de erros de código.
O esqueleto tem testes da interface pública de ajuda/versão e argumentos;
gates verdes não significam paridade funcional do agente.

`deny.toml` define a política inicial de licenças e fontes. Quando houver
dependências, valide-a com `cargo deny check` em ambiente com cargo-deny e acesso
aos índices/advisories. Essa checagem adicional ainda não integra o CI mínimo.
Não amplie a lista de licenças sem revisar a necessidade concreta.

## Git e entrega

O primeiro commit vai na `main`. Depois, use um branch por milestone, como
`feat/m1-store`, e PR para a `main`. Use Conventional Commits em inglês.

Atualize a matriz, rode os gates e resuma entrega, testes realmente executados e
pendências. Revise os arquivos staged em busca de segredos. Mostre o diff stat,
os arquivos e a mensagem proposta e peça autorização explícita antes do commit.
Depois, peça autorização para push e abertura do PR. Alterar remotes, criar tags,
mergear PRs e qualquer operação destrutiva também exigem autorização explícita.
Nunca use force push, publique no npm ou crie tags de release sem pedido do usuário.
