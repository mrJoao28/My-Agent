#![forbid(unsafe_code)]

use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

const HELP: &str = "My-Agent — assistente de programação de terminal.

Uso: myagent [OPÇÕES]

Opções:
  -h, --help     Exibe esta ajuda.
  -v, --version  Exibe a versão.
";

fn write_output(output: &str) -> ExitCode {
    match io::stdout().lock().write_all(output.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn invalid_arguments() -> ExitCode {
    eprintln!("Argumentos inválidos. Use myagent --help.");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let first = args.next();
    if args.next().is_some() {
        return invalid_arguments();
    }

    match first.as_deref() {
        None => write_output(HELP),
        Some(arg) if arg == "--help" || arg == "-h" => write_output(HELP),
        Some(arg) if arg == "--version" || arg == "-v" => {
            write_output(&format!("myagent {}\n", env!("CARGO_PKG_VERSION")))
        }
        Some(_) => invalid_arguments(),
    }
}
