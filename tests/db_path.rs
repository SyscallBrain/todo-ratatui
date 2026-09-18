//! Teste de precedência do caminho da base de dados.
//!
//! Vive sozinho num ficheiro de teste a propósito: manipula variáveis de
//! ambiente do processo, que são estado global — um segundo teste neste
//! binário correria em paralelo e corromperia o resultado.

use std::path::Path;
use std::path::PathBuf;

use todo_ratatui::core::{ENV_DB, resolve_path};

#[test]
fn precedencia_do_caminho_da_base_de_dados() {
    let explicito = PathBuf::from("/tmp/explicito/db.json");
    assert_eq!(
        resolve_path(Some(&explicito)).unwrap(),
        explicito,
        "--db precede tudo"
    );

    // Seguro em edição 2024: este binário tem um único teste.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg");
        std::env::set_var(ENV_DB, "/tmp/do-ambiente/db.json");
    }
    assert_eq!(
        resolve_path(None).unwrap(),
        PathBuf::from("/tmp/do-ambiente/db.json"),
        "TODO_RATATUI_DB precede o diretório de configuração"
    );

    unsafe {
        std::env::remove_var(ENV_DB);
    }
    assert_eq!(
        resolve_path(None).unwrap(),
        Path::new("/tmp/xdg").join("todo-ratatui").join("db.json"),
        "sem env, cai no XDG_CONFIG_HOME"
    );

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::set_var("HOME", "/tmp/casa");
    }
    assert_eq!(
        resolve_path(None).unwrap(),
        Path::new("/tmp/casa")
            .join(".config")
            .join("todo-ratatui")
            .join("db.json"),
        "sem XDG, cai em $HOME/.config"
    );
}
