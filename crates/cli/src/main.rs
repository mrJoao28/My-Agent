#![forbid(unsafe_code)]

use clap::{Arg, ArgAction, Command, error::ErrorKind};
use std::process::ExitCode;

fn command() -> Command {
    Command::new("myagent")
        .version(env!("CARGO_PKG_VERSION"))
        .about("My-Agent — assistente de programação de terminal.")
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .long("help")
                .short('h')
                .help("Exibe esta ajuda.")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .short('v')
                .help("Exibe a versão.")
                .action(ArgAction::SetTrue),
        )
        .group(
            clap::ArgGroup::new("display")
                .args(["help", "version"])
                .multiple(false),
        )
}

fn main() -> ExitCode {
    let mut command = command();
    let matches = match command.clone().try_get_matches() {
        Ok(matches) => matches,
        Err(error) => {
            let code = if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                0
            } else {
                2
            };
            let _ = error.print();
            if code != 0 {
                eprintln!("Use myagent --help para consultar as opções disponíveis.");
            }
            return ExitCode::from(code);
        }
    };
    if matches.get_flag("version") {
        println!("myagent {}", env!("CARGO_PKG_VERSION"));
        ExitCode::SUCCESS
    } else {
        match command.print_help() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        }
    }
}
