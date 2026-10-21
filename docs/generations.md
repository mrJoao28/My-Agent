# Providers e gerações — M2

## Fronteiras

O core contém portas Provider, Journal, Tool e InstructionSource. Providers
só traduzem requisições/eventos; não executam ferramentas. StoreJournal faz
a ponte com SQLite do M1. O core envia I/O bloqueante do journal e das
instruções para spawn_blocking, sem bloquear o executor assíncrono.

As APIs são de biblioteca. A CLI continua oferecendo ajuda/versão; interface
de conversa, comandos e TUI entram nos seus milestones. Nenhuma ferramenta
de arquivos, shell, Git ou web é registrada automaticamente no M2.
Ferramentas reais precisam implementar autorização/validação e execução;
os testes usam implementações fictícias. Learn não foi implementado.

## Modelos e configuração

O catálogo embutido é uma fotografia dos 13 modelos de
packages/shared/src/models.ts do legado, com seus IDs, preços e janelas.
Não é uma promessa de disponibilidade ou preço atual de uma API. O modelo
padrão preservado é claude-opus-4-6.

ModelRegistry::load recebe o caminho explícito do models.json. O chamador
pode usar o home resolvido pelo StoreConfig do M1 e anexar models.json.
O loader é somente leitura, limita o arquivo a 1 MiB e 1000 entradas,
recusa links/junctions/hard links, campos desconhecidos, IDs duplicados,
ChatGPT manual, janelas inválidas e preços negativos/não finitos.
JSON inválido é recusado e preservado. Edição/diálogo de modelos fica para
a integração da interface; não há reescrita automática do arquivo.

O parser aceita id, provider, displayName, apiKeyEnv, contextWindow e pricing.
Overrides conhecidos mantêm o catálogo como fallback dos campos omitidos.
O nome de variável é apiKeyEnv, quando explícito, ou:

`MYAGENT_<PROVIDER>_<ID_NORMALIZADO>_API_KEY`

A normalização replica getModelApiKeyEnv: maiúsculas, grupos não alfanuméricos
como underscore, sem underscores nas pontas. Por exemplo,
MYAGENT_OPENAI_GPT_5_4_API_KEY. Nomes reservados de ambiente são recusados.
HttpProvider::resolve recebe lookup injetável; testes fornecem valores
fictícios e não consultam chaves reais. Chave cloud ausente é erro antes
da rede. Nenhum login ou provisionamento de credencial foi executado.

Ollama usa OpenAI Chat Completions, OLLAMA_BASE_URL com padrão
http://localhost:11434/v1 e OLLAMA_API_KEY opcional (fallback ollama).
Endpoints locais podem usar HTTP; cloud usa HTTPS, exceto loopback de teste.
Redirects não são seguidos e o cliente usa conexões diretas. As chaves
ficam em headers sensíveis, nunca na URL ou no Debug/erro público.

resolve_window distingue override, catálogo exato, família com sufixo
YYYY-MM-DD, hint válido do provider e fallback estimado: 16.384 tokens local,
65.536 cloud. Origem e estimativa são retornadas explicitamente.

## HTTP e streaming

| Provider | Dialeto e estado preservado |
| --- | --- |
| Anthropic | Messages SSE; thinking/text deltas, JSON de tool_use, signatures, uso e message_stop com stop_reason válido. Mantém o budget de thinking do catálogo legado quando cabe na saída. |
| OpenAI | Responses SSE; texto/reasoning, argumentos fragmentados, function calls, encrypted reasoning e response.completed. Não trata incomplete/failed/EOF como conclusão. |
| Google | streamGenerateContent SSE; thought/text, functionCall, thoughtSignature e usageMetadata. STOP válido mais fim do corpo concluem o stream. |
| Ollama | Chat Completions SSE; reasoning/reasoning_content, texto, chamadas fragmentadas, usage e marcador DONE com finish_reason válido. |

Leitura limitada durante o recebimento: 1 MiB por linha/payload SSE,
16 MiB por resposta HTTP e 64 KiB de argumentos por chamada. Esse último
é o limite efetivo via modelo, embora a API local de mutações aceite até
8 MiB; alterações grandes devem ser divididas em chamadas direcionadas. O parser monta
UTF-8 a partir dos bytes, suporta LF/CRLF/CR, BOM, comentários e data multilinha.
Chamada incompleta ou argumentos finais inconsistentes falham antes de efeitos.
Status HTTP, content-type, JSON, contagens e conclusão são conferidos.
Erros públicos não incluem chave, URL com credencial ou corpo remoto.

Frames viram ProviderEvent; o core persiste e envia StreamEvent em canais
limitados, incluindo texto, reasoning, chamada, resultado, usage, avisos,
done e error. Não há servidor HTTP/token/SSE local entre core e UI.

Metadados opacos necessários para continuar uma etapa são salvos antes de
efeitos. Na retomada são associados aos IDs da geração; um novo pedido do
usuário não reintroduz reasoning da geração anterior. Metadados nunca
concedem capacidades nem aprovação.

## Loop e journal

O lock por sessão é adquirido antes de qualquer await de I/O. Leases
SQLite continuam sendo a exclusão entre processos. Cada geração cria
ASSISTANT/IN_PROGRESS antes de inferir. Deltas são gravados incrementalmente;
textos/reasoning consecutivos compartilham uma part. Chamadas são gravadas
como pending, depois started antes da execução, e resultados antes da
próxima inferência. Só a conclusão explícita e validada permite COMPLETE.
Falhas e cancelamento preservam os parciais como INTERRUPTED.

O envio para providers só ocorre depois de validar, ajustar e salvar o
snapshot de contexto. O snapshot retornado pelo journal é o enviado.
Na etapa final, a resposta completa fica no journal; SessionContext mantém
o último snapshot de entrada salvo. Ao abrir uma nova geração, o histórico
é reconstruído das mensagens, preservando a ordem dos pares e do texto.

MYAGENT_MAX_TOOL_CALLS é interpretado na fronteira por
configured_max_tool_calls(lookup). O padrão é 40; aceita apenas inteiros de
1 a 200. Inválidos, incluindo valores acima de 200, voltam a 40, conforme
o pedido M2 (o legado limitava valores grandes a 200). Quando restam até
duas chamadas, a próxima etapa recebe ferramentas vazias e prompt de
síntese. Valores 1/2 começam diretamente em síntese. Um lote acima do
orçamento é recusado antes de executar seus efeitos.

Canonização ignora ordem de chaves de objetos, preservando arrays. Até
três propostas idênticas são admitidas; resultado completo ainda presente
no snapshot evita reexecutar a mesma ferramenta. Escrita, abreviação,
remoção do contexto e notas de undo invalidam evidências pertinentes. Notas
locais de undo são mensagens USER e invalidam o cache antes do filtro de
chamadas ASSISTANT, inclusive no histórico reconstruído do SQLite.
O orçamento e as contagens de repetição são preservados na retomada.

Retomar requer pedido explícito do chamador, modelo/modo compatíveis e USER
pendente ou ASSISTANT interrompido. O generationId original é preservado.
Ferramenta com efeito já started, ou de legado sem execution explícito,
não é repetida automaticamente, mesmo sem resultado. O estado precisa ser
inspecionado e uma nova instrução solicitada. Recuperação de órfãs respeita
workspaces e não interrompe gerações com leases vivos de outro processo.

A execução tem deadline global de cinco minutos. Um monitor independente
verifica o lease a cada cinco segundos, inclusive durante ferramentas longas,
e cancela a geração se a propriedade for perdida. Cancelamento usa árvore de
tokens, chega ao HTTP e ao contexto de execução das ferramentas, e também é
acionado quando o consumidor abandona a geração. O cleanup respeita o prazo declarado por cada ferramenta: dois segundos
por padrão, quinze para execução Docker e 310 para alterações com captura
durável do estado posterior (até 300 segundos). Os prazos partem do mesmo
instante de encerramento, sem acumular um novo prazo por tarefa. Falha, panic
ou timeout no cleanup produzem erro explícito e estado INTERRUPTED; uma falha
de remoção Docker continua visível mesmo após cancelar a inferência. O supervisor de processo adota um Child já autorizado,
limita stdout/stderr com orçamento compartilhado, mata e aguarda o processo.
Ele não cria comandos, não é um executor no host e não implementa Docker;
encerrar um futuro container inteiro cabe ao adapter Docker.

## Contexto, prompts e tokens

Sistema, schemas e saída têm reservas explícitas. Janelas menores que
32.768 usam ferramentas essenciais, incluindo test_run e type_check quando
o adapter Docker estiver disponível. PLAN continua sem bash ou mutações. A 70% do orçamento, evidência
antiga é resumida localmente, preservando o pedido atual e pares completos.
Somente resultados confirmados atribuem alterações; falhas e efeitos
desconhecidos viram pendências. Resultados abreviados são marcados;
páginas de read_file mantêm JSON/paginação e somente linhas completas.
Pedido atual que não cabe é rejeitado, sem corte silencioso.
O histórico completo permanece no SQLite.

A estimativa segue a heurística do legado (bytes JSON/3 + overhead por
mensagem); não é um tokenizer exato e não é consumo medido. Uso informado
pelo provider é acumulado por etapa sem duplicar reportes cumulativos,
persistido inclusive em interrupções. token_usage soma somente medições
existentes e informa quantas mensagens não tiveram usage. Ausência não
vira estimativa nem zero fictício.

prompts.rs é a única fonte dos seis blocos do sistema e dos métodos
PLAN/BUILD. A lista real de ferramentas é montada pelo código: PLAN exclui
mutações, inclusive nomes conhecidos classificados incorretamente como leitura.
Texto de prompt/arquivo/resultado jamais amplia essa lista.

AGENTS.md tem snapshot por geração, raiz antes de subpastas, entrega única,
até 128 candidatos/profundidade 32 e 16 KiB totais com envelopes/Unicode.
A fonte de arquivos é somente leitura e recusa binários, caminhos sensíveis,
symlinks, hard links e junctions. Antes de autorizar um lote que contém mutações, todos os seus caminhos são
consultados. Se houver instruções novas, nenhuma chamada desse lote executa
ou consome aprovação: resultados explícitos devolvem as regras ao modelo,
que precisa propor novamente as chamadas. O orçamento de chamadas continua
valendo; chamadas adiadas ficam pending com resultado de adiamento.
A fonte normaliza caminhos absolutos, relativos e separadores Windows para
escopos relativos ao workspace. Listas files de objetos e strings são
aceitas; alvos de runners são relativos ao path selecionado. A verificação de ancestrais não elimina uma
corrida com outro processo que possa trocar diretórios simultaneamente.

## Dependências e validação

| Dependência direta nova neste M2 | Necessidade |
| --- | --- |
| tokio | Runtime, canais limitados, tarefas, timers, rede simulada e supervisão assíncrona de processos. test-util somente em dev-dependencies para testar cinco minutos com relógio virtual. |
| tokio-util | CancellationToken hierárquico compartilhado por rede, loop e processos. |
| reqwest com rustls-tls | HTTP incremental/TLS sem OpenSSL do sistema. JSON usa serde_json já adotado, sem SDK de LLM ou execução automática de tools. |
| winapi-util em providers/tools, Windows | Reutiliza a dependência do M1 para recusar hard links antes de ler models.json/AGENTS.md com API segura. |
| tempfile/tokio em testes de adapters | Fixtures isoladas, arquivos fictícios, loopback e relógio virtual; nenhum home ou segredo real. |

serde/serde_json/thiserror são reutilizados para contratos e erros tipados.
Não foram adicionados async-trait, SDK de LLM, tokenizer, regex ou crates de
busca/outline. A política de licenças foi revista com Cargo metadata e
arquivos LICENSE locais: ISC (ring/webpki/untrusted), BSD-3-Clause (subtle)
e CDLA-Permissive-2.0 (raízes TLS) foram acrescentadas. Avisos das dependências
devem ser preservados na distribuição. cargo-deny não foi executado.

Os testes usam providers roteirizados, SQLite temporário, servidores
127.0.0.1 com chaves fictícias e processos filhos de fixture. Não houve
chamada paga, inferência Ollama real, login ChatGPT ou acesso a credenciais.
Os resultados finais dos gates e limitações de ambiente ficam no PARITY.md.

## Referências dos codecs

Código do legado prevalece para catálogo, opções e comportamento. Para os
dialetos HTTP, foram consultadas as referências oficiais de
[streaming OpenAI](https://developers.openai.com/api/docs/guides/streaming-responses),
[function calling OpenAI](https://developers.openai.com/api/docs/guides/function-calling),
[streaming Anthropic](https://platform.claude.com/docs/en/build-with-claude/streaming)
e [streamGenerateContent Google](https://ai.google.dev/api/generate-content).
