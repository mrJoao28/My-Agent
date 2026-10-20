# Persistência local e contratos — M1

## Contratos

`myagent-protocol` contém Session, Message, SessionContext, SessionLease,
MessagePart e StreamEvent, sem acesso a arquivos, rede ou runtime. Mode é
não exaustivo, com PLAN/BUILD; variantes desconhecidas são rejeitadas. Learn
não foi implementado.

Os derives serde definem o formato. Nas bordas, use `decode_json`,
`encode_json` ou `Validate`; desserializar diretamente com serde não aplica
as regras semânticas. Os serviços validam entrada e registros carregados.
Tags das parts/eventos e campos camelCase seguem o legado. Datas usam
inteiros de milissegundos desde Unix, em vez dos objetos Date/JSON ISO do
legado; uma futura importação fará a conversão explicitamente.
Message.duration usa segundos; done.durationMs usa milissegundos.
Usage ausente continua None.

A entrada USER é aparada e limitada a 9.000 unidades UTF-16, como no
legado. JSON tem orçamento de 16 MiB, profundidade máxima de 64 nos valores
opacos e limite de entradas; listas de parts têm no máximo 10.000 itens.
Consultas aceitam páginas de 1–500 registros e limitam os bytes acumulados.
Contextos preservam metadados dos providers; somente o envelope role/content
é validado aqui. Validar o conteúdo específico de cada provider fica para
seus adapters. Nenhum dado JSON concede permissão.

## Localização e abertura

O runtime resolve, nesta ordem:

1. MYAGENT_HOME, se não vazio.
2. MYAGENT_CONFIG_DIR, alias legado, se não vazio.
3. USERPROFILE no Windows (fallback HOME), ou HOME no Unix, mais .myagent.

O arquivo é **myagent-rust.sqlite3**, com sidecars SQLite próprios. O arquivo
legado **myagent.db nunca é aberto, migrado ou escrito**. Não há importação
automática. A CLI ainda oferece somente ajuda/versão e não abre o store.

A implementação recusa outro nome de banco, links simbólicos, junctions,
reparse points e arquivos com múltiplos hard links. Valida os ancestrais
antes de abrir o SQLite. Isso não elimina uma corrida maliciosa entre a
checagem e a abertura; não há promessa de proteção contra um processo que
possa trocar os diretórios simultaneamente. No Unix, novos diretórios/arquivo
recebem modos 0700/0600; ACLs Windows são herdadas, sem alteração das ACLs do
usuário. Workspaces precisam existir e ser diretórios canônicos; a chave
usa minúsculas no Windows. Descobrir a raiz Git de uma subpasta fica para a
camada CLI/core.

A conexão pertence a uma thread dedicada. Uma fila síncrona limitada a 64
operações serializa trabalho e manutenção, com resposta por operação.
Consumidores assíncronos deverão fazer a ponte sem bloquear o executor.
Fechar o último handle espera a thread e libera somente seus próprios
leases. M1 não exige Tokio.

## Banco e serviços

WAL em arquivo, busy_timeout de 5000 ms, foreign_keys ativadas e
trusted_schema desativado. As três migrations SQL são embutidas, sequenciais
e aplicadas numa única transação IMMEDIATE, com PRAGMA user_version e
application_id próprio. Erro reverte o conjunto pendente; versões futuras
ou bancos de outra aplicação são recusados.

Criar sessão gera contexto inicial e lease na mesma transação. Listar não
altera atividade; abrir valida workspace, adquire lease, renova atividade
e cancela aviso de retenção atomicamente. Leitura de histórico/contexto
não exige tomar o lease; escrita exige o lease vigente.

ASSISTANT nasce IN_PROGRESS. Cada append persiste content/parts e atividade
na mesma transação, antes de retornar. Reasoning permanece nas parts, sem
entrar no texto concatenado. Chamadas/resultados são associados por ID/nome;
resultado idêntico repetido é idempotente e resultado conflitante é rejeitado.
O core futuro deverá chamar mark_tool_started antes de qualquer efeito.
Completar exige resultados para todas as chamadas; interromper preserva
parts parciais. M1 não executa ferramentas, não recupera órfãs automaticamente
e não reexecuta gerações.

Leases usam owner aleatório por conexão, heartbeat automático a cada 5 s e
expiração de 30 s, com disputa atômica entre processos. Heartbeat só renova
um lease próprio ainda vigente. Perdas ficam consultáveis em lost_sessions;
as escritas seguintes falham. O loop futuro deve observar essas perdas e
cancelar a geração. Falha de heartbeat não vira renovação fictícia.

Retenção padrão: 90 dias, configurável por
MYAGENT_SESSION_RETENTION_DAYS (0 desativa; inteiros de 0 a 365000).
Uma limpeza persiste o aviso sem renovar atividade; a exclusão só ocorre
após ao menos 24 h, reavaliando os critérios na mesma transação. Abrir a
sessão cancela o aviso. Gerações IN_PROGRESS, mensagens recentes e leases
não expirados preservam a sessão. Exclusão usa cascata para mensagens,
contextos e leases; artefatos de ferramentas ainda não existem no Rust.

A limpeza roda na inicialização e depois a cada hora, sem sobreposição.
cleanup retorna RetentionReport com avisos/prazos e exclusões; os avisos
também estão em Session.retention_warned_at nas listagens, inclusive após
reiniciar. A apresentação desses avisos na TUI/CLI e o cancelamento do
loop por perda de lease ficam para os respectivos milestones.

## Dependências introduzidas

| Crate | Necessidade concreta |
| --- | --- |
| serde + serde_json | Contratos tipados e JSON de parts/contextos, preservando metadados. |
| rusqlite, features bundled/limits | SQLite embutido, SQL/transações explícitas e limites de alocação por conexão; exige compilador C. |
| thiserror | Erros tipados nos contratos e store, sem incluir conteúdo sensível nas mensagens públicas. |
| clap, sem derive/default features | Substituir o parser provisório de ajuda/versão conforme decisão do M0; não acrescenta comandos. |
| winapi-util, somente Windows | Consultar quantidade de hard links por API segura; a API equivalente de std ainda exige nightly. Não há unsafe próprio. |
| tempfile, somente testes | Isolar fixtures SQLite e processos de teste sem tocar no home real nem em dados do legado. |

As demais dependências previstas no ADR continuam adiadas, incluindo
tree-sitter. O Cargo.lock foi gerado pelo Cargo após o fetch e é versionado.
A licença Zlib de foldhash 0.2, dependência transitiva de
hashbrown/hashlink/rusqlite, foi revisada no arquivo LICENSE da crate e
incluída no deny.toml. cargo-deny não foi executado neste fechamento.

## Verificação

Os testes usam somente dados fictícios, diretórios temporários e injeção
de relógio/configuração. Abrir o store padrão não faz parte dos testes.
Há casos de contratos, SQLite real em arquivo, migrations com rollback,
reabertura, contexto, leases em conexões e processos distintos, heartbeat
real, retenção e links/junctions por plataforma. No fechamento local do M1,
os 24 testes passaram no Windows, sem falhas ou ignorados. O teste Unix de
symlink e as demais plataformas aguardam o CI. Resultados e limitações
estão registrados em [PARITY.md](PARITY.md).

## Integração M2

StoreJournal implementa as portas síncronas do core, que usa spawn_blocking.
Recuperação respeita leases/workspaces, retomadas preservam generationId e
usage é atualizado durante a geração. Contexto com signatures é salvo antes
de efeitos; ao iniciar novo turno, reasoning antigo não é reintroduzido.
Detalhes e testes em [Gerações M2](generations.md).
