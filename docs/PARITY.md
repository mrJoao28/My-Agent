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

Nenhuma funcionalidade do assistente foi portada neste bootstrap; o crate-base
é vazio e não tem dependências externas. Não se exige reproduzir a arquitetura
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
| MODE-01 | PLAN/BUILD | PLAN recebe apenas ferramentas sem mutação do projeto; BUILD acrescenta escrita, shell e Git de alteração. A lista e a política no backend garantem a restrição. | `S/tools/index.ts`, `S/security/permissions.ts` | pendente | `S/security/permissions.test.ts`, `S/security/docker-availability.test.ts` |
| MODE-02 | PLAN/BUILD | CUI inicia em BUILD, isola modo por sessão e restaura o último modo persistido; modelo selecionado é compartilhado. Modo em execução permanece distinguível da próxima mensagem. `exec` inicia em PLAN. | `C/providers/prompt-config/index.tsx`, `C/screens/session.tsx`, `C/components/session-shell.tsx`, `M/options.ts` | pendente | `C/dialogs/agents-dialog.test.tsx`, `C/components/session-shell.test.tsx`, `M/options.test.ts`, `M/exec.test.ts` |
| PROMPT-01 | Prompts | Builder por modo, workspace, ferramentas efetivamente disponíveis e sandbox; seis blocos de instruções; perfis compacto/completo. Não anunciar capacidades indisponíveis nem tratar texto externo como permissão. | `S/prompts/index.ts`, `S/prompts/system.ts`, `S/prompts/coding.ts`, `S/prompts/editing.ts`, `S/prompts/tool-rules.ts`, `S/services/context-plan.ts` | pendente | `S/prompts/prompts.test.ts`, `S/services/context-plan.test.ts`, `S/services/message-format.test.ts` |
| PERM-01 | Aprovações | Y permite uma vez, A memoriza por sessão/ferramenta/workspace/escopo, N nega. Enter não aprova; Esc/Ctrl+C interrompem. Prévia rolável de diff, caminho, comando ou URL. | `S/security/permissions.ts`, `S/security/approval-scope.ts`, `S/security/tool-policy.ts`, `C/components/approval-panel.tsx`, `C/components/approval-preview.ts` | pendente | `S/security/permissions.test.ts`, `S/security/approval-scope.test.ts`, `C/components/approval-panel.test.tsx` |
| PERM-02 | Aprovações | Timeout de 120 s, consumo único, isolamento e limpeza ao cancelar/desconectar; até 256 escopos. A é proibido para delete_file, git_commit, git_init, git_remote_add e git_push. Ferramentas desconhecidas são negadas. | `S/security/permissions.ts`, `H/schemas.ts` | pendente | `S/security/permissions.test.ts`, `S/security/approval-scope.test.ts`, `H/schemas.test.ts` |
| PERM-03 | Automação | `none` autoriza leitura local; `reads` acrescenta web; `all` autoriza ações. Push exige opção separada e não autoaprova main/master. Comandos continuam dependentes de Docker e do modo. | `S/security/permissions.ts`, `M/options.ts`, `M/exec.ts` | pendente | `M/exec.test.ts`, `S/tools/git-workflow.test.ts`, `S/tools/test-tools.test.ts` |
| GEN-01 | Streaming | Texto, reasoning, chamadas/resultados associados por ID, avisos, uso e estados terminais estruturados; contratos validados; UTF-8/CRLF fragmentados e EOF sem done não equivalem a sucesso. | `H/schemas.ts`, `S/services/stream-service.ts`, `C/lib/chat-events.ts`, `C/hooks/use-chat.ts`, `M/exec.ts` | pendente | `H/schemas.test.ts`, `S/services/stream-service.test.ts`, `C/lib/chat-events.test.ts`, `C/hooks/use-chat.test.tsx`, `M/exec.test.ts` |
| GEN-02 | Ciclo da geração | Uma geração ativa por sessão; lock antes de awaits; diferenciar erro, conclusão e interrupção; persistir parciais, propagar cancelamento e aguardar cleanup no encerramento. Falha no SSE terminal não duplica gravação. | `S/services/chat-service.ts`, `S/services/stream-service.ts`, `S/index.ts` | pendente | `S/services/chat-recovery.test.ts`, `S/services/stream-service.test.ts`, `S/launcher.test.ts` |
| GEN-03 | Limites de tools | Orçamento padrão 40 chamadas, configurável até 200, verificado antes do executor; aviso perto do limite; timeout global de 5 min. Argumentos equivalentes são canonizados; resultado visível evita repetição; repetição persistente interrompe. | `S/services/tool-service.ts`, `S/services/stream-service.ts` | pendente | `S/services/tool-service.test.ts`, `S/services/tool-guard.test.ts`, `S/services/stream-service.test.ts` |
| GEN-04 | Evidência no contexto | Releituras voltam a ser permitidas quando evidência é removida/abreviada; edição invalida leitura anterior; nota de undo invalida evidências sem ampliar permissões ou orçamento. | `S/services/tool-service.ts`, `S/services/session-context.ts` | pendente | `S/services/tool-guard.test.ts`, `S/services/stream-service.test.ts`, `S/services/session-context.test.ts` |
| TASK-01 | Tarefas | `todo_write` mantém itens e status em memória, separados por sessão, com limpeza; não é plano de execução durável nem coordenação de múltiplos agentes. | `S/tools/todo-write.ts`, `S/services/session-lifecycle.ts` | pendente | `S/tools/files.test.ts`, `S/services/session-lifecycle.test.ts` |

## Ferramentas de arquivos, inspeção e processos

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| READ-01 | Leitura | `read_file`: até 5 MiB, páginas de linhas inteiras, offset zero-based, numeração opcional, saída até 10 mil caracteres, nextOffset/continuation; erro LINE_TOO_LONG sem saltar o meio da página. | `S/tools/read-file.ts` | pendente | `S/tools/files.test.ts`, `S/tools/file-editing.test.ts`, `S/tools/io-cancellation.test.ts`, `S/services/session-context.test.ts` |
| READ-02 | Leitura em lote | `read_files`: 1–8 arquivos, até 1 MiB cada/4 MiB no lote/30 mil caracteres, seleção de linhas e sucesso parcial; recusa binários e caminhos protegidos. | `S/tools/read-files.ts` | pendente | `S/tools/inspection.test.ts` |
| READ-03 | Diretórios | `list_directory`: paginação, limites e cancelamento; omissão de entradas sensíveis e links; continuidade pressupõe diretório inalterado. | `S/tools/list-directory.ts` | pendente | `S/tools/files.test.ts`, `S/tools/io-cancellation.test.ts` |
| FIND-01 | Glob | `glob`: padrões relativos, zero/múltiplos níveis e exclusões seguras; usa regras do .gitignore da raiz, inclusive em buscas por subpasta. | `S/tools/glob.ts`, `S/lib/search-ignore.ts` | pendente | `S/tools/files.test.ts`, `S/tools/search-filters.test.ts` |
| FIND-02 | Grep | `grep`: literal/regex, include, contexto de 0–20 linhas, files_only; limites de bytes/resultados/saída/tempo; regex em worker cancelável. | `S/tools/grep.ts`, `S/lib/grep-worker.ts`, `S/lib/search-ignore.ts` | pendente | `S/tools/grep.test.ts`, `S/tools/search-filters.test.ts`, `S/tools/files.test.ts` |
| FIND-03 | Ignores | Leitura limitada do .gitignore raiz, negações e poda de pais excluídos; não consulta ignores aninhados/globais nem índice Git. Negação não libera segredos, links ou dependências fixamente excluídas. | `S/lib/search-ignore.ts` | pendente | `S/tools/search-filters.test.ts` |
| INSPECT-01 | Projeto | `project_inspect`: manifests, scripts/dependências, configs e testes por inspeção estática com limites; não executa arquivos de configuração. | `S/tools/project-inspect.ts`, `S/lib/project-inspection.ts` | pendente | `S/tools/inspection.test.ts` |
| INSPECT-02 | Código | `file_outline`: declarações e linhas JS/TS/JSX/TSX por AST em worker. `symbol_search`: busca literal de declarações; não equivale a referências nem resolução de tipos. | `S/tools/file-outline.ts`, `S/tools/symbol-search.ts`, `S/lib/source-analysis.ts`, `S/lib/source-worker.ts` | pendente | `S/tools/inspection.test.ts` |
| INSPECT-03 | Ambiente | `environment_check`: diagnóstico passivo sem processos ou valores de ambiente; não comprova daemon/imagem/ferramentas da sandbox. | `S/tools/environment-check.ts` | pendente | `S/tools/inspection.test.ts` |
| WRITE-01 | Criação | `write_file`: escrita atômica, criação de pais, proteção contra sobrescrita e backup; modo existente preservado, novos arquivos 0644 em POSIX. | `S/tools/write-file.ts` | pendente | `S/tools/files.test.ts`, `S/tools/file-editing.test.ts` |
| WRITE-02 | Edição | `edit_file`: substituição única, contagem de ocorrências ambíguas/ausentes; compara LF/CRLF e usa EOL dominante no trecho inserido, preservando bytes restantes e modo. | `S/tools/edit-file.ts` | pendente | `S/tools/files.test.ts`, `S/tools/file-editing.test.ts` |
| WRITE-03 | Árvores | `move_file` e `delete_file`: mover/excluir com validação de descendentes, destino existente e raiz protegida. Exclusão sempre exige aprovação individual. | `S/tools/move-file.ts`, `S/tools/delete-file.ts`, `S/utils/path.ts` | pendente | `S/tools/files.test.ts`, `S/security/checkpoints.test.ts` |
| WRITE-04 | Patch | `apply_patch`: diff unificado, checkOnly e validação de todos os caminhos antes de alterar; recusa binários, links, rename/copy e caminhos com espaços. | `S/tools/apply-patch.ts` | pendente | `S/tools/git.test.ts`, `S/security/checkpoints.test.ts` |
| TEST-01 | Descoberta | `test_discover`: convenções JS/TS, Python e Go, scripts e indícios de runners, com inspeção limitada; não enumera casos gerados em runtime. | `S/tools/test-discover.ts`, `S/lib/project-inspection.ts` | pendente | `S/tools/inspection.test.ts` |
| TEST-02 | Execução | `test_run`: Bun, Node, Vitest, Jest e Pytest, runners fixos com argumentos escapados; cópia descartável em /tmp, dependências referenciadas somente leitura; timeout/cancelamento/saída limitada; não infere quantidade de testes pelo exit code. | `S/tools/test-run.ts` | pendente | `S/tools/test-tools.test.ts`, `S/security/sandbox.integration.test.ts` |
| TEST-03 | Tipos | `type_check`: diagnóstico TypeScript, caminhos Windows e linha/coluna; tsc local antes do global, somente na sandbox; disponível também em PLAN com Docker. | `S/tools/type-check.ts` | pendente | `S/services/message-format.test.ts`, `S/security/permissions.test.ts`, `S/security/docker-availability.test.ts` |
| SHELL-01 | Shell | `bash`: exclusivo de BUILD e Docker; timeout/saída/cancelamento; projeto somente leitura no container e saídas descartáveis em /tmp. Não há fallback de execução no host. | `S/tools/bash.ts`, `S/security/sandbox.ts` | pendente | `S/security/permissions.test.ts`, `S/security/sandbox.test.ts`, `S/security/sandbox.integration.test.ts` |

## Git, checkpoints, sandbox e rede

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| GIT-01 | Inspeção | `git_status`, `git_diff`, `git_log`, `git_show`, `git_blame`, `diff_summary`: escopo seguro, staged separado, histórico e atribuição de linhas; não expor caminhos protegidos. | `S/tools/git-status.ts`, `S/tools/git-diff.ts`, `S/tools/git-log.ts`, `S/tools/git-show.ts`, `S/tools/git-blame.ts`, `S/tools/diff-summary.ts`, `S/lib/git-inspection.ts` | pendente | `S/tools/git.test.ts`, `S/tools/git-inspection.test.ts`, `S/utils/git-scope.test.ts` |
| GIT-02 | Commit | `git_commit`: arquivos explícitos, prévia para aprovação individual e preservação de staged fora do escopo; identidade Git existente, sem inventar configuração local. | `S/tools/git-commit.ts`, `S/utils/git.ts` | pendente | `S/tools/git.test.ts`, `S/tools/git-workflow.test.ts`, `S/security/approval-scope.test.ts` |
| GIT-03 | Bootstrap/branches | `git_init` em main apenas fora de repositórios, `git_switch` cria/troca sem descarte/force, `git_remote_add` HTTPS validado sem substituir URL existente. | `S/tools/git-workflow.ts` | pendente | `S/tools/git-workflow.test.ts` |
| GIT-04 | Push/sincronização | `git_push` só da branch atual, URL/branch vinculadas à aprovação, sem force/delete/tags; main/master pedem confirmação adicional Y e M. Após push, parar até liberação do usuário. `git_sync` usa fetch sem tags e merge ff-only da principal, com workspace/index limpos. | `S/tools/git-workflow.ts`, `S/security/tool-policy.ts`, `S/services/stream-service.ts`, `C/components/approval-panel.tsx` | pendente | `S/tools/git-workflow.test.ts`, `S/services/stream-service.test.ts`, `C/components/approval-panel.test.tsx` |
| GIT-05 | Isolamento Git | Worktree explícita, ambiente reduzido; hooks/filtros/assinatura/pager/fsmonitor/diffs externos desativados; recusa metadata externa, links e alternates. Escritas Git serializadas. | `S/utils/git.ts`, `S/security/tool-policy.ts` | pendente | `S/utils/git-scope.test.ts`, `S/tools/git.test.ts`, `S/tools/git-inspection.test.ts`, `S/tools/git-workflow.test.ts` |
| UNDO-01 | Checkpoints | Checkpoint privado global por sessão antes de mutações de arquivos em BUILD; preserva bytes, EOL e modos; limites por arquivo/passo/store; inclui undo após falhas parciais. Não é commit Git nem snapshot de todo efeito externo. | `S/security/checkpoints.ts`, `S/security/tool-policy.ts` | pendente | `S/security/checkpoints.test.ts` |
| UNDO-02 | /undo | Desfaz último passo sem sobrescrever alterações posteriores; recusa conflito/geração ativa, valida workspace/lease, persiste nota para invalidar evidência anterior. Não cria commits. | `S/services/chat-service.ts`, `C/hooks/use-chat.ts`, `C/components/command-menu/commands.tsx` | pendente | `S/security/checkpoints.test.ts`, `S/services/chat-recovery.test.ts`, `C/components/approval-undo.test.tsx` |
| BOX-01 | Disponibilidade | Docker opcional para abrir o app; sem daemon não registrar bash/type_check/test_run. Verificação limitada, imagem padrão Bun, preparação/pull sob demanda e progresso, depois pull=never. | `S/security/sandbox.ts`, `S/index.ts`, `S/tools/index.ts`, `M/doctor.ts` | pendente | `S/security/docker-availability.test.ts`, `S/security/sandbox.test.ts`, `S/launcher.test.ts` |
| BOX-02 | Container | Rede desativada, rootfs e bind mount somente leitura, usuário não root, capabilities removidas, no-new-privileges, tmpfs privado e limites de CPU/memória/processos. Cancelamento/timeout encerram processos/container. | `S/security/sandbox.ts` | pendente | `S/security/sandbox.test.ts`, `S/security/sandbox.integration.test.ts` |
| BOX-03 | Snapshot | Cópia filtrada, sem metadata Git, segredos, links e builds; limites por arquivo/bytes/entradas/profundidade. Dependências omitidas por padrão; opt-in filtrado para node_modules/.venv e tratamento restrito de .bin. | `S/security/snapshot.ts` | pendente | `S/security/snapshot.test.ts`, `S/security/sandbox.integration.test.ts` |
| BOX-04 | Cache de snapshot | Reuso por sessão/workspace e invalidação por inventário/metadados/configuração, leases serializados, expiração ociosa e limite de entradas. Não é hash de conteúdo nem cache persistente entre processos. | `S/security/snapshot.ts` | pendente | `S/security/snapshot.test.ts` |
| WEB-01 | Busca | `web_search` via Tavily: aprovação, snippets estruturados, schema, deadline, limite durante leitura e cancelamento; falhas sanitizadas. | `S/tools/web-search.ts`, `S/lib/tool-io.ts` | pendente | `S/tools/web.test.ts`, `S/lib/tool-io.test.ts` |
| WEB-02 | Fetch | `web_fetch`: aprovação por origem completa, redirects não podem ampliar grant; extração de texto HTML, remoção de conteúdo ativo, entidades, limite de bytes/texto e cancelamento. | `S/tools/web-fetch.ts`, `S/security/approval-scope.ts` | pendente | `S/tools/web.test.ts`, `S/security/approval-scope.test.ts` |
| WEB-03 | SSRF | Bloqueio de destinos internos, IPs mapeados/IPv6 e DNS misto; conexão presa ao IP validado, preservando Host/TLS; sem segunda resolução permissiva. | `S/tools/web-fetch.ts` | pendente | `S/tools/web-ssrf.test.ts`, `S/tools/web.test.ts` |

## Persistência, sessões, contexto e instruções

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| DB-01 | SQLite | Banco local/libsql, WAL, busy_timeout e foreign_keys; abertura lazy, transações e fechamento explícito; não depende de PostgreSQL/DATABASE_URL em runtime. | `D/client.ts`, `D/index.ts`, `P/schema.prisma` | pendente | `D/database.integration.test.ts`, `D/migrations.test.ts` |
| DB-02 | Migrations | Runner SQL embutido, baseline e evoluções transacionais/idempotentes; enums/JSON/índices/cascatas. Prisma CLI não é requisito de runtime. | `D/migrations.ts`, `D/migrate.ts`, `P/migrations/20261016000000_sqlite_baseline/migration.sql`, `P/migrations/20261016010000_workspace_sessions/migration.sql`, `P/migrations/20261018000000_durable_generations/migration.sql` | pendente | `D/migrations.test.ts`, `D/database.integration.test.ts` |
| SESSION-01 | Workspace | Raiz Git/caminho canônico, caixa Windows, espaços/subpastas e recusa de junctions; título com caminho; criar ou selecionar sessão mais recente por workspace. | `S/utils/workspace.ts`, `S/services/workspace-sessions.ts`, `S/routes/sessions.ts` | pendente | `S/utils/workspace.test.ts`, `S/services/workspace-sessions.test.ts`, `S/routes/sessions.test.ts` |
| SESSION-02 | Histórico | Listagem por atividade, criação com mensagem inicial opcional, reabertura com mensagens/parts/status, contexto por sessão e validação de usuário local/workspace. Abrir/enviar/retomar renova atividade. | `S/routes/sessions.ts`, `S/services/message-service.ts`, `S/services/session-lifecycle.ts` | pendente | `S/routes/sessions.test.ts`, `S/services/session-lifecycle.test.ts`, `D/database.integration.test.ts` |
| SESSION-03 | Leases | Exclusão entre processos no SQLite por sessão, owner aleatório, heartbeat de 5 s e expiração de 30 s; perda encerra execução, fechamento libera lease. Locks em memória continuam distintos. | `S/services/session-lease.ts`, `S/index.ts` | pendente | `S/services/workspace-sessions.test.ts`, `S/launcher.test.ts`, `D/database.integration.test.ts` |
| SESSION-04 | Retenção | Padrão 90 dias, configurável e desativável com zero; aviso e carência mínima de 24 h; abrir cancela aviso. Limpeza horária não sobreposta, preserva leases ativos/gerações e remove artefatos. | `S/services/session-lifecycle.ts` | pendente | `S/services/session-lifecycle.test.ts`, `S/services/workspace-sessions.test.ts`, `D/database.integration.test.ts` |
| RECOVER-01 | Journal durável | ASSISTANT começa IN_PROGRESS; journal serializa chamadas/resultados e grava antes de efeitos e antes de devolver resultados ao modelo. Órfãs viram INTERRUPTED sem afetar leases vivos de outro processo. | `S/services/generation-journal.ts`, `S/services/stream-service.ts` | pendente | `S/services/generation-journal.test.ts`, `S/services/stream-service.test.ts` |
| RECOVER-02 | Retomada | Retomar USER pendente ou ASSISTANT interrompido com confirmação explícita na CUI; preservar generationId e resultados anteriores; bloquear replay de ações já iniciadas, inclusive efeito desconhecido sem resultado. | `S/services/chat-service.ts`, `S/services/generation-journal.ts`, `C/hooks/use-chat.ts`, `C/components/resume-panel.tsx` | pendente | `S/services/chat-recovery.test.ts`, `S/services/generation-journal.test.ts`, `S/services/stream-service.test.ts`, `C/components/resume-panel.test.tsx` |
| CTX-01 | Histórico estruturado | Reconstruir pares chamada/resultado e texto; chamadas incompletas viram avisos. Omitir reasoning de novos turnos, preservar metadados necessários durante geração viva. Histórico completo continua persistido. | `S/services/message-format.ts`, `S/services/session-context.ts`, `S/services/context-summary.ts` | pendente | `S/services/message-format.test.ts`, `S/services/session-context.test.ts` |
| CTX-02 | Janela do modelo | Resolver override, catálogo exato, snapshot datado, hint do provider e fallback local/cloud com origem/estimativa explícitas; preferências persistidas, validação de valores e catálogo sem corrupção silenciosa. | `S/lib/context-window.ts`, `S/lib/model-config.ts`, `H/models.ts` | pendente | `S/lib/model-context.test.ts`, `S/lib/private-config.test.ts`, `S/services/context-plan.test.ts`, `H/schemas.test.ts` |
| CTX-03 | Orçamento | Reservar sistema, schemas, opções do provider e saída; perfil essencial para janelas pequenas; validar e persistir snapshot antes de cada etapa e entregar ao modelo o snapshot retornado. Falha de persistência impede inferência. | `S/services/context-plan.ts`, `S/services/session-context.ts`, `S/services/stream-service.ts` | pendente | `S/services/context-plan.test.ts`, `S/services/context-plan.integration.test.ts`, `S/services/session-context.test.ts`, `S/services/stream-service.test.ts` |
| CTX-04 | Compactação | Ao atingir limiar de 70%, resumir evidência antiga e preservar turnos recentes; manter pares completos, abreviar só com indicação, read_file somente no fim de linhas; usuário atual grande demais é rejeitado, sem corte silencioso. | `S/services/session-context.ts`, `S/services/context-summary.ts` | pendente | `S/services/session-context.test.ts`, `S/services/context-plan.integration.test.ts` |
| USAGE-01 | Tokens consumidos | Acumular uso válido informado em cada etapa uma vez, persistir por mensagem inclusive em erro/interrupção e restaurar na sessão. Ausência de uso não vira estimativa nem zero fictício. | `S/services/stream-service.ts`, `C/hooks/use-chat.ts`, `C/lib/token-usage.ts`, `C/components/token-usage-bar.tsx` | pendente | `S/services/stream-service.test.ts`, `C/lib/token-usage.test.ts`, `C/components/token-usage-bar.test.tsx`, `C/hooks/use-chat.test.tsx` |
| AGENTS-01 | Instruções de projeto | AGENTS.md raiz a cada geração, subpastas conforme arquivos lidos/buscados/editados; precedência por escopo, irmãos isolados, cache imutável por geração e entrega única. | `S/lib/project-instructions.ts`, `S/services/context-plan.ts`, `S/tools/index.ts` | pendente | `S/lib/project-instructions.test.ts`, `S/services/stream-service.test.ts` |
| AGENTS-02 | Segurança das instruções | Limite total de 16 KiB incluindo envelope JSON/Unicode, 128 candidatos e profundidade 32; recusar links/binários/caminhos sensíveis; I/O somente após autorização. Instruções nunca ampliam permissões nem geram documentação automaticamente. | `S/lib/project-instructions.ts`, `S/security/tool-policy.ts` | pendente | `S/lib/project-instructions.test.ts`, `S/prompts/prompts.test.ts` |

## Providers, modelos e configuração

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| AI-01 | Anthropic | Resolver modelo configurado, chave por modelo e opções de thinking do catálogo. | `S/lib/providers/anthropic.ts`, `S/lib/providers/resolver.ts` | pendente | `S/lib/config.test.ts`, `S/services/context-plan.test.ts`; sem teste dedicado do adapter Anthropic identificado |
| AI-02 | OpenAI | Adapter OpenAI e opções de reasoning por modelo; resolução explícita, falha por modelo/chave ausente. | `S/lib/providers/openai.ts`, `S/lib/providers/resolver.ts` | pendente | `S/lib/config.test.ts`, `S/services/stream-service.test.ts`, `M/exec.test.ts` (provider simulado); sem chamada real validada |
| AI-03 | Google | Adapter Google com thinkingBudget por modelo, catálogo e chave por modelo. | `S/lib/providers/google.ts`, `S/lib/providers/resolver.ts` | pendente | `S/lib/config.test.ts`, `S/services/context-plan.test.ts`; sem teste dedicado do adapter Google identificado |
| AI-04 | Ollama | Endpoint compatível com OpenAI Chat Completions, URL/chave opcionais, modelos locais/customizados e janela configurável; não exige chave cloud. | `S/lib/providers/ollama.ts`, `S/lib/providers/resolver.ts` | pendente | `S/lib/config.test.ts`, `S/lib/model-context.test.ts`; sem inferência Ollama real validada |
| AI-05 | Plano ChatGPT | Login habilitado por padrão, desativação explícita; OAuth com PKCE, callback loopback, state/nonce, identidade assinada e registro de cliente; múltiplas contas, ativação/logout/revogação. | `S/lib/chatgpt.ts`, `S/lib/chatgpt-store.ts`, `S/routes/chatgpt.ts`, `C/dialogs/chatgpt-dialog.tsx` | pendente | `S/lib/chatgpt.test.ts` |
| AI-06 | Credenciais ChatGPT | Armazenamento privado/atômico, refresh serializado e cancelável; troca de conta interrompe geração; falha de grant invalida apenas conta afetada; não fallback para API key nem retry automático de inferência. Nenhum arquivo real de credenciais foi aberto. | `S/lib/chatgpt.ts`, `S/lib/chatgpt-store.ts`, `S/lib/providers/chatgpt.ts` | pendente | `S/lib/chatgpt.test.ts`, `S/lib/providers/chatgpt.test.ts` |
| AI-07 | Stream ChatGPT | Modelos descobertos por conta com IDs chatgpt:, instruções developer e tools no namespace do provider; validação de conclusão SSE, UTF-8 fragmentado e respostas JSON/HTML/incompletas/oversized com erros sanitizados. | `S/lib/providers/chatgpt.ts`, `S/lib/providers/chatgpt-stream.ts`, `S/lib/chatgpt.ts` | pendente | `S/lib/providers/chatgpt.test.ts`, `S/lib/chatgpt.test.ts` |
| MODEL-01 | Registry | Catálogo compartilhado de IDs/provider/pricing/contexto, default claude-opus-4-6; adição de modelos customizados, detecção de configuração e rejeição de duplicatas/IDs ChatGPT manuais. | `H/models.ts`, `S/lib/model-registry.ts`, `S/lib/model-config.ts`, `S/routes/models.ts` | pendente | `H/schemas.test.ts`, `S/lib/config.test.ts`, `S/lib/model-context.test.ts`, `S/lib/private-config.test.ts`, `C/dialogs/models-dialog.test.tsx` |
| CONFIG-01 | Configuração global | Diretório global estável, override de home e alias legado; ambiente prevalece sobre config; chave com nome determinístico por modelo, atualização atômica preservando outros valores; JSON inválido gera erro e não é apagado. | `H/home.ts`, `S/lib/env.ts`, `S/lib/private-config.ts`, `S/lib/model-config.ts` | pendente | `S/lib/config.test.ts`, `S/lib/private-config.test.ts`, `S/lib/chatgpt.test.ts` |

## CLI, TUI e API local

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| CLI-01 | Entrada | `myagent` retoma última sessão do workspace ou cria; `--new` força nova; launcher integra backend em porta livre e CUI com token efêmero; SIGINT/SIGTERM e cleanup. Bun/Node são detalhes do legado a substituir. | `M/cli.ts`, `packages/myagent/bin/myagent.js`, `S/index.ts`, `C/index.tsx` | pendente | `M/launcher.test.ts`, `M/options.test.ts`, `S/launcher.test.ts`, `scripts/workspace-lock.test.ts` |
| CLI-02 | Resume | `resume` escolhe sessão da pasta, `resume --all` permite outras pastas e troca workspace; seleção/aborto e validação. | `M/resume.ts`, `M/options.ts`, `S/services/workspace-sessions.ts` | pendente | `M/options.test.ts`, `S/services/workspace-sessions.test.ts`; sem teste dedicado da seleção interativa em resume.ts identificado |
| CLI-03 | Doctor | Diagnóstico de runtime, diretório gravável, SQLite/migrations, Docker/imagem, ferramentas detectadas versus imagem e configuração de modelos sem valores de chaves. O doctor legado pode escrever/migrar: não foi executado nesta inspeção. | `M/doctor.ts`, `M/doctor-tools.ts` | pendente | `M/doctor-tools.test.ts`, `S/security/docker-availability.test.ts`; sem teste completo do comando doctor identificado |
| CLI-04 | Headless | `exec` cria sessão nova, prompt até 9.000 caracteres, mode/model, JSONL opcional, permissões explícitas e allow-push; exits 0 sucesso, 1 falha, 3 negação e 130 cancelamento. | `M/options.ts`, `M/exec.ts` | pendente | `M/exec.test.ts` (inclui HTTP → geração → SDK → tools simulados) |
| CLI-05 | Ajuda/versão | `--help`/`-h`, `--version`/`-v`, rejeição de argumentos ambíguos/duplicados/inválidos; ajuda não abre banco nem providers. | `M/options.ts`, `M/cli.ts` | pendente | `M/options.test.ts`, `M/launcher.test.ts`, `M/exec.test.ts` |
| UI-01 | Telas | Home, nova sessão e sessão/histórico; shell compartilhado, título/caminho, mensagens usuário/assistente/erro, reasoning/ferramentas, avisos e interrupção visível. | `C/screens/home.tsx`, `C/screens/new-session.tsx`, `C/screens/session.tsx`, `C/components/messages/bot-message.tsx`, `C/components/session-shell.tsx` | pendente | `C/screens/session-render.test.tsx`, `C/components/session-shell.test.tsx`, `C/hooks/use-chat.test.tsx`, `scripts/cui-text.test.ts` |
| UI-02 | Entrada | Texto multilinha, Ctrl+J para quebra, Enter para envio, Tab para modo, autocomplete de menções @ a arquivos/diretórios e comandos /; limite de mensagem e bloqueio durante aprovação/retomada. | `C/components/input-bar.tsx`, `C/components/command-menu/use-command-menu.ts` | pendente | `C/components/session-shell.test.tsx`, `C/dialogs/agents-dialog.test.tsx`, `C/components/approval-panel.test.tsx`; sem suíte dedicada completa de menções identificada |
| UI-03 | Menu de comandos | `/undo`, `/sessions`, `/theme`, `/models`, `/agents`, `/exit`, `/new`; filtro, seleção e navegação. `/agents` alterna modos, não agentes autônomos. | `C/components/command-menu/commands.tsx`, `C/components/command-menu/filter-commands.ts`, `C/components/command-menu/index.tsx` | pendente | `C/dialogs/agents-dialog.test.tsx`, `C/dialogs/models-dialog.test.tsx`, `C/components/approval-undo.test.tsx`; sem teste dedicado de todo o menu identificado |
| UI-04 | Diálogos/foco | Sessões, modelos, modos/agentes, temas e contas ChatGPT; busca/lista, camadas de teclado, Enter/Esc/Tab/Ctrl+C sem acionar input de fundo; toast e erros de rota. | `C/dialogs/index.tsx`, `C/dialogs/sessions-dialog.tsx`, `C/dialogs/models-dialog.tsx`, `C/dialogs/agents-dialog.tsx`, `C/dialogs/theme-dialog.tsx`, `C/dialogs/chatgpt-dialog.tsx`, `C/providers/keyboard-layer/index.tsx`, `C/providers/dialog/index.tsx`, `C/components/route-error.tsx` | pendente | `C/dialogs/agents-dialog.test.tsx`, `C/dialogs/models-dialog.test.tsx`, `C/components/approval-panel.test.tsx`, `C/components/resume-panel.test.tsx`; cobertura não é completa para todos os diálogos |
| UI-05 | Temas | Paletas claras/escuras, Nightfox padrão, seleção e persistência global em preferences.json; falha de preferência não impede interface. | `C/theme.ts`, `C/providers/theme/index.tsx`, `C/dialogs/theme-dialog.tsx` | pendente | `C/components/token-usage-bar.test.tsx`, `C/components/session-shell.test.tsx`; sem teste dedicado de persistência do tema identificado |
| UI-06 | Uso/status | Barra de tokens acumulados com cores do tema e formatos k/M, escala visual de 1 milhão sem limitar consumo/contexto; status, duração, modo em execução e aviso de comandos indisponíveis. | `C/components/token-usage-bar.tsx`, `C/lib/token-usage.ts`, `C/components/status-bar.tsx`, `C/components/session-shell.tsx` | pendente | `C/lib/token-usage.test.ts`, `C/components/token-usage-bar.test.tsx`, `C/components/session-shell.test.tsx` |
| UI-07 | Navegador | Abrir URL de autenticação no navegador por plataforma com validação e diagnóstico de falha, sem shell arbitrário. | `C/lib/open-browser.ts`, `C/dialogs/chatgpt-dialog.tsx` | pendente | `C/lib/open-browser.test.ts` |
| API-01 | API local | Bind loopback, token privado/efêmero, negar Origin de navegador e Host externo/rebinding, limite de corpo 1 MiB e validação de workspace/proprietário. Não é serviço multiusuário. | `S/index.ts`, `S/security/auth.ts`, `S/security/workspaces.ts`, `H/local-auth.ts`, `C/lib/api-client.ts` | pendente | `S/security/auth.test.ts`, `S/routes/sessions.test.ts`, `S/launcher.test.ts`, `S/lib/chatgpt.test.ts` |
| API-02 | Contratos | Rotas locais de sessões, chat/retomada/undo, aprovações, modelos/chaves e ChatGPT; erros/validação HTTP e SSE. Preservar comportamento sem obrigar a mesma separação cliente/servidor em Rust. | `S/routes/chat.ts`, `S/routes/permissions.ts`, `S/routes/sessions.ts`, `S/routes/models.ts`, `S/routes/chatgpt.ts`, `C/lib/http-errors.ts` | pendente | `S/routes/sessions.test.ts`, `S/services/chat-recovery.test.ts`, `M/exec.test.ts`, `S/lib/chatgpt.test.ts`, `C/hooks/use-chat.test.tsx` |

## Segurança transversal, avaliações e distribuição

| ID | Área | Funcionalidade confirmada no código | Onde está no legado | Status | Testes legados correspondentes |
| --- | --- | --- | --- | --- | --- |
| SAFE-01 | Caminhos | Confinamento ao workspace; traversal, volumes/UNC, ADS, dispositivos Windows, pontos/espaços finais, ancestrais inexistentes, links/junctions e hard links recusados; raiz e descendentes protegidos. Não garante ausência de corrida contra outro processo local malicioso. | `S/utils/path.ts`, `S/security/workspaces.ts` | pendente | `S/utils/path.test.ts`, `S/utils/workspace.test.ts`, `S/tools/files.test.ts`, `S/tools/inspection.test.ts` |
| SAFE-02 | I/O limitado | Limites durante leitura, UTF-8 preservado entre chunks, cancelamento de body/processos/workers, deadline e liberação de timers/listeners/locks. | `S/lib/tool-io.ts`, `S/lib/bounded-json.ts`, `S/security/sandbox.ts`, `S/lib/source-analysis.ts` | pendente | `S/lib/tool-io.test.ts`, `S/tools/io-cancellation.test.ts`, `S/tools/inspection.test.ts`, `S/tools/web.test.ts`, `S/security/sandbox.test.ts` |
| SAFE-03 | Privacidade | Não expor valores de credenciais em erros/diagnóstico/resultados; escrita privada/atômica das configurações; arquivos sensíveis recusados nas ferramentas, busca, Git e snapshots. | `S/lib/private-config.ts`, `S/lib/logger.ts`, `S/utils/path.ts`, `S/security/auth.ts`, `S/security/snapshot.ts` | pendente | `S/lib/private-config.test.ts`, `S/security/auth.test.ts`, `S/security/snapshot.test.ts`, `S/tools/files.test.ts`, `S/tools/git.test.ts` |
| EVAL-01 | Tarefas | Nove fixtures verificáveis: leitura, corrigir operador, função com teste, refatorar CRLF, script executável, busca em 240 módulos, AGENTS.md, leitura de 300 linhas e 30 leituras sucessivas. Inventário completo e diff esperado, sem executar código gerado no host. | `E/tasks.ts` | pendente | `E/runner.test.ts` |
| EVAL-02 | Runner | Comparar vários modelos/tarefas pelo comando público exec/JSONL em Git/home temporários; relatório incremental de sucesso, calls, tokens reportados, duração e exit code; separar falhas de infraestrutura e tokens ausentes. | `E/runner.ts`, `E/cleanup.ts`, `E/README.md` | pendente | `E/runner.test.ts`, `M/exec.test.ts` |
| EVAL-03 | Simulação | Dois perfis determinísticos, sem provider externo, passando pelo pipeline real; perfil B introduz regressão CRLF esperada; modo bundle/source identificado e cleanup Windows com retries/limites de diretório. Simulação não mede qualidade de modelos reais. | `E/simulation.ts`, `E/runner.ts`, `E/cleanup.ts` | pendente | `E/runner.test.ts`, `M/exec.test.ts`; simulação completa não executada nesta inspeção |
| DIST-01 | Distribuição | Launcher público, versão, pacote executável e build com recursos/workers incluídos; portar instalação/execução em Rust com suporte aos três sistemas, sem obrigação de npm/Bun. Não publicar nem criar release nesta etapa. | `packages/myagent/build.ts`, `packages/myagent/package.json`, `packages/myagent/bin/myagent.js`, `scripts/package-smoke.ps1` | pendente | `M/launcher.test.ts`; smoke de pacote é script manual, não executado |

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
| CI Windows/Linux/macOS | em andamento | Workflow configurado; publicação e execução no GitHub ainda não confirmadas. |
| Git local/remoto | em andamento | init em main funcionou. A inspeção local mostra main sem commits e nenhum remote/ref. O usuário apresentou falhas de dubious ownership: .git pertence a CodexSandboxOffline, mas o terminal externo usa joaof. Configuração de confiança no perfil do usuário e confirmação de origin ainda pendentes. |
| Commit/push | pendente | As saídas posteriores apresentadas pelo usuário mostram que as tentativas de `chore: bootstrap repository` e push foram recusadas por dubious ownership. Nenhum commit/push foi confirmado ou executado por este agente. |

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

### Reconferência após o relato do usuário

O usuário informou ter configurado origin, criado o commit
`chore: bootstrap repository` e feito push da main no Git Bash. A inspeção
posterior das ferramentas disponíveis apresentou:

- `git log --oneline -5`: falhou com main sem commits.
- `git status`: main sem commits, arquivos do bootstrap untracked e nada staged.
- `git branch -vv`, `git remote -v`, `git show-ref` e `git ls-files`: sem entradas.
- `git rev-parse --show-toplevel --absolute-git-dir`: confirmou
  `C:/Users/joaof/My-Agent` e `C:/Users/joaof/My-Agent/.git`.
- Consulta de commits pela API GitHub para `mrJoao28/My-Agent`: HTTP 409,
  `Git Repository is empty`; busca da branch main retornou lista vazia.

As saídas posteriormente fornecidas pelo usuário mostram que `git commit` e
`git push` falharam com `detected dubious ownership`: `.git` pertence ao usuário
`CodexSandboxOffline`, enquanto o PowerShell externo executa como `joaof`.
Essas tentativas não criaram o commit nem publicaram a main.

O próprio Git indicou uma exceção de confiança restrita ao repositório. Ela
deve ser configurada pelo usuário no seu próprio terminal/perfil, sem usar uma
exceção global para todos os diretórios:

```powershell
git config --global --add safe.directory C:/Users/joaof/My-Agent
git status
git remote -v
```

Este agente não alterou a configuração global do usuário. Após essa correção,
confirmar origin e consultar `git ls-remote origin` antes de qualquer commit;
se houver commits remotos, parar e consultar o usuário. Revisar staged e seguir
as autorizações de commit/push do `AGENTS.md`. Confirmar o commit e a publicação
antes de criar o branch do próximo milestone.
Nesta reconferência foi alterada apenas esta documentação; os gates registrados
acima são da preparação inicial e não foram executados novamente.
