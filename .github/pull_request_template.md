## Problema e comportamento resultante

Descreva o cenário resolvido e a mudança observável.

## Paridade e escopo

- Milestone e linhas atualizadas em docs/PARITY.md:
- Evidência e testes no legado:
- Novos crates e justificativa, se houver:

## Validação realmente executada

- cargo fmt --check:
- cargo clippy --all-targets -- -D warnings:
- cargo test (incluindo testes ignorados):
- Windows, Linux e macOS (diferencie local e CI):

## Pendências e riscos

Registre falhas de ambiente separadamente de erros de código, limitações e
eventual justificativa documentada para unsafe. Confirme revisão dos arquivos
staged em busca de segredos, sem expor seus valores.
