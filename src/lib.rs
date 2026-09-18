//! `todo_ratatui` — TODO list em TUI.
//!
//! Fronteira de camadas, verificável por `grep`:
//!
//! * [`core`] — modelo, persistência e operações. **Não pode** mencionar
//!   `ratatui` nem `crossterm`: é testável sem TTY.
//! * [`tui`] — estado da aplicação, teclas e render. Depende de [`core`],
//!   nunca o contrário.

pub mod core;
pub mod tui;
