# TUI Grafite — Milestone 6

## Revisão da interface (21/10/2026)

O cabeçalho apresenta somente o último componente do diretório (ou a raiz),
incluindo caminhos Windows/UNC e Unix. O workspace completo continua intacto
no estado e na integração com autorização, ferramentas e persistência.

O campo de mensagem ocupa sempre cinco linhas externas, com três linhas de
texto e rolagem interna acompanhando o cursor. Quebra visual e posição do
cursor compartilham o mesmo cálculo por grafemas, inclusive nas bordas da
linha. Histórico, barra única de tokens e rodapé não mudam de posição ao
digitar ou colar várias linhas.

Caracteres imprimíveis com AltGr (Ctrl+Alt) são aceitos como texto. A barra
`/` também é preservada quando reportada com modificadores. Sugestões aparecem
somente enquanto o texto é prefixo de um comando; Tab completa a seleção e
Esc fecha as sugestões preservando o texto. Enter executa somente nomes
exatos de comandos. URLs, caminhos e mensagens multilinha iniciados por `/`
podem ser enviados como texto comum. Paste passa pelo mesmo limite de 9000
unidades UTF-16 e pela normalização de CRLF já existentes.

Respostas possuem moldura discreta e rótulo do modo de origem. O modo vem da
geração em streaming ou da mensagem persistida, sem depender do seletor da
próxima mensagem. Erros possuem rótulo e moldura próprios; painéis sucessivos
por arquivo continuam separados. NO_COLOR mantém molduras e rótulos.

A home centraliza instruções curtas. A aprovação usa um painel centralizado
horizontalmente, com largura máxima de 100 colunas, ação/escopo roláveis e
decisões separadas da prévia. AltGr nunca aprova. As políticas de Y/A/N/M,
cancelamento e ausência de efeitos antes do consentimento permanecem no
gerenciador existente. Em terminais baixos, o painel pode cobrir a entrada
temporariamente, que fica sem foco enquanto há aprovação.

Os screenshots anexados e o mockup não estavam disponíveis nesta revisão;
o resultado segue a descrição textual e os snapshots do TestBackend. As
regras do legado em `docs/APROVACOES_E_UNDO.md` foram lidas nesta revisão.
As limitações e os resultados efetivamente executados estão em PARITY.md.

A entrada interativa usa ratatui/crossterm diretamente com o core Rust.
Não há servidor HTTP local nem processo Node/Bun entre a interface e as
sessões. O layout escolhido pelo usuário tem PLAN/BUILD no cabeçalho,
indicador colorido à esquerda do campo de mensagem e uma única barra
horizontal cumulativa, sob o campo, com escala de **10.000.000 tokens**.
O atalho Tab aparece apenas no rodapé.

## Executar

Em um terminal interativo, na raiz do repositório:

```sh
cargo run --locked -p myagent-cli --bin myagent -- --demo
cargo run --locked -p myagent-cli --bin myagent
cargo run --locked -p myagent-cli --bin myagent -- --new
cargo run --locked -p myagent-cli --bin myagent -- --cwd caminho/do/projeto
```

Esses comandos funcionam também no PowerShell; informe caminhos com espaços
entre aspas. A versão Rust e as ferramentas de compilação são as do README.
`--model ID` escolhe o modelo inicial. `--help`/`--version` continuam sem
abrir estado ou rede; sem argumentos e sem TTY, a CLI mostra ajuda.
Opções interativas sem TTY falham antes de abrir estado.

`--demo` cria projeto e estado temporários próprios, usa provider fictício e
não acessa rede, credenciais ou configuração do usuário. Em BUILD, qualquer
mensagem demonstra dois arquivos sucessivos, cada um com autorização real,
SQLite e checkpoints reais. Em PLAN, retorna um plano sem escrita. Tudo é
descartado ao sair normalmente. O uso de 1200 tokens por rodada é fictício
e exclusivo da demonstração; não comprova medição de um provider externo.

Na execução normal, configuração de modelos/chaves segue
[Gerações](generations.md); diretório de estado e retenção seguem
[Persistência](persistence.md). Não há editor de chaves na TUI. Modelos de API
sem chave aparecem sinalizados; Ollama não exige chave, mas sua presença na
lista não comprova que o daemon/modelo está disponível. Docker indisponível
remove bash/testes/tipos da lista entregue ao modelo, sem fallback no host.

## Telas e teclado

A interface abre a sessão mais recente do workspace autorizado ou cria uma.
Home e nova sessão oferecem o mesmo campo de entrada; o histórico aparece
na tela de sessão. Sessões listadas em `/sessions` pertencem ao workspace
atual. Interrupções preservam o histórico; retomada exige seleção explícita.

| Ação | Tecla/comando |
| --- | --- |
| Enviar | Enter |
| Quebrar linha | Ctrl+J; Shift+Enter quando o terminal o distingue |
| Alternar próximo modo | Tab fora de aprovação/modal/completação |
| Interromper geração | Esc ou Ctrl+C |
| Fechar diálogo | Esc ou Ctrl+C, sem atingir a camada de fundo |
| Rolar histórico | PgUp/PgDn ou roda do mouse |
| Navegar diálogo/menu | Setas, PgUp/PgDn, Enter; Tab avança no diálogo |
| Completar arquivo seguro | @prefixo e Tab; até oito sugestões |
| Comandos | /new, /sessions, /models, /agents, /theme, /undo, /chatgpt, /exit |
| Sair ocioso | /exit ou Ctrl+C com entrada vazia |

O modo que está executando permanece indicado na barra de status mesmo se
Tab selecionar outro para a próxima mensagem. Entrada multilinha respeita
grafemas, largura Unicode, CRLF, paste e o limite de 9000 unidades UTF-16.
Modais têm prioridade sobre aprovação, que tem prioridade sobre a entrada.
`/agents` alterna PLAN/BUILD; não cria agentes autônomos.

Grafite é o padrão solicitado, com Nightfox e Papel como alternativas em
`/theme`. A escolha é salva em `preferences.json` no diretório de estado.
JSON inválido é preservado e gera aviso; falha de gravação mantém o tema da
sessão e mostra toast. `NO_COLOR`, inclusive vazio, desativa cores. Texto de
arquivo/provider é sanitizado antes de chegar ao terminal.

## Código e aprovações

`write_file`, `edit_file` e `apply_patch` aparecem em retângulos sucessivos por
arquivo. Prévias parciais são dados efêmeros; não entram no journal e nunca
autorizam efeitos. O nome precisa estar disponível nos argumentos para
identificar o arquivo. Providers que só entregam argumentos completos
mostram o painel nessa etapa, sem simular escrita incremental.

Os estados distinguem geração de prévia/não salvo, aguardando aprovação,
alteração aplicada, checkOnly validado/não aplicado, falha e interrupção.
Somente o resultado associado à chamada muda o painel para aplicado.
O histórico persistido reconstrói chamadas/resultados ao reabrir a sessão.

O painel de aprovação mostra comando bash, diff colorido de edição/patch,
caminho/tamanho/prévia de escrita, URL de fetch, origem/destino de movimento
e alvo de exclusão. Setas, PgUp/PgDn, Home/End e mouse rolam a prévia.
Limites de 1000 linhas e 80 mil unidades UTF-16 têm aviso explícito.
Enter e Tab **nunca** aprovam. Y permite uma vez, A permite o escopo na sessão
quando a política permite, N nega; Esc/Ctrl+C interrompem. Delete e operações
Git individuais não aceitam A; push a main/master exige M específico.

`/undo` chama o mecanismo local de checkpoints antes de qualquer provider.
Conflitos/ausência de checkpoint aparecem como erro. Sua semântica é descrita
em [Alterações](mutations.md). Bash interativo usa o mesmo gerenciador de
aprovações antes de chegar ao Docker; runners fixos preservam a política M5.

A barra soma medições reportadas ao longo da sessão; ausências permanecem
desconhecidas e são indicadas. A escala de 10M é visual: não altera janela de
contexto, orçamento nem consumo permitido. O preenchimento satura em 100%.

## Verificação e limites

`TestBackend`/`insta` verificam home, nova sessão, sessão com dois arquivos,
aprovação e tema. Testes cobrem foco, teclas que nunca aprovam, rolagem,
Unicode, limites, NO_COLOR e preferências. Integrações usam provider fictício
com Agent/StoreJournal/ferramentas reais para validar PLAN, duas aprovações,
cancelamento, uso acumulado e undo. Nenhuma chamada paga é necessária.

O guard restaura raw mode, cursor, paste, mouse e alternate screen ao sair,
em erro e no hook de panic. A interface espera o cleanup de tarefas antes de
liberar sessão/terminal; SIGTERM solicita encerramento ordenado no Unix.
`cargo run -p myagent-tui --example terminal_probe -- --panic` reproduz um
panic intencional, sem abrir estado/provider. SIGKILL e encerramento forçado
do sistema não permitem executar código de restauração.

Windows/macOS são cobertos pelo CI, mas isso não substitui testes manuais
em Windows Terminal/PowerShell e Terminal.app. A evidência efetivamente
executada e o workflow correspondente estão em [PARITY](PARITY.md).
Para conferência manual, execute `--demo`: observe os dois painéis, teste
Enter/Tab/Y/N na aprovação, `/undo`, diálogos, Esc/Ctrl+C, resize e saída.

O diálogo `/chatgpt` informa que login de conta pertence ao M7; não declara
conta conectada. No fechamento original do M6, o legado `packages/cui/src` e
`docs/APROVACOES_E_UNDO.md` não estavam acessíveis naquele ambiente. Nesta
revisão, o documento de aprovação/undo foi lido no checkout local somente
leitura do legado. Não houve comparação com screenshots ou mockups ausentes.
