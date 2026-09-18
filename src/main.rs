use std::path::PathBuf;
use std::process::ExitCode;

use todo_ratatui::core::{Store, resolve_path};
use todo_ratatui::tui;

const USAGE: &str = "\
todo-ratatui — TODO list em TUI

USO:
    todo-ratatui [--db <caminho>]

OPÇÕES:
    --db <caminho>   Ficheiro da base de dados (precede TODO_RATATUI_DB)
    -h, --help       Esta ajuda
";

struct Args {
    db: Option<PathBuf>,
    help: bool,
}

fn parse_args(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        db: None,
        help: false,
    };
    let mut argv = argv.into_iter();
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-h" | "--help" => args.help = true,
            "--db" => {
                let value = argv
                    .next()
                    .ok_or_else(|| "--db precisa de um caminho".to_owned())?;
                args.db = Some(PathBuf::from(value));
            }
            other if other.starts_with("--db=") => {
                args.db = Some(PathBuf::from(&other["--db=".len()..]));
            }
            other => return Err(format!("argumento desconhecido: {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("erro: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let path = match resolve_path(args.db.as_deref()) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("erro: {err}");
            return ExitCode::FAILURE;
        }
    };
    let store = match Store::open(path) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("erro: {err}");
            return ExitCode::FAILURE;
        }
    };

    match tui::run(store) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("erro: {err}");
            ExitCode::FAILURE
        }
    }
}
