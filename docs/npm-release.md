# Distribuição npm — M8

O pacote principal é `@j2008/myagent`, comando `myagent`. Seu único código
runtime é `bin/myagent.js`, sem dependências Node de runtime. O shim resolve
o pacote opcional, confere nome/versão e inicia diretamente o binário Rust
com `spawn`, `shell:false` e `stdio:inherit`, preservando cwd, ambiente,
argumentos e código de saída. Não há postinstall, compilação, fetch de binário
nem exigência de Rust/Bun na instalação.

| Pacote opcional | os / cpu | Target Rust | Runner de build e smoke |
| --- | --- | --- | --- |
| `@j2008/myagent-win32-x64` | win32 / x64 | x86_64-pc-windows-msvc | windows-2022 |
| `@j2008/myagent-linux-x64` | linux / x64 | x86_64-unknown-linux-musl | ubuntu-24.04 |
| `@j2008/myagent-linux-arm64` | linux / arm64 | aarch64-unknown-linux-musl | ubuntu-24.04-arm |
| `@j2008/myagent-darwin-arm64` | darwin / arm64 | aarch64-apple-darwin | macos-14 |
| `@j2008/myagent-darwin-x64` | darwin / x64 | x86_64-apple-darwin | macos-15-intel |

O catálogo em `scripts/npm/common.mjs` gera a matriz. Cada build confere a
arquitetura real do runner. Linux usa musl-tools nativo e rejeita ELF com
PT_INTERP, evitando publicar acidentalmente um binário glibc dinâmico.
Windows verifica PE x64; macOS verifica Mach-O64 da arquitetura esperada,
com deployment target 11.0. Não há build cruzado configurado nesta matriz.
Build e execução verificada são etapas distintas; a publicação exige ambas.

## Publicação automática da main

Depois da ativação abaixo, um push/merge de código na main prepara uma release
sem digitar versão, criar tag ou executar npm publish manualmente. Alterações
em crates, Cargo, toolchain, pacotes npm, scripts de distribuição, workflows
de CI/distribuição e LICENSE entram no fluxo. Docs/README fora dos pacotes
não geram versão. Commits sem prefixo que alterem esses arquivos recebem patch.

`scripts/npm/auto-release.mjs` usa o histórico desde a maior tag estável
alcançável na main: `fix:` e outros commits de código geram patch; `feat:` gera
minor; `tipo!:` ou `BREAKING CHANGE:` gera major. Antes da primeira tag e sem
pacotes publicados, conserva a versão inicial do Cargo.toml. Uma versão
preparada manualmente acima das anteriores também é conservada. Consulta as
versões estáveis de **todos os seis pacotes**, inclusive plataformas deixadas
por publicação parcial, e nunca reutiliza uma versão ocupada no registry.
Somente E404 estruturado é ausência; rede, autenticação e JSON inválido abortam.

O prepare faz checkout da main mais recente, sincroniza os oito arquivos de
versão, cria `chore(release): vX.Y.Z` e envia commit/tag com `git push --atomic`.
Não há force push. Se a main avançar durante a preparação, o envio inteiro é
recusado; o próximo run pode preparar a main atual. A tag é um ponto de build,
não prova de publicação: é criada **antes** dos gates. Falhas posteriores
impedem npm publish e a criação da GitHub Release. Todos os jobs fazem checkout
do mesmo SHA imutável; a versão efetivamente testada é a empacotada/publicada.

Builds, qualidade e publicação continuam **no mesmo workflow**: eventos criados
pelo GITHUB_TOKEN não disparam outro workflow de push. Assim, não é necessário
PAT para encadear a tag e não há ciclo do commit de versão. Uma fila compartilhada
serializa releases e não cancela uma publicação em andamento. O GitHub pode
substituir um run pendente por outro mais novo; preparar a main mais recente
inclui essas mudanças acumuladas. Depois dos seis pacotes confirmados no npm,
o job final registra a GitHub Release com notas geradas pelo GitHub.

### Ativação única

1. Confira a existência e sua permissão no scope dos seis pacotes. Se nunca
   foram publicados, faça primeiro o bootstrap descrito abaixo; o código não
   cria credenciais nem tenta contornar o cadastro de Trusted Publishers.
2. Cadastre os seis Trusted Publishers como descrito abaixo: usuário mrJoao28,
   repositório My-Agent, workflow release.yml, environment npm-release.
3. No GitHub, crie/configure npm-release para permitir **main e tags v***.
   Para publicação totalmente automática, não configure required reviewers
   nesse environment. Se preferir revisão por versão, mantenha reviewers:
   nesse caso a publicação aguardará essa aprovação.
4. Permita contents:write para o job prepare e o envio do bot à main conforme
   as regras do repositório. Proteções incompatíveis interrompem o envio;
   o workflow não as remove nem faz bypass automático.
5. Em Settings → Secrets and variables → Actions → **Variables**, crie a
   variável de repositório `NPM_AUTO_RELEASE` com valor `true`. Não é secret.
   Ausente/false, pushes na main apenas compilam/testam a distribuição.
6. Para incluir imediatamente as correções que já estão na main, dispare a
   primeira release depois desse cadastro:

```powershell
gh variable set NPM_AUTO_RELEASE --body true --repo mrJoao28/My-Agent
gh workflow run release.yml --ref main -f publish=true --repo mrJoao28/My-Agent
```

Daí em diante basta enviar código à main. Para pausar, altere a variável para
false; isso impede novas preparações e o guard do publisher automático também
recusa a publicação. Não apaga tags nem reverte pacotes já publicados. Para
parar um run antes da publicação, cancele-o também no GitHub Actions.

A execução manual **sem** publish=true continua sendo somente build/testes.
Um push manual de tag v* continua sendo um caminho de publicação explícita,
independente da variável da automação; exige todos os mesmos gates e environment.

## Versão e pacotes

`Cargo.toml` é a fonte da versão estável. O validador confere nove entradas
Rust no Cargo.lock, os seis manifests npm, dependências opcionais exatas e
tag `v<VERSION>`. O smoke compara isso à saída real `myagent <VERSION>`.
Não se publica uma tag apenas parecida, como `v0.1`, nem dependência `^0.1.0`.

```powershell
node scripts/npm/version.mjs
# Somente quando for decidido mudar a versão:
node scripts/npm/version.mjs --set 0.1.1
node scripts/npm/version.mjs v0.1.1
```

O último comando apenas valida; não cria tag. Alterar a versão exige repetir
os builds, packs e smokes. Não altere manifests manualmente após os testes.
Releases prerelease não estão habilitadas neste fluxo.

Os manifests ficam em `npm/`; o empacotador copia somente manifest, README,
LICENSE e um arquivo em bin para um staging novo. `npm pack --ignore-scripts`
produz o tarball real e confere a lista exata. O principal deve ter menos de
20 KB descompactados. Binaries/tarballs/staging ficam em `release/`, ignorado.
Cada pacote tem SHA256SUMS e receipt.json com SHA-256 do tarball e do binário
interno, além da integridade SHA-512 npm. O verificador reabre os tarballs,
verifica cabeçalhos tar, arquivos, manifest, arquitetura, permissões e hashes.

O principal de produção é empacotado no Linux com modo executável; npm pack
no Windows usa 0644, mesmo após chmod. Isso é aceito apenas na prova local
Windows: o npm cria myagent.cmd. Os binários POSIX exigem modo executável
também quando seus tarballs são inspecionados em Windows.

## Workflow sem publicar

`.github/workflows/release.yml` roda em PRs relevantes, pushes de código na
main, tags v* e workflow_dispatch. PRs, main sem ativação e execução manual
com publish=false preparam/testam sem publicar. Com a automação ativa, o
prepare fixa o SHA da release antes destes jobs.

1. Valida versões/tag, empacota o principal e produz a matriz única.
2. Executa fmt, Clippy, testes Rust e testes Node em Windows/Linux/macOS.
3. Compila/empacota cinco binários em runners nativos.
4. Em cada plataforma, baixa os seis tarballs e usa um registry HTTP
   efêmero em loopback, sem proxy para npm público. O cache está vazio;
   instala `@j2008/myagent@VERSION` por nome e versão em prefixo com espaços.
   Isso testa a resolução dos pacotes inéditos sem reescrever optionalDependencies
   para caminhos locais nem substituir os tarballs reais.
5. Confere seleção por os/cpu, manifesto instalado, --version, doctor com
   SQLite/estado temporário e exec PLAN offline. O PATH contém apenas Node e,
   no Windows, System32; rustc, cargo e Bun devem estar inacessíveis.
6. Repete o Linux x64 em `node:24-bookworm-slim`, conferindo que não há Rust
   instalado na imagem. Os runners nativos podem ter Rust instalado fora do
   PATH restrito; essa distinção aparece na evidência de validação.
7. O job publish roda para uma release automática preparada ou push explícito
   de tag v*, após todos os gates/smokes e controles do environment npm-release.
   Reconfere recibos e relatórios das
   cinco execuções usando exatamente os mesmos tarballs.

O launcher usa o console herdado. Em POSIX, encaminha SIGINT/SIGTERM/SIGHUP
recebidos pelo PID do launcher, espera cleanup do filho e preserva saída por
sinal. No Windows, Ctrl+C é entregue ao processo nativo no mesmo console;
o launcher não chama kill(), que forçaria encerramento e impediria cleanup.
Encerramento forçado do SO/Task Manager não garante cleanup. Há testes POSIX
reais para sinais no CI; eles são ignorados no Windows, sem contar como
aprovados. TTY e Ctrl+C do console Windows precisam também de teste manual.

Para preparar o par de tarballs Windows localmente, use saídas novas:

```powershell
cargo build --locked --release -p myagent-cli
node scripts/npm/pack.mjs main - release/check/main
node scripts/npm/pack.mjs win32-x64 target/release/myagent.exe release/check/win32-x64
node scripts/npm/smoke.mjs release/check release/check/win32-x64.smoke.json
```

O smoke retém sua fixture em temporário para inspeção, com home/npmrc/cache
isolados e sem copiar credenciais do usuário. Não lê estado real do My-Agent.
Seu registry nunca encaminha requisições para serviços externos. Os testes
do publisher simulam publicação em memória: não chamam npm publish.

## Cadastro antes da primeira tag

Siga a documentação oficial de [Trusted Publishers](https://docs.npmjs.com/trusted-publishers/)
e [provenance](https://docs.npmjs.com/generating-provenance-statements/).
O workflow usa Node 24, npm 11.19.0, runners GitHub hospedados e id-token:write
somente no job de publicação. Não há token npm persistente no workflow.

Para **cada um dos seis pacotes**, cadastre no npm Settings → Trusted
Publisher → GitHub Actions:

- Organization/user: `mrJoao28`.
- Repository: `My-Agent`.
- Workflow filename: `release.yml` (sem o diretório).
- Environment: `npm-release`.
- Permissão para `npm publish` direto; o fluxo não usa npm stage publish.

Crie também o environment npm-release no GitHub com main e tags v* permitidas.
Reviewers são opcionais: se habilitados, cada publicação aguarda aprovação.
Não considere esse controle ativo apenas porque seu nome aparece no YAML.
Confira permissão de publicação
no scope @j2008 e acesso público/repository dos seis manifests.

O cadastro pode exigir que o pacote já exista: a primeira publicação pode
precisar ser manual e exige autorização própria. Não contorne isso com uma
tag às cegas. Prepare os seis tarballs e relatórios do workflow manual,
publique primeiro as plataformas e confira suas integridades públicas.
Somente depois publique o principal. Cadastro novo de Trusted Publisher
pode expirar se não for usado a tempo; confira o prazo atual no npm.

Uma publicação local manual pode exigir login/2FA e **não gera provenance
GitHub OIDC**. Essa eventual exceção de bootstrap deve ser aprovada
explicitamente; para exigir provenance desde a primeira versão, realize o
bootstrap em CI hospedado com autenticação autorizada temporariamente e
`--provenance`. O workflow preparado usa Trusted Publisher e não inclui um
caminho alternativo automático com token. Retire qualquer credencial de
bootstrap após o cadastro e não a grave no repositório.

## Checklist manual para bootstrap ou release por tag

Este caminho é alternativo à automação autorizada. Os exemplos não foram
executados contra npm/GitHub nesta implementação; os testes usam fixtures.

1. Revisar diff e documentação, cadastrar publishers/environment e obter
   autorização para commit. Depois, obter autorização para push/PR/merge.
2. No checkout aprovado, conferir versão e gates:

```powershell
node scripts/npm/version.mjs
node --test scripts/npm/launcher.test.mjs scripts/npm/auto-release.test.mjs
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --workspace -- --test-threads=1
```

3. Depois do workflow estar no remoto, executar **sem publicação** e revisar
   os relatórios nativos e container:

```powershell
gh workflow run release.yml --ref main
gh run list --workflow release.yml
# Substitua RUN_ID pelo run manual concluído:
gh run download RUN_ID --dir release/validated
node scripts/npm/verify.mjs release/validated
```

4. Solicitar autorização **separada** para criar a tag e outra para seu push.
   Push da tag deixa o job publish aguardando a proteção do environment:

```powershell
# Use a versão efetivamente validada:
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin refs/tags/v0.1.0
```

5. Solicitar autorização explícita de **publicação npm** antes de aprovar o
   deployment npm-release. O workflow publica com `--provenance --access
   public`, plataformas primeiro, confere integridades no npm público e
   reconfirma todas imediatamente antes do principal. Erro de rede/autorização
   não é interpretado como pacote ausente. Artefato divergente impede escrita.
6. Para o eventual bootstrap manual autorizado, usar os tarballs já testados,
   `npm login`/`npm whoami` fora do repositório de credenciais, e publicar cada
   plataforma com `npm publish CAMINHO.tgz --access public`. Em CI hospedado
   autorizado, incluir `--provenance`. Depois de confirmar as cinco versões
   e dist.integrity, publicar o tarball principal por último. Nunca publicar
   diretórios reconstruídos no lugar dos tarballs revisados.
7. Após publicação, conferir as seis versões, attestations e instalação limpa:

```powershell
node scripts/npm/publish.mjs --check release/validated
npm i -g @j2008/myagent
myagent --version
myagent doctor
```

O comando --check consulta o registry público e não escreve. E404 na primeira
publicação é ausência esperada, mas não comprova autorização para publicar.
No PowerShell, use npm.cmd se a política local bloquear npm.ps1.

## Recuperação de publicação parcial

Não tente sobrescrever versões. Preserve os artefatos e recibos originais
do run que publicou parte dos pacotes. Se ele falhou no publish, confira
`npm view PACOTE@VERSAO dist.integrity --json` para os seis nomes. Reexecute
somente o job publish falho (e respeite eventual revisão do environment); ele reutiliza
os mesmos artefatos, ignora versões presentes com integridade idêntica e
publica somente as ausentes, confirmando disponibilidade antes do principal.

Se algum pacote existir com integridade diferente, o preflight interrompe
antes de publicar qualquer outro. Recupere os artefatos exatos do primeiro
run; se isso não for possível, escolha uma nova versão para **todos** os
pacotes, sincronize com version.mjs --set e repita a validação completa.
Uma recompilação pode produzir bytes diferentes; não use re-run de todos
os jobs como substituto dos artefatos originais. Não mova/recrie tag já
publicada nem faça unpublish automático. Nunca promova o principal enquanto
faltar uma plataforma prevista, mesmo que optionalDependencies torne a
instalação npm aparentemente bem-sucedida.

Comandos para inspecionar e retomar uma falha exclusivamente no publish:

```powershell
npm.cmd view '@j2008/myagent-win32-x64@0.1.0' dist.integrity --json
# Repita para linux-x64, linux-arm64, darwin-arm64, darwin-x64 e principal.
# Compare com os recibos originais e obtenha nova autorização de publicação:
gh run rerun RUN_ID --failed
```

Esse último comando pode voltar a publicar os pacotes ausentes; não é um
dry-run. No fluxo automático autorizado não é necessário reenviar a tag. Não o execute no lugar do workflow_dispatch
usado somente para preparar e testar.

Se uma release automática falhar antes de publicar qualquer pacote, a tag
preparada continua reservada. Corrigir código na main prepara a próxima
versão; reexecutar somente jobs falhos conserva o mesmo SHA. Não reexecute
todos os jobs depois de uma publicação parcial, pois recompilar pode alterar
a integridade dos tarballs. Reexecução de prepare sem mudanças desde sua tag
não publica novamente.
