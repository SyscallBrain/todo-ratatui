//! Fronteira de camadas, verificada pelo `cargo test`.
//!
//! O ADR exige que o `core` seja testável sem TTY e sem backend de terminal.
//! Um comentário a prometer isso não trava nada; este teste trava: falha se
//! qualquer linha de código (fora de comentários) em `src/core/` mencionar
//! `ratatui` ou `crossterm`.

use std::fs;
use std::path::{Path, PathBuf};

fn ficheiros_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("ler diretório") {
        let entry = entry.expect("entrada");
        let path = entry.path();
        if path.is_dir() {
            ficheiros_rs(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn core_nao_depende_da_camada_de_terminal() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("core");
    let mut ficheiros = Vec::new();
    ficheiros_rs(&raiz, &mut ficheiros);
    assert!(!ficheiros.is_empty(), "não encontrei ficheiros em src/core");

    // Procuramos a *dependência* (`ratatui::`, `crossterm::`, `extern crate`),
    // não a palavra: `todo-ratatui` (nome do crate e do directório de
    // configuração) aparece legitimamente em código do core.
    let proibidos = [
        "ratatui::",
        "crossterm::",
        "extern crate ratatui",
        "extern crate crossterm",
    ];
    let mut violacoes = Vec::new();
    for ficheiro in &ficheiros {
        let conteudo = fs::read_to_string(ficheiro).expect("ler ficheiro");
        for (numero, linha) in conteudo.lines().enumerate() {
            let codigo = linha.trim_start();
            // Comentários (`//`, `///`, `//!`) podem falar das crates; código não.
            if codigo.starts_with("//") {
                continue;
            }
            if let Some(proibido) = proibidos.iter().find(|p| codigo.contains(**p)) {
                violacoes.push(format!(
                    "{}:{} menciona {proibido}: {}",
                    ficheiro.display(),
                    numero + 1,
                    codigo.trim()
                ));
            }
        }
    }
    assert!(
        violacoes.is_empty(),
        "a camada core tem de ser independente do terminal:\n{}",
        violacoes.join("\n")
    );
}
