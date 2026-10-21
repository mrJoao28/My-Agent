#![forbid(unsafe_code)]

use clap::{Arg, ArgAction, Command, error::ErrorKind};
use myagent_tui::{
    app::{App, Screen},
    ports::Backend,
    preferences::Preferences,
};
use std::process::ExitCode;
use std::{io::IsTerminal, path::PathBuf, sync::Arc};
mod backend;
mod demo;

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
        .arg(
            Arg::new("new")
                .long("new")
                .help("Inicia uma nova sessão.")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("cwd")
                .long("cwd")
                .value_name("PATH")
                .help("Diretório do projeto."),
        )
        .arg(
            Arg::new("model")
                .long("model")
                .value_name("ID")
                .help("Modelo inicial do catálogo."),
        )
        .arg(
            Arg::new("demo")
                .long("demo")
                .help("Demonstração offline em diretório temporário descartável.")
                .action(ArgAction::SetTrue)
                .conflicts_with("cwd"),
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
    } else if matches.get_flag("help")
        || (!std::io::stdin().is_terminal() && std::env::args_os().len() == 1)
    {
        match command.print_help() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        }
    } else {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            eprintln!("A TUI requer um terminal interativo. Use myagent --help.");
            return ExitCode::from(2);
        }
        match interactive(&matches) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("myagent: {error}");
                ExitCode::FAILURE
            }
        }
    }
}

fn interactive(matches: &clap::ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let demo = matches.get_flag("demo");
    let temporary = if demo {
        Some(tempfile::Builder::new().prefix("myagent-demo-").tempdir()?)
    } else {
        None
    };
    let (cwd, config) = if let Some(temp) = &temporary {
        let root = std::fs::canonicalize(temp.path())?;
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace)?;
        (
            workspace,
            myagent_store::StoreConfig {
                home: root.join("state"),
                retention_days: 0,
            },
        )
    } else {
        (
            matches
                .get_one::<String>("cwd")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?),
            myagent_store::StoreConfig::from_env()?,
        )
    };
    let preferences_path = config.home.join("preferences.json");
    let preferences = Preferences::load(&preferences_path);
    let backend = Arc::new(runtime.block_on(backend::LocalBackend::open(&cwd, config, demo))?);
    let mut view = runtime.block_on(backend.initial(matches.get_flag("new")))?;
    if let Some(model) = matches.get_one::<String>("model") {
        if !backend.valid_model(model) {
            return Err("Modelo desconhecido; consulte /models ou models.json.".into());
        }
        view.model = model.clone();
    }
    let mut app = App::new(
        backend.workspace().into(),
        backend.models(),
        view.model.clone(),
        preferences
            .as_ref()
            .map(|value| value.theme)
            .unwrap_or_default(),
        std::env::var_os("NO_COLOR").is_some(),
    );
    let screen = if matches.get_flag("new") {
        Screen::NewSession
    } else if view.messages.is_empty() {
        Screen::Home
    } else {
        Screen::Session
    };
    app.load(view, screen, true);
    if preferences.is_err() {
        app.notify(
            "preferences.json inválido ou inacessível; usando Grafite sem sobrescrever o arquivo.",
            true,
        );
    } else if demo {
        app.notify(
            "DEMO offline: arquivos e sessões temporários são removidos ao sair.",
            false,
        );
    } else if !backend.sandbox_available {
        app.notify(
            "Docker indisponível: bash, testes e tipos não serão oferecidos ao modelo.",
            false,
        );
    }
    myagent_tui::runtime::run(app, backend, preferences_path, &runtime)?;
    Ok(())
}
