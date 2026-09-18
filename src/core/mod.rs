//! Camada de domínio e persistência.
//!
//! Invariante da camada: nada aqui pode depender de `ratatui`/`crossterm`.
//! Se precisares de terminal para testar algo deste módulo, a fronteira está
//! mal colocada.

pub mod model;
pub mod store;

pub use model::{Priority, Todo, TodoError, TodoId};
pub use store::{Db, ENV_DB, SCHEMA_VERSION, Store, StoreError, Trashed, resolve_path};
