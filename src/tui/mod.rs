//! Camada de terminal: estado, teclas e render.
//!
//! Esta camada pode (e deve) depender de [`crate::core`]; o contrário não é
//! permitido.
//!
//! O [`run`] é o único sítio que liga as três peças: o terminal (aqui), o
//! estado e as teclas ([`app::App`] + [`event::map_key`]) e o desenho
//! ([`ui::ui`]). O `App` é **possuído pelo loop** — a camada de cima
//! (`main.rs`) entrega um [`Store`] aberto e não volta a tocar nos dados.

pub mod app;
pub mod event;
pub mod ui;

pub use app::{App, Status};
pub use event::{Action, InputMode, map_key};
pub use ui::ui;

use std::io;

use crossterm::event::Event;

use crate::core::Store;

/// Arranca o terminal, corre o loop e restaura o terminal.
///
/// `ratatui::try_init()` instala o panic hook que restaura o terminal — não
/// escrevemos o hook à mão (hooks próprios, a existirem, vão **antes** do
/// `init`). Usamos `try_init` e não `init` para que a falta de TTY (por
/// exemplo, arrancar o binário numa pipe) saia como erro com mensagem, em vez
/// de um panic com stack trace.
///
/// Recebe o [`Store`] já aberto: falhar a **abrir** recusa o arranque em
/// `main.rs` (adenda 2 do ADR) e nunca chega aqui com uma lista vazia em
/// memória a caminho de sobrescrever o ficheiro.
pub fn run(store: Store) -> io::Result<()> {
    // A falha de arranque é distinguida da falha a meio do loop: quem lê a
    // mensagem (uma `pipe`, um cron, um terminal sem TTY) precisa de saber que
    // o problema é não haver terminal interactivo — e não que a TUI tenha
    // rebentado a meio. O `try_init` instala o panic hook, liga o raw mode e só
    // depois entra no ecrã alternativo: aqui falhou no raw mode, logo não há
    // nada para restaurar.
    let mut terminal = ratatui::try_init().map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("não há terminal interactivo para a TUI: {err}"),
        )
    })?;
    let result = event_loop(&mut terminal, store);
    ratatui::restore();
    result
}

/// O loop de eventos: desenhar → ler uma tecla → aplicar → sair quando o `App`
/// o pedir.
///
/// Só as teclas mexem em nada: um `Resize`/`FocusGained` cai no `if let` sem
/// corpo e o ciclo volta a desenhar (é o que faz o ecrã acompanhar um
/// redimensionamento do terminal).
///
/// **Não há gravação aqui.** O [`App::handle`] já grava depois de cada
/// mutação — um `dirty`/`save` neste loop seria um segundo caminho de
/// gravação para o mesmo dado.
fn event_loop(terminal: &mut ratatui::DefaultTerminal, store: Store) -> io::Result<()> {
    let mut app = App::new(store);
    loop {
        terminal.draw(|frame| ui::ui(frame, &app))?;

        if let Event::Key(key) = crossterm::event::read()? {
            // `Release`/`Repeat` não são filtrados aqui: `map_key` já os
            // traduz em `Action::Ignore`.
            app.on_key(key);
        }

        if app.should_quit() {
            return Ok(());
        }
    }
}
