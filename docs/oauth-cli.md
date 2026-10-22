# OAuth ChatGPT e comandos sem TUI

O login usa o [fluxo oficial para clientes open source](https://developers.openai.com/siwc/token-sharing-open-source/sign-in).
É independente de `OPENAI_API_KEY`. Não importa credenciais do Codex, do
legado nem de outros aplicativos. Os endpoints normais são exclusivamente
`auth.openai.com` e a API pública `api.openai.com/v1`.

## Conta e modelos

Na TUI, `/chatgpt` oferece login, reautorização de um registro, ativação e
logout. Abra no navegador a URL apresentada; Esc ou Ctrl+C cancela a espera.
O callback loopback usa porta aleatória, state, nonce e PKCE S256. A identidade
é validada pela assinatura JWKS, issuer, audience do client_id emitido,
expiração e subject; o callback válido é consumido uma única vez.

```powershell
cargo run -p myagent-cli -- chatgpt login
cargo run -p myagent-cli -- chatgpt status
cargo run -p myagent-cli -- chatgpt use CLIENT_ID
cargo run -p myagent-cli -- chatgpt login --account CLIENT_ID
cargo run -p myagent-cli -- chatgpt logout CLIENT_ID
```

O host recebe um identificador opaco estável antes do primeiro login. O
primeiro registro usa `dynamic_agent_client`; o client_id emitido é retido
para reautorização. Cada registro tem sua identidade validada e catálogo;
email igual não mescla contas. Logout remove tokens e catálogo daquele
registro, mas preserva seu client_id, identidade e vínculo com o host.

O seletor `/models` consulta os modelos visíveis da conta ativa, preservando
a ordem do servidor. A escolha do modelo é explícita. Sem os escopos de uso
do plano, ou diante de recusa/limite da assinatura, a interface informa a
situação e oferece reautorizar ou escolher outro modelo. Não troca conta,
modelo nem credencial automaticamente. O catálogo usa `GET /v1/models`;
inferência usa `POST /v1/responses` com `store=false` e `stream=true`.
Somente `response.completed` encerra a resposta com sucesso; erro ou fim
prematuro preserva o texto parcial e não provoca nova inferência automática.

## Persistência e concorrência

O serviço privado do Store mantém os registros em `oauth-rust` sob o home
configurado da aplicação. Tokens não entram nas mensagens nem no SQLite
das sessões. O arquivo JSON é protegido antes da primeira gravação e
substituído atomicamente com tokens rotacionados, escopos e validade juntos.
Um arquivo de lock estável serializa as operações entre processos, inclusive
a requisição de refresh e sua gravação. Caminhos com links/junctions,
hardlinks, dono incorreto ou permissões amplas são recusados. Unix exige
0700/0600 e UID atual; Windows exige DACL protegida exclusiva do usuário
atual e dono correspondente. Um arquivo inválido é preservado.

O refresh usa o client_id do registro e o refresh token atual, sem reenviar
scope, respeitando `earliest_refresh_at`. `invalid_grant`, token expirado sem
renovação ou 401 invalidam apenas as credenciais afetadas. Logout tenta
revogar remotamente e limpa o estado local; se a revogação não for confirmada,
a interface informa isso. Troca de conta/logout cancela a geração vinculada
à conta anterior; o backend observa também mudanças feitas por outro
processo. O cancelamento segue o cleanup normal do Agent e das ferramentas.

## Exec e doctor

```powershell
cargo run -p myagent-cli -- --demo exec "demonstrar arquivos" --json
cargo run -p myagent-cli -- --demo exec "demonstrar arquivos" --mode build --approve all --json
cargo run -p myagent-cli -- --demo doctor --json
cargo run -p myagent-cli -- --model chatgpt:MODELO exec "Analise o projeto" --mode plan
```

`exec` cria uma sessão do workspace e usa o mesmo backend da TUI: Agent,
StoreJournal, leases, checkpoints e aprovação. PLAN é o padrão e o core
filtra as ferramentas mutáveis. `--approve none` nega pedidos; `reads`
autoriza apenas ferramentas web; `all` concede aprovação uma vez por pedido.
Push ainda exige `--allow-push`, e a confirmação específica de main/master
nunca é concedida automaticamente. Nenhuma ferramenta é executada antes
da decisão válida. O limite do prompt continua sendo 9.000 unidades UTF-16.

`--json` em exec produz eventos JSONL e um registro `exec-result` final com
status, sessão, modo, modelo, texto, tokens medidos e quantidade de chamadas.
`doctor --json` produz um documento com `schemaVersion`, `checks` e
`exitCode`. Doctor abre/migra SQLite; não executa scripts do projeto. Verifica
workspace/configuração, persistência, disponibilidade de sandbox, modelos
configurados e estado privado OAuth. Não exibe valores de chaves/tokens.

| Código | exec | doctor |
| --- | --- | --- |
| 0 | Geração concluída | Verificações essenciais aprovadas; avisos são possíveis |
| 1 | Configuração, autenticação, rede, streaming ou persistência falhou | Verificação essencial falhou |
| 2 | Argumentos ou prompt inválidos | Argumentos inválidos |
| 3 | Alguma aprovação foi negada | Não utilizado |
| 130 | Interrompido | Interrompido |

Nenhum comando retorna sucesso após erro de streaming ou ausência de evento
terminal. A política de saída prioriza interrupção, negação e falha nessa ordem.

## Dependências e limites da validação

`jsonwebtoken` valida JWT por implementação criptográfica existente; `sha2`,
`base64` e `getrandom` implementam PKCE e valores aleatórios. `tempfile`
oferece substituição atômica. `windows-permissions` fornece wrappers seguros
de ACL; `rustix` permite consultar o UID sem unsafe local. `rsa` e `rand`
são dependências de teste para um emissor/JWKS fictício. A dependência de
providers para Store reutiliza somente o serviço privado de arquivos, sem
acoplar tokens ao journal de sessões.

A evidência de execução está em [PARITY.md](PARITY.md). Login com conta real,
inferência externa paga e revogação real não fazem parte dos testes fictícios.
O login apresenta uma URL para abertura manual; abertura automática do
navegador e paridade integral do diagnóstico de ferramentas na imagem
Docker continuam pendentes. Os screenshots/mockup anteriormente citados
não estavam disponíveis; nenhuma comparação visual com eles foi declarada.
