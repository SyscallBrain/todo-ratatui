//! Camada de domínio e persistência.
//!
//! Invariante da camada: nada aqui pode depender de `ratatui`/`crossterm`.
//! Se precisares de terminal para testar algo deste módulo, a fronteira está
//! mal colocada.

pub mod import_export;
pub mod model;
pub mod ops;
pub mod query;
pub mod store;

pub use import_export::{
    CSV_COLUMNS, ExportError, ImportError, ImportReport, export_csv_to_path, import_from_path,
};
pub use model::{Priority, Todo, TodoError, TodoId};
pub use ops::{ClearOutcome, OpsError, TRASH_LIMIT};
pub use query::{Counts, Filter, SortKey};
pub use store::{Db, ENV_DB, SCHEMA_VERSION, Store, StoreError, Trashed, resolve_path};
