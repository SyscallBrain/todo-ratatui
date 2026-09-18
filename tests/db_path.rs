//! Teste de precedência do caminho da base de dados.
//!
//! Vive sozinho num ficheiro de teste a propósito: manipula variáveis de
//! ambiente do processo, que são estado global — um segundo teste neste
//! binário correria em paralelo e corromperia o resultado.
//!
//! Precedência fixada pelo ADR (ponto 4 + adenda 1), da esquerda para a
//! direita: `--db`, `TODO_RATATUI_DB` e `dirs::data_dir()` (que resolve para
//! `$XDG_DATA_HOME`, ou `$HOME/.local/share`). Ficheiros de **dados**, não de
//! configuração: nada de `~/.config`.

use std::path::{Path, PathBuf};

use todo_ratatui::core::{ENV_DB, StoreError, resolve_path};

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
        std::env::set_var("HOME", "/tmp/casa");
        std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-dados");
        std::env::set_var(ENV_DB, "/tmp/do-ambiente/db.json");
    }
    assert_eq!(
        resolve_path(Some(&explicito)).unwrap(),
        explicito,
        "--db precede também o TODO_RATATUI_DB"
    );
    assert_eq!(
        resolve_path(None).unwrap(),
        PathBuf::from("/tmp/do-ambiente/db.json"),
        "TODO_RATATUI_DB precede o diretório de dados"
    );

    unsafe {
        std::env::remove_var(ENV_DB);
    }
    assert_eq!(
        resolve_path(None).unwrap(),
        Path::new("/tmp/xdg-dados")
            .join("todo-ratatui")
            .join("db.json"),
        "sem env, cai no XDG_DATA_HOME (dados, não configuração)"
    );

    unsafe {
        std::env::remove_var("XDG_DATA_HOME");
    }
    assert_eq!(
        resolve_path(None).unwrap(),
        Path::new("/tmp/casa")
            .join(".local")
            .join("share")
            .join("todo-ratatui")
            .join("db.json"),
        "sem XDG_DATA_HOME, cai em $HOME/.local/share"
    );

    // Medido com o `dirs` 6 (dirs-sys 0.5): com `HOME` removido o `data_dir()`
    // **ainda resolve**, porque cai no `pw_dir` da entrada de passwd do uid. O
    // erro `NoDataDir` só aparece quando não há variáveis *nem* entrada de
    // passwd — não é alcançável a partir de um teste. Fica assertado o que
    // acontece de facto, sem inventar um erro que a máquina não dá.
    unsafe {
        std::env::remove_var("HOME");
    }
    match resolve_path(None) {
        Ok(path) => assert!(
            path.ends_with("todo-ratatui/db.json"),
            "sem HOME o passwd ainda dá um diretório de dados, veio «{}»",
            path.display()
        ),
        Err(erro) => assert!(
            erro.to_string().contains("XDG_DATA_HOME"),
            "se a resolução falhar, a mensagem tem de nomear as variáveis"
        ),
    }

    // A mensagem do erro fixa-se directamente: é a que o `tui` mostra quando o
    // `dirs` não consegue decidir, e tem de nomear as duas variáveis e a saída.
    let mensagem = StoreError::NoDataDir.to_string();
    assert!(
        mensagem.contains("XDG_DATA_HOME") && mensagem.contains("HOME"),
        "a mensagem tem de nomear as duas variáveis, veio «{mensagem}»"
    );
    assert!(
        mensagem.contains("--db"),
        "e tem de dizer a saída, veio «{mensagem}»"
    );
}
