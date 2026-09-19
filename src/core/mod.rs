//! Camada de domínio e persistência.
//!
//! Invariante da camada: nada aqui pode depender de `ratatui`/`crossterm`.
//! Se precisares de terminal para testar algo deste módulo, a fronteira está
//! mal colocada.

pub mod atomic;
pub mod config;
pub mod import_export;
pub mod model;
pub mod ops;
pub mod query;
pub mod store;

pub use atomic::write_atomic;
pub use import_export::{
    CSV_COLUMNS, ExportError, ImportError, ImportReport, export_csv_to_path, import_from_path,
};
pub use model::{Category, CategoryId, Priority, Todo, TodoError, TodoId};
pub use ops::{ClearOutcome, OpsError, TRASH_LIMIT};
pub use query::{CategoryFilter, Counts, Filter, SortKey};
pub use store::{
    Db, ENV_DB, SCHEMA_VERSION, Store, StoreError, Trashed, backup_path_for, resolve_path,
};

// `config` fica fora do `pub use`: o seu `resolve_path` tem o mesmo nome do do
// `store` (um resolve os **dados**, o outro a **preferência**), e reexportar os
// dois tornaria `core::resolve_path` ambíguo. `Config`, `ConfigError` e as
// constantes não têm esse problema.
pub use config::{CONFIG_FILE_NAME, Config, ConfigError, ENV_CONFIG};
