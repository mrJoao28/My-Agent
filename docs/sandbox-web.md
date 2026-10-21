# M5: execução isolada e web

## Registro e fronteiras

`ExecutionRegistry::detect` verifica o daemon Docker em até três segundos e
aceita apenas containers Linux. Sem Docker, `bash`, `type_check` e `test_run`
não entram na lista. Nenhuma falha seleciona shell, runner ou compilador do
host. O único processo de infraestrutura deste adapter é a CLI Docker.

`ToolRegistry::with_execution` e `with_web` compõem os adapters com as
ferramentas anteriores. Com todos os adapters e Docker disponíveis, PLAN tem
21 ferramentas e BUILD tem 31. PLAN nunca inclui bash ou alterações do projeto;
essa restrição está na construção da lista e na API de execução por modo.
Sem Docker, os totais são 19 e 28, respectivamente. CLI/TUI ainda precisam
conectar essas bibliotecas no milestone de interface.

## Imagem e runners

A recomendação implementada é `rust:1.99.1-slim-bookworm`, imagem oficial com
versão alinhada ao toolchain fixado do My-Agent. A imagem Bun do legado atende
JavaScript, mas não fornece o compilador e o Cargo necessários à reescrita.
Uma imagem Rust é uma base menor e mais previsível do que instalar todos os
runtimes durante cada execução. Não se instala nada pela rede no container.

`MYAGENT_SANDBOX_IMAGE` aceita uma imagem confiável alternativa, incluindo uma
imagem preparada com caches e os runtimes necessários. O adapter verifica se
ela existe e faz pull sob demanda, limitado a 180 segundos; a execução usa
`--pull=never`. A imagem padrão não promete Bun, Node, TypeScript ou Pytest.
Os resultados de pull são limitados; sua apresentação interativa permanece
para a interface futura. A tag fixa não equivale a um digest imutável.

O conjunto preserva Bun, Node (`--test`), Vitest, Jest e Pytest e acrescenta
Cargo. O runner automático é Cargo quando há Cargo.toml, Bun nos demais
projetos. `type_check` escolhe Cargo ou tsc, com override explícito.

- Cargo: `test` ou `check`, sempre `--offline --locked`, artefatos em /tmp.
- Bun: `bun test`; Node: `node --test`.
- Vitest: `vitest run --maxWorkers=1`.
- Jest: `jest --runInBand --watch=false`.
- Pytest: `pytest -q -p no:cacheprovider`.
- TypeScript: `tsc --noEmit --pretty false`, com executáveis locais antes do PATH.

Cargo usa CARGO_HOME temporário para locks e estado próprio, com referência
somente leitura ao registry preparado na imagem, sem copiá-lo inteiro para
o tmpfs. Crates precisam estar previamente disponíveis e desempacotadas nesse
cache; dependências ausentes falham em modo offline. Nenhum cache privado do
host é montado.

`test_run` aceita até dez caminhos explícitos. Para Cargo, arquivos devem ser
`tests/<target>.rs`, convertidos em argumentos `--test <target>`; não se passa
um caminho arbitrário como flag do Cargo. Argumentos são escapados para shell.
Exit code zero significa sucesso do processo, sem inferir quantidade de testes.
Runner ausente, dependência ausente, incompatibilidade Windows/Linux, memória
ou tmpfs insuficiente também produzem falha explícita.

## Confinamento e ciclo de vida

O container usa rede desativada, root filesystem somente leitura,
UID/GID 65534, capabilities removidas, no-new-privileges, um CPU, 512 MiB de
memória sem swap adicional e limite de 128 processos. O único bind mount é
o snapshot filtrado em /workspace, somente leitura, sem submounts recursivos.
Não recebe socket Docker, variáveis do servidor nem diretórios de credenciais.

/tmp é descartável, nosuid/nodev, modo 1777. O padrão é 64 MiB;
`MYAGENT_SANDBOX_TMPFS_MIB` aceita inteiros de 1 a 4096. O limite de memória
do container continua valendo, mesmo se o tmpfs configurado for maior.
Bash opera sobre /workspace; runners usam uma cópia descartável em /tmp.
Dependências permanecem referenciadas no mount somente leitura.

Bash tem timeout padrão de 30 segundos, máximo de 120; testes usam 60,
máximo de 120; type_check usa 60. A supervisão limita stdout e stderr a
80.000 bytes combinados durante a leitura e encerra o processo ao exceder.
A resposta apresenta até 20.000 unidades UTF-16 por stream; type_check até
15.000 de saída bruta. Em timeout, a saída parcial pode ser descartada e isso
é informado. Esse teto de bytes é uma proteção adicional ao limite textual.

Cancelamento também alcança a CLI e a remoção forçada do container. Um worker
mantém o snapshot vivo até terminar a limpeza, mesmo se o consumidor abandonar
o futuro. A remoção tem limite de dez segundos. O core concede quinze segundos para
encerrar o worker de execução, confirmar a remoção e liberar o snapshot;
a geração permanece IN_PROGRESS durante essa limpeza. Falhas de limpeza
interrompem a geração com erro explícito, inclusive durante cancelamento. Falha de limpeza retorna erro e
conserva o temporário, em vez de remover um mount possivelmente ativo. Não se
declara sucesso nesse caso. Não há aplicação de artefatos ao workspace.

## Snapshot

A cópia usa temporário exclusivo fora do workspace, revalida caminhos e
handles, não segue symlinks/junctions e omite hard links. Credenciais,
`.env*`, metadata Git, configuração privada, logs, bancos, caches e diretórios
de build não são copiados. Arquivos acima de 5 MiB são omitidos. Os limites
de 200 MiB e 20 mil entradas são conferidos durante a leitura; há também
limite de profundidade 64 e cancelamento entre blocos. Alterações de tamanho,
mtime ou ctime durante a cópia invalidam a preparação.

`MYAGENT_SANDBOX_DEPENDENCIES=1` permite cópia filtrada de node_modules e
.venv; o padrão é 0. Mesmo no opt-in, os limites globais solicitados de 5 MiB
por arquivo, 200 MiB e 20 mil entradas são mantidos. Isso é mais restritivo
que os limites maiores de dependências do legado. Scripts .bin podem ser
materializados somente quando apontam diretamente para um arquivo regular
seguro, incluído no snapshot. Links externos ou encadeados não são seguidos.

Nesta etapa cada execução recebe um snapshot novo. Reuso por metadados e TTL
do legado permanece pendente na matriz; isso evita introduzir cache com
invalidação incompleta nesta entrega. Comandos no workspace são serializados.

## Web e consentimento

`web_fetch` e `web_search` exigem ApprovalManager, inclusive em PLAN.
Y vale uma vez, A memoriza somente ferramenta/sessão/origem exata e N nega.
Origem inclui protocolo, host e porta. Uma autorização consumível vincula
argumentos, cancelamento e sessão à execução; executar sem ela é recusado.
Permissões não são persistidas. Resultados e páginas são dados não confiáveis.

Fetch aceita apenas HTTP(S), sem credenciais na URL. Bloqueia loopback,
redes privadas, link-local, CGNAT, documentação, multicast, IPv4 mapeado,
NAT64, redes de transição e endpoints conhecidos de metadata de nuvem.
Todo registro DNS precisa ser público. O transporte recebe somente os IPs
validados, preserva Host/SNI e verificação TLS, desativa proxies e pooling.
Cada redirecionamento resolve e valida novamente. Outra origem é recusada
antes da conexão; são permitidos até cinco redirects na mesma origem.

Fetch tem prazo global de 15 segundos, até 5 MiB durante a leitura e texto
final de até 10.000 unidades UTF-16. Recusa compressão não solicitada, trata
falhas/status explicitamente e remove script/style/comentários ao converter
HTML em texto. Não é um renderizador de navegador.

Busca usa somente `https://api.tavily.com/search` e `TAVILY_API_KEY`, injetada
pela configuração confiável, nunca pelos argumentos do modelo. O padrão é
cinco resultados, máximo dez; query até 4.000 unidades UTF-16. O JSON recebido
é limitado a 1.000.000 bytes e validado. Answer tem até 2.000 unidades, título
300 e snippet 500. Redirects da busca são recusados; URLs retornadas não
concedem autorização de fetch. Nenhuma chave real é necessária nos testes.

## Dependências, testes e referências

Não há crate nova no grafo: reqwest/rustls é reutilizada para HTTP, tempfile
passa a dependência de runtime para os snapshots e thiserror é reutilizada
para erros do adapter Docker. Bibliotecas de segurança existentes continuam
centralizando caminhos, metadados, consentimento e cancelamento. Não houve
novo unsafe.

Testes normais usam driver Docker simulado, DNS/HTTP simulados, HTTP local,
workspaces temporários e valores fictícios. A integração real tem `#[ignore]`
explícito e requer `MYAGENT_TEST_DOCKER_IMAGE`; não conta como aprovada na suíte
normal. Para executar, em uma máquina com daemon Linux e imagem confiável:

```powershell
$env:MYAGENT_TEST_DOCKER_IMAGE = 'alpine:3'
cargo test --locked -p myagent-tools --test docker_real -- --ignored --nocapture
```

O teste verifica usuário, capabilities, mounts somente leitura, ausência de
variáveis do servidor/socket Docker, rede isolada, bytes do workspace e
timeout. O CI adiciona um job Linux opcional, acionado por workflow_dispatch
com docker_integration=true; os gates normais seguem nos três sistemas.
Os resultados realmente executados estão em [PARITY.md](PARITY.md).

Referências primárias consultadas:
[imagem oficial Rust](https://hub.docker.com/_/rust/),
[docker run](https://docs.docker.com/reference/cli/docker/container/run),
[tmpfs](https://docs.docker.com/engine/storage/tmpfs) e
[ClientBuilder do reqwest](https://docs.rs/reqwest/0.12.28/reqwest/struct.ClientBuilder.html).
