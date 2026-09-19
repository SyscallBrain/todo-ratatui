//! Injectão de caracteres de controlo no ecrã (T3, achado SA-05).
//!
//! O modelo **não** valida caracteres de controlo nos títulos (`Todo::try_new`
//! só faz `trim`) e o import também não (desserializa `Todo` por serde). A
//! defesa que existe é a do `ratatui-core`: o `set_stringn` filtra
//! (`char::is_control`) antes de a célula do `Buffer` existir, logo nada do que
//! o `App` desenha pode escrever um escape no terminal.
//!
//! Isso é uma propriedade de **outra crate** — e é exactamente por isso que
//! este teste existe: no dia em que o `ratatui` deixe de filtrar, isto tem de
//! dar `cargo test` vermelho em vez de um achado futuro. Não substitui uma
//! validação no modelo (o ADR recusa-a por custo e por cobertura falsa: metade
//! das entradas — as do import — não passa por `Todo::try_new`).
//!
//! Medido com o `ratatui-core` 0.1.2 (versão presa no `Cargo.lock`): a defesa é
//! **uma só** — o `Buffer::set_stringn` filtra os caracteres de controlo
//! (`buffer.rs:351`) antes de a célula existir. O `Cell::set_symbol` **não**
//! valida nada (só guarda o símbolo), e o `debug_assert!` do `cell_width.rs`
//! («control character passed to cell_width without filtering») só dispara em
//! `debug` e na medição da largura, não na escrita do símbolo. O alarme deste
//! ficheiro é o que sobrevive se o filtro do `set_stringn` desaparecer.

use std::fs;
use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;

use todo_ratatui::core::{Store, Todo, import_from_path};
use todo_ratatui::tui::App;
use todo_ratatui::tui::ui::ui;

/// A sequência do achado (OSC 52: copia texto para a área de transferência de
/// quem lê o terminal).
const OSC: &str = "\x1b]52;c;SGVsbG8=\x07";

/// A mesma sequência escrita para dentro de um ficheiro JSON: um carácter de
/// controlo cru dentro de uma string não é JSON válido, logo num ficheiro vai
/// sempre na forma escapada.
const OSC_JSON: &str = r"\u001b]52;c;SGVsbG8=\u0007";

/// O que resta da sequência depois de o ratatui filtrar os caracteres de
/// controlo — o que o utilizador vê.
const VISIVEL: &str = "]52;c;SGVsbG8=";

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "todo-ratatui-injecao-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("criar directório de teste");
    dir
}

fn loja(tag: &str) -> Store {
    let path = dir(tag).join("db.json");
    Store::open(&path).expect("abrir a base de dados do teste")
}

/// Todas as células do `Buffer` que têm um carácter de controlo, com a
/// posição: as posições são o que torna a falha diagnosticável.
fn celulas_com_controlo(buffer: &Buffer) -> Vec<(u16, u16, String)> {
    let mut encontrados = Vec::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let Some(celula) = buffer.cell((x, y)) else {
                continue;
            };
            if celula.symbol().chars().any(char::is_control) {
                encontrados.push((x, y, celula.symbol().to_owned()));
            }
        }
    }
    encontrados
}

/// O ecrã desenhado, como `Buffer` (80x24, o tamanho dos goldens).
fn desenha(app: &App) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal de teste");
    terminal
        .draw(|frame| ui(frame, app))
        .expect("desenhar o ecrã");
    terminal.backend().buffer().clone()
}

/// O ecrã inteiro como texto, para provar que o valor lá chegou (sem os
/// controlos): um teste que não vê conteúdo nenhum passaria por vazio.
fn ecra(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).map_or(" ", Cell::symbol))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// O detector não é um teste vazio: posto à frente de uma célula com um
/// carácter de controlo, encontra-a. É a prova que o ADR pede (uma mutação
/// deliberada tem de fazer o teste falhar), feita de forma permanente em vez de
/// num rascunho local.
#[test]
fn o_detector_encontra_um_caractere_de_controlo() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
    buffer
        .cell_mut((1, 0))
        .expect("célula dentro do ecrã")
        .set_symbol(OSC);

    assert_eq!(
        celulas_com_controlo(&buffer),
        vec![(1, 0, OSC.to_owned())],
        "o detector deixou passar uma célula com controlos"
    );
}

/// A via da UI: `Todo::try_new` aceita o ESC e o BEL no título (só faz `trim`)
/// e a descrição leva-os com uma mudança de linha pelo meio.
#[test]
fn o_titulo_com_escapes_vindo_da_ui_nao_poe_controlos_no_ecra() {
    let mut store = loja("ui");
    let titulo = format!("comprar café {OSC}");
    let mut todo = Todo::try_new(&titulo).expect("título válido");
    todo.title = titulo;
    todo.description = format!("linha 1\n{OSC}\nlinha 3");
    store.insert(todo).expect("inserir a tarefa");

    let app = App::new(store);
    let buffer = desenha(&app);
    let texto = ecra(&buffer);

    assert!(
        texto.contains(VISIVEL),
        "o título não chegou ao ecrã — o teste estaria a passar por vazio: {texto}"
    );
    assert_eq!(
        celulas_com_controlo(&buffer),
        Vec::new(),
        "o ecrã tem células com caracteres de controlo"
    );
}

/// A via do import: o `Todo` vem do serde (`import_export.rs`, o caminho que o
/// ADR nomeia) e **não** passa por `Todo::try_new`, logo é a prova de que a
/// defesa não depende da entrada usada.
#[test]
fn o_titulo_com_escapes_vindo_do_import_tambem_nao_poe_controlos_no_ecra() {
    let dir = dir("import");
    let origem = dir.join("entrada.json");
    fs::write(
        &origem,
        format!(
            r#"[
                {{
                    "id": "importada-1",
                    "title": "tarefa importada {OSC_JSON}",
                    "description": "linha 1\nlinha 2 {OSC_JSON}",
                    "done": false,
                    "priority": "low",
                    "created_at": "2026-09-18T10:00:00+01:00"
                }}
            ]"#
        ),
    )
    .expect("escrever o ficheiro de import");

    let mut store = loja("import-db");
    let relatorio = import_from_path(&mut store, &origem).expect("importar");
    assert_eq!(relatorio.inseridos, 1, "a tarefa entrou pelo import");
    assert!(
        store.todos()[0].title.contains('\x1b'),
        "o título guardado tem mesmo o escape — o import não valida nada"
    );

    let app = App::new(store);
    let buffer = desenha(&app);
    let texto = ecra(&buffer);

    assert!(
        texto.contains(VISIVEL),
        "o título importado não chegou ao ecrã: {texto}"
    );
    assert_eq!(
        celulas_com_controlo(&buffer),
        Vec::new(),
        "o ecrã tem células com caracteres de controlo"
    );
}
