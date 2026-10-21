# M4: consentimento, alterações e desfazer

As bibliotecas oferecem `write_file`, `edit_file`, `delete_file`, `move_file`,
`apply_patch`, `git_commit`, `git_init`, `git_remote_add` e `git_push`.
`ToolRegistry::with_mutations` recebe a configuração confiável da aplicação.
A lista de PLAN permanece somente leitura; BUILD recebe as nove ferramentas.
CLI e TUI ainda precisam conectar os diálogos e comandos às APIs implementadas.

## Aprovações

`ApprovalManager` mantém permissões em memória por sessão. Y autoriza uma
execução, A autoriza a ferramenta e o conjunto exato de caminhos ou destino
Git naquela sessão, e N nega. Backups e ambos os caminhos de uma movimentação
fazem parte do escopo. Há no máximo 256 escopos autorizados por sessão.
No Windows, a comparação usa caminhos canônicos e normalização ASCII de caixa.

Excluir arquivos, criar commits, inicializar Git, adicionar remotes e fazer
push sempre exigem autorização individual. O push apresenta URL e branch;
main/master exigem outro pedido, confirmado explicitamente com M. Enter e
respostas para outra sessão não autorizam uma operação. Pedidos expiram em
120 segundos e são removidos ao cancelar, encerrar ou esquecer a sessão.

A autorização gera uma permissão consumível vinculada aos argumentos e ao
estado apresentado. Executar diretamente sem essa permissão é recusado.
Alterações posteriores ao preview invalidam a execução. Conteúdo de arquivos
e respostas de ferramentas não criam permissões.

## Checkpoints e desfazer

`CheckpointStore::from_home` usa `checkpoints/<sessionId>` sob o diretório
resolvido por `StoreConfig`: padrão `~/.myagent`, override `MYAGENT_HOME` e
alias `MYAGENT_CONFIG_DIR`. A aplicação fornece esse diretório confiável;
argumentos de ferramentas não escolhem o armazenamento.

Antes da escrita, um registro guarda os bytes anteriores em base64, modos,
SHA-256, identidade do arquivo e tempos. Inclui arquivos binários e não
rastreados, CRLF, backups, diretórios e ancestrais que serão criados.
Não usa stash nem altera o índice Git.

Os limites são 5 MiB por arquivo, 20 MiB por passo, 1000 entradas, profundidade
32, 20 passos e 64 MiB de registros por sessão. A reserva de espaço considera
serialização e estado posterior; pode recusar antecipadamente uma operação.
Somente passos completos antigos podem ser removidos para liberar orçamento.
Registros incompletos e entradas desconhecidas fazem a operação falhar.

`MutationRegistry::try_local_command` reconhece `/undo` localmente, sem
provider. O mesmo bloqueio de sessão usado pela geração impede desfazer
durante uma geração; um mutex por workspace serializa alterações e restauração.
A sessão precisa pertencer ao workspace e manter seu lease SQLite.

Antes de restaurar qualquer caminho, compara todos os estados posteriores:
hash, modo, identidade/inode, mtime, ctime e estrutura dos diretórios.
Qualquer conflito recusa o passo inteiro. Não há opção de forçar.
Uma falha durante a restauração conserva o registro incompleto, sem repetir
automaticamente uma restauração parcial. A nota local `[UNDO]` preserva modelo
e modo da última mensagem e só então o checkpoint concluído é removido.
O comando nunca cria commits.

## Escrita, patch e Git

Escritas usam arquivos temporários e publicação atômica por arquivo. Um patch
multiarquivo é um único checkpoint, com todos os hunks validados antes de
qualquer escrita. `checkOnly` valida e apresenta o plano, sem escrever nem criar
checkpoint, mantendo a exigência de consentimento. O parser próprio aceita
patches unified de texto, criação, exclusão, modos e ausência de newline final;
recusa patches binários, links e metadados de rename/copy. Diferentemente do
executor Git do legado, não exige um repositório Git para aplicar o patch.
Não oferece transação atômica entre arquivos: falhas de I/O preservam a
pré-imagem necessária para diagnosticar ou desfazer o estado parcial.

Git usa argumentos fixos, caminhos literais, hooks desativados e proteções
contra filtros, diff externo, textconv e configurações de transporte inseguras.
Commit seleciona somente os arquivos aprovados e preserva staged alheio.
Push usa HTTPS, host permitido, URL sem credenciais e refspec explícito, sem
force, mirror, tags ou exclusão. O transporte é substituível por simulação.
Um push bem-sucedido encerra a geração atual; não pode compartilhar um lote
com outras chamadas de ferramentas. Bash permanece para o M5.

## Dependências e metadados nativos

- `sha2`: verificação de integridade e detecção de conflitos.
- `base64`: persistência reversível de bytes, inclusive binários.
- `getrandom`: identificadores imprevisíveis de aprovações e temporários.
- `url`: validação estrutural dos destinos Git.
- `serde`: contratos privados versionáveis dos checkpoints.
- `windows-sys`: acesso ao ChangeTime real no Windows.

As dependências já presentes no workspace são reutilizadas quando possível.
O crate `platform` isola a única exceção ao `unsafe_code`: uma chamada Windows
a `GetFileInformationByHandleEx`, necessária porque ChangeTime não é o tempo
de criação e sua exposição pela biblioteca padrão não está estável no toolchain
fixado. Em Unix, ctime é obtido por APIs seguras da biblioteca padrão.

A chamada recebe um handle emprestado de `File` vivo, uma estrutura
`FILE_BASIC_INFO` inicializada, alinhada e exclusivamente emprestada, e seu
tamanho exato. O ponteiro não escapa. Falhas retornam erro e impedem a operação.
O módulo Windows permite somente essa exceção; os demais crates continuam
com `forbid(unsafe_code)`. Referências:
[MetadataExt](https://doc.rust-lang.org/stable/std/os/windows/fs/trait.MetadataExt.html)
e [FILE_BASIC_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_basic_info).

## Validação

Os testes usam workspaces temporários, sessões e identidades fictícias,
provider simulado e transporte de push simulado. Não acessam o armazenamento
real do usuário nem fazem chamadas pagas. Os resultados dos gates e suas
limitações ficam em `docs/PARITY.md`; a implementação não constitui fechamento
do milestone enquanto a suíte final estiver pendente.
