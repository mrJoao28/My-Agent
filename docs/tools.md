# Ferramentas de inspeção — M3

O crate myagent-tools oferece 17 ferramentas locais ao núcleo do agente.
A CLI continua com ajuda/versão; a integração de conversa da CLI/TUI permanece
para os milestones próprios. Não há registro de web, shell, test_run ou
type_check neste M3. Inspeção Git é a exceção explícita de processo: somente
comandos fixos do Git confiável, sem interpretar comandos fornecidos pelo modelo.

## Contratos e autorização

Tool, ToolSpec, ToolContext, Effect e ToolLimits ficam no core, sem dependência
cíclica. O registry recebe um SafeWorkspace já autorizado e monta a lista por
modo no código. PLAN e BUILD recebem apenas as capacidades deste M3.
Modo futuro desconhecido recebe lista vazia. O core mantém sua segunda barreira
contra ferramentas de mutação.

Cada ferramenta publica descrição e schema de entrada; o validador rejeita
campos desconhecidos, tipos incorretos, enum inválido, NUL e valores fora dos
intervalos. O mesmo schema publicado é usado na autorização e na execução.
ToolLimits estabelece 128 KiB de argumentos JSON e 256 KiB de resultado;
o core também confere esses limites na borda do executor. Há limites específicos
menores abaixo. Erros retornam códigos estáveis, sem conteúdo de arquivo,
stderr bruto, tokens ou caminhos privados. O loop preserva o código em
Tool failed: CODE, distinguindo-o de evidência bem-sucedida.

SafeWorkspace::new recebe uma raiz confiável absoluta existente.
SafeWorkspace::authorize confina uma pasta solicitada a uma lista explícita
de raízes confiáveis. A aplicação deve obter essas raízes do usuário, nunca
de arquivos do projeto ou saídas de ferramentas.

resolve_safe_path normaliza componentes e verifica cada ancestral existente
antes de aceitar níveis finais ausentes. Recusa fuga por ../, volumes externos,
UNC de entrada, ADS, NUL, dispositivos Windows, pontos/espaços finais,
nomes sensíveis, symlinks, reparse points/junctions e arquivos com múltiplos
hard links. Revalida a raiz e o handle aberto. Caminhos absolutos internos,
incluindo prefixo canônico de volume Windows, são aceitos; o prefixo é retirado
somente ao montar os argumentos do Git for Windows. A caixa ASCII é ignorada
nos limites de caminho Windows; variantes não ASCII ambíguas falham fechadas.

A lista sensível inclui a do legado e as restrições permanentes do repositório:
.env* (exceto .env.example), config.env, chatgpt-credentials.json, api-token,
*.log, *.db, diretórios privados e extensões de chave. Testes de recusa usam
nomes de caminhos, sem criar ou ler os arquivos proibidos. Nenhum home real,
banco antigo ou segredo foi usado.

A checagem por metadados não elimina uma corrida com outro processo local
malicioso que troque ancestrais entre verificação e abertura. Operações de
filesystem bloqueadas pelo sistema operacional também não podem ser
interrompidas instantaneamente com std::fs. O código verifica cancelamento
entre entradas, linhas, blocos de 32768 bytes e progresso do parser.

## Limites confirmados no código legado

Caracteres seguem unidades UTF-16, como String.length em JavaScript;
truncamento preserva caracteres Unicode completos. Bytes são limites distintos.

| Ferramenta/área | Limites e comportamento |
| --- | --- |
| read_file | Arquivo de 5 MiB; página de 10000 caracteres incluindo numeração. offset zero-based, numbered=true; linhas completas, nextOffset/continuation, LINE_TOO_LONG para linha que não cabe. Arquivo vazio tem zero linhas. |
| read_files | 1–8 arquivos; 1 MiB/arquivo e orçamento conservador de 4 MiB/lote; 10000 caracteres/arquivo, 30000 de conteúdo total; limit 1–2000 linhas. Erros individuais consomem allowance; BATCH_LIMIT explícito. |
| list_directory | offset até 20000, limit 1–500 (padrão 500), até 20000 entradas examinadas; diretórios primeiro na página; continuidade exige diretório inalterado. Links e hard links não aparecem. |
| glob | *, ** e ?; demais metacaracteres literais; até 500 resultados e 20000 entradas. Padrão até 1000 caracteres, relativo a path. |
| grep | Padrão 1–1000 caracteres; literal ou regex, caseSensitive, include, files_only, contexto 0–20; até 200 resultados, 60000 caracteres agregados, 300 por trecho, 1000000 bytes/arquivo e 20000000 bytes totais; 20000 entradas e 15 s. |
| .gitignore de busca | Apenas raiz; até 65536 bytes e 2000 regras. Comentários/escapes, CRLF/BOM, regras ancoradas, diretórios, negações e poda de pais. Não carrega ignores globais/aninhados. Chaves/parênteses literais, conforme legado. |
| Exclusões fixas de busca | node_modules, .git, dist, build e .next, inclusive com path apontando diretamente para um filho ignorado. Negação não libera links/segredos. |
| Varredura de inspeção | Até 2000 entradas, 500 arquivos e profundidade 12. Exclui node_modules, dist, .dist, coverage, .next, vendor, target e __pycache__. |
| project_inspect | Até 20 manifests de 256 KiB; 100 nomes de scripts/dependências, 200 caracteres/nome; 50 configs, 30 lockfiles, 50 testes e 100 caminhos; 15 s. Só nomes de scripts, nunca seus comandos. |
| test_discover | Convenções JS/TS, Python e Go; 100 testes, 20 manifests e 30 configs; scripts test/test:* e indícios de runners. 15 s; não executa nem enumera casos dinâmicos. |
| file_outline | JS/TS, JSX/TSX, MJS/CJS/MTS/CTS; 262144 bytes, 200 declarações, nomes de 200 caracteres, 30 s. Linhas 1-indexed, kind e exported; analysis=syntax-only. |
| symbol_search | Query literal aparada de 1–200 caracteres; 100 arquivos, 2097152 bytes totais, 262144/arquivo, 100 resultados, 30 s. Não resolve referências ou tipos. |
| Inspeção estruturada | Até 30000 caracteres JSON; reduz arrays com truncated=true, preservando estrutura. Vale para project_inspect, test_discover, AST e diff_summary. |
| environment_check | Diagnóstico passivo de OS/arquitetura/versão My-Agent e configuração declarada da sandbox. Não mostra valores/imagem nem comprova Docker/dependências. |
| Git | Até 1000 caminhos seguros atualmente rastreados; por processo, 15 s (operações auxiliares 5 s) e 10 MiB stdout+stderr durante leitura. |
| git_diff/git_show | Até 20000 caracteres; staged separado; show aceita hash hexadecimal de 4–40 caracteres, HEAD ou HEAD~N com 1–3 dígitos. Sem arquivos históricos já ausentes do índice. |
| git_log | 20 commits por padrão, máximo 100; hash/data/autor/subject; escopo dos caminhos seguros atuais. |
| git_blame | startLine 1–1000000, limit padrão 100/máximo 200, blob histórico até 1 MiB, até 20000 caracteres; nextLine e faixa real. Recusa ignoreRevsFile configurado. |
| diff_summary | Working tree/staged, até 1000 arquivos contabilizados, 100 retornados; totais de adições/remoções/binários, sem untracked ou renames. |
| todo_write | Substitui a lista completa em memória por sessão; pending/in_progress/completed, IDs opacos novos e contagens. O legado não tem limite próprio: aplicam-se as bordas gerais de entrada/saída. |

## Diferenças deliberadas e executor Git

O regex Rust tem complexidade limitada e não oferece backreferences/lookaround
de JavaScript; expressões não suportadas retornam INVALID_PATTERN. Executa em
worker bloqueante com cancelamento cooperativo e prazo, sem bloquear o runtime
assíncrono. Não há execução de padrões como código.

O outline usa tree-sitter em vez do compilador TypeScript. Mantém os campos
e as classes de declarações do legado; não promete equivalência entre árvores
para código malformado ou sintaxe ainda não suportada pelas gramáticas.
Construtores e accessors não entram na lista do legado e são omitidos.
O callback de progresso interrompe parsing cancelado; nenhum plugin/tsconfig é
carregado. A imagem de CI precisa de compilador C, conforme o ADR aceito.

environment_check informa o runtime Rust/My-Agent, substituindo Bun/Node.
todo_write tem efeito SessionState, disponível em PLAN, sem escrever projeto
ou banco. O dono da sessão deve chamar clear_session ao encerrar a sessão.

O executor Git não é público nem aceita comandos arbitrários. Resolve git/git.exe
por PATH absoluto, rejeitando executáveis dentro do workspace, e usa caminho
absoluto para evitar a busca por git.exe no diretório corrente do Windows.
Limpa o ambiente; desativa configuração global/sistema, pager, hooks, filtros,
fsmonitor, assinaturas, credenciais, diff externo, textconv, rede e submódulos.
Fixa git-dir/work-tree e usa pathspecs literais após --.
Não realiza commit, escrita Git ou operações de rede.

Antes da execução, valida até 20000 entradas de metadata local, recusa worktrees
com .git externo, links/hard links/alternates e configuração com includes.
A configuração local tem leitura limitada a 256 KiB; nomes de filtros são
consultados sem includes e neutralizados. Valida arquivos de controle dos
ancestrais dos caminhos. Não lê conteúdo de caminhos protegidos.
Blame também neutraliza mailmap externo.

git_status usa o Git apenas para os caminhos seguros rastreados e enumera
untracked pelo scanner protegido com ignore da raiz. Isso impede o Git de
percorrer diretórios privados. Esse scanner compartilha o teto de 20000 entradas
e não usa ignores aninhados; truncated indica enumeração incompleta.
Repos com config inclusiva são recusados, em vez de seguir fontes externas.
Essas restrições são mais conservadoras que o status irrestrito do legado.

Processos Git são supervisionados pelo sandbox existente, com kill/wait em
cancelamento, timeout ou excesso de saída. Não há fallback para shell,
execução de scripts do projeto ou Docker neste milestone.

## Dependências, testes e integração

| Dependência direta | Justificativa |
| --- | --- |
| regex | Matcher sem backtracking exponencial, com limites de compilação/DFA e cancelamento entre linhas. |
| ignore | Regras gitignore da raiz; construção em memória após leitura segura, sem configuração global. |
| globset | Glob include do grep, com separadores literais. O glob público simples mantém sua semântica literal própria. |
| tree-sitter | AST sintática e callback de progresso/cancelamento. |
| tree-sitter-javascript | Gramática JS/JSX; sem runtime JavaScript. |
| tree-sitter-typescript | Gramáticas TS/TSX, aceitas no M0 com compilador C. |
| tokio/tokio-util já no workspace | Worker bloqueante, timeout, processos supervisionados e CancellationToken. |
| serde_json/thiserror já no workspace | Schemas/saída estruturada e códigos de erro explícitos. |
| winapi-util/tempfile existentes | Reparse/hard links Windows por API segura e fixtures isoladas. |

Não foram adicionados SDKs de LLM, interpretadores, similar ou ferramentas web.
As licenças das dependências adicionadas estão cobertas pela política existente;
cargo-deny não foi executado.

Os testes em crates/tools/tests/read_only.rs portam comportamentos de
files/file-editing, search-filters/grep, inspection/io-cancellation,
git/git-inspection/git-scope e path.test.ts. Usam somente arquivos fictícios,
processos Git confiáveis e repositórios temporários. Não fazem commit no projeto,
não usam credenciais nem chamam modelos ou rede.

Windows cobre junctions, hard links, volumes, prefixos canônicos e caixa de
drive; Unix acrescenta symlinks por cfg(unix). Os testes Unix são executáveis
pelo CI Linux/macOS; não foram executados localmente no Windows. Não há teste
de criação de symlink de arquivo Windows que exija privilégio adicional.
A proteção Windows usa o mesmo atributo de reparse verificado com junctions.

Resultados finais e pendências do milestone ficam em [PARITY.md](PARITY.md).
A API de parsing foi conferida na
[documentação oficial do binding tree-sitter](https://docs.rs/tree-sitter/0.27.1/tree_sitter/struct.Parser.html);
a construção de ignore, em
[GitignoreBuilder](https://docs.rs/ignore/0.4.33/ignore/gitignore/struct.GitignoreBuilder.html).
