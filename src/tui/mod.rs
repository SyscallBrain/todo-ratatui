//! Camada de terminal: estado, teclas e render.
//!
//! Esta camada pode (e deve) depender de [`crate::core`]; o contrário não é
//! permitido.
//!
//! O [`run`] é o único sítio que liga as três peças: o terminal (aqui), o
//! estado e as teclas ([`app::App`] + [`event::map_key`]) e o desenho
//! ([`ui::ui`]). O `App` é **possuído pelo loop** — a camada de cima
//! (`main.rs`) entrega um [`Store`] aberto e não volta a tocar nos dados.
//!
//! [`theme`] é a quarta peça, e é só dados: os quatro temas com os seus papéis
//! de cor e o modo de cor do terminal. O desenho lê-os, o `core` nunca os vê.
//! [`arranque`] é a decisão que vem de fora: qual dos quatro temas, com que
//! modo de cor e com que ficheiro de preferências a sessão arranca.

pub mod app;
pub mod arranque;
pub mod event;
pub mod theme;
pub mod ui;

pub use app::{App, Status};
pub use arranque::{Ambiente, Opcoes, Resolucao};
pub use event::{Action, InputMode, map_key};
pub use theme::{ModoCor, Theme};
pub use ui::ui;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::Event;

use crate::core::Store;

/// De quanto em quanto tempo o loop acorda sem evento nenhum.
///
/// É a única excepção à regra «sem `tick`» (§6 do desenho): sem ela a mensagem
/// de acção da linha 22 ficava a tapar a descrição do selecionado até à tecla
/// seguinte. O `poll` **espera** pelo evento — não é um ciclo a girar, em
/// repouso não há CPU a queimar — e o `draw` continua no topo da iteração,
/// portanto a expiração aparece no ciclo seguinte sem sinalizador nenhum de
/// redesenho.
const ESPERA: Duration = Duration::from_millis(250);

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
///
/// Recebe a [`Resolucao`] do arranque (T4) e não três argumentos soltos: o
/// tema, o caminho das preferências e os avisos são decididos num sítio só
/// ([`arranque::resolver`]), e é essa decisão inteira que desce para aqui — o
/// `main.rs` já a imprimiu antes de o ecrã alternativo ligar
/// ([`Resolucao::avisar`]).
pub fn run(store: Store, sessao: &Resolucao) -> io::Result<()> {
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
    let result = event_loop(&mut terminal, store, sessao);
    ratatui::restore();
    result
}

/// O loop de eventos: desenhar → esperar uma tecla (ou 250 ms) → aplicar →
/// expirar a mensagem de acção → sair quando o `App` o pedir.
///
/// Só as teclas mexem em nada: um `Resize`/`FocusGained` cai no `if let` sem
/// corpo e o ciclo volta a desenhar (é o que faz o ecrã acompanhar um
/// redimensionamento do terminal).
///
/// **Não há gravação aqui.** O [`App::handle`] já grava depois de cada
/// mutação — um `dirty`/`save` neste loop seria um segundo caminho de
/// gravação para o mesmo dado — e o [`App::tick`] só expira a mensagem de
/// acção, sem tocar nos dados.
fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    store: Store,
    sessao: &Resolucao,
) -> io::Result<()> {
    let mut app = App::com_tema(store, sessao.tema, sessao.config_path.clone(), sessao.modo);
    loop {
        terminal.draw(|frame| ui::ui(frame, &app))?;

        // `poll` com prazo: com evento lê-se a tecla, sem evento o ciclo
        // segue para o `tick` — é ele que faz a mensagem de acção sair sozinha
        // aos 3 s. `Release`/`Repeat` não são filtrados aqui: `map_key` já os
        // traduz em `Action::Ignore`. (O caminho é qualificado porque
        // `crate::tui::event` sombreia `crossterm::event` neste módulo.)
        if crossterm::event::poll(ESPERA)?
            && let Event::Key(key) = crossterm::event::read()?
        {
            app.on_key(key);
        }

        // Corre nos dois caminhos (com e sem evento). Não grava nada: a
        // gravação continua reservada ao `App::handle`.
        app.tick(Instant::now());

        if app.should_quit() {
            return Ok(());
        }
    }
}
