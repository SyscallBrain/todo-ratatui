//! Camada de terminal: estado, teclas e render.
//!
//! Esta camada pode (e deve) depender de [`crate::core`]; o contrário não é
//! permitido.
//!
//! Estado actual: o estado da aplicação e as teclas já são reais — [`app::App`]
//! guarda a seleção, o modo, a mensagem, a ordem, o filtro e a busca, e
//! [`event::map_key`] traduz a tabela de teclas do @designer. O loop, o
//! `init`/`restore` e o caminho da base de dados também já são reais; o desenho
//! é um marcador que o T6 substitui (o layout depende do @designer) e é o T7 que
//! liga o loop ao [`app::App`].

pub mod app;
pub mod event;

pub use app::{App, Status};
pub use event::{Action, InputMode, map_key};

use std::io;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};

use crate::core::Store;

/// Arranca o terminal, corre o loop e restaura o terminal.
///
/// `ratatui::try_init()` instala o panic hook que restaura o terminal — não
/// escrevemos o hook à mão (hooks próprios, a existirem, vão **antes** do
/// `init`). Usamos `try_init` e não `init` para que a falta de TTY (por
/// exemplo, arrancar o binário numa pipe) saia como erro com mensagem, em vez
/// de um panic com stack trace.
pub fn run(store: Store) -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, &store);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, store: &Store) -> io::Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, store))?;
        if let Event::Key(key) = crossterm::event::read()?
            && handle_key(key)
        {
            return Ok(());
        }
    }
}

/// Devolve `true` quando é para sair.
fn handle_key(key: KeyEvent) -> bool {
    if key.kind != KeyEventKind::Press {
        return false;
    }
    matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
}

fn draw(frame: &mut Frame, store: &Store) {
    let todo_count = store.todos().len();
    let lines = vec![
        Line::from("todo-ratatui — esqueleto (T1)"),
        Line::from(""),
        Line::from(format!("base de dados: {}", store.path().display())),
        Line::from(format!("tarefas: {todo_count}")),
        Line::from(""),
        Line::from("q / Esc — sair"),
    ];
    let block = Block::bordered().title(" todo-ratatui ");
    frame.render_widget(Paragraph::new(lines).block(block), frame.area());
}
