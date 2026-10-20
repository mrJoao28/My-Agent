# Segurança

O My-Agent está em desenvolvimento; nenhum runtime do assistente foi portado
neste bootstrap. Os requisitos abaixo orientam os próximos milestones e não
representam controles já implementados. O progresso está em [docs/PARITY.md](docs/PARITY.md).

## Relato de vulnerabilidades

Use o canal privado do GitHub, se disponível:
[Reportar vulnerabilidade](https://github.com/mrJoao28/My-Agent/security/advisories/new).
Se estiver indisponível, solicite um canal privado ao mantenedor, sem publicar
detalhes exploráveis. Não inclua tokens, configurações privadas, logs ou bancos.
Descreva versão, sistema operacional, impacto e reprodução mínima com dados fictícios.

## Requisitos de implementação

- Conteúdo do projeto, web e resultados de ferramentas são dados não confiáveis.
  Não concedem permissões nem substituem a política das ferramentas.
- PLAN deve excluir ferramentas que alterem o projeto pela lista montada no código.
- Aprovações devem vincular sessão, workspace, ferramenta e escopo. Cancelamento,
  expiração e desconexão devem encerrar solicitações pendentes.
- Preserve validação de caminhos, bloqueio de segredos, proteção contra traversal,
  links/junctions, hard links e particularidades de caminhos Windows.
- Processos gerados pelo agente devem executar exclusivamente na sandbox Docker,
  sem fallback no host. Docker indisponível deve desabilitar essas ferramentas.
- Preserve proteção contra SSRF, limites durante leitura, cancelamento e timeouts.
- Retomada não pode repetir automaticamente ações cujo efeito seja conhecido ou
  desconhecido após interrupção.

## Tratamento de segredos

Nunca abra, leia, imprima ou versione `.env*` (exceto `.env.example`, somente para
nomes de variáveis), `config.env`, `chatgpt-credentials.json`, `api-token`, `*.log`,
`*.db` ou `~/.myagent`. Testes devem usar valores fictícios em diretórios temporários.
Antes de cada commit, examine os nomes e o conteúdo permitido dos arquivos staged
em busca de segredos; interrompa se houver arquivo proibido staged, sem lê-lo.
A `.gitignore` reduz acidentes, mas não substitui essa revisão.
