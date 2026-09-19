//! Persistência: envelope JSON, escrita atómica e rotação do `.bak`.
//!
//! Formato em disco (`schema: 2`):
//!
//! ```json
//! { "schema": 2, "todos": [...], "trash": [...] }
//! ```
//!
//! O envelope é deliberado: o `rtodo` antigo gravava um `Vec<Todo>` cru, o que
//! torna impossível distinguir «ficheiro vazio» de «ficheiro de outro formato»
//! e deixa o formato sem versão. O `Db` **rejeita** explicitamente as duas
//! formas antigas ([`StoreError::LegacyShape`], [`StoreError::NotEnveloped`])
//! em vez de as aceitar em silêncio — a conversão é trabalho do import (T4).

use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use super::atomic;
use super::model::Todo;

/// Versão do formato em disco. O lixo (`trash`) entrou no schema 2.
pub const SCHEMA_VERSION: u32 = 2;

/// Variável de ambiente que substitui o caminho da base de dados.
pub const ENV_DB: &str = "TODO_RATATUI_DB";

const DB_FILE_NAME: &str = "db.json";
const APP_DIR: &str = "todo-ratatui";

/// Tarefa removida, guardada em vez de apagada.
///
/// O `undo` é **dado, não RAM**: substitui o diálogo de confirmação, logo não
/// pode ser mais frágil do que o diálogo (um `Ctrl-C` depois de «limpar
/// concluídas» perderia tarefas de forma irrecuperável se vivesse em memória).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trashed {
    pub todo: Todo,
    /// Índice que a tarefa ocupava na lista quando foi removida, para o `undo`
    /// a devolver à posição original.
    pub index: usize,
    pub deleted_at: DateTime<Local>,
    /// Remoção a que pertence: um «limpar concluídas» cria um lote com várias
    /// entradas, e o `u` desfaz o lote inteiro de uma vez.
    pub batch: u64,
}

/// Conteúdo completo da base de dados.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Db {
    pub schema: u32,
    #[serde(default)]
    pub todos: Vec<Todo>,
    #[serde(default)]
    pub trash: Vec<Trashed>,
    /// Quantas entradas saíram do lixo por este ter atingido o limite.
    ///
    /// Persistido de propósito: o aviso de que nada sai em silêncio não pode
    /// depender de a aplicação ainda estar de pé quando transbordou.
    #[serde(default)]
    pub trash_dropped: u64,
}

impl Db {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            todos: Vec::new(),
            trash: Vec::new(),
            trash_dropped: 0,
        }
    }
}

impl Default for Db {
    fn default() -> Self {
        Self::empty()
    }
}

/// Erros de persistência. Cada variante identifica o ficheiro: um erro de
/// dados sem caminho é inútil para quem o lê.
#[derive(Debug)]
pub enum StoreError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// O ficheiro é um array JSON — a forma do `rtodo` antigo (`Vec<Todo>`).
    LegacyShape {
        path: PathBuf,
    },
    /// Objeto JSON sem o campo `schema` — não é um envelope nosso.
    NotEnveloped {
        path: PathBuf,
    },
    UnsupportedSchema {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    NoDataDir,
    NoBackup(PathBuf),
}

impl StoreError {
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Io { path, .. }
            | Self::Json { path, .. }
            | Self::LegacyShape { path }
            | Self::NotEnveloped { path }
            | Self::UnsupportedSchema { path, .. }
            | Self::NoBackup(path) => Some(path),
            Self::NoDataDir => None,
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "falha de I/O em «{}»: {source}", path.display())
            }
            Self::Json { path, source } => {
                write!(f, "JSON inválido em «{}»: {source}", path.display())
            }
            Self::LegacyShape { path } => write!(
                f,
                "«{}» é um array JSON sem envelope: parece o formato legado do rtodo; \
                 converte-o com o import em vez de o abrires como base de dados",
                path.display()
            ),
            Self::NotEnveloped { path } => write!(
                f,
                "«{}» é um objeto JSON sem o campo `schema`: não é uma base de dados \
                 do todo-ratatui",
                path.display()
            ),
            Self::UnsupportedSchema {
                path,
                found,
                expected,
            } => write!(
                f,
                "schema {found} em «{}» não é suportado (esperado {expected})",
                path.display()
            ),
            Self::NoDataDir => {
                f.write_str("não consigo determinar o diretório de dados: define XDG_DATA_HOME ou HOME, ou usa --db <caminho>")
            }
            Self::NoBackup(path) => write!(
                f,
                "não há backup para restaurar: «{}» não existe",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Resolve o caminho da base de dados, por ordem de precedência:
/// `--db <caminho>` → `TODO_RATATUI_DB` → `dirs::data_dir()/todo-ratatui/db.json`
/// (= `$XDG_DATA_HOME` → `$HOME/.local/share`).
///
/// `data_dir()` e não `config_dir()`: as tarefas são dados, não configuração
/// (ADR, ponto 4 e adenda 1). O `~/.config` costuma estar em dotfiles e fora
/// de muitos backups — um `db.json` lá dentro iria para o próximo commit.
pub fn resolve_path(explicit: Option<&Path>) -> Result<PathBuf, StoreError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(from_env) = env::var_os(ENV_DB).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(from_env));
    }
    let base = dirs::data_dir().ok_or(StoreError::NoDataDir)?;
    Ok(base.join(APP_DIR).join(DB_FILE_NAME))
}

/// Caminho do backup (`db.json.bak`) correspondente a um caminho de base de
/// dados.
///
/// Existe como função livre porque há quem precise de o nomear **sem** ter um
/// [`Store`] aberto: é o caso da mensagem de recusa de arranque em `main.rs`,
/// que corre precisamente quando a abertura falhou. A convenção do nome vive
/// em [`atomic::backup_path_for`], que é quem efectivamente roda o ficheiro —
/// [`Store::backup_path`] e esta função usam-na, para não haver duas regras.
#[must_use]
pub fn backup_path_for(path: &Path) -> PathBuf {
    atomic::backup_path_for(path)
}

/// Base de dados aberta em memória e ligada a um ficheiro.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
    db: Db,
}

impl Store {
    /// Abre o ficheiro. Ficheiro inexistente => base de dados vazia (e o
    /// diretório-pai é criado só na primeira escrita).
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path = path.into();
        let db = if path.exists() {
            Self::load(&path)?
        } else {
            Db::empty()
        };
        Ok(Self { path, db })
    }

    fn load(path: &Path) -> Result<Db, StoreError> {
        let bytes = fs::read(path).map_err(|source| StoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        // Ler como `Value` primeiro: é o que permite distinguir «array do
        // rtodo antigo» e «objeto sem envelope» de JSON simplesmente inválido,
        // e dar uma mensagem que diz o que fazer.
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|source| StoreError::Json {
                path: path.to_path_buf(),
                source,
            })?;

        let db: Db = match value {
            serde_json::Value::Array(_) => {
                return Err(StoreError::LegacyShape {
                    path: path.to_path_buf(),
                });
            }
            serde_json::Value::Object(ref map) if !map.contains_key("schema") => {
                return Err(StoreError::NotEnveloped {
                    path: path.to_path_buf(),
                });
            }
            other => serde_json::from_value(other).map_err(|source| StoreError::Json {
                path: path.to_path_buf(),
                source,
            })?,
        };

        if db.schema != SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchema {
                path: path.to_path_buf(),
                found: db.schema,
                expected: SCHEMA_VERSION,
            });
        }
        Ok(db)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn db(&self) -> &Db {
        &self.db
    }

    #[must_use]
    pub fn todos(&self) -> &[Todo] {
        &self.db.todos
    }

    pub fn db_mut(&mut self) -> &mut Db {
        &mut self.db
    }

    /// Caminho do ficheiro de backup (geração anterior).
    #[must_use]
    pub fn backup_path(&self) -> PathBuf {
        backup_path_for(&self.path)
    }

    /// Caminho do ficheiro onde fica o estado anterior a um `restore`.
    #[must_use]
    pub fn pre_restore_path(&self) -> PathBuf {
        self.path
            .with_file_name(format!("{DB_FILE_NAME}.pre-restore"))
    }

    /// Grava a base de dados em memória.
    ///
    /// A escrita é a partilhada com o `config` ([`atomic::write_atomic`]): o
    /// temporário no mesmo directório é escrito por inteiro e sincronizado, a
    /// geração anterior é rodada para o `.bak` e o temporário ocupa o lugar do
    /// destino — sempre por `rename`, logo ou o ficheiro fica inteiro e novo,
    /// ou fica o antigo. O `.bak` é a geração anterior, para corrupção/schema —
    /// não é mecanismo de undo (esse é o `trash`).
    pub fn save(&self) -> Result<(), StoreError> {
        let mut json = serde_json::to_vec_pretty(&self.db).map_err(|source| StoreError::Json {
            path: self.path.clone(),
            source,
        })?;
        json.push(b'\n');

        let parent = self.path.parent().filter(|p| !p.as_os_str().is_empty());
        if let Some(dir) = parent {
            fs::create_dir_all(dir).map_err(|source| StoreError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
        }

        atomic::write_atomic(&self.path, &json).map_err(|source| StoreError::Io {
            path: self.path.clone(),
            source,
        })?;

        if let Some(dir) = parent {
            Self::sync_dir(dir)?;
        }
        Ok(())
    }

    /// Substitui o conteúdo em memória e grava.
    pub fn save_db(&mut self, db: Db) -> Result<(), StoreError> {
        self.db = db;
        self.save()
    }

    /// Recarrega o ficheiro do disco.
    pub fn reload(&mut self) -> Result<(), StoreError> {
        self.db = if self.path.exists() {
            Self::load(&self.path)?
        } else {
            Db::empty()
        };
        Ok(())
    }

    /// Restaura a geração anterior (o `.bak`) e devolve o caminho onde ficou
    /// guardado o estado que existia antes do restore.
    ///
    /// A recuperação não pode ser ela própria uma perda: o `db.json` actual é
    /// copiado para `db.json.pre-restore` antes de o `.bak` ocupar o lugar.
    /// O `.bak` deixa de existir (passou a ser o `db.json`).
    pub fn restore_backup(&mut self) -> Result<PathBuf, StoreError> {
        let bak = self.backup_path();
        if !bak.is_file() {
            return Err(StoreError::NoBackup(bak));
        }
        let pre_restore = self.pre_restore_path();
        if self.path.is_file() {
            fs::copy(&self.path, &pre_restore).map_err(|source| StoreError::Io {
                path: pre_restore.clone(),
                source,
            })?;
            Self::sync_dir(pre_restore.parent().unwrap_or(Path::new(".")))?;
        }
        fs::rename(&bak, &self.path).map_err(|source| StoreError::Io {
            path: self.path.clone(),
            source,
        })?;
        self.reload()?;
        Ok(pre_restore)
    }

    fn sync_dir(dir: &Path) -> Result<(), StoreError> {
        let handle = File::open(dir).map_err(|source| StoreError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        handle.sync_all().map_err(|source| StoreError::Io {
            path: dir.to_path_buf(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Priority;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "todo-ratatui-test-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("criar diretório de teste");
        dir
    }

    fn store_with(tag: &str, titles: &[&str]) -> (PathBuf, Vec<crate::core::model::Todo>) {
        let dir = temp_dir(tag);
        let path = dir.join(DB_FILE_NAME);
        let store = Store::open(&path).expect("abrir store");
        let mut todos = Vec::new();
        for title in titles {
            let todo = crate::core::model::Todo::try_new(*title).expect("título válido");
            todos.push(todo);
        }
        let mut store = store;
        store
            .save_db(Db {
                todos: todos.clone(),
                ..Db::empty()
            })
            .expect("gravar");
        (path, todos)
    }

    #[test]
    fn ficheiro_inexistente_abre_vazio() {
        let dir = temp_dir("vazio");
        let store = Store::open(dir.join(DB_FILE_NAME)).expect("abrir");
        assert!(store.todos().is_empty());
        assert_eq!(store.db().schema, SCHEMA_VERSION);
        assert_eq!(store.db().trash, Vec::new());
    }

    #[test]
    fn save_round_trip_preserva_ids_e_datas() {
        let (path, todos) = store_with("round-trip", &["comprar café", "escrever testes"]);

        let store = Store::open(&path).expect("reabrir");
        assert_eq!(store.todos().len(), 2);
        let titles: Vec<&str> = store.todos().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["comprar café", "escrever testes"]);
        for (original, reloaded) in todos.iter().zip(store.todos()) {
            assert_eq!(original.id, reloaded.id);
            assert_eq!(original.created_at, reloaded.created_at);
        }

        let raw = fs::read_to_string(&path).expect("ler");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(value["schema"], serde_json::json!(SCHEMA_VERSION));
        assert!(value["todos"].is_array());
        assert!(value["trash"].is_array());
    }

    #[test]
    fn save_nao_deixa_ficheiros_temporarios() {
        let (path, _) = store_with("sem-tmp", &["a"]);
        let mut store = Store::open(&path).expect("reabrir");

        let mut db = store.db().clone();
        db.todos
            .push(crate::core::model::Todo::try_new("b").unwrap());
        store.save_db(db).expect("segunda gravação");

        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .expect("listar")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "ficaram temporários: {leftovers:?}");
    }

    #[test]
    fn save_rotaciona_bak_para_a_geracao_anterior() {
        let (path, _) = store_with("rotacao", &["primeira"]);
        assert!(!Store::open(&path).unwrap().backup_path().exists());

        let mut store = Store::open(&path).expect("reabrir");
        let mut db = store.db().clone();
        db.todos
            .push(crate::core::model::Todo::try_new("segunda").unwrap());
        store.save_db(db).expect("segunda gravação");

        let bak = store.backup_path();
        let back: Db = serde_json::from_slice(&fs::read(&bak).expect("ler bak")).expect("json bak");
        assert_eq!(back.todos.len(), 1);
        assert_eq!(back.todos[0].title, "primeira");
        assert_eq!(store.todos().len(), 2);
    }

    #[test]
    fn bak_do_primeiro_save_nao_existe() {
        let dir = temp_dir("sem-bak-inicial");
        let path = dir.join(DB_FILE_NAME);
        let mut store = Store::open(&path).expect("abrir");
        let mut db = Db::empty();
        db.todos
            .push(crate::core::model::Todo::try_new("única").unwrap());
        store.save_db(db).expect("primeira gravação");
        assert!(!store.backup_path().exists());
    }

    #[test]
    fn restore_backup_e_reversivel() {
        let (path, _) = store_with("restore", &["estado A"]);
        let mut store = Store::open(&path).expect("reabrir");
        let mut db = store.db().clone();
        db.todos
            .push(crate::core::model::Todo::try_new("estado B").unwrap());
        store.save_db(db).expect("gravação B");
        assert_eq!(store.todos().len(), 2);

        let pre_restore = store.restore_backup().expect("restaurar");
        assert_eq!(store.todos().len(), 1, "voltou ao estado A");
        assert_eq!(store.todos()[0].title, "estado A");
        assert_eq!(pre_restore, store.pre_restore_path());
        assert!(pre_restore.is_file(), "o estado anterior ficou guardado");

        let guardado: Db =
            serde_json::from_slice(&fs::read(&pre_restore).expect("ler")).expect("json");
        assert_eq!(guardado.todos.len(), 2, "estado B recuperável");
    }

    #[test]
    fn restore_sem_bak_da_erro_explicito() {
        let dir = temp_dir("restore-sem-bak");
        let mut store = Store::open(dir.join(DB_FILE_NAME)).expect("abrir");
        let err = store.restore_backup().expect_err("tinha de falhar");
        assert!(matches!(err, StoreError::NoBackup(_)));
        assert!(err.to_string().contains("db.json.bak"));
    }

    #[test]
    fn rejeita_array_legado_do_rtodo() {
        let dir = temp_dir("legado");
        let path = dir.join(DB_FILE_NAME);
        let legado = r#"[{"title":"antiga","description":"","done":false,"time":"High","date":"2023-01-05"}]"#;
        fs::write(&path, legado).expect("escrever legado");

        let err = Store::open(&path).expect_err("tinha de falhar");
        assert!(matches!(err, StoreError::LegacyShape { .. }));
        assert!(err.to_string().contains("formato legado"));
    }

    #[test]
    fn rejeita_objeto_sem_envelope() {
        let dir = temp_dir("sem-envelope");
        let path = dir.join(DB_FILE_NAME);
        fs::write(&path, r#"{"todos":[]}"#).expect("escrever");

        let err = Store::open(&path).expect_err("tinha de falhar");
        assert!(matches!(err, StoreError::NotEnveloped { .. }));
    }

    #[test]
    fn rejeita_schema_desconhecido() {
        let dir = temp_dir("schema");
        let path = dir.join(DB_FILE_NAME);
        fs::write(&path, r#"{"schema":99,"todos":[]}"#).expect("escrever");

        let err = Store::open(&path).expect_err("tinha de falhar");
        match err {
            StoreError::UnsupportedSchema {
                found, expected, ..
            } => {
                assert_eq!(found, 99);
                assert_eq!(expected, SCHEMA_VERSION);
            }
            other => panic!("erro inesperado: {other:?}"),
        }
    }

    #[test]
    fn json_invalido_da_erro_com_caminho() {
        let dir = temp_dir("json-mau");
        let path = dir.join(DB_FILE_NAME);
        fs::write(&path, "{ isto não é json").expect("escrever");

        let err = Store::open(&path).expect_err("tinha de falhar");
        assert_eq!(err.path(), Some(path.as_path()));
        assert!(err.to_string().contains("JSON inválido"));
    }

    #[test]
    fn trash_sobrevive_ao_round_trip() {
        let dir = temp_dir("trash");
        let path = dir.join(DB_FILE_NAME);
        let mut todo = crate::core::model::Todo::try_new("apagada").unwrap();
        todo.priority = Priority::High;
        let mut store = Store::open(&path).expect("abrir");
        store
            .save_db(Db {
                trash: vec![Trashed {
                    todo: todo.clone(),
                    index: 3,
                    deleted_at: Local::now(),
                    batch: 7,
                }],
                ..Db::empty()
            })
            .expect("gravar");

        let recarregado = Store::open(&path).expect("reabrir");
        assert_eq!(recarregado.db().trash.len(), 1);
        assert_eq!(recarregado.db().trash[0].index, 3);
        assert_eq!(recarregado.db().trash[0].todo.id, todo.id);
    }
}
