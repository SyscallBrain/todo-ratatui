use std::path::PathBuf;
use std::process::ExitCode;

use todo_ratatui::core::{Store, backup_path_for, resolve_path};
use todo_ratatui::tui;
use todo_ratatui::tui::arranque::{self, Opcoes};

const USAGE: &str = "\
todo-ratatui — TODO list em TUI

USO:
    todo-ratatui [--db <caminho>] [--theme <slug>] [--color <auto|rgb|ansi>]
                 [--config <caminho>]

OPÇÕES:
    --db <caminho>      Ficheiro da base de dados (precede TODO_RATATUI_DB)
    --theme <slug>      Tema do ecrã: tokyo-night (por omissão),
                        tokyo-night-storm, tokyo-night-moon, classico
                        (precede TODO_RATATUI_THEME e o config.json)
    --color <modo>      auto (por omissão), rgb ou ansi (precede
                        TODO_RATATUI_COLOR); em ansi um tema com fundo passa a
                        classico, só nesta sessão
    --config <caminho>  Ficheiro da preferência de tema, onde a caixa de temas
                        grava (precede TODO_RATATUI_CONFIG)
    -h, --help          Esta ajuda

Cada opção aceita «--opção valor» e «--opção=valor».

O tema vem de --theme, senão TODO_RATATUI_THEME, senão o config.json, senão
tokyo-night. Um valor desconhecido avisa e não impede abrir a lista, e nem a
flag nem a variável gravam nada: só o Enter da caixa de temas (T) grava.
";

struct Args {
    db: Option<PathBuf>,
    opcoes: Opcoes,
    help: bool,
}

fn parse_args(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        db: None,
        opcoes: Opcoes::default(),
        help: false,
    };
    let mut argv = argv.into_iter();
    while let Some(arg) = argv.next() {
        // `--theme=x` e `--theme x` valem o mesmo: separa-se no primeiro `=` e
        // o valor é o resto (sem ele, o argumento seguinte).
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, valor)) => (flag, Some(valor)),
            None => (arg.as_str(), None),
        };
        match flag {
            "-h" | "--help" => args.help = true,
            "--db" => args.db = Some(PathBuf::from(valor(flag, inline, &mut argv)?)),
            "--theme" => args.opcoes.theme = Some(valor(flag, inline, &mut argv)?),
            "--color" => args.opcoes.color = Some(valor(flag, inline, &mut argv)?),
            "--config" => {
                args.opcoes.config = Some(PathBuf::from(valor(flag, inline, &mut argv)?));
            }
            other => return Err(format!("argumento desconhecido: {other}")),
        }
    }
    Ok(args)
}

/// Valor de uma flag: o que veio depois do `=` ou o argumento seguinte.
///
/// Um `=` a mais fica dentro do valor (não há valores com `=` hoje, e
/// adivinhar onde cortar era pior do que dizer o que falta).
fn valor(
    flag: &str,
    inline: Option<&str>,
    argv: &mut impl Iterator<Item = String>,
) -> Result<String, String> {
    match inline {
        Some(valor) => Ok(valor.to_owned()),
        None => argv
            .next()
            .ok_or_else(|| format!("{flag} precisa de um valor")),
    }
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            // O `err` do `parse_args` embute o *token* cru do `argv`: passa pela
            // fronteira de impressão como tudo o resto que não é literal (o
            // `USAGE` é, esse, literal). O `\n\n` fica **fora** do saneamento.
            eprintln!("erro: {}\n\n{USAGE}", arranque::imprimivel(&err));
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
            eprintln!("erro: {}", arranque::imprimivel(&err.to_string()));
            return ExitCode::FAILURE;
        }
    };
    let store = match Store::open(&path) {
        Ok(store) => store,
        Err(err) => {
            // Adenda 2 do ADR: falhar ao **abrir** recusa o arranque — nunca
            // uma TUI sobre uma lista vazia em memória, que à primeira
            // gravação reescreveria o ficheiro que o utilizador ainda podia
            // recuperar. O `Store::open` nunca reescreve o original, e a saída
            // do utilizador é o `.bak`, logo a mensagem nomeia os dois.
            eprintln!("erro: {}", arranque::imprimivel(&err.to_string()));
            eprintln!(
                "a base de dados não foi alterada; a geração anterior está em «{}»",
                arranque::imprimivel(&backup_path_for(&path).display().to_string())
            );
            return ExitCode::FAILURE;
        }
    };

    // A preferência resolve-se **depois** de a base de dados abrir: uma recusa
    // de arranque sai com a mensagem de hoje, sem avisos de tema pelo meio, e a
    // lista abre na mesma com uma preferência estragada (ADR §Decisão 7).
    let sessao = arranque::resolver(&args.opcoes, &arranque::Ambiente::do_processo());
    // Antes do ecrã alternativo: depois do `try_init` a mensagem ficava por
    // baixo do primeiro desenho.
    sessao.avisar();

    match tui::run(store, &sessao) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("erro: {}", arranque::imprimivel(&err.to_string()));
            ExitCode::FAILURE
        }
    }
}
