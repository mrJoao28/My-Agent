# My-Agent

Assistente de programação de terminal em Rust, com TUI Grafite, PLAN/BUILD,
aprovações, checkpoints e undo. Node serve apenas como launcher do binário.

```sh
npm i -g @j2008/myagent
myagent --version
myagent doctor
myagent
```

Requer Node.js >= 22.14.0 e npm com dependências opcionais habilitadas.
Windows x64, Linux x64/arm64 e macOS x64/arm64 têm pacotes nativos separados.
Não requer Rust, Bun ou compilador na instalação. Não há postinstall,
download adicional nem compilação local. Docker é opcional para a sandbox.

`myagent --demo` usa projeto e estado temporários, sem credenciais reais.
`MYAGENT_HOME` define o diretório de estado; preserve esse diretório privado.
Consulte [documentação e segurança](https://github.com/mrJoao28/My-Agent).
