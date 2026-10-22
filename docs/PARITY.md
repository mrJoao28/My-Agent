# Paridade funcional com o My-Code-Agent

Inventário inicial em 19/10/2026, por inspeção estática do legado em
`C:\Users\joaof\My-Code-Agent`, mantido somente leitura. O código prevalece
sobre os documentos. Nenhum teste legado, provider real, banco do usuário ou
aplicativo legado foi executado para produzir esta matriz. A existência de um
teste indica uma referência a portar, não que o comportamento foi validado aqui.

## Como acompanhar

O status descreve a implementação **no My-Agent/Rust**, não no legado:

- **pendente**: comportamento identificado, ainda não portado e validado.
- **em andamento**: implementação ou validação incompleta.
- **feito**: comportamento portado com evidência de validação e gates aprovados.
- **descartado com motivo**: decisão explícita, com justificativa registrada.

O bootstrap inicial tinha um crate-base vazio, sem dependências externas. A
etapa de arquitetura acrescenta o workspace e a CLI mínima descritos ao final;
o agente ainda não está implementado. Não se exige reproduzir a arquitetura
Hono/React/Prisma, o protocolo interno ou os nomes de crates para obter paridade.
Devem ser preservados os comportamentos observáveis e os controles de segurança.
Não há descarte funcional decidido nesta etapa.

Os caminhos abreviados nas tabelas são relativos à raiz do legado:

| Prefixo | Diretório |
| --- | --- |
| `S/` | `packages/server/src/` |
| `C/` | `packages/cui/src/` |
| `M/` | `packages/myagent/src/` |
| `D/` | `packages/database/src/` |
| `P/` | `packages/database/prisma/` |
| `H/` | `packages/shared/src/` |
| `E/` | `evals/` |

“Sem teste dedicado identificado” registra uma lacuna encontrada na inspeção;
testes próximos podem fornecer cobertura indireta, sem comprová-la integralmente.

## Modos, geração e permissões

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| MODE-01 | PLAN/BUILD | PLAN recebe apenas ferramentas sem mutação do projeto; BUILD acrescenta escrita, shell e Git de alteração. A lista e a política no backend garantem a restrição. | `S/tools/index.ts`, `S/security/permissions.ts` | em andamento | `S/security/permissions.test.ts`, `S/security/docker-availability.test.ts` |
| MODE-02 | PLAN/BUILD | CUI inicia em BUILD, isola modo por sessão e restaura o último modo persistido; modelo selecionado é compartilhado. Modo em execução permanece distinguível da próxima mensagem. `exec` inicia em PLAN. | `C/providers/prompt-config/index.tsx`, `C/screens/session.tsx`, `C/components/session-shell.tsx`, `M/options.ts` | pendente | `C/dialogs/agents-dialog.test.tsx`, `C/components/session-shell.test.tsx`, `M/options.test.ts`, `M/exec.test.ts` |
| PROMPT-01 | Prompts | Builder por modo, workspace, ferramentas efetivamente disponíveis e sandbox; seis blocos de instruções; perfis compacto/completo. Não anunciar capacidades indisponíveis nem tratar texto externo como permissão. | `S/prompts/index.ts`, `S/prompts/system.ts`, `S/prompts/coding.ts`, `S/prompts/editing.ts`, `S/prompts/tool-rules.ts`, `S/services/context-plan.ts` | pendente | `S/prompts/prompts.test.ts`, `S/services/context-plan.test.ts`, `S/services/message-format.test.ts` |
| PERM-01 | Aprovações | Y permite uma vez, A memoriza por sessão/ferramenta/workspace/escopo, N nega. Enter não aprova; Esc/Ctrl+C interrompem. Prévia rolável de diff, caminho, comando ou URL. | `S/security/permissions.ts`, `S/security/approval-scope.ts`, `S/security/tool-policy.ts`, `C/components/approval-panel.tsx`, `C/components/approval-preview.ts` | em andamento | `S/security/permissions.test.ts`, `S/security/approval-scope.test.ts`, `C/components/approval-panel.test.tsx` |
| PERM-02 | Aprovações | Timeout de 120 s, consumo único, isolamento e limpeza ao cancelar/desconectar; até 256 escopos. A é proibido para delete_file, git_commit, git_init, git_remote_add e git_push. Ferramentas desconhecidas são negadas. | `S/security/permissions.ts`, `H/schemas.ts` | feito | `S/security/permissions.test.ts`, `S/security/approval-scope.test.ts`, `H/schemas.test.ts` |
| PERM-03 | Automação | `none` autoriza leitura local; `reads` acrescenta web; `all` autoriza ações. Push exige opção separada e não autoaprova main/master. Comandos continuam dependentes de Docker e do modo. | `S/security/permissions.ts`, `M/options.ts`, `M/exec.ts` | pendente | `M/exec.test.ts`, `S/tools/git-workflow.test.ts`, `S/tools/test-tools.test.ts` |
| GEN-01 | Streaming | Texto, reasoning, chamadas/resultados associados por ID, avisos, uso e estados terminais estruturados; contratos validados; UTF-8/CRLF fragmentados e EOF sem done não equivalem a sucesso. | `H/schemas.ts`, `S/services/stream-service.ts`, `C/lib/chat-events.ts`, `C/hooks/use-chat.ts`, `M/exec.ts` | em andamento | `H/schemas.test.ts`, `S/services/stream-service.test.ts`, `C/lib/chat-events.test.ts`, `C/hooks/use-chat.test.tsx`, `M/exec.test.ts` |
| GEN-02 | Ciclo da geração | Uma geração ativa por sessão; lock antes de awaits; diferenciar erro, conclusão e interrupção; persistir parciais, propagar cancelamento e aguardar cleanup no encerramento. Falha no SSE terminal não duplica gravação. | `S/services/chat-service.ts`, `S/services/stream-service.ts`, `S/index.ts` | em andamento | `S/services/chat-recovery.test.ts`, `S/services/stream-service.test.ts`, `S/launcher.test.ts` |
| GEN-03 | Limites de tools | Orçamento padrão 40 chamadas, configurável até 200, verificado antes do executor; aviso perto do limite; timeout global de 5 min. Argumentos equivalentes são canonizados; resultado visível evita repetição; repetição persistente interrompe. | `S/services/tool-service.ts`, `S/services/stream-service.ts` | feito | `S/services/tool-service.test.ts`, `S/services/tool-guard.test.ts`, `S/services/stream-service.test.ts` |
| GEN-04 | Evidência no contexto | Releituras voltam a ser permitidas quando evidência é removida/abreviada; edição invalida leitura anterior; nota de undo invalida evidências sem ampliar permissões ou orçamento. | `S/services/tool-service.ts`, `S/services/session-context.ts` | feito | `S/services/tool-guard.test.ts`, `S/services/stream-service.test.ts`, `S/services/session-context.test.ts` |
| TASK-01 | Tarefas | `todo_write` mantém itens e status em memória, separados por sessão, com limpeza; não é plano de execução durável nem coordenação de múltiplos agentes. | `S/tools/todo-write.ts`, `S/services/session-lifecycle.ts` | em andamento | `S/tools/files.test.ts`, `S/services/session-lifecycle.test.ts` |

## Ferramentas de arquivos, inspeção e processos

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| READ-01 | Leitura | `read_file`: até 5 MiB, páginas de linhas inteiras, offset zero-based, numeração opcional, saída até 10 mil caracteres, nextOffset/continuation; erro LINE_TOO_LONG sem saltar o meio da página. | `S/tools/read-file.ts` | feito | `S/tools/files.test.ts`, `S/tools/file-editing.test.ts`, `S/tools/io-cancellation.test.ts`, `S/services/session-context.test.ts` |
| READ-02 | Leitura em lote | `read_files`: 1–8 arquivos, até 1 MiB cada/4 MiB no lote/30 mil caracteres, seleção de linhas e sucesso parcial; recusa binários e caminhos protegidos. | `S/tools/read-files.ts` | feito | `S/tools/inspection.test.ts` |
| READ-03 | Diretórios | `list_directory`: paginação, limites e cancelamento; omissão de entradas sensíveis e links; continuidade pressupõe diretório inalterado. | `S/tools/list-directory.ts` | feito | `S/tools/files.test.ts`, `S/tools/io-cancellation.test.ts` |
| FIND-01 | Glob | `glob`: padrões relativos, zero/múltiplos níveis e exclusões seguras; usa regras do .gitignore da raiz, inclusive em buscas por subpasta. | `S/tools/glob.ts`, `S/lib/search-ignore.ts` | feito | `S/tools/files.test.ts`, `S/tools/search-filters.test.ts` |
| FIND-02 | Grep | `grep`: literal/regex, include, contexto de 0–20 linhas, files_only; limites de bytes/resultados/saída/tempo; regex em worker cancelável. | `S/tools/grep.ts`, `S/lib/grep-worker.ts`, `S/lib/search-ignore.ts` | feito | `S/tools/grep.test.ts`, `S/tools/search-filters.test.ts`, `S/tools/files.test.ts` |
| FIND-03 | Ignores | Leitura limitada do .gitignore raiz, negações e poda de pais excluídos; não consulta ignores aninhados/globais nem índice Git. Negação não libera segredos, links ou dependências fixamente excluídas. | `S/lib/search-ignore.ts` | feito | `S/tools/search-filters.test.ts` |
| INSPECT-01 | Projeto | `project_inspect`: manifests, scripts/dependências, configs e testes por inspeção estática com limites; não executa arquivos de configuração. | `S/tools/project-inspect.ts`, `S/lib/project-inspection.ts` | feito | `S/tools/inspection.test.ts` |
| INSPECT-02 | Código | `file_outline`: declarações e linhas JS/TS/JSX/TSX por AST em worker. `symbol_search`: busca literal de declarações; não equivale a referências nem resolução de tipos. | `S/tools/file-outline.ts`, `S/tools/symbol-search.ts`, `S/lib/source-analysis.ts`, `S/lib/source-worker.ts` | feito | `S/tools/inspection.test.ts` |
| INSPECT-03 | Ambiente | `environment_check`: diagnóstico passivo sem processos ou valores de ambiente; não comprova daemon/imagem/ferramentas da sandbox. | `S/tools/environment-check.ts` | feito | `S/tools/inspection.test.ts` |
| WRITE-01 | Criação | `write_file`: escrita atômica, criação de pais, proteção contra sobrescrita e backup; modo existente preservado, novos arquivos 0644 em POSIX. | `S/tools/write-file.ts` | feito | `S/tools/files.test.ts`, `S/tools/file-editing.test.ts` |
| WRITE-02 | Edição | `edit_file`: substituição única, contagem de ocorrências ambíguas/ausentes; compara LF/CRLF e usa EOL dominante no trecho inserido, preservando bytes restantes e modo. | `S/tools/edit-file.ts` | feito | `S/tools/files.test.ts`, `S/tools/file-editing.test.ts` |
| WRITE-03 | Árvores | `move_file` e `delete_file`: mover/excluir com validação de descendentes, destino existente e raiz protegida. Exclusão sempre exige aprovação individual. | `S/tools/move-file.ts`, `S/tools/delete-file.ts`, `S/utils/path.ts` | feito | `S/tools/files.test.ts`, `S/security/checkpoints.test.ts` |
| WRITE-04 | Patch | `apply_patch`: diff unificado, checkOnly e validação de todos os caminhos antes de alterar; recusa binários, links, rename/copy e caminhos com espaços. | `S/tools/apply-patch.ts` | feito | `S/tools/git.test.ts`, `S/security/checkpoints.test.ts` |
| TEST-01 | Descoberta | `test_discover`: convenções JS/TS, Python e Go, scripts e indícios de runners, com inspeção limitada; não enumera casos gerados em runtime. | `S/tools/test-discover.ts`, `S/lib/project-inspection.ts` | feito | `S/tools/inspection.test.ts` |
| TEST-02 | Execução | `test_run`: Bun, Node, Vitest, Jest e Pytest, runners fixos com argumentos escapados; cópia descartável em /tmp, dependências referenciadas somente leitura; timeout/cancelamento/saída limitada; não infere quantidade de testes pelo exit code. | `S/tools/test-run.ts` | feito | `S/tools/test-tools.test.ts`, `S/security/sandbox.integration.test.ts` |
| TEST-03 | Tipos | `type_check`: diagnóstico TypeScript, caminhos Windows e linha/coluna; tsc local antes do global, somente na sandbox; disponível também em PLAN com Docker. | `S/tools/type-check.ts` | feito | `S/services/message-format.test.ts`, `S/security/permissions.test.ts`, `S/security/docker-availability.test.ts` |
| SHELL-01 | Shell | `bash`: exclusivo de BUILD e Docker; timeout/saída/cancelamento; projeto somente leitura no container e saídas descartáveis em /tmp. Não há fallback de execução no host. | `S/tools/bash.ts`, `S/security/sandbox.ts` | feito | `S/security/permissions.test.ts`, `S/security/sandbox.test.ts`, `S/security/sandbox.integration.test.ts` |

## Git, checkpoints, sandbox e rede

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| GIT-01 | Inspeção | `git_status`, `git_diff`, `git_log`, `git_show`, `git_blame`, `diff_summary`: escopo seguro, staged separado, histórico e atribuição de linhas; não expor caminhos protegidos. | `S/tools/git-status.ts`, `S/tools/git-diff.ts`, `S/tools/git-log.ts`, `S/tools/git-show.ts`, `S/tools/git-blame.ts`, `S/tools/diff-summary.ts`, `S/lib/git-inspection.ts` | feito | `S/tools/git.test.ts`, `S/tools/git-inspection.test.ts`, `S/utils/git-scope.test.ts` |
| GIT-02 | Commit | `git_commit`: arquivos explícitos, prévia para aprovação individual e preservação de staged fora do escopo; identidade Git existente, sem inventar configuração local. | `S/tools/git-commit.ts`, `S/utils/git.ts` | feito | `S/tools/git.test.ts`, `S/tools/git-workflow.test.ts`, `S/security/approval-scope.test.ts` |
| GIT-03 | Bootstrap/branches | `git_init` em main apenas fora de repositórios, `git_switch` cria/troca sem descarte/force, `git_remote_add` HTTPS validado sem substituir URL existente. | `S/tools/git-workflow.ts` | em andamento | `S/tools/git-workflow.test.ts` |
| GIT-04 | Push/sincronização | `git_push` só da branch atual, URL/branch vinculadas à aprovação, sem force/delete/tags; main/master pedem confirmação adicional Y e M. Após push, parar até liberação do usuário. `git_sync` usa fetch sem tags e merge ff-only da principal, com workspace/index limpos. | `S/tools/git-workflow.ts`, `S/security/tool-policy.ts`, `S/services/stream-service.ts`, `C/components/approval-panel.tsx` | em andamento | `S/tools/git-workflow.test.ts`, `S/services/stream-service.test.ts`, `C/components/approval-panel.test.tsx` |
| GIT-05 | Isolamento Git | Worktree explícita, ambiente reduzido; hooks/filtros/assinatura/pager/fsmonitor/diffs externos desativados; recusa metadata externa, links e alternates. Escritas Git serializadas. | `S/utils/git.ts`, `S/security/tool-policy.ts` | em andamento | `S/utils/git-scope.test.ts`, `S/tools/git.test.ts`, `S/tools/git-inspection.test.ts`, `S/tools/git-workflow.test.ts` |
| UNDO-01 | Checkpoints | Checkpoint privado global por sessão antes de mutações de arquivos em BUILD; preserva bytes, EOL e modos; limites por arquivo/passo/store; inclui undo após falhas parciais. Não é commit Git nem snapshot de todo efeito externo. | `S/security/checkpoints.ts`, `S/security/tool-policy.ts` | feito | `S/security/checkpoints.test.ts` |
| UNDO-02 | /undo | Desfaz último passo sem sobrescrever alterações posteriores; recusa conflito/geração ativa, valida workspace/lease, persiste nota para invalidar evidência anterior. Não cria commits. | `S/services/chat-service.ts`, `C/hooks/use-chat.ts`, `C/components/command-menu/commands.tsx` | em andamento | `S/security/checkpoints.test.ts`, `S/services/chat-recovery.test.ts`, `C/components/approval-undo.test.tsx` |
| BOX-01 | Disponibilidade | Docker opcional para abrir o app; sem daemon não registrar bash/type_check/test_run. Verificação limitada, imagem padrão Bun, preparação/pull sob demanda e progresso, depois pull=never. | `S/security/sandbox.ts`, `S/index.ts`, `S/tools/index.ts`, `M/doctor.ts` | em andamento | `S/security/docker-availability.test.ts`, `S/security/sandbox.test.ts`, `S/launcher.test.ts` |
| BOX-02 | Container | Rede desativada, rootfs e bind mount somente leitura, usuário não root, capabilities removidas, no-new-privileges, tmpfs privado e limites de CPU/memória/processos. Cancelamento/timeout encerram processos/container. | `S/security/sandbox.ts` | feito | `S/security/sandbox.test.ts`, `S/security/sandbox.integration.test.ts` |
| BOX-03 | Snapshot | Cópia filtrada, sem metadata Git, segredos, links e builds; limites por arquivo/bytes/entradas/profundidade. Dependências omitidas por padrão; opt-in filtrado para node_modules/.venv e tratamento restrito de .bin. | `S/security/snapshot.ts` | feito | `S/security/snapshot.test.ts`, `S/security/sandbox.integration.test.ts` |
| BOX-04 | Cache de snapshot | Reuso por sessão/workspace e invalidação por inventário/metadados/configuração, leases serializados, expiração ociosa e limite de entradas. Não é hash de conteúdo nem cache persistente entre processos. | `S/security/snapshot.ts` | pendente | `S/security/snapshot.test.ts` |
| WEB-01 | Busca | `web_search` via Tavily: aprovação, snippets estruturados, schema, deadline, limite durante leitura e cancelamento; falhas sanitizadas. | `S/tools/web-search.ts`, `S/lib/tool-io.ts` | feito | `S/tools/web.test.ts`, `S/lib/tool-io.test.ts` |
| WEB-02 | Fetch | `web_fetch`: aprovação por origem completa, redirects não podem ampliar grant; extração de texto HTML, remoção de conteúdo ativo, entidades, limite de bytes/texto e cancelamento. | `S/tools/web-fetch.ts`, `S/security/approval-scope.ts` | feito | `S/tools/web.test.ts`, `S/security/approval-scope.test.ts` |
| WEB-03 | SSRF | Bloqueio de destinos internos, IPs mapeados/IPv6 e DNS misto; conexão presa ao IP validado, preservando Host/TLS; sem segunda resolução permissiva. | `S/tools/web-fetch.ts` | feito | `S/tools/web-ssrf.test.ts`, `S/tools/web.test.ts` |

## Persistência, sessões, contexto e instruções

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| DB-01 | SQLite | Banco local/libsql, WAL, busy_timeout e foreign_keys; abertura lazy, transações e fechamento explícito; não depende de PostgreSQL/DATABASE_URL em runtime. | `D/client.ts`, `D/index.ts`, `P/schema.prisma` | feito | `D/database.integration.test.ts`, `D/migrations.test.ts` |
| DB-02 | Migrations | Runner SQL embutido, baseline e evoluções transacionais/idempotentes; enums/JSON/índices/cascatas. Prisma CLI não é requisito de runtime. | `D/migrations.ts`, `D/migrate.ts`, `P/migrations/20261016000000_sqlite_baseline/migration.sql`, `P/migrations/20261016010000_workspace_sessions/migration.sql`, `P/migrations/20261018000000_durable_generations/migration.sql` | feito | `D/migrations.test.ts`, `D/database.integration.test.ts` |
| SESSION-01 | Workspace | Raiz Git/caminho canônico, caixa Windows, espaços/subpastas e recusa de junctions; título com caminho; criar ou selecionar sessão mais recente por workspace. | `S/utils/workspace.ts`, `S/services/workspace-sessions.ts`, `S/routes/sessions.ts` | em andamento | `S/utils/workspace.test.ts`, `S/services/workspace-sessions.test.ts`, `S/routes/sessions.test.ts` |
| SESSION-02 | Histórico | Listagem por atividade, criação com mensagem inicial opcional, reabertura com mensagens/parts/status, contexto por sessão e validação de usuário local/workspace. Abrir/enviar/retomar renova atividade. | `S/routes/sessions.ts`, `S/services/message-service.ts`, `S/services/session-lifecycle.ts` | em andamento | `S/routes/sessions.test.ts`, `S/services/session-lifecycle.test.ts`, `D/database.integration.test.ts` |
| SESSION-03 | Leases | Exclusão entre processos no SQLite por sessão, owner aleatório, heartbeat de 5 s e expiração de 30 s; perda encerra execução, fechamento libera lease. Locks em memória continuam distintos. | `S/services/session-lease.ts`, `S/index.ts` | em andamento | `S/services/workspace-sessions.test.ts`, `S/launcher.test.ts`, `D/database.integration.test.ts` |
| SESSION-04 | Retenção | Padrão 90 dias, configurável e desativável com zero; aviso e carência mínima de 24 h; abrir cancela aviso. Limpeza horária não sobreposta, preserva leases ativos/gerações e remove artefatos. | `S/services/session-lifecycle.ts` | em andamento | `S/services/session-lifecycle.test.ts`, `S/services/workspace-sessions.test.ts`, `D/database.integration.test.ts` |
| RECOVER-01 | Journal durável | ASSISTANT começa IN_PROGRESS; journal serializa chamadas/resultados e grava antes de efeitos e antes de devolver resultados ao modelo. Órfãs viram INTERRUPTED sem afetar leases vivos de outro processo. | `S/services/generation-journal.ts`, `S/services/stream-service.ts` | feito | `S/services/generation-journal.test.ts`, `S/services/stream-service.test.ts` |
| RECOVER-02 | Retomada | Retomar USER pendente ou ASSISTANT interrompido com confirmação explícita na CUI; preservar generationId e resultados anteriores; bloquear replay de ações já iniciadas, inclusive efeito desconhecido sem resultado. | `S/services/chat-service.ts`, `S/services/generation-journal.ts`, `C/hooks/use-chat.ts`, `C/components/resume-panel.tsx` | em andamento | `S/services/chat-recovery.test.ts`, `S/services/generation-journal.test.ts`, `S/services/stream-service.test.ts`, `C/components/resume-panel.test.tsx` |
| CTX-01 | Histórico estruturado | Reconstruir pares chamada/resultado e texto; chamadas incompletas viram avisos. Omitir reasoning de novos turnos, preservar metadados necessários durante geração viva. Histórico completo continua persistido. | `S/services/message-format.ts`, `S/services/session-context.ts`, `S/services/context-summary.ts` | feito | `S/services/message-format.test.ts`, `S/services/session-context.test.ts` |
| CTX-02 | Janela do modelo | Resolver override, catálogo exato, snapshot datado, hint do provider e fallback local/cloud com origem/estimativa explícitas; preferências persistidas, validação de valores e catálogo sem corrupção silenciosa. | `S/lib/context-window.ts`, `S/lib/model-config.ts`, `H/models.ts` | em andamento | `S/lib/model-context.test.ts`, `S/lib/private-config.test.ts`, `S/services/context-plan.test.ts`, `H/schemas.test.ts` |
| CTX-03 | Orçamento | Reservar sistema, schemas, opções do provider e saída; perfil essencial para janelas pequenas; validar e persistir snapshot antes de cada etapa e entregar ao modelo o snapshot retornado. Falha de persistência impede inferência. | `S/services/context-plan.ts`, `S/services/session-context.ts`, `S/services/stream-service.ts` | feito | `S/services/context-plan.test.ts`, `S/services/context-plan.integration.test.ts`, `S/services/session-context.test.ts`, `S/services/stream-service.test.ts` |
| CTX-04 | Compactação | Ao atingir limiar de 70%, resumir evidência antiga e preservar turnos recentes; manter pares completos, abreviar só com indicação, read_file somente no fim de linhas; usuário atual grande demais é rejeitado, sem corte silencioso. | `S/services/session-context.ts`, `S/services/context-summary.ts` | feito | `S/services/session-context.test.ts`, `S/services/context-plan.integration.test.ts` |
| USAGE-01 | Tokens consumidos | Acumular uso válido informado em cada etapa uma vez, persistir por mensagem inclusive em erro/interrupção e restaurar na sessão. Ausência de uso não vira estimativa nem zero fictício. | `S/services/stream-service.ts`, `C/hooks/use-chat.ts`, `C/lib/token-usage.ts`, `C/components/token-usage-bar.tsx` | em andamento | `S/services/stream-service.test.ts`, `C/lib/token-usage.test.ts`, `C/components/token-usage-bar.test.tsx`, `C/hooks/use-chat.test.tsx` |
| AGENTS-01 | Instruções de projeto | AGENTS.md raiz a cada geração, subpastas conforme arquivos lidos/buscados/editados; precedência por escopo, irmãos isolados, cache imutável por geração e entrega única. | `S/lib/project-instructions.ts`, `S/services/context-plan.ts`, `S/tools/index.ts` | em andamento | `S/lib/project-instructions.test.ts`, `S/services/stream-service.test.ts` |
| AGENTS-02 | Segurança das instruções | Limite total de 16 KiB incluindo envelope JSON/Unicode, 128 candidatos e profundidade 32; recusar links/binários/caminhos sensíveis; I/O somente após autorização. Instruções nunca ampliam permissões nem geram documentação automaticamente. | `S/lib/project-instructions.ts`, `S/security/tool-policy.ts` | feito | `S/lib/project-instructions.test.ts`, `S/prompts/prompts.test.ts` |

## Providers, modelos e configuração

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| AI-01 | Anthropic | Resolver modelo configurado, chave por modelo e opções de thinking do catálogo. | `S/lib/providers/anthropic.ts`, `S/lib/providers/resolver.ts` | feito | `S/lib/config.test.ts`, `S/services/context-plan.test.ts`; sem teste dedicado do adapter Anthropic identificado |
| AI-02 | OpenAI | Adapter OpenAI e opções de reasoning por modelo; resolução explícita, falha por modelo/chave ausente. | `S/lib/providers/openai.ts`, `S/lib/providers/resolver.ts` | feito | `S/lib/config.test.ts`, `S/services/stream-service.test.ts`, `M/exec.test.ts` (provider simulado); sem chamada real validada |
| AI-03 | Google | Adapter Google com thinkingBudget por modelo, catálogo e chave por modelo. | `S/lib/providers/google.ts`, `S/lib/providers/resolver.ts` | feito | `S/lib/config.test.ts`, `S/services/context-plan.test.ts`; sem teste dedicado do adapter Google identificado |
| AI-04 | Ollama | Endpoint compatível com OpenAI Chat Completions, URL/chave opcionais, modelos locais/customizados e janela configurável; não exige chave cloud. | `S/lib/providers/ollama.ts`, `S/lib/providers/resolver.ts` | feito | `S/lib/config.test.ts`, `S/lib/model-context.test.ts`; sem inferência Ollama real validada |
| AI-05 | Plano ChatGPT | Login habilitado por padrão, desativação explícita; OAuth com PKCE, callback loopback, state/nonce, identidade assinada e registro de cliente; múltiplas contas, ativação/logout/revogação. | `S/lib/chatgpt.ts`, `S/lib/chatgpt-store.ts`, `S/routes/chatgpt.ts`, `C/dialogs/chatgpt-dialog.tsx` | em andamento | Implementação Rust e testes OAuth/JWKS fictícios em `crates/providers/src/chatgpt/tests.rs`; detalhes e limites na integração de 21/10/2026 abaixo. `S/lib/chatgpt.test.ts` é referência histórica. |
| AI-06 | Credenciais ChatGPT | Armazenamento privado/atômico, refresh serializado e cancelável; troca de conta interrompe geração; falha de grant invalida apenas conta afetada; não fallback para API key nem retry automático de inferência. Nenhum arquivo real de credenciais foi aberto. | `S/lib/chatgpt.ts`, `S/lib/chatgpt-store.ts`, `S/lib/providers/chatgpt.ts` | pendente | `S/lib/chatgpt.test.ts`, `S/lib/providers/chatgpt.test.ts` |
| AI-07 | Stream ChatGPT | Modelos descobertos por conta com IDs chatgpt:, instruções developer e tools no namespace do provider; validação de conclusão SSE, UTF-8 fragmentado e respostas JSON/HTML/incompletas/oversized com erros sanitizados. | `S/lib/providers/chatgpt.ts`, `S/lib/providers/chatgpt-stream.ts`, `S/lib/chatgpt.ts` | pendente | `S/lib/providers/chatgpt.test.ts`, `S/lib/chatgpt.test.ts` |
| MODEL-01 | Registry | Catálogo compartilhado de IDs/provider/pricing/contexto, default claude-opus-4-6; adição de modelos customizados, detecção de configuração e rejeição de duplicatas/IDs ChatGPT manuais. | `H/models.ts`, `S/lib/model-registry.ts`, `S/lib/model-config.ts`, `S/routes/models.ts` | em andamento | `H/schemas.test.ts`, `S/lib/config.test.ts`, `S/lib/model-context.test.ts`, `S/lib/private-config.test.ts`, `C/dialogs/models-dialog.test.tsx` |
| CONFIG-01 | Configuração global | Diretório global estável, override de home e alias legado; ambiente prevalece sobre config; chave com nome determinístico por modelo, atualização atômica preservando outros valores; JSON inválido gera erro e não é apagado. | `H/home.ts`, `S/lib/env.ts`, `S/lib/private-config.ts`, `S/lib/model-config.ts` | em andamento | `S/lib/config.test.ts`, `S/lib/private-config.test.ts`, `S/lib/chatgpt.test.ts` |

## CLI, TUI e API local

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| CLI-01 | Entrada | `myagent` retoma última sessão do workspace ou cria; `--new` força nova; launcher integra backend em porta livre e CUI com token efêmero; SIGINT/SIGTERM e cleanup. Bun/Node são detalhes do legado a substituir. | `M/cli.ts`, `packages/myagent/bin/myagent.js`, `S/index.ts`, `C/index.tsx` | em andamento | `M/launcher.test.ts`, `M/options.test.ts`, `S/launcher.test.ts`, `scripts/workspace-lock.test.ts` |
| CLI-02 | Resume | `resume` escolhe sessão da pasta, `resume --all` permite outras pastas e troca workspace; seleção/aborto e validação. | `M/resume.ts`, `M/options.ts`, `S/services/workspace-sessions.ts` | pendente | `M/options.test.ts`, `S/services/workspace-sessions.test.ts`; sem teste dedicado da seleção interativa em resume.ts identificado |
| CLI-03 | Doctor | Diagnóstico de runtime, diretório gravável, SQLite/migrations, Docker/imagem, ferramentas detectadas versus imagem e configuração de modelos sem valores de chaves. O doctor legado pode escrever/migrar: não foi executado nesta inspeção. | `M/doctor.ts`, `M/doctor-tools.ts` | em andamento | `crates/cli/tests/cli.rs` cobre o comando offline/JSON. Comparação integral das ferramentas na imagem permanece pendente. Referências: `M/doctor-tools.test.ts`, `S/security/docker-availability.test.ts`. |
| CLI-04 | Headless | `exec` cria sessão nova, prompt até 9.000 caracteres, mode/model, JSONL opcional, permissões explícitas e allow-push; exits 0 sucesso, 1 falha, 3 negação e 130 cancelamento. | `M/options.ts`, `M/exec.ts` | em andamento | Rust integrado ao backend da TUI; testes de PLAN, aprovação/negação, JSONL e limite em `crates/cli/tests/cli.rs`. Referência: `M/exec.test.ts`. |
| CLI-05 | Ajuda/versão | `--help`/`-h`, `--version`/`-v`, rejeição de argumentos ambíguos/duplicados/inválidos; ajuda não abre banco nem providers. | `M/options.ts`, `M/cli.ts` | em andamento | `M/options.test.ts`, `M/launcher.test.ts`, `M/exec.test.ts`; início do port em `crates/cli/tests/cli.rs`, ainda sem os demais comandos |
| UI-01 | Telas | Home, nova sessão e sessão/histórico; shell compartilhado, título/caminho, mensagens usuário/assistente/erro, reasoning/ferramentas, avisos e interrupção visível. | `C/screens/home.tsx`, `C/screens/new-session.tsx`, `C/screens/session.tsx`, `C/components/messages/bot-message.tsx`, `C/components/session-shell.tsx` | feito | `C/screens/session-render.test.tsx`, `C/components/session-shell.test.tsx`, `C/hooks/use-chat.test.tsx`, `scripts/cui-text.test.ts` |
| UI-02 | Entrada | Texto multilinha, Ctrl+J para quebra, Enter para envio, Tab para modo, autocomplete de menções @ a arquivos/diretórios e comandos /; limite de mensagem e bloqueio durante aprovação/retomada. | `C/components/input-bar.tsx`, `C/components/command-menu/use-command-menu.ts` | em andamento | `C/components/session-shell.test.tsx`, `C/dialogs/agents-dialog.test.tsx`, `C/components/approval-panel.test.tsx`; sem suíte dedicada completa de menções identificada |
| UI-03 | Menu de comandos | `/undo`, `/sessions`, `/theme`, `/models`, `/agents`, `/exit`, `/new`; filtro, seleção e navegação. `/agents` alterna modos, não agentes autônomos. | `C/components/command-menu/commands.tsx`, `C/components/command-menu/filter-commands.ts`, `C/components/command-menu/index.tsx` | feito | `C/dialogs/agents-dialog.test.tsx`, `C/dialogs/models-dialog.test.tsx`, `C/components/approval-undo.test.tsx`; sem teste dedicado de todo o menu identificado |
| UI-04 | Diálogos/foco | Sessões, modelos, modos/agentes, temas e contas ChatGPT; busca/lista, camadas de teclado, Enter/Esc/Tab/Ctrl+C sem acionar input de fundo; toast e erros de rota. | `C/dialogs/index.tsx`, `C/dialogs/sessions-dialog.tsx`, `C/dialogs/models-dialog.tsx`, `C/dialogs/agents-dialog.tsx`, `C/dialogs/theme-dialog.tsx`, `C/dialogs/chatgpt-dialog.tsx`, `C/providers/keyboard-layer/index.tsx`, `C/providers/dialog/index.tsx`, `C/components/route-error.tsx` | em andamento | `C/dialogs/agents-dialog.test.tsx`, `C/dialogs/models-dialog.test.tsx`, `C/components/approval-panel.test.tsx`, `C/components/resume-panel.test.tsx`; cobertura não é completa para todos os diálogos |
| UI-05 | Temas | Paletas claras/escuras, Nightfox padrão, seleção e persistência global em preferences.json; falha de preferência não impede interface. | `C/theme.ts`, `C/providers/theme/index.tsx`, `C/dialogs/theme-dialog.tsx` | feito | `C/components/token-usage-bar.test.tsx`, `C/components/session-shell.test.tsx`; sem teste dedicado de persistência do tema identificado |
| UI-06 | Uso/status | Barra de tokens acumulados com cores do tema e formatos k/M, escala visual de 1 milhão sem limitar consumo/contexto; status, duração, modo em execução e aviso de comandos indisponíveis. | `C/components/token-usage-bar.tsx`, `C/lib/token-usage.ts`, `C/components/status-bar.tsx`, `C/components/session-shell.tsx` | em andamento | `C/lib/token-usage.test.ts`, `C/components/token-usage-bar.test.tsx`, `C/components/session-shell.test.tsx` |
| UI-07 | Navegador | Abrir URL de autenticação no navegador por plataforma com validação e diagnóstico de falha, sem shell arbitrário. | `C/lib/open-browser.ts`, `C/dialogs/chatgpt-dialog.tsx` | em andamento | Rust: `crates/tui/src/links.rs`, testes de foco/renderização e smoke Linux/PTY com helpers fictícios; browser gráfico/clipboard real Windows/macOS ainda não verificados. `C/lib/open-browser.test.ts` é referência histórica. |
| API-01 | API local | Bind loopback, token privado/efêmero, negar Origin de navegador e Host externo/rebinding, limite de corpo 1 MiB e validação de workspace/proprietário. Não é serviço multiusuário. | `S/index.ts`, `S/security/auth.ts`, `S/security/workspaces.ts`, `H/local-auth.ts`, `C/lib/api-client.ts` | pendente | `S/security/auth.test.ts`, `S/routes/sessions.test.ts`, `S/launcher.test.ts`, `S/lib/chatgpt.test.ts` |
| API-02 | Contratos | Rotas locais de sessões, chat/retomada/undo, aprovações, modelos/chaves e ChatGPT; erros/validação HTTP e SSE. Preservar comportamento sem obrigar a mesma separação cliente/servidor em Rust. | `S/routes/chat.ts`, `S/routes/permissions.ts`, `S/routes/sessions.ts`, `S/routes/models.ts`, `S/routes/chatgpt.ts`, `C/lib/http-errors.ts` | pendente | `S/routes/sessions.test.ts`, `S/services/chat-recovery.test.ts`, `M/exec.test.ts`, `S/lib/chatgpt.test.ts`, `C/hooks/use-chat.test.tsx` |

## Segurança transversal, avaliações e distribuição

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| SAFE-01 | Caminhos | Confinamento ao workspace; traversal, volumes/UNC, ADS, dispositivos Windows, pontos/espaços finais, ancestrais inexistentes, links/junctions e hard links recusados; raiz e descendentes protegidos. Não garante ausência de corrida contra outro processo local malicioso. | `S/utils/path.ts`, `S/security/workspaces.ts` | feito | `S/utils/path.test.ts`, `S/utils/workspace.test.ts`, `S/tools/files.test.ts`, `S/tools/inspection.test.ts` |
| SAFE-02 | I/O limitado | Limites durante leitura, UTF-8 preservado entre chunks, cancelamento de body/processos/workers, deadline e liberação de timers/listeners/locks. | `S/lib/tool-io.ts`, `S/lib/bounded-json.ts`, `S/security/sandbox.ts`, `S/lib/source-analysis.ts` | pendente | `S/lib/tool-io.test.ts`, `S/tools/io-cancellation.test.ts`, `S/tools/inspection.test.ts`, `S/tools/web.test.ts`, `S/security/sandbox.test.ts` |
| SAFE-03 | Privacidade | Não expor valores de credenciais em erros/diagnóstico/resultados; escrita privada/atômica das configurações; arquivos sensíveis recusados nas ferramentas, busca, Git e snapshots. | `S/lib/private-config.ts`, `S/lib/logger.ts`, `S/utils/path.ts`, `S/security/auth.ts`, `S/security/snapshot.ts` | pendente | `S/lib/private-config.test.ts`, `S/security/auth.test.ts`, `S/security/snapshot.test.ts`, `S/tools/files.test.ts`, `S/tools/git.test.ts` |
| EVAL-01 | Tarefas | Nove fixtures verificáveis: leitura, corrigir operador, função com teste, refatorar CRLF, script executável, busca em 240 módulos, AGENTS.md, leitura de 300 linhas e 30 leituras sucessivas. Inventário completo e diff esperado, sem executar código gerado no host. | `E/tasks.ts` | pendente | `E/runner.test.ts` |
| EVAL-02 | Runner | Comparar vários modelos/tarefas pelo comando público exec/JSONL em Git/home temporários; relatório incremental de sucesso, calls, tokens reportados, duração e exit code; separar falhas de infraestrutura e tokens ausentes. | `E/runner.ts`, `E/cleanup.ts`, `E/README.md` | pendente | `E/runner.test.ts`, `M/exec.test.ts` |
| EVAL-03 | Simulação | Dois perfis determinísticos, sem provider externo, passando pelo pipeline real; perfil B introduz regressão CRLF esperada; modo bundle/source identificado e cleanup Windows com retries/limites de diretório. Simulação não mede qualidade de modelos reais. | `E/simulation.ts`, `E/runner.ts`, `E/cleanup.ts` | pendente | `E/runner.test.ts`, `M/exec.test.ts`; simulação completa não executada nesta inspeção |
| DIST-01 | Distribuição | Launcher público, versão e pacote executável; distribuição Rust via npm no M8, com binários nativos opcionais e sem compilação na instalação. Publicação/tag exigem autorizações separadas. | `packages/myagent/build.ts`, `packages/myagent/package.json`, `packages/myagent/bin/myagent.js`, `scripts/package-smoke.ps1` | em andamento | Launcher e tarballs Windows x64 testados; matriz nativa de cinco plataformas preparada, pendente de execução remota. Evidência do M8 abaixo e em `docs/npm-release.md`. |

## Inventário de ferramentas e disponibilidade

O registro completo em `S/tools/index.ts` oferece **21 ferramentas comuns** e
**12 adicionais em BUILD** (33 no máximo). A oferta efetiva é intersectada com
workspace, disponibilidade Docker e perfil de contexto em `S/services/context-plan.ts`.

- Comuns: `read_files`, `project_inspect`, `file_outline`, `environment_check`,
  `symbol_search`, `test_run`, `test_discover`, `git_show`, `git_blame`,
  `diff_summary`, `read_file`, `list_directory`, `glob`, `grep`, `web_search`,
  `web_fetch`, `git_status`, `git_diff`, `git_log`, `type_check`, `todo_write`.
- BUILD acrescenta: `write_file`, `edit_file`, `delete_file`, `move_file`,
  `apply_patch`, `bash`, `git_commit`, `git_init`, `git_switch`, `git_remote_add`,
  `git_push`, `git_sync`.
- `test_run`, `type_check` e `bash` não são registrados sem Docker. No modo
  interativo, processos isolados dispensam aprovação com Docker; automação
  continua sujeita à política explícita. Web exige aprovação em ambos os modos.
- O perfil compacto mantém somente a interseção das ferramentas disponíveis com
  `read_file`, `read_files`, `list_directory`, `glob`, `grep`, `edit_file`,
  `write_file`, `apply_patch`, `git_status`, `git_diff`, `git_commit`, `bash`.
  Esse perfil nunca reintroduz uma ferramenta proibida em PLAN.

## Documentos do legado e divergências confirmadas

Foram consultados os documentos de `docs/`, os READMEs relevantes e os módulos
de `packages/`/`evals/` citados acima, sem copiar artefatos ou dados privados.
Os documentos antigos não fornecem resultados de testes válidos para este port.

| Documento/tema | Leitura à luz do código atual |
| --- | --- |
| `docs/ANALISE_PROJETO.md`, `docs/ANALISE_PROMPTS_E_TOOLS.md`, `docs/RELATORIO_AGENTS.md` | Fotografias com lacunas/propostas antigas. Confirmar cada item no código; loader AGENTS, aprovação, modo por sessão e retomada já possuem implementações/testes posteriores. |
| `docs/CICLO_DE_VIDA_SESSOES.md` | Diz 10 dias e expiração sem reativação; o código atual usa 90 dias por padrão, aviso com carência e cancelamento do aviso ao abrir. Ver SESSION-04. |
| `docs/JANELA_CONTEXTO_E_PROMPTS.md` | Descreve limite fixo antigo, loader futuro e ausência de DNS pinning; hoje há orçamento por modelo, loader AGENTS e transporte preso ao IP. Ver CTX, AGENTS e WEB-03. |
| `docs/POSTGRES_DOCKER.md`, `docs/CONTADOR_TOKENS.md` | Instruções de migrations PostgreSQL/db.deploy são históricas. Runtime e migrations ativos são SQLite local/runner embutido; PostgreSQL está em `P/migrations-postgres-legacy/`. Não importar dados antigos automaticamente. |
| `docs/CHATGPT_E_MODELOS_LOCAIS.md`, `docs/CONTEXTO_DO_MODELO.md`, `docs/INTEGRACAO_CLAUDE.md` | Conferir adapters/registry; ChatGPT está habilitado por padrão no código e é separado da chave OpenAI. Não assumir capacidade de família/modelo por nome. |
| `docs/APROVACOES_E_UNDO.md`, `docs/SEGURANCA_E_TESTES.md` | Referências para segurança, aprovações, checkpoints e sandbox; implementação limita grants e preserva decisões no backend. |
| `docs/CLI_GLOBAL.md`, `docs/EDICAO_E_BUSCA.md`, `docs/FERRAMENTAS_DIAGNOSTICO.md`, `docs/INSTRUCOES_DO_PROJETO.md` | Referências de UX e limites verificadas nos módulos correspondentes; não executar scripts/comandos antigos de instalação/migration nesta preparação. |
| `docs/PREPARACAO_PUBLICACAO_NPM.md`, `README.md`, `packages/myagent/README.md`, `E/README.md` | Contexto de build/instalação/evals do legado; publicar npm não faz parte do bootstrap nem é requisito arquitetural do port. |

Não foram identificados no runtime agentes autônomos configuráveis, MCP,
coordenação multiagente, worktrees isoladas por tarefa ou importação automática
do PostgreSQL. Propostas históricas não entram como funcionalidades existentes.
Decisões de expansão ou descarte exigem escopo próprio.

## Bootstrap e próximos milestones

| Entrega | Status | Evidência/limite |
| --- | --- | --- |
| Regras permanentes | feito | `AGENTS.md` criado na etapa anterior e preservado. |
| Inventário de paridade | feito | Matriz por comportamento, ferramentas, fontes e testes; inspeção estática, sem executar legado. |
| Toolchain e arquivos de repositório | feito | Rust 1.99.1 fixado, crate vazio e Cargo.lock, padrões de texto, licença, contribuição, segurança, deny e configuração GitHub; revisão estática e gates locais aprovados. |
| Gates locais | feito | fmt, Clippy sem warnings e cargo test com exit 0 no Windows; 0 testes unitários, 0 doctests e 0 ignorados. Não há testes funcionais no crate-base. |
| CI Windows/Linux/macOS | em andamento | Workflow publicado com o bootstrap; resultado da execução no GitHub ainda não conferido. |
| Git local/remoto | feito | main inicializada, origin aponta para https://github.com/mrJoao28/My-Agent.git e acompanha origin/main. O usuário executou ls-remote antes do primeiro commit, sem refs remotas. Bloqueio de ownership superado no terminal do usuário. |
| Commit/push | feito | Commit inicial `205ee94 chore: bootstrap repository`, criado e enviado pelo usuário; push criou main e configurou upstream. Log, status e branch locais confirmam main em 205ee94, alinhada a origin/main. |

A sequência técnica sugerida, sem iniciar outro milestone nesta etapa, é:
persistência/contratos e sessões → caminhos/política/approvals/checkpoints →
ferramentas/sandbox/rede → providers/registry/contexto/loop → CLI/TUI → evals e
validação integrada. Delimitar cada milestone e seus critérios antes de implementar.
Manter esta matriz atualizada e encerrar cada milestone conforme `AGENTS.md`.

## Validação do bootstrap em 19/10/2026

| Ferramenta | Versão/resultado observado |
| --- | --- |
| rustc | `1.99.1 (58cf8f9c3 2026-10-13)` |
| cargo | `1.99.1 (5f94df478 2026-08-27)` |
| git | `2.56.0.windows.2` |
| gh (opcional) | Não encontrado no PATH. |
| Docker CLI (opcional) | `29.7.2, build a7dcaa6`; aviso de acesso negado à configuração local. Daemon e containers não foram verificados. |

Rust 1.99.1 foi confirmado como stable atual no
[anúncio oficial de 15/10/2026](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/).
A consulta direta ao manifesto de distribuição falhou por conexão. A toolchain
instalada tem o nome `stable-x86_64-pc-windows-msvc`; foi usado
`RUSTUP_TOOLCHAIN=stable` somente no processo dos gates, com versão confirmada
por `rustc --version`, evitando instalar uma cópia numérica fora do workspace.
O projeto e o CI fixam `1.99.1`, não o canal móvel stable.

Executados com sucesso no Windows:

```text
cargo fmt --check                         exit 0
cargo clippy --all-targets -- -D warnings exit 0
cargo test                               exit 0
```

`cargo test` compilou o crate e executou 0 testes unitários e 0 doctests,
sem falhas ou ignorados. Sem dependências externas, não há crate novo a justificar.
Não foram executados cargo-deny, CI remoto, testes funcionais, integração Docker,
testes do legado, providers reais ou avaliações.

Revisão estática adicional: TOMLs parseados, YAMLs parseados e matriz de CI
conferida para os três sistemas; Dependabot contém cargo/github-actions/npm.
A entrada npm está preparada para eventual tooling; não existe package.json a
monitorar neste bootstrap. Foram verificados UTF-8 sem BOM, LF e newline final,
whitespace dos diffs e padrões comuns de credenciais nos 20 arquivos candidatos,
sem achados. Essa busca não garante detecção de todo segredo possível.
A matriz tem 87 comportamentos e 210 referências abreviadas únicas existentes
no legado. Nenhum comportamento funcional recebeu status feito.

Na preparação inicial, remote add falhou ao gravar `.git/config` e git add
falhou ao criar `.git/index.lock`, ambos por permissão. `git ls-remote origin`
falhou porque o remote não estava configurado, e a consulta direta à URL falhou
por conexão. Nenhum commit/push/PR/tag foi executado por este agente e o legado
não foi alterado. Não contornar restrições com outro diretório Git, escrita
manual de metadata ou publicação pela API. Antes de qualquer commit futuro,
conferir o histórico/remoto, revisar os arquivos staged em busca de segredos
e obter a autorização exigida pelo `AGENTS.md`; não repetir um bootstrap já
feito externamente.

### Fechamento Git confirmado

As primeiras tentativas no terminal externo falharam por `dubious ownership`:
`.git` pertence a `CodexSandboxOffline`, enquanto o usuário executa como `joaof`.
O bloqueio foi superado pelo usuário; este agente não alterou sua configuração
global. As observações anteriores de repositório vazio descreviam o estado
anterior ao commit bem-sucedido e não são pendências atuais.

O usuário apresentou as saídas de remote add, `git ls-remote origin` sem refs,
staging dos 20 arquivos, commit inicial e push. O commit criado foi
`205ee94 chore: bootstrap repository`, com 20 arquivos e 753 inserções.
O push criou `main` no origin e configurou o acompanhamento de `origin/main`.

A reconferência local deste agente confirmou:

```text
git log --oneline -3
205ee94 chore: bootstrap repository

git status
On branch main
Your branch is up to date with 'origin/main'.
nothing to commit, working tree clean

git branch -vv
* main 205ee94 [origin/main] chore: bootstrap repository
```

`git remote -v` também confirmou a URL esperada para fetch e push. O status
limpo acima foi observado antes desta atualização de `docs/PARITY.md`; ela fica
como alteração local, sem novo commit/push. O resultado do CI nos três sistemas
ainda precisa ser conferido; os gates locais já registrados são da preparação
inicial e não foram executados novamente nesta alteração documental.

A base está preparada para receber o Prompt 2. Criar um branch próprio para o
próximo milestone quando seu escopo for definido, preservando esta atualização
documental. Continuam válidas as autorizações explícitas para commits, pushes e PRs.

## Arquitetura e esqueleto — Prompt 2 (gates finais aprovados; commit pendente)

O [ADR 0001](adr/0001-arquitetura.md) compara organização dos crates, transporte,
SQLite, bibliotecas, parsing JS/TS, SDKs de LLM e modos extensíveis. Está
**aceito**: canais internos, armazenamento separado com importação explícita
posterior, SQLite bundled/tree-sitter com compilador C e esqueleto sem crates
externas. Clap e serde entram no M1; não foram adicionados nesta etapa.

Nos itens API-01/API-02, os comportamentos de sessão, autorização e comandos
continuam pendentes de port. A compatibilidade do transporte HTTP/SSE interno
não será reproduzida, por decisão explícita no ADR 0001. Controles de token e
Host/Origin só se aplicariam a um futuro adapter HTTP; não substituem as
validações obrigatórias de workspace, modo, ferramentas e aprovações no core.

| Entrega | Estado nesta etapa |
| --- | --- |
| Workspace | Oito pacotes myagent-protocol/core/providers/tools/sandbox/store/tui/cli, com manifests locais, resolver 3, metadados e lints comuns. Raiz virtual substitui o crate vazio inicial. |
| Dependências | Grafo sem ciclos verificado com cargo metadata; core depende somente de protocol entre os membros. Providers/tools/store dependem das futuras portas do core, nunca o inverso; sandbox independente. |
| Modos | Mode não exaustivo em protocol com apenas Plan/Build, reexportado por core. Sem Learn, serde, prompts ou política executável nesta etapa. |
| CLI | Binário myagent com --help/-h, --version/-v, ajuda sem argumentos e saída 2 para argumentos inválidos/ambíguos. Não inicializa banco, configuração, providers ou ferramentas. |
| Bibliotecas externas | Nenhuma adicionada, por decisão aceita. Clap/serde entram no M1. O parser std mínimo não será expandido para implementar os demais comandos. |
| CI | Workflow ampliado para fmt/check/clippy/test do workspace e smoke de ajuda/versão em Windows, Linux e macOS. Mudança local ainda não publicada nem executada remotamente. |
| Validação local | Gates finais aprovados no Windows após a aceitação do ADR: fmt, check, Clippy sem warnings e cargo test; três testes de integração, zero falhas e zero ignorados. Ajuda e versão também executadas com sucesso. |
| Git | O usuário criará feat/m0-architecture externamente, após a falha de permissão de criação da ref neste ambiente. Na checagem inicial deste fechamento, a branch ativa ainda era main. Sem commit, push ou PR desta etapa. |

A tentativa de consultar crates.io pelo Cargo falhou ao conectar a
`index.crates.io:443`, antes de obter dependências. Isso é falha de ambiente;
não foi usado como motivo para alterar a recomendação arquitetural. O cache
local de Cargo fica em `.cargo-cache/`, ignorado pelo Git, para não escrever
fora do workspace. Nenhuma credencial ou configuração privada foi lida.

Todos os targets próprios proíbem unsafe. Os demais crates são fronteiras
compiláveis, sem implementações fictícias de I/O ou funcionalidades do agente.
O resultado dos três testes da CLI não comprova a paridade dos modos, do loop,
das ferramentas, da persistência ou dos providers.

Validação preliminar executada com `RUSTUP_TOOLCHAIN=stable` (rustc/cargo 1.99.1)
e CARGO_HOME apontando para `.cargo-cache/`:

```text
cargo generate-lockfile --offline                                  exit 0
cargo fmt --all --check                                            exit 0
cargo check --locked --workspace --all-targets --offline            exit 0
cargo clippy --locked --workspace --all-targets --offline -- -D warnings
                                                                  exit 0
cargo test --locked --workspace --offline                           exit 0
cargo run --locked --offline -p myagent-cli --bin myagent -- --help  exit 0
cargo run --locked --offline -p myagent-cli --bin myagent -- --version
                                                                  exit 0
```

O primeiro fmt --check apontou uma asserção a formatar. A geração dos arquivos
também perdeu acentos na passagem pelo PowerShell; os nove arquivos afetados
foram corrigidos com UTF-8 explícito e a integração da ajuda passou a verificar
o texto em português. Depois de `cargo fmt --all`, os gates acima foram
executados novamente e passaram. Os testes cobrem ajuda (incluindo ausência de
argumentos e alias), versão do pacote e rejeição de opções inválidas/ambíguas.
Os demais targets não possuem testes de features do agente.

`cargo metadata --no-deps --format-version 1 --offline` foi usado para verificar
os oito membros e seu grafo acíclico. `git diff --check` passou para os arquivos
rastreados. As validações não acessaram o legado em escrita nem dados privados;
não houve instalação de dependências, chamadas de IA, execução Docker ou CI remoto.

As decisões arquiteturais foram respondidas e incorporadas. Concluir a
revisão dos arquivos staged em busca de segredos e apresentar diff stat,
arquivos e mensagem de commit para autorização. Confirmar
que feat/m0-architecture está ativa antes de executar um commit autorizado;
push do branch e abertura do PR serão pedidos separadamente depois do commit.

### Fechamento após a aceitação do ADR

Reexecutados no Windows com Rust 1.99.1, `RUSTUP_TOOLCHAIN=stable`, CARGO_HOME
local e `CARGO_NET_OFFLINE=true`, todos com exit 0:

```text
cargo fmt --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo run --locked -p myagent-cli --bin myagent -- --help
cargo run --locked -p myagent-cli --bin myagent -- --version
```

Resultado: três testes de integração aprovados, nenhum reprovado ou ignorado;
os demais targets não têm testes de features do agente. A versão exibida foi
`myagent 0.1.0`. A revisão dos 25 arquivos com conteúdo novo/modificado não
encontrou padrões comuns de credenciais; UTF-8/LF e manifests foram conferidos,
assim como os três sistemas da matriz CI. Essa revisão não garante detectar
todo segredo possível e não substitui a conferência do staged antes do commit.

O `git add` dos 26 caminhos desta entrega foi tentado após os gates e recusado
ao criar `.git/index.lock` por falta de permissão (exit 128). Nenhum arquivo
ficou staged; a branch ativa continuava main. O diff para revisão deve incluir
também os arquivos novos, pois git diff --stat sozinho não mostra untracked.

Pendências: criação externa e confirmação de feat/m0-architecture, preparação
e revisão do staged no ambiente com permissão, autorização de commit, posterior push/PR e resultado do
CI remoto. A importação de dados fica para uma etapa posterior; clap/serde
entram no M1. Nenhum banco, provider, ferramenta executável, TUI ou modo Learn
foi implementado e nenhuma credencial real foi acessada. Mensagem de commit
proposta: `chore: define architecture and scaffold workspace`.

## M1 — Tipos compartilhados e persistência

**Estado: implementação e testes locais concluídos em 20/10/2026;
aguardando autorização de commit, push/PR e resultado do CI.**

Referências: `packages/shared/src/schemas.ts`, schema Prisma, enums e
migrations em `packages/database`, message-service/message-format,
session-lifecycle e session-lease. O código do legado foi somente lido;
nenhum banco, credencial ou teste legado foi aberto/executado.

| Entrega | Implementação Rust | Testes portados/equivalentes | Status |
| --- | --- | --- | --- |
| Session, Message, Mode, status, parts e eventos | protocol; serde, validação de borda, limites e contadores opcionais | protocol/tests/contracts.rs; H/schemas.test.ts e S/services/message-format.test.ts | feito |
| SessionContext e SessionLease | protocol; JSON com metadados, datas em milissegundos, duração em segundos | contracts.rs e store/tests/persistence.rs; D/database.integration.test.ts | feito |
| SQLite e migrations | store e três migrations embutidas; WAL, timeout 5000 ms, FK, IMMEDIATE/user_version | store/src/migrations.rs e persistence.rs; D/migrations.test.ts | feito |
| Sessões por workspace | criar/listar/abrir, caminho canônico, chave em minúsculas no Windows | persistence.rs; S/services/workspace-sessions.test.ts e S/routes/sessions.test.ts | feito |
| Mensagens incrementais | IN_PROGRESS antes dos deltas; parts e texto duráveis; associação de tools por ID/nome | persistence.rs; S/services/message-format.test.ts e S/services/generation-journal.test.ts | feito |
| Exclusão entre processos | lease atômico, heartbeat 5 s, TTL 30 s, owner aleatório e liberação condicional | persistence.rs, incluindo outro processo e heartbeat real; workspace-sessions/database.integration | feito |
| Retenção | 90 dias/zero desativa, aviso durável, 24 h, abertura cancela; protege gerações/leases/mensagens recentes | persistence.rs; S/services/session-lifecycle.test.ts | feito |
| Home independente do banco legado | MYAGENT_HOME > MYAGENT_CONFIG_DIR > perfil/.myagent; somente myagent-rust.sqlite3 | config.rs e persistence.rs; H/home.ts e S/lib/config.test.ts | feito |
| CLI | ajuda/versão portadas para clap sem abrir banco | três testes de integração existentes da CLI | feito |

Contratos, fronteiras e justificativas de crates:
[Persistência M1](persistence.md). O ADR recebeu uma adenda sobre o arquivo
Rust independente no home solicitado. Não houve importação nem acesso ao
myagent.db ou ao home real durante os testes.

As linhas gerais GEN-01, SESSION-01/02/03/04, RECOVER-01, CTX-01 e CONFIG-01
continuam em andamento quando incluem comportamento além do M1: descoberta
da raiz Git, seleção automática na CLI, apresentação de avisos na TUI,
cancelamento da geração quando perde lease, recuperação de órfãs, reconstrução
de contexto e limpeza de futuros artefatos. Os tipos e os serviços de
armazenamento não comprovam esses fluxos. Learn permanece ausente.

### Validação executada

Ambiente Windows/MSVC, Rust 1.99.1 fixado, CARGO_HOME=.cargo-cache/,
CARGO_TARGET_DIR=target/m1-validation, dependências do lockfile em modo offline:

| Comando | Resultado |
| --- | --- |
| cargo fmt --check | Passou. |
| cargo check --locked --workspace --all-targets --offline | Compilou; a primeira execução apontou retorno de set_limit não tratado, corrigido antes do Clippy final. |
| cargo clippy --locked --workspace --all-targets --offline -- -D warnings | Passou após as correções. |
| cargo test --locked --workspace --offline | Passou: 24 testes, zero falhas e zero ignorados. |
| cargo metadata --locked --format-version 1 --offline | Passou; dependências e licenças declaradas inspecionadas. |
| git diff --check | Passou para arquivos rastreados. |

Distribuição: CLI 3, contratos 5, configuração/migrations 4, persistência 12.
Os demais targets e doc-tests não têm testes de features do agente.
Foram exercitados Unicode/CRLF, tags e validação, SQLite real, reabertura,
contexto com metadados, journal incremental, rollback/idempotência/upgrade,
cascatas, lease em dois processos, expiração/perda/liberação, heartbeat de
5 s, retenção com carência/cancelamento e recusa de hard links/junctions.
O caso de symlink Unix será executado pelo CI nas plataformas correspondentes.

A tentativa anterior de fetch falhou por rede; o usuário resolveu em seu
terminal, gerou o lockfile e criou feat/m1-store. A primeira tentativa de
testes neste fechamento encontrou LNK1104 ao abrir um executável existente
em target/debug. Uma saída de build nova em target/m1-validation resolveu
essa falha de ambiente, sem remover artefatos existentes.

A execução revelou duas correções de código/teste: tratar o Result de
Connection::set_limit e manter a orientação --help em erros da CLI.
A fixture de retenção na reabertura foi ajustada para ativar a política
que o próprio teste verifica. Depois dessas correções, a suíte completa
passou. A checagem auxiliar anterior de SQL em Python/SQLite 3.50.4 não foi
usada como substituta dos testes Rust.

O CI já está configurado para Windows/Linux/macOS e ainda não foi publicado
ou executado para este M1. Não foram feitas chamadas de IA, Docker, importação
de banco, leitura de segredos, publicação npm ou criação de tag.
cargo-deny não foi executado; a licença Zlib de foldhash 0.2 foi revisada
localmente e permitida no deny.toml pela dependência transitiva do rusqlite.

### Git e proposta de entrega

Branch confirmada: feat/m1-store. Nenhum commit, push ou PR executado pelo
assistente nesta etapa. A revisão do staged e o resultado da preparação
serão registrados no fechamento. Mensagem proposta:

`feat: add shared types and SQLite persistence`

Arquivos para o commit:

- Cargo.toml e Cargo.lock
- CONTRIBUTING.md, README.md e deny.toml
- crates/cli/Cargo.toml e crates/cli/src/main.rs
- crates/core/src/lib.rs
- crates/protocol/Cargo.toml, crates/protocol/src/lib.rs e crates/protocol/tests/contracts.rs
- crates/store/Cargo.toml e crates/store/src/lib.rs
- crates/store/src/config.rs, migrations.rs, records.rs e services.rs
- crates/store/migrations/0001_sessions.sql, 0002_context_leases.sql e 0003_indexes.sql
- crates/store/tests/persistence.rs
- docs/PARITY.md, docs/adr/0001-arquitetura.md e docs/persistence.md

### Preparação final do commit

Os três gates finais passaram no Windows com a toolchain fixada 1.99.1:
fmt --check, Clippy do workspace/all-targets com -D warnings e cargo test
do workspace (24 aprovados, zero falhas/ignorados). As dependências foram
resolvidas pelo lockfile e verificadas em modo offline.

A revisão dos 24 arquivos novos/modificados passou para UTF-8/LF, TOML e
padrões comuns de credenciais, sem encontrar arquivos proibidos. Isso não
garante detectar todo segredo possível. O git add explícito desses caminhos
foi recusado ao criar .git/index.lock por falta de permissão. A revisão do
staged continua obrigatória antes do commit; ele estava vazio na conferência
final. O usuário precisa preparar o staged no seu terminal. Nenhum commit,
push, PR ou alteração de remote foi executado.

## M2 — Providers e orquestração de gerações

Implementação preparada no branch **feat/m2-agent**, baseada no commit do M1
3920dfc. Validação local concluída; aguardando autorização para commit e,
depois, push/PR e execução do CI remoto. Nenhum commit/push/PR deste M2.

Referências lidas no legado: providers Anthropic/OpenAI/Google/Ollama, resolver
e registry; model-registry/model-config, shared/models; chat-service,
stream-service, tool-service, context-plan, context-summary, session-context,
generation-journal, project-instructions e os módulos/testes de prompts.
Código prevalece sobre documentação. ChatGPT permanece reservado ao M7.
Não foram executados testes Bun nem acessados bancos, home privado ou segredos.

| Entrega M2 | Código Rust | Testes equivalentes/adicionais | Situação |
| --- | --- | --- | --- |
| Quatro providers e registry | providers/{http,wire,registry,catalog.json} | providers/src/tests.rs: dialetos nativos, catálogo/overrides/env, modelos inválidos e erro sem chave | feito |
| SSE incremental limitado | providers/sse.rs e http.rs | UTF-8 fragmentado, CRLF/CR/BOM, multiline, limites, status/content-type, EOF, erros e cancelamento em TCP loopback | feito |
| Modos e prompts únicos | core/{agent,prompts,ports}.rs | core/tests/policy.rs e store/tests/generations.rs: seis blocos e escrita ausente em PLAN inclusive classificação incorreta | feito |
| Guardas do loop | core/guard.rs | env 40/200, valores inválidos, lote acima do orçamento, três repetições, síntese sem tools e timeout virtual de cinco minutos | feito |
| Journal integrado | store/journal.rs e services.rs | SQLite real; parciais/usage, chamada persistida antes de efeito, contexto persistido antes da rede, falha impede inferência | feito |
| Locks, cancelamento e cleanup | core/agent.rs e sandbox/process.rs | concorrência por sessão, cancelamento de HTTP/processo/consumidor, espera por cleanup e perda de lease durante ferramenta longa | feito |
| Retomada | core/agent.rs e StoreJournal | USER pendente, generationId, orçamento acumulado, replay bloqueado mesmo com efeito desconhecido e preservação de lease estrangeiro | feito |
| Contexto e tokens | core/{context,tokens}.rs | ordem texto/call/result, orçamento/summary, usuário preservado, linhas/paginação, invalidação por escrita/undo, medições ausentes/cumulativas | feito |
| Estado específico de provider | wire.rs, SessionContext e StoreJournal | signatures/encrypted reasoning no codec, persistência durante geração e retomada, omissão no novo turno | feito |
| Instruções do projeto | core/instructions.rs e tools/instructions.rs | raiz/subescopos/cache, budget total de 16 KiB, candidatos, segredo por nome, hard link e junction; symlink Unix no CI | feito |

Detalhes de contratos, configuração, dependências e referências oficiais:
[Providers e gerações](generations.md).

Diferenças deliberadas em relação ao legado, conforme o pedido:
valores MYAGENT_MAX_TOOL_CALLS acima de 200 voltam a 40; o legado os limitava
a 200. A síntese começa com até duas chamadas restantes, inclusive já no
primeiro passo quando o limite é 1/2. A retomada também conserva orçamento
e repetições do generationId, para não reabrir permissões ao interromper.
Ferramentas executam sequencialmente e só depois da conclusão validada do
stream do provider. O journal permanece incremental.

Os adapters são testados com chaves fictícias e servidores 127.0.0.1, sem
chamadas pagas nem inferência real. Isso valida contratos simulados, não
disponibilidade dos modelos ou compatibilidade observada de uma conta real.
Não houve login, OAuth, refresh, leitura de .env ou credenciais, acesso ao
myagent.db nem importação automática.

As linhas gerais que incluem UI/CLI/exec, políticas concretas de aprovação,
ferramentas de projeto, Docker e shutdown da aplicação continuam em andamento
ou pendentes nos milestones próprios. O núcleo não registra tools de produção
nem faz shell no host. O supervisor só adota processos já criados/autorizados;
a futura integração Docker encerrará o container inteiro. Modelos customizados
podem ser carregados de models.json; edição/diálogo e preferências da interface
permanecem posteriores. A estimativa de contexto é heurística, separada do uso
real reportado e da soma de tokens ausentes.

### Validação e ambiente do M2

Rust fixado 1.99.1/MSVC, CARGO_HOME=.cargo-cache/. As dependências foram
baixadas pelo usuário e o lockfile foi atualizado pelo Cargo. A criação do
branch também foi feita pelo usuário, após a recusa de escrita em .git.

Uma primeira suíte passou com 59 testes e uma execução posterior com 75.
Foram corrigidos o uso de API de slice ainda instável, lints do Clippy e
cenários de contexto/cleanup identificados na revisão. Esses resultados
intermediários não substituem a suíte do código final.

Repetições do build encontraram LNK1104 ao abrir executáveis existentes no
diretório de saída anterior. Não foi encontrado um processo de fixture
pendente na consulta realizada. A causa exata não foi confirmada. A validação
final usa um diretório novo (target/m2-gates), sem apagar artefatos anteriores.
Isso é distinto de uma falha de teste ou de compilação Rust.

O CI existente cobre Windows/Linux/macOS e executa fmt/check/clippy/test
do workspace. O CI remoto do M2 ainda não foi publicado/executado.
A consulta do PR do M1 por gh também falhou por restrição de rede; o merge
remoto do M1 não foi presumido. Não houve alteração de remote, tag, release,
publicação npm, commit ou push do M2.

### Fechamento local do M2

Gates aprovados no Windows/MSVC com a toolchain 1.99.1:

- `cargo fmt --check`.
- `cargo clippy --locked --workspace --all-targets --offline -- -D warnings`.
- `cargo test --locked --workspace --offline`: **76 aprovados, zero falhas e zero ignorados**.

Distribuição: CLI 3, core 9, protocol 5, providers 16, sandbox 4,
store 4, integração de gerações 20, persistência 12 e tools 3.
Testes finais em target/m2-gates; fmt/Clippy em target/m2-validation.
Nenhuma alteração de código ocorreu depois desses testes.

Permanecem pendentes o CI remoto nas três plataformas e as integrações
CLI/TUI, ferramentas concretas/aprovações, Docker e ChatGPT dos milestones
posteriores. Não houve teste com API paga ou credenciais reais.

Commit proposto: `feat: add providers and generation orchestration`.
Os arquivos deste M2 para revisão/commit estão listados abaixo.

- `Cargo.lock`
- `Cargo.toml`
- `README.md`
- `crates/core/Cargo.toml`
- `crates/core/src/agent.rs`
- `crates/core/src/context.rs`
- `crates/core/src/guard.rs`
- `crates/core/src/instructions.rs`
- `crates/core/src/lib.rs`
- `crates/core/src/ports.rs`
- `crates/core/src/prompts.rs`
- `crates/core/src/tokens.rs`
- `crates/core/tests/policy.rs`
- `crates/providers/Cargo.toml`
- `crates/providers/src/catalog.json`
- `crates/providers/src/http.rs`
- `crates/providers/src/lib.rs`
- `crates/providers/src/registry.rs`
- `crates/providers/src/sse.rs`
- `crates/providers/src/tests.rs`
- `crates/providers/src/wire.rs`
- `crates/sandbox/Cargo.toml`
- `crates/sandbox/src/lib.rs`
- `crates/sandbox/src/process.rs`
- `crates/sandbox/tests/process.rs`
- `crates/store/Cargo.toml`
- `crates/store/src/journal.rs`
- `crates/store/src/lib.rs`
- `crates/store/src/services.rs`
- `crates/store/tests/generations.rs`
- `crates/tools/Cargo.toml`
- `crates/tools/src/instructions.rs`
- `crates/tools/src/lib.rs`
- `crates/tools/tests/instructions.rs`
- `deny.toml`
- `docs/PARITY.md`
- `docs/adr/0001-arquitetura.md`
- `docs/generations.md`
- `docs/persistence.md`

### Revisão para o commit do M2

Foram revisados os 39 arquivos listados: UTF-8/LF, TOML, git diff --check,
nomes proibidos e padrões comuns de credenciais, sem ocorrências. Essa
checagem não garante detectar todo segredo possível. O grafo dos oito
crates do workspace foi verificado e não tem ciclos.

O git add foi recusado por falta de permissão para criar .git/index.lock.
O staged permanece vazio; o usuário precisará prepará-lo no seu terminal
e a revisão do staged continua obrigatória antes do commit. Nenhum commit,
push ou PR do M2 foi executado.

## M3 — Base de ferramentas e inspeção local

Implementação no branch feat/m3-tools, baseada no M2 a142da4.
O branch e o download das dependências foram feitos pelo usuário depois dos
bloqueios de escrita em .git e acesso à rede deste ambiente.
Nenhum commit, push ou PR do M3 foi executado.

Referências lidas: tools/index.ts e implementações das 17 ferramentas pedidas;
security/workspaces.ts, utils/path.ts e git.ts; project-inspection, search-ignore,
source-analysis/source-worker, grep-worker, tool-io e git-inspection.
Foram conferidos os testes de caminhos, leitura/edição, busca/ignore,
inspeção, cancelamento e Git, além de FERRAMENTAS_DIAGNOSTICO.md e EDICAO_E_BUSCA.md.
Código do legado prevaleceu para os limites numéricos.

| Entrega M3 | Código | Evidência |
| --- | --- | --- |
| Contratos e registry | core/ports.rs, core/agent.rs; tools/registry.rs | ToolLimits e erros explícitos; schema compartilhado por publicação/validação; 17 ferramentas nos dois modos; modo desconhecido negado. PLAN não recebe mutação do projeto. |
| Caminhos seguros | tools/path.rs | traversal, ancestrais ausentes, limites de raízes confiáveis, nomes sensíveis, volumes, prefixos canônicos, caixa ASCII Windows, hard links e junctions; symlinks Unix por cfg. |
| read_file/read_files/list_directory | tools/files.rs e io.rs | páginas Unicode/CRLF sem gaps, arquivo vazio, linha longa, limites de arquivo/lote/página, falhas individuais e exclusão de hard links. |
| glob/grep/ignores | tools/search.rs e io.rs | glob literal, ignore raiz/negações/poda por pai inclusive acesso direto, contexto/include/files_only, regex inválida/patológica, cancelamento ativo e tetos de 500/200 resultados e 20 MB. |
| file_outline/symbol_search | tools/source.rs | AST JS/TS/JSX/TSX e variantes, declarações em vez de comentários/referências, tipos/membros, limites 200/100/256 KiB/2 MiB e cancelamento ativo. |
| project_inspect/environment_check/test_discover | tools/inspect.rs e registry.rs | comandos de manifests não executados/não retornados; diagnóstico passivo, runners e convenções, categorias e truncamento explícito. |
| Git de inspeção e diff_summary | tools/git.rs | fixtures Git reais: branch/status, working tree/staged distintos, log/show, blame histórico e limites de linhas; preservação do index. Includes/alternates e ignoreRevsFile bloqueados, filtros/textconv/diff externo desativados. |
| todo_write | tools/registry.rs | substituição integral, contagens, IDs opacos, isolamento e clear_session; nenhum arquivo ou banco alterado. |

Contratos, descrições, todos os limites e justificativas das dependências:
[Ferramentas M3](tools.md). O registry pode ser passado ao Agent pela interface
Tool existente; a conversa da CLI/TUI permanece posterior.
todo_write está pronto como ferramenta; a conexão automática de sua limpeza
ao encerramento da aplicação continua dependente do lifecycle futuro.
O isolamento de Git de leitura está pronto; serialização de escritas Git
continua pendente com as ferramentas de alteração.

### Diferenças deliberadas

- Regex usa o matcher Rust sem backtracking exponencial; backreferences e
  lookaround de JavaScript retornam INVALID_PATTERN. AST usa tree-sitter em
  vez de TypeScript; equivalência de recuperação de sintaxe inválida não é prometida.
- As bordas gerais têm 128 KiB de entrada e 256 KiB de saída; os limites
  específicos do legado são mantidos. Há profundidade defensiva de 256 na busca.
- environment_check descreve My-Agent/OS/arquitetura, substituindo Bun/Node.
- git_status usa paths seguros e scanner de untracked limitado, com ignore
  apenas da raiz. Não enumera diretórios privados. Config Git com includes é
  recusada antes da execução; configuração de até 256 KiB.
- Revalidação de caminhos/handles não elimina corrida contra processo local
  malicioso que troque ancestrais simultaneamente. I/O bloqueado pelo sistema
  operacional não pode ser cancelado instantaneamente por std::fs.

### Validação local final

Rust 1.99.1/MSVC no Windows; CARGO_HOME=.cargo-cache e
CARGO_TARGET_DIR=target/m3-validation. Lockfile atualizado em modo offline.
Os três gates passaram:

- cargo fmt --check.
- cargo clippy --locked --workspace --all-targets --offline -- -D warnings.
- cargo test --locked --workspace --offline: **101 aprovados, zero falhas e zero ignorados**.

Distribuição: CLI 3, core 9, protocol 5, providers 16, sandbox 4, store 4,
gerações 20, persistência 12, instruções de arquivos 3 e ferramentas M3 25.
Nenhuma alteração de implementação ocorreu depois dessa suíte final.

A primeira suíte encontrou quatro falhas reais nos testes novos: semântica
de chaves literais em gitignore e prefixo canônico Windows nos argumentos Git.
Foram corrigidas e os cenários passaram na suíte final. Uma consulta auxiliar
de metadata sem CARGO_HOME apontou falta de cache; foi repetida com o cache
correto. Falhas de rede do fetch e de permissão do branch são de ambiente,
distintas das correções de código.

Cargo metadata confirmou licenças MIT/Apache-2.0 ou alternativa MIT nas
19 dependências transitivas/diretas adicionadas. Nenhuma expansão de deny.toml
foi necessária. cargo-deny não foi executado.
Não foram executados os testes Bun nem chamadas a modelos pagos, Docker ou web.

O CI existente executa os gates em Windows/Linux/macOS; não houve execução
remota do M3. Windows validou junctions/hard links e volumes; testes de
symlinks Unix aguardam o CI. Não foi criado symlink de arquivo Windows que
exija privilégios adicionais; reparse points foram exercitados por junctions.

Web, shell, test_run/type_check ficam para o M5, conforme o pedido.
Escrita/patch/checkpoints/aprovações e integração CLI/TUI continuam nos
milestones próprios. Nenhuma ferramenta desse escopo futuro foi registrada.

Commit proposto: `feat: add safe read-only tools and registry`.

Arquivos para revisão/commit:

- `Cargo.lock`
- `README.md`
- `crates/core/src/agent.rs`
- `crates/core/src/ports.rs`
- `crates/tools/Cargo.toml`
- `crates/tools/src/files.rs`
- `crates/tools/src/git.rs`
- `crates/tools/src/inspect.rs`
- `crates/tools/src/io.rs`
- `crates/tools/src/lib.rs`
- `crates/tools/src/path.rs`
- `crates/tools/src/registry.rs`
- `crates/tools/src/search.rs`
- `crates/tools/src/source.rs`
- `crates/tools/tests/read_only.rs`
- `docs/PARITY.md`
- `docs/tools.md`


### Preparação do commit M3

Revisados os 17 arquivos listados para nomes proibidos, padrões comuns
de credenciais, UTF-8/LF, TOML e whitespace, sem ocorrências. Essa busca
não garante detectar todo segredo possível. O grafo dos oito crates
permanece sem ciclos. A conferência final de cargo fmt --check passou.

O git add foi recusado por falta de permissão para criar .git/index.lock.
O staged está vazio nesta preparação; o usuário deve prepará-lo no seu
terminal, e seus blobs precisam de revisão antes de qualquer commit.
Nenhum commit, push ou PR deste milestone foi executado.

## M4 — alterações com consentimento e desfazer

Implementadas as nove ferramentas solicitadas, ApprovalManager por sessão,
checkpoints de bytes e metadados, executor Git protegido e API local de `/undo`.
Contratos, limites, diferenças do patch e justificativa do acesso nativo ao
ctime Windows estão em [mutations.md](mutations.md).
PLAN continua sem ferramentas de alteração na lista montada pelo código.
A conexão dos diálogos e do comando local à CLI/TUI permanece pendente.

A validação final no Windows passou com **138 testes, zero falhas e zero
ignorados**: CLI 3, core 9, protocol 5, providers 16, sandbox 4, store 4,
gerações 20, persistência 12, ferramentas unitários 7, instruções 3,
mutações 30 e ferramentas somente leitura 25.

Os testes cobrem Y/A/N, confirmação adicional M para main/master, isolamento,
timeout, cancelamento e limite de 256 escopos; checkpoint e falha parcial,
CRLF, binários, identidade/modo/mtime/ctime, conflito em todos os caminhos,
lease perdido, geração ativa, concorrência e limites de armazenamento;
patch multiarquivo/checkOnly sem tocar no índice; commit seletivo inclusive
de exclusões e push com transporte simulado. Provider e push foram simulados,
sem chamadas pagas ou push real. Testes Bun do legado não foram executados.

Na tentativa da suíte completa, o diretório de build reutilizado apresentou
erro de linker LNK1104. Um novo diretório de build apresentou falta de espaço
em disco (erro 112), antes de concluir a compilação. São falhas de ambiente;
a compilação foi retomada após o usuário liberar espaço.

A suíte completa revelou uma falha real na restauração Windows: um componente
vazio acrescentava uma barra ao caminho do arquivo antes de restaurar suas
permissões. A correção preserva o caminho raiz sem essa barra e passou na
suíte final. Diagnósticos temporários foram removidos.

Gates finais executados com CARGO_HOME=.cargo-cache e target/m4-gates:

- `cargo fmt --all --check`: passou.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings`: passou.
- `cargo test --locked --offline --workspace --config 'profile.test.package.myagent-tools.codegen-units=15'`: passou.

Nesta validação, CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0 e
CARGO_INCREMENTAL=0 reduziram uso de disco. O override de codegen-units
somente para tools gerou outro executável após LNK1104 ao sobrescrever o
anterior. Não mudou fontes, features, assertions nem seleção de testes.
Nenhum ajuste de perfil foi gravado na configuração do repositório.

O CI existente contempla Windows, Linux e macOS; os dois últimos não foram
executados localmente nem houve execução remota do M4. A interface de
diálogos e `/undo` na CLI/TUI permanece para seu milestone; a API local está
pronta. `git_switch`, `git_sync`, web e bash continuam pendentes, fora deste
escopo. O parser próprio de patches e a exceção unsafe isolada em platform
estão justificados em mutations.md e na adenda do ADR.

Cargo metadata confirmou nove crates sem ciclos e licenças MIT/Apache-2.0
nas dependências adicionadas; cargo-deny não foi executado. A revisão dos
27 arquivos alterados/novos não encontrou nomes proibidos nem padrões comuns
de credenciais. Isso não garante detectar todo segredo possível; o staged
deve ser conferido novamente antes de cada commit.

Commit proposto: `feat: add approved mutations and checkpoint undo`.
Nenhum commit, push ou PR do M4 foi executado pelo assistente.

### Preparação do commit M4

O git add dos 27 arquivos foi recusado por falta de permissão para criar
`.git/index.lock`; o staged permanece vazio. Após o usuário preparar os
arquivos no terminal, os blobs staged devem ser conferidos novamente antes
do commit autorizado. A conferência final de formatação, UTF-8/LF e
`git diff --check` passou; não houve alteração de implementação depois dos
gates finais.

Arquivos para revisão e commit:

- `Cargo.lock`
- `Cargo.toml`
- `README.md`
- `crates/core/src/agent.rs`
- `crates/core/src/lib.rs`
- `crates/core/src/ports.rs`
- `crates/platform/Cargo.toml`
- `crates/platform/src/lib.rs`
- `crates/store/src/journal.rs`
- `crates/store/src/services.rs`
- `crates/tools/Cargo.toml`
- `crates/tools/src/approval.rs`
- `crates/tools/src/checkpoints.rs`
- `crates/tools/src/file_mutations.rs`
- `crates/tools/src/git.rs`
- `crates/tools/src/git_mutations.rs`
- `crates/tools/src/lib.rs`
- `crates/tools/src/mutation_io.rs`
- `crates/tools/src/mutations.rs`
- `crates/tools/src/registry.rs`
- `crates/tools/tests/mutations.rs`
- `crates/tools/tests/read_only.rs`
- `docs/PARITY.md`
- `docs/adr/0001-arquitetura.md`
- `docs/mutations.md`
- `docs/persistence.md`
- `docs/tools.md`

## M5 — execução isolada e acesso à web

Implementados `bash`, `type_check`, `test_run`, `web_fetch` e `web_search`.
Fontes conferidas: `S/security/sandbox.ts`, `S/security/snapshot.ts`,
`S/tools/bash.ts`, `S/tools/type-check.ts`, `S/tools/test-run.ts`,
`S/tools/web-fetch.ts`, `S/tools/web-search.ts`, `S/tools/web-ssrf.test.ts`
e `docs/SEGURANCA_E_TESTES.md` do legado. Contratos, diferenças e justificativas
estão em [Sandbox e web M5](sandbox-web.md) e na adenda do ADR.

Docker é detectado em até três segundos; sem daemon Linux os três comandos
somem da lista. PLAN nunca oferece bash. O container tem rede desativada,
rootfs/snapshot somente leitura, usuário não root, capabilities removidas,
no-new-privileges e limites de recursos. Não há fallback no host. A limpeza
precede a remoção do snapshot e, se falhar, conserva o mount e retorna erro.

A imagem recomendada e implementada é rust:1.99.1-slim-bookworm. Acrescentados
Cargo test/check offline, preservando os cinco runners legados. Snapshot
filtrado aplica 5 MiB por arquivo, 200 MiB, 20 mil entradas e profundidade 64;
opt-in de dependências continua sujeito aos limites globais. A cópia é nova
por execução: o cache de metadados/TTL legado permanece pendente em BOX-04.

Web exige aprovação por ferramenta/sessão/origem, valida todos os registros
DNS e fixa os IPs no transporte, preservando Host e verificação TLS. Recusa
redirecionamento para outra origem, proxies, redes internas/reservadas e
metadata de nuvem. Os limites são aplicados durante a leitura; conteúdo
retornado não concede permissões. Tavily usa chave fictícia nos testes.

### Validação M5 — Windows, 21/10/2026

Gates finais passaram com CARGO_HOME=.cargo-cache e target/m4-gates,
reaproveitando dependências já compiladas:

- `cargo fmt --all --check`.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings`.
- `cargo test --locked --offline --workspace --config 'profile.test.package.myagent-tools.codegen-units=11' --config 'profile.test.package.myagent-sandbox.codegen-units=11'`.

**161 testes aprovados, zero falhas e 1 ignorado**, que não entra nos aprovados.
Distribuição: CLI 3, core 9, protocol 5, providers 16, sandbox/processos 4,
store 4, gerações 20, persistência 12, unitários tools 14, instruções 3,
execução isolada 8, mutações 30, ferramentas somente leitura 25 e web 8.
Os doc-tests não contêm casos e não foram somados como testes aprovados.

O teste ignorado é exclusivamente a integração Docker real. A variável
MYAGENT_TEST_DOCKER_IMAGE não está definida neste ambiente; nenhum container
real foi executado. Há comando documentado e job Linux opcional no CI para
essa validação. O teste de shims Unix também aguarda execução no CI, pois não
é compilado no Windows. Linux/macOS e CI remoto não foram executados aqui.

Os casos executados cobrem ausência de Docker/lista por modo, deadline de
detecção, flags e mount filtrado, runners, timeout/cancelamento/abandono,
falha de limpeza, limites de snapshot, hard links e junctions; IPs reservados,
DNS misto, rebinding inicial e após redirect, escopo de origem, negação,
consumo de autorização, HTML, tamanhos, erros e respostas Tavily.
HTTP local real confere IP fixado, Host, limite em chunks e encerramento
da conexão cancelada. Não houve chamada externa a Tavily ou modelos pagos,
nem testes Bun do legado. TLS público real não foi exercitado.

As primeiras verificações Clippy identificaram condições aninhadas e um
guard de mutex com escopo que abrangia await no teste; ambos corrigidos.
Não houve falha de testes na suíte completa inicial ou final do M5.
Na revisão, o cache Cargo da imagem passou a ser referenciado em vez de
copiado ao tmpfs; diagnóstico tsc/Cargo e falhas de limpeza receberam testes.

CARGO_PROFILE_DEV_DEBUG=0, CARGO_PROFILE_TEST_DEBUG=0 e CARGO_INCREMENTAL=0
reduziram o consumo de disco. Os overrides locais de codegen-units evitaram
reutilizar nomes de executáveis já usados no ambiente Windows; não mudam
fontes, features, assertions ou seleção de testes. Esses perfis não foram
gravados no repositório. Nenhum unsafe novo nem crate inédita foi adicionado;
reqwest, tempfile e thiserror foram reutilizadas por necessidade concreta.

Permanecem pendentes: Docker real, CI Linux/macOS, BOX-04 e integração de
CLI/TUI/doctor/progresso de preparação. A matriz mantém BOX-01 e MODE-01 em
andamento por esses pontos de interface e integração, sem marcar a paridade
completa do produto. Learn não foi implementado.

Commit proposto: `feat: add Docker sandbox and approved web tools`.
Nenhum commit, push ou PR do M5 foi executado pelo assistente.

### Preparação do commit M5

Revisados os 18 arquivos alterados/novos: nenhum nome proibido nem padrão
comum de credenciais encontrado, UTF-8/LF e `git diff --check` válidos.
Essa busca não garante detectar todo segredo possível. Cargo metadata
confirmou nove crates sem ciclos e licenças MIT/Apache-2.0 nas dependências
reutilizadas. cargo-deny não foi executado.

O git add foi recusado por falta de permissão para criar `.git/index.lock`.
O staged está vazio; o usuário precisa prepará-lo no terminal e os blobs
staged precisam de nova conferência antes de qualquer commit autorizado.
Não houve alterações de implementação após os gates finais.

Arquivos para revisão/commit:

- `.github/workflows/ci.yml`
- `Cargo.lock`
- `README.md`
- `crates/sandbox/Cargo.toml`
- `crates/sandbox/src/lib.rs`
- `crates/sandbox/src/docker.rs`
- `crates/tools/Cargo.toml`
- `crates/tools/src/lib.rs`
- `crates/tools/src/registry.rs`
- `crates/tools/src/execution.rs`
- `crates/tools/src/snapshot.rs`
- `crates/tools/src/web.rs`
- `crates/tools/tests/docker_real.rs`
- `crates/tools/tests/isolated_execution.rs`
- `crates/tools/tests/web.rs`
- `docs/PARITY.md`
- `docs/adr/0001-arquitetura.md`
- `docs/sandbox-web.md`

## Correções de integração e macOS — 21/10/2026

Correções autorizadas diretamente na main pelo usuário, sem mudanças na CLI/TUI.

| Área | Correção | Evidência de regressão |
| --- | --- | --- |
| GEN-04 / UNDO | Notas locais USER/[UNDO] invalidam evidência antes do filtro ASSISTANT; próxima geração pode reler o arquivo restaurado. | policy.rs e mutations.rs: edição real, leitura, undo e nova geração com SQLite. |
| Instruções por escopo | Lotes com mutações descobrem instruções antes de qualquer aprovação ou efeito. Regras novas adiam o lote inteiro e exigem nova proposta do modelo. | mutations.rs: leitura + edição no mesmo lote, uma única aprovação e um checkpoint; caminhos relativos, absolutos e separadores Windows. |
| Caminhos | files aceita objetos de read_files e strings de git_commit/runners; alvos dos runners respeitam path. | policy.rs e fonte FilesystemInstructions. |
| Cleanup | Prazo por ferramenta (2s padrão, 15s Docker, 310s mutações/checkpoints), com início comum. Falhas e timeouts não pulam a persistência terminal. Erro de remoção Docker sobrevive ao cancelamento. | isolated_execution.rs: Agent real com driver Docker simulado, limpeza de 3s, sucesso e falha; estado IN_PROGRESS até terminar a tentativa. |
| Perfil compacto | test_run/type_check continuam disponíveis com Docker; PLAN continua sem bash ou mutações. | generations.rs: janela de 16384 e registro de ferramentas enviado ao provider. |
| macOS | platform reconhece somente /var e /tmp com destinos fixos em /private (absolutos ou relativos à raiz). Providers, store, instruções e diretórios de checkpoints usam a mesma exceção restrita. | Fixture temporária não canonicalizada em providers/store/instruções; testes de links arbitrários e hard links; teste específico dos aliases no macOS. |

Não foram adicionadas crates externas. Providers e store agora reutilizam
myagent-platform; Cargo.lock registra somente essas duas arestas internas.
O limite efetivo de argumentos via provider continua sendo 64 KiB; a API
local de mutações pode aceitar entradas maiores. Isso está explícito em
docs/generations.md; escritas grandes via modelo devem ser direcionadas/divididas.

Validação local Linux com Rust 1.99.1: cargo fmt --all --check,
cargo check --locked --workspace --all-targets,
cargo clippy --locked --workspace --all-targets -- -D warnings e
cargo test --locked --workspace. A suíte registrou 169 testes aprovados,
nenhuma falha e um teste Docker real ignorado. Ajuda e versão também conferidas.
Windows/macOS não foram executados nativamente nesta máquina; os resultados
remotos devem ser conferidos no workflow CI do commit correspondente.
Docker real e providers externos não foram executados.

## M6 — TUI Grafite (implementação entregue; validação manual Windows pendente)

O usuário autorizou a branch feat/m6-tui, commits por etapa e merge após
validação. O layout escolhido mantém PLAN/BUILD no cabeçalho (sem Tab
repetido), painéis sucessivos por arquivo e uma única barra cumulativa
com escala de 10 milhões de tokens.

O legado packages/cui/src e docs/APROVACOES_E_UNDO.md não estão disponíveis
neste ambiente. O repositório My-Code-Agent retornou 404 na conexão GitHub.
A implementação usa os contratos e a matriz acima; não se afirma inspeção
do legado nem paridade visual verificada com ele.

Etapa 1: eventos efêmeros ToolPreview ligam fragmentos de argumentos dos
providers à interface. Não são chamadas, não entram no journal nem autorizam
efeitos. JSON incompleto não executa ferramentas. Google só mostra prévias
quando o provider efetivamente disponibiliza os argumentos.

Etapa 2: ratatui 0.29 + crossterm 0.28 fornecem renderização e eventos de
terminal; unicode-segmentation/unicode-width mantêm cursor e largura de
texto; insta é somente dependência de testes. São dependências diretas
justificadas pela TUI; não se adiciona SDK de IA. Grafite é o tema padrão
escolhido pelo usuário, com Nightfox/Papel alternativos e NO_COLOR.
preferences.json tem tamanho limitado e gravação por temporário; falhas de
leitura preservam o arquivo e permitem fallback visual. Entrada multilinha
respeita 9000 unidades UTF-16, grafemas Unicode e sanitização de controles.

Etapa 3: reducer independente de terminal para home/nova sessão/sessão,
modais, comandos locais, retomada explícita e foco. Aprovações têm escopo,
prévia limitada, Y/A/N e confirmação M para main; Enter/Tab não aprovam.
Prévias parciais decodificam escapes Unicode sem inferir efeito aplicado;
painéis de cada arquivo preservam estados diferentes. /undo é ação local.

Etapa 4: renderização Grafite com seletor no cabeçalho sem Tab duplicado,
caixa de entrada com indicador lateral, painéis de código por arquivo e uma
única barra cumulativa de 10M. TestBackend/insta cobrem home, nova sessão,
sessão, aprovação e diálogo; tamanhos reduzidos e NO_COLOR são verificados.

Etapa 5: loop de terminal separado das tarefas assíncronas, canal limitado,
cancelamento e espera de cleanup antes de sair. Guard de terminal com hook
de panic restaura raw mode, cursor, mouse, paste e alternate screen. SIGTERM
também solicita encerramento ordenado. Há um probe sem provider/estado real
para reproduzir saída normal e panic em PTY.

Etapa 6: a composição interativa exige aprovação do comando bash antes do
Docker, vinculada a argumentos, sessão, workspace e cancelamento. Chamadas
diretas não contornam a autorização quando o gate está configurado. Os
runners fixos de teste/tipos mantêm a política isolada do M5. Teste com
driver simulado confirma que nenhum container inicia antes do Y.

Etapa 7: CLI interativa ligada a StoreJournal/Agent/providers e aos registros
reais de ferramentas; carrega a sessão recente por workspace e oferece
nova sessão, modelos e retomada explícita. /undo usa MutationRegistry local.
Uso total soma todas as páginas persistidas, mesmo quando só as mensagens
recentes são desenhadas. --demo usa provider fictício sem rede/credenciais,
mas mantém SQLite, checkpoint, autorização e undo reais em temporários
descartáveis. Testes cobrem duas escritas, cancelamento, PLAN, escopo e uso.
tempfile agora também atende essa demonstração isolada na CLI; tokio e
serde_json reutilizam dependências existentes para tarefas e contratos.

Etapas 8–9: após um panic de worker a TUI encerra sem redesenhar sobre o
terminal já restaurado, aguardando as demais tarefas. A demonstração envia
nome de arquivo antes do conteúdo para permitir observar o crescimento do
painel. Regressões adicionais cobrem rolagem isolada de aprovação (teclas e
mouse), limite Unicode de 80 mil unidades, alvos das ações e associação de
resultado ao painel correto: uma prévia nunca significa arquivo aplicado.

### Fechamento M6

Implementação, comandos e roteiro manual estão em [tui.md](tui.md). O
usuário autorizou commits/push/PR/merge neste fluxo; por isso não se repete
a solicitação de autorização prevista no fechamento padrão.

Gates locais do fechamento, em Linux/Rust 1.99.1: cargo fmt --all --check,
cargo check --locked --workspace --all-targets,
cargo clippy --locked --workspace --all-targets -- -D warnings e
cargo test --locked --workspace passaram. Resultado: **193 testes
aprovados, zero falhas e um teste Docker real ignorado**. O teste ignorado
não conta como validação Docker.

| Item | Verificado | Limite |
| --- | --- | --- |
| Layout Grafite | PLAN/BUILD no cabeçalho sem Tab repetido; indicador lateral na entrada; uma barra de 10M; retângulos sucessivos por arquivo | A imagem de referência anterior não estava disponível; o layout segue a descrição aprovada. |
| Telas/estado | TestBackend e cinco snapshots insta; home, nova sessão, sessão, aprovação e diálogo de tema; NO_COLOR e resize | Não equivale a comparação direta com o legado. |
| Aprovações/foco | Y/A/N e M específico; Enter/Tab nunca aprovam; diálogo prioritário; setas/Pg/mouse; prévia limitada | Bash com driver Docker simulado; nenhum container real executado. |
| Core integrado | Provider fictício com Agent, SQLite, duas escritas aprovadas, PLAN, cancelamento, uso e /undo reais em temporários | Nenhum provider externo/chave real utilizado. |
| Terminal Linux | Interação em PTY a 80×24 e smoke a 120×40; preview incremental, Enter/Tab bloqueados, Y, dois arquivos, /undo, temas/modelos/agents/conta/sessões, nova sessão, Esc/Ctrl+C, resize e SIGTERM | Terminal cloud/PTY, não uma sessão gráfica de Windows Terminal. |
| Restauração | Probe normal e panic intencional: flags termios iguais antes/depois, cursor mostrado e alternate screen encerrada | SIGKILL/encerramento forçado do SO não executa cleanup. |
| CI multiplataforma | [Workflow 39668919756](https://github.com/mrJoao28/My-Agent/actions/runs/39668919756): fmt, check, Clippy e testes passaram em Windows, Linux e macOS no commit d775cca | CI do fechamento deve ser consultado no [PR #8](https://github.com/mrJoao28/My-Agent/pull/8). |
| Windows Terminal/PowerShell | Não executado manualmente neste ambiente | Roteiro reproduzível com --demo em tui.md; verificação visual/teclas do terminal do usuário permanece pendente. |

Os IDs da matriz descrevem toda a paridade histórica, não apenas este
milestone. UI-01/03/05 estão implementados no escopo solicitado, com Grafite
padrão por escolha do usuário. UI-02 permanece parcial quanto a sugestões
de diretórios (a TUI sugere arquivos seguros); UI-04/07 e CHATGPT dependem da
autenticação real do M7. UI-06 permanece parcial na paridade histórica de
duração/status; a barra de 10M e o modo em execução estão implementados.
CLI resume --all/doctor/exec e edição de configurações continuam nos seus
milestones, sem serem anunciados como disponíveis na TUI.

### Revisão corretiva da TUI — 21/10/2026

Trabalho no checkout atual, sem criar branches, commits, push ou PR. Nenhum
crate novo. Integração em crates/cli e regras de aprovação/undo revisadas;
o caminho completo continua sendo a identidade do workspace no backend.

- Cabeçalho mostra somente o nome do diretório, incluindo caminhos Windows,
  UNC e Unix; raízes recebem um rótulo curto.
- Entrada com cinco linhas externas fixas e rolagem interna. Cursor e texto
  compartilham a quebra por grafemas; corrigida também a divergência quando
  uma linha ocupa exatamente a largura disponível antes de uma quebra.
- `/` é preservado em teclado/paste, com modificadores e AltGr. O filtro de
  Ctrl/Alt descartava caracteres; o despacho de qualquer texto iniciado por
  `/` como comando consumia caminhos/mensagens comuns. Agora Enter executa
  apenas comandos exatos, Tab completa e Esc fecha sugestões sem apagar.
- Respostas têm moldura e modo de origem retido no streaming e no histórico;
  mudar PLAN/BUILD para a próxima solicitação não recolore mensagens antigas.
  Erros e painéis sucessivos por arquivo permanecem distintos, com NO_COLOR.
- Home reorganizada e aprovação centralizada horizontalmente, limitada a
  100 colunas, com escopo/ação roláveis e controles separados da prévia.
  Nenhum efeito foi adicionado antes da autorização. AltGr não aprova.
- Sugestões ficam limitadas ao histórico, preservando a entrada ao reduzir
  o terminal. Mantidas uma única barra cumulativa de 10M e indicação lateral.

Regressões adicionadas em focus.rs, primitives.rs e rendering.rs: entrada
normal e paste, CRLF, URLs/caminhos, Ctrl+J, AltGr, fechamento/completação do
menu, cursor Unicode, geometria estável, resize, rótulos de workspace, modos
persistidos/streaming e cores/rótulos sem cores. Cinco snapshots TestBackend
foram revisados como células de terminal antes da atualização.

Validação final desta revisão no Windows: `cargo fmt --check` e
`cargo clippy --all-targets -- -D warnings` passaram. A suíte completa passou
com `cargo test --locked --workspace -- --test-threads=1`: **201 testes
aprovados, zero falhas e um teste Docker real ignorado**, que não conta como
aprovação. Os 25 testes da TUI passaram com os cinco snapshots revisados,
sem opções de aceitação automática na rodada final.

Na rodada paralela, dois testes de Git em mutations.rs retornaram Error::Git
durante commits fictícios. Ambos passaram isoladamente e na rodada completa
em série; a causa da intermitência não foi comprovada. Não foram alterados
os testes nem o código de Git para mascarar as falhas. Falhas intermediárias
LNK1104 e acesso negado ao executar/substituir binários ocorreram no ambiente
Windows. A demonstração iniciada para tentar o teste interativo foi encerrada
antes da rodada final, liberando myagent.exe. Não houve alterações em
crates/cli nem nas políticas de autorização/undo.

Os screenshots anexados e o mockup Grafite não estavam disponíveis nesta
conversa nem no repositório. Não houve comparação visual com essas imagens.
O documento legado APROVACOES_E_UNDO.md estava acessível e foi lido.
Ubuntu/WSL foi consultado, mas não possui cargo/rustc; testes Linux pendentes.
A tentativa de terminal interativo Windows não retornou sessão controlável;
Windows Terminal/PowerShell com teclado ABNT2 físico permanece sem validação
manual. O CI existente cobre Windows/Linux/macOS, mas não foi disparado nesta
revisão. Testes ignorados não contam como aprovados.

### Integração OAuth e CLI — 21/10/2026

Implementação no checkout atual, preservando as correções e snapshots do M6,
sem branch, commit, push ou merge. O fluxo segue a documentação oficial de
[login OSS](https://developers.openai.com/siwc/token-sharing-open-source/sign-in),
[registros e sessões](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions),
[modelos e inferência](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)
e [tokens](https://developers.openai.com/siwc/token-sharing-open-source/token-reference).
Detalhes operacionais, dependências justificadas e códigos de saída estão em
[oauth-cli.md](oauth-cli.md).

- CLI e TUI usam o mesmo serviço OAuth e o mesmo backend de geração.
  `/chatgpt` deixou de ser placeholder: oferece registros separados,
  ativação, reautorização e logout. O login apresenta uma URL para abertura
  manual, espera o callback loopback e permite cancelar com Esc/Ctrl+C.
  Eventos de login cancelado não sobrescrevem uma operação posterior.
- Client_id emitido, host opaco e identidade validada por issuer/subject são
  persistidos por registro. Email não é chave de conta. State, nonce,
  PKCE S256, assinatura JWKS, audience, expiração e consumo único do callback
  são verificados; o registro emitido é retido antes da troca do código.
- ModelRegistry recebe apenas o catálogo visível da conta ativa. A API
  pública Responses recebe `store=false` e streaming; conclusão exige
  evento terminal válido. Falta de permissão/limite do plano gera estado
  explícito e exige escolha; não existe fallback automático de modelo,
  conta ou API key.
- Store ganhou persistência privada separada do SQLite de sessões, com
  proteção owner-only, rejeição de links/junctions/hardlinks, substituição
  atômica e lock estável entre processos. Refresh e gravação conjunta de
  tokens/validade ocorrem sob o mesmo lock. Logout tenta revogação remota
  antes de limpar o registro local e informa quando não há confirmação.
- Gerações ficam vinculadas à conta; troca/logout em outra instância são
  observados para cancelamento pelo backend. PLAN continua filtrando as
  ferramentas no core. Não foram ampliadas políticas de aprovação, undo,
  checkpoints, leases ou isolamento por workspace.
- `exec` cria sessão e usa o pipeline existente, com PLAN padrão, políticas
  explícitas, JSONL e saídas 0/1/2/3/130. `doctor` verifica estado gravável,
  SQLite, configuração, sandbox e contas, com JSON e saídas 0/1/2/130.
  Diagnóstico detalhado de ferramentas dentro da imagem e abertura automática
  do navegador permanecem lacunas da paridade histórica CLI-03/UI-07.

Arquivos principais: `crates/providers/src/chatgpt.rs` e seus testes,
`registry.rs`, `http.rs`, `wire.rs`; `crates/store/src/private.rs` e
`tests/private.rs`; `crates/cli/src/commands.rs`, `main.rs`, `backend.rs` e
`tests/cli.rs`; ports/app/runtime/render e `tests/focus.rs` da TUI;
`AgentError` do core, manifests, lockfile e documentação. Os snapshots do M6
não foram alterados nesta integração.

Validação efetivamente concluída no Windows/PowerShell:

- `cargo fmt --check`: passou.
- `cargo clippy --all-targets -- -D warnings`: passou.
- `cargo test --locked --workspace -- --test-threads=1`: passou com
  **217 testes, zero falhas e um teste Docker real ignorado**. Esse teste
  ignorado não conta como aprovação. A rodada inclui os cinco snapshots
  existentes, sem atualizar nem aceitar snapshots automaticamente.
- Servidor OAuth/JWKS fictício local: state, nonce, audience, issuer,
  subject divergente, token expirado, callback repetido/Host incorreto,
  cancelamento antes/durante a troca, contas com mesmo email, permissão
  ausente, logout e invalidação por 401/invalid_grant; dois processos
  disputando refresh produzem uma única rotação. Streaming parcial/EOF,
  erro, recusa do plano, MIME inválido e conclusão válida foram exercitados
  sem fallback ou repetição automática da inferência.
- Arquivos temporários fictícios: ACL privada Windows, recusa de ACL ampla,
  lock cancelável, atualização atômica, JSON inválido preservado e hardlink
  recusado. CLI pública: PLAN, BUILD com aprovação/negação, JSONL, limites
  e doctor offline. A TUI cobre cancelamento do diálogo sem enviar mensagem,
  além das regressões anteriores de `/`, paste, Unicode, modos e geometria.

Os builds de validação usaram `CARGO_PROFILE_TEST_DEBUG=0`,
`CARGO_PROFILE_DEV_DEBUG=0` e `CARGO_INCREMENTAL=0`, apenas no ambiente dos
comandos, por falta de espaço em disco. Não houve alteração de perfis no
projeto. A falta de espaço interrompeu uma rodada intermediária; os dois
arquivos de testes afetados foram restaurados e passaram na rodada final.
Foram removidos apenas caches incrementais e símbolos PDB gerados, com
autorização, e encerrada uma demonstração anterior que bloqueava myagent.exe.
O erro de ACL encontrado nos testes foi corrigido usando explicitamente
SE_FILE_OBJECT e aceitando a representação canônica P/PAI da DACL protegida,
sempre com apenas uma ACE do usuário atual. LNK1104 transitório exigiu repetir
o gate completo; a rodada final terminou com código zero.

Não houve login, inferência ou revogação com conta real, nem leitura de
credenciais existentes. Os testes usam somente temporários e valores
fictícios. Não houve verificação manual de Windows Terminal/ABNT2, Linux ou
macOS nesta integração. WSL previamente consultado não tinha cargo/rustc;
o CI existente de três sistemas foi preservado e não foi disparado. As
imagens/mockup continuam indisponíveis. A validação automatizada Windows
não substitui essas verificações externas/manuais.

### M8 — distribuição npm preparada — 22/10/2026

Foram lidos no legado, somente leitura, `.github/workflows/release.yml`,
`docs/PREPARACAO_PUBLICACAO_NPM.md` e `docs/CLI_GLOBAL.md`. Reaproveitadas as
lições de igualdade tag/versão, provenance/Trusted Publishers, tarball real
e prefixo temporário com espaços. Nenhum artefato, configuração privada ou
dependência instalada do legado foi copiado.

Implementado `@j2008/myagent` com shim Node pequeno e cinco pacotes opcionais
por plataforma, com os/cpu e versões exatas. O shim executa diretamente o
binário existente, sem shell, herdando terminal, cwd, ambiente e argumentos.
Preserva saída e trata sinais, com a distinção necessária entre POSIX e o
console Windows. Não há compilação, downloads ou scripts de instalação.
Nenhum crate novo ou alteração de implementação Rust foi necessário.
PLAN, OAuth, Store, aprovações, checkpoints, leases e o layout M6 permanecem
no mesmo binário e arquitetura já existentes.

O workflow release.yml prepara builds nativos Windows x64, Linux musl
x64/arm64 e macOS x64/arm64. Inspeciona formato/arquitetura, rejeita ELF com
interpretador dinâmico, gera hashes e tarballs allowlisted. Os smokes usam
registry local sem proxy, cache vazio e os seis tarballs reais, para resolver
as optionalDependencies inéditas sem alterar manifests. Cinco relatórios de
execução, receipts e checksums são exigidos antes da publicação. Há ainda
um smoke Linux x64 em imagem Node sem Rust instalado.

Cargo.toml, Cargo.lock, seis manifests npm, optionalDependencies exatas,
tag vVERSION e --version são conferidos em conjunto. O helper version.mjs
atualiza versões de forma coordenada; o teste faz isso numa cópia temporária.
O principal é empacotado em Linux e deve manter modo executável no tarball.
Os arquivos JS/MJS agora têm eol=lf explícito para preservar o shebang.

Publicação foi apenas implementada e testada com callbacks fictícios.
O job com OIDC exige push de tag, gates completos e environment npm-release;
workflow_dispatch nunca publica. O publisher confere integridades existentes
antes de qualquer escrita, publica plataformas ausentes primeiro, confirma
disponibilidade e reconfirma o conjunto antes do principal, com --provenance.
Versões existentes idênticas são reutilizadas; divergência bloqueia tudo.
Não tenta sobrescrever versões nem tratar erro de rede como E404.

O [guia de distribuição](npm-release.md) contém os seis cadastros Trusted
Publisher, revisão obrigatória do environment, bootstrap manual eventual,
comandos de validação e autorizações separadas para commit/push, criação e
push de tag e publicação. Documenta recuperação parcial com os artefatos
originais, sem re-publicar versões existentes ou mover tags. README inclui
`npm i -g @j2008/myagent` após publicação autorizada.

Validações realmente executadas em Windows x64/PowerShell, Node 26.8.1 e
npm 11.19.0:

| Verificação | Resultado |
| --- | --- |
| `cargo fmt --check` | Passou |
| `cargo clippy --all-targets -- -D warnings` | Passou |
| `cargo test --locked --workspace -- --test-threads=1` | 217 aprovados, zero falhas, um teste Docker real ignorado |
| `cargo build --locked --release -p myagent-cli` | Build otimizado Windows x64 concluído |
| `node --test scripts/npm/launcher.test.mjs` | 8 aprovados, 2 testes POSIX ignorados no Windows |
| Sintaxe JS e YAML | node --check e parser YAML passaram |
| actionlint 1.7.12 | Passou para release.yml; shellcheck/pyflakes desabilitados; binário obtido da release oficial e SHA-256 conferido |
| Versões | Rust/npm/tag v0.1.0 conferidos; --version instalado retornou myagent 0.1.0 |
| npm pack real | Principal 2.615 bytes; Windows x64 6.366.310 bytes; quatro arquivos por tarball, hashes e PE conferidos |
| Instalação inédita simulada | npm global por nome/versão em registry local, cache vazio e prefixo com espaços; manifests não reescritos; ambos os tarballs efetivamente baixados |
| Comando instalado | myagent.cmd --version, doctor --json com SQLite/estado gravável e exec PLAN offline passaram fora do checkout |
| Isolamento do smoke | Home, npmrc e cache temporários; rustc/cargo/Bun inacessíveis no PATH; sem credenciais reais |
| Launcher | Argumentos com espaços/aspas/metacaracteres/Unicode, stdin/stdout/stderr, cwd, ambiente, códigos 0/1/2/3/130 e erros de pacote passaram |
| Política de publicação fictícia | Ordem, confirmação antes do principal, retomada idempotente, integridade divergente, pacote ausente e versão não visível cobertos sem publicar |

Tarballs e recibos locais ficam em release/local-test, ignorado pelo Git.
SHA-256 do principal:
`b988c113c9d13c3c6d74c08272b9170b39b5966b47dd1a1f75c2cc74eddfbb58`.
SHA-256 do pacote Windows:
`0e4ade9290d8e1e7eceaa0478292259f9d72ae2fa9849c087f99353ee0e7d14d`.
Esses são os tarballs locais Windows; o principal de produção será empacotado
em Linux e testado novamente em toda a matriz, incluindo permissões POSIX.

Houve bloqueios de cache npm e execução de .cmd na sandbox; a repetição
autorizada passou. Corrigidas suposições do harness sobre hoisting da
optionalDependency, quoting do .cmd em caminho com espaços e modo 0644 do
npm pack Windows. LNK1104 em executáveis antigos impediu rodadas Rust;
a rodada final passou em target/m8-gates, com dois jobs de build e testes
em série. Foram usados apenas no ambiente CARGO_PROFILE_TEST_DEBUG=0,
CARGO_PROFILE_DEV_DEBUG=0 e CARGO_INCREMENTAL=0. Símbolos PDB e cache
incremental antigos foram removidos com autorização para liberar disco.

Pendente: executar o workflow remoto nas cinco plataformas e no container
sem Rust, sinais POSIX reais, TTY/Ctrl+C manual Windows, cadastro dos Trusted
Publishers, autenticação/provenance e instalação a partir do npm público.
Não houve build cruzado nem execução Rust Linux/macOS nesta entrega. A
matriz configurada não é evidência de execução. Não houve publicação,
criação/push de tag, commit, push, merge ou criação de branch. Arquivos novos
foram incluídos somente com intenção de adição para permitir revisão do diff.

### Correção dos gates Windows/macOS — 22/10/2026

O [CI de main 39763260874](https://github.com/mrJoao28/My-Agent/actions/runs/39763260874)
passou em Linux/macOS e falhou em Windows em oito testes OAuth, com
AgentError::Persistence durante start_login. Compilação, formatação e Clippy
Windows passaram; portanto, não foi uma falha de compilador Rust nesse run.

A investigação reproduziu um defeito na validação de ACL: a comparação
textual exigia o SID numérico de whoami, enquanto a serialização SDDL pode
usar um alias para a mesma identidade, como SY, LS, NS ou LA. A documentação
de [SID strings da Microsoft](https://learn.microsoft.com/en-us/windows/win32/secauthz/sid-strings)
descreve as duas representações. Agora dono e destinatário da ACE são
comparados pelo valor do SID. Continuam obrigatórios DACL protegida, uma
única ACE de permissão, acesso total exclusivo desse SID e ausência de
flags de herança. DACL nula/vazia é recusada antes do accessor da dependência,
que pode entrar em panic para uma DACL nula.

Regressões em memória cobrem aliases e SIDs numéricos, inclusive administrador
local, e recusam dono/destinatário diferente, Everyone adicional, ACL sem
proteção, nula/vazia, direitos incompletos, ACE de negação e herança. Nenhum
arquivo real de credenciais ou proprietário de arquivo do sistema foi
alterado. A fixture OAuth inicializa o armazenamento privado explicitamente
para expor um eventual erro de infraestrutura com sua causa IO no teste,
em vez de apenas Persistence. A mensagem de produção continua sanitizada.

No [workflow de distribuição 39763188633](https://github.com/mrJoao28/My-Agent/actions/runs/39763188633),
o gate Rust macOS passou, mas um teste Node falhou por comparar literalmente
/var/folders/... com /private/var/folders/... . A asserção agora compara os
caminhos físicos; o launcher não muda de cwd. Os dois testes POSIX de sinais
passaram naquela execução anterior, sem que isso valide ainda esta revisão.

Formatação, Clippy e os oito testes Node aplicáveis passaram localmente no
Windows; dois testes POSIX foram ignorados e não contam como aprovados.
A suíte Rust completa passou localmente no Windows: 219 testes aprovados,
nenhuma falha e um teste Docker real ignorado, que não conta como aprovado.
Comando: cargo test --locked --workspace -- --test-threads=1. Houve LNK1104
local em executáveis antigos, distinto da falha de persistência do CI;
a execução aprovada usou target/windows-acl-ci com símbolos/incremental
desativados, sem alterar os perfis do projeto.

O novo CI Windows/macOS ainda precisa executar esta revisão para confirmar
o resultado nos runners originais. Os logs antigos não expõem a causa IO de
Persistence; a correção do defeito de representação é comprovada por
regressão local, mas a resolução daquele erro remoto depende dessa rodada.
Nenhum gate foi removido, teste ignorado para contornar falha ou proteção
de permissões relaxada. Sem commit, push, tag ou publicação desta revisão.

### Correção do login ChatGPT na TUI — 22/10/2026

O menu `/` agora executa com Enter a sugestão escolhida pelas setas. Antes,
selecionar `/chatgpt` com o texto ainda em `/` enviava esse texto ao modelo,
produzindo um erro de API key em vez de abrir a conta. Digitar `/` sem navegar
continua preservando texto literal. Sem contas registradas, `/chatgpt` inicia
diretamente o login; contas existentes mantêm seu seletor.

O painel de login tem URL somente leitura, rolagem, abertura automática do
navegador, Enter/O para tentar abrir novamente e C para copiar a URL inteira.
Integrações: xdg-open/gio e wl-copy/xclip/xsel no Linux, wslview/PowerShell
no WSL, open/pbcopy no macOS e PowerShell no Windows. Sem clipboard nativo,
OSC 52 solicita a cópia ao terminal sem afirmar que ela foi aceita. Esc e
Ctrl+C cancelam o listener e descartam resultados atrasados. Falhas mantêm
um diálogo com nova tentativa, sem confundir autenticação ChatGPT com API key.

Links são validados antes da execução, passados como argumento ou stdin e
nunca interpolados em código de shell. Debug oculta a URL efêmera. As crates
url e base64 já estavam no lockfile; agora a TUI as declara diretamente para
validar a origem do link e codificar OSC 52, respectivamente. Nenhuma versão
nova foi introduzida. Não foram alterados OAuth/PKCE, providers ou credenciais.

Validação local Linux: fmt, check all-targets, Clippy com warnings negados e
suíte completa aprovada (225 testes; um teste Docker real ignorado). Após
ajustes finais de edição/Clone, check, Clippy e os 34 testes da TUI foram
executados novamente e passaram. Snapshots/TestBackend cobrem painel pequeno,
links longos, rolagem e prioridade do teclado.

Um smoke test em PTY Linux com estado temporário e helpers simulados verificou
o percurso `/` → seis setas → Enter, abertura, cópia byte a byte, nova abertura,
callback OAuth sintético de recusa, falha dos helpers, fallback OSC 52,
cancelamento, nova tentativa e restauração do terminal ao sair. Não houve
login real, troca de tokens ou inferência. Navegador/clipboard reais no Ubuntu,
Windows Terminal e macOS ainda exigem validação manual. O CI dos três sistemas
deve validar esta revisão após o envio. Correção diretamente na main conforme
autorização expressa do usuário; sem tag nem publicação npm.
