//! Persistência: envelope JSON, escrita atómica e rotação do `.bak`.
//!
//! Formato em disco (`schema: 3`):
//!
//! ```json
//! { "schema": 3, "todos": [...], "trash": [...], "categories": [...] }
//! ```
//!
//! O envelope é deliberado: o `rtodo` antigo gravava um `Vec<Todo>` cru, o que
//! torna impossível distinguir «ficheiro vazio» de «ficheiro de outro formato»
//! e deixa o formato sem versão. O `Db` **rejeita** explicitamente as duas
//! formas antigas ([`StoreError::LegacyShape`], [`StoreError::NotEnveloped`])
//! em vez de as aceitar em silêncio — a conversão é trabalho do import (T4).
//!
//! Gerações: o `schema` 2 (o lixo) e o 3 (as categorias, [`SCHEMAS_ACEITES`]);
//! a gravação emite sempre [`SCHEMA_VERSION`]. Um ficheiro `2` lê-se tal e qual
//! — os campos novos nascem vazios pelos `#[serde(default)]` — e é reescrito
//! como `3` na gravação seguinte. Um ficheiro **mais novo** é recusado: um
//! binário que ignora campos que não conhece apagá-los-ia na gravação seguinte,
//! e recusar e dizê-lo é a única versão desta decisão em que nada se perde em
//! silêncio (ADR §Decisão 3).
//!
//! O `Db` também **tolera a mais**: uma chave que não conheça é ignorada na
//! leitura e desaparece na gravação seguinte. É o que acontece ao
//! `dangling_recovered` — um contador de v1.2.0 que nunca chegou a ser
//! publicado e que saiu do `schema: 3` (ADR Adenda 1): nenhum ficheiro no
//! caminho de dados do utilizador o tem, mas um que o tenha abre sem erro.

use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use super::atomic;
use super::model::{Category, CategoryId, Todo};

/// Versão do formato em disco. O lixo (`trash`) entrou no schema 2; as
/// categorias no schema 3.
pub const SCHEMA_VERSION: u32 = 3;

/// Gerações de ficheiro que a leitura aceita: a actual e a anterior.
///
/// A `2` é a da v1.1.0: não tem `categories` nem `category_id`, e os
/// `#[serde(default)]` fazem-nos nascer vazios. Um `1` ou `0` nunca existiu
/// (o formato do `rtodo` antigo não tem envelope nenhum) e é recusado como
/// qualquer desconhecido.
const SCHEMAS_ACEITES: [u32; 2] = [SCHEMA_VERSION - 1, SCHEMA_VERSION];

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
    /// Categorias conhecidas, por ordem de inserção (é a ordem que a caixa
    /// mostra). O `#[serde(default)]` é o que faz o `schema: 2` — que não tem
    /// esta chave — ler-se como vazio (ADR §Decisão 3).
    #[serde(default)]
    pub categories: Vec<Category>,
}

impl Db {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            todos: Vec::new(),
            trash: Vec::new(),
            trash_dropped: 0,
            categories: Vec::new(),
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
    /// Quantas referências pendentes **esta leitura** corrigiu (ADR Adenda 1).
    ///
    /// É contagem de leitura, não dado: não vai para o ficheiro e não se
    /// acumula entre sessões. Vive aqui para quem abriu a base poder dizer o
    /// que a leitura normalizou sem voltar a percorrer as tarefas — a `Db`
    /// entregue não guarda rasto de quantas referências corrigiu.
    referencias_recuperadas: u64,
}

impl Store {
    /// Abre o ficheiro. Ficheiro inexistente => base de dados vazia (e o
    /// diretório-pai é criado só na primeira escrita).
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path = path.into();
        let (db, referencias_recuperadas) = if path.exists() {
            Self::load(&path)?
        } else {
            (Db::empty(), 0)
        };
        Ok(Self {
            path,
            db,
            referencias_recuperadas,
        })
    }

    fn load(path: &Path) -> Result<(Db, u64), StoreError> {
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

        let mut db: Db = match value {
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

        if !SCHEMAS_ACEITES.contains(&db.schema) {
            return Err(StoreError::UnsupportedSchema {
                path: path.to_path_buf(),
                found: db.schema,
                expected: SCHEMA_VERSION,
            });
        }

        // Normalizar é o último passo antes de entregar a `Db`: quem a recebe
        // nunca vê uma referência que não resolve, nem um schema antigo. O
        // ficheiro fica como está.
        //
        // A `Db` em memória é sempre a geração actual: um ficheiro `2` é
        // migrado em memória (nada é escrito) e a gravação seguinte emite `3`
        // sem ter de saber de onde veio — é isso que faz o `save` emitir sempre
        // [`SCHEMA_VERSION`] sem um caso especial lá dentro.
        db.schema = SCHEMA_VERSION;
        let referencias_recuperadas = Self::normalizar_referencias(&mut db);
        Ok((db, referencias_recuperadas))
    }

    /// Põe a `None` qualquer `category_id` que não resolva numa categoria
    /// existente — na lista **e** no lixo — e devolve quantos corrigiu.
    ///
    /// Um ficheiro editado à mão (ou fundido com outro) pode trazer um `uuid`
    /// que não existe. Recusar arrancar por causa disso puniria o dono dos
    /// dados por um erro de edição num campo que é metadado, quando a tarefa e
    /// tudo o que ela tem estão intactos: a tarefa fica **visível** e sem
    /// categoria (ADR §Decisão 4). O lixo é normalizado também — é o que o
    /// `delete_category` faz (T2), e sem isso o `u` repunha uma tarefa a
    /// apontar para uma categoria que não existe.
    ///
    /// A contagem é **desta leitura** (ADR Adenda 1): quantas referências
    /// o ficheiro que acabou de ser lido trazia por resolver. Não se acumula
    /// nem vai para o disco — enquanto a referência pendente lá estiver, cada
    /// abertura volta a contá-la e a avisar; a primeira gravação que a cure
    /// cala o aviso.
    ///
    /// Esta leitura **não escreve nada**: o valor corrigido só vai para o disco
    /// na próxima gravação, como qualquer alteração.
    fn normalizar_referencias(db: &mut Db) -> u64 {
        let conhecidas: HashSet<CategoryId> = db.categories.iter().map(|c| c.id.clone()).collect();
        let mut corrigidas = 0u64;
        for todo in db
            .todos
            .iter_mut()
            .chain(db.trash.iter_mut().map(|entrada| &mut entrada.todo))
        {
            // Sem `category_id`, ou com um que resolve: nada a fazer. A
            // referência pendente é o único caso que muda a `Db`.
            let Some(id) = todo.category_id.as_ref() else {
                continue;
            };
            if conhecidas.contains(id) {
                continue;
            }
            todo.category_id = None;
            corrigidas += 1;
        }
        corrigidas
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

    /// Quantas referências de categoria **a leitura que abriu esta base**
    /// normalizou para «sem categoria» (ADR Adenda 1).
    ///
    /// Conta a lista **e** o lixo — é o que a leitura percorre — e é zero no
    /// caso normal. Quem avisa é quem abre: o [`crate::tui::app::App`] nasce
    /// com o aviso quando este número não é zero.
    #[must_use]
    pub const fn referencias_recuperadas(&self) -> u64 {
        self.referencias_recuperadas
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
            // `0700` quando é o programa a criar o directório dos dados; um que
            // já exista fica como está (ADR §Decisão 4).
            atomic::criar_directorio_privado(dir).map_err(|source| StoreError::Io {
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
    ///
    /// É uma leitura como a da abertura: a contagem de referências recuperadas
    /// passa a ser a desta leitura (ADR Adenda 1).
    pub fn reload(&mut self) -> Result<(), StoreError> {
        let (db, referencias_recuperadas) = if self.path.exists() {
            Self::load(&self.path)?
        } else {
            (Db::empty(), 0)
        };
        self.db = db;
        self.referencias_recuperadas = referencias_recuperadas;
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
            // Pelo helper de `atomic`, e não por `fs::copy`: o `copy` é o
            // `O_CREAT|O_TRUNC` de sempre — segue um symlink plantado num
            // caminho que aqui é **fixo e previsível**, e deixa o modo ao
            // `umask` (os achados SA-02/SA-03). O conteúdo passa a ir pelo
            // descritor que o `create_new` abriu, com `sync_all` antes de o
            // `.bak` ocupar o lugar do `db.json`.
            let mut origem = File::open(&self.path).map_err(|source| StoreError::Io {
                path: self.path.clone(),
                source,
            })?;
            let mut destino =
                atomic::criar_privado(&pre_restore).map_err(|source| StoreError::Io {
                    path: pre_restore.clone(),
                    source,
                })?;
            io::copy(&mut origem, &mut destino).map_err(|source| StoreError::Io {
                path: pre_restore.clone(),
                source,
            })?;
            destino.sync_all().map_err(|source| StoreError::Io {
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
    use std::os::unix::fs::PermissionsExt;

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

    // ------------------------------------------------------------- SA-02/03

    /// Os bits de permissão do caminho, sem o tipo (`0o600`, `0o700`, …).
    fn modo(path: &Path) -> u32 {
        fs::metadata(path).expect("metadados").permissions().mode() & 0o777
    }

    /// A gravação é o sítio onde o programa cria o directório dos dados, o
    /// `db.json`, o `.bak` e o pré-restore: todos nascem privados. O `.bak` é o
    /// caso que o `mode(0o600)` do temporário **não** fechava, porque o
    /// `rename` lhe dá o modo do ficheiro rodado — o `db.json` é posto a `664`
    /// antes da segunda gravação (a instalação da v1.0.1 com `umask 0002`) para
    /// que seja mesmo o `set_permissions` do `.bak` a fazer o trabalho.
    #[test]
    fn gravacao_cria_o_directorio_e_os_ficheiros_privados() {
        let base = temp_dir("privado");
        // O directório da aplicação ainda não existe: quem o cria é a primeira
        // gravação.
        let dir = base.join("todo-ratatui");
        let path = dir.join(DB_FILE_NAME);
        let mut store = Store::open(&path).expect("abrir");

        let mut db = Db::empty();
        db.todos
            .push(crate::core::model::Todo::try_new("privada").unwrap());
        store.save_db(db).expect("primeira gravação");
        assert_eq!(modo(&dir), 0o700, "o directório criado pelo programa");
        assert_eq!(modo(&path), 0o600, "o db.json");

        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).expect("modo antigo");
        let mut db = store.db().clone();
        db.todos
            .push(crate::core::model::Todo::try_new("segunda").unwrap());
        store.save_db(db).expect("segunda gravação");
        assert_eq!(
            modo(&store.backup_path()),
            0o600,
            "o .bak rotado herdou os 664 do db.json que rodou"
        );
        assert_eq!(modo(&path), 0o600, "o db.json volta a nascer privado");

        let pre_restore = store.restore_backup().expect("restaurar");
        assert_eq!(modo(&pre_restore), 0o600, "o estado anterior ao restore");
        assert_eq!(modo(&path), 0o600, "o destino depois do restore");
    }

    // ------------------------------------------------ categorias (schema 3)

    /// O `schema` que está escrito no ficheiro — o que uma **leitura** não pode
    /// mudar.
    fn ficheiro_schema(path: &Path) -> u64 {
        let valor: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("ler o ficheiro")).expect("json");
        valor["schema"].as_u64().expect("schema numérico")
    }

    /// O envelope da v1.1.0 (`schema: 2`) **com** os campos novos lá dentro é o
    /// caso mais exigente da tolerância: um ficheiro já migrado, aberto e
    /// gravado pelo binário antigo volta a ter `2` com `categories` a mais. A
    /// leitura aceita-o tal e qual, e é a gravação seguinte que o passa a `3`.
    #[test]
    fn schema_2_com_campos_novos_le_e_migra_na_gravacao_seguinte() {
        let dir = temp_dir("schema-2-tolerante");
        let path = dir.join(DB_FILE_NAME);
        let categoria = CategoryId::new();
        fs::write(
            &path,
            format!(
                r#"{{
  "schema": 2,
  "todos": [
    {{
      "id": "t1",
      "title": "da v1.1.0",
      "created_at": "2026-09-19T09:00:00+01:00"
    }},
    {{
      "id": "t2",
      "title": "já com categoria",
      "created_at": "2026-09-19T09:00:00+01:00",
      "category_id": "{categoria}"
    }}
  ],
  "trash": [],
  "categories": [{{"id": "{categoria}", "name": "Trabalho"}}]
}}"#
            ),
        )
        .expect("escrever o ficheiro da v1.1.0");

        let store = Store::open(&path).expect("um ficheiro 2 lê-se");
        assert_eq!(
            store.db().schema,
            SCHEMA_VERSION,
            "em memória a `Db` é já a geração actual"
        );
        assert_eq!(
            ficheiro_schema(&path),
            2,
            "…mas o ficheiro em disco fica como estava: ler não grava"
        );
        assert_eq!(store.todos().len(), 2, "as duas tarefas continuam lá");
        assert_eq!(
            store.todos()[0].category_id,
            None,
            "sem o campo => sem categoria"
        );
        assert_eq!(store.todos()[1].category_id, Some(categoria.clone()));
        assert_eq!(store.db().categories.len(), 1);
        assert_eq!(store.db().categories[0].name, "Trabalho");
        assert_eq!(
            store.referencias_recuperadas(),
            0,
            "não havia nada pendente"
        );

        store.save().expect("gravar");
        let valor: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("reler")).expect("json");
        assert_eq!(valor["schema"], serde_json::json!(SCHEMA_VERSION));
        assert_eq!(valor["todos"].as_array().map(Vec::len), Some(2));
        assert_eq!(valor["categories"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            valor["todos"][1]["category_id"],
            serde_json::json!(categoria.as_str())
        );
    }

    /// A gravação escreve o formato novo **com a chave explícita**: um
    /// `categories: []` está no ficheiro (não ausente), para a leitura seguinte
    /// não ter de adivinhar o que é uma base vazia e o que é um campo esquecido.
    #[test]
    fn gravacao_emite_schema_3_com_categories_explicito() {
        let dir = temp_dir("grava-schema-3");
        let path = dir.join(DB_FILE_NAME);
        let mut store = Store::open(&path).expect("abrir");
        store.save_db(Db::empty()).expect("gravar");

        let valor: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("ler")).expect("json");
        assert_eq!(valor["schema"], serde_json::json!(3));
        assert_eq!(valor["categories"], serde_json::json!([]));
        assert!(
            valor.get("dangling_recovered").is_none(),
            "a chave saiu do `schema: 3` (ADR Adenda 1): a contagem é da leitura"
        );
    }

    /// Um ficheiro da v1.2.0 não publicada, que ainda tenha o
    /// `dangling_recovered`, **abre sem erro**: o `Db` não recusa chaves a mais
    /// (e a chave desaparece na gravação seguinte). É o que faz a saída do
    /// campo ser uma não-migração.
    #[test]
    fn chave_a_mais_no_ficheiro_le_se_sem_erro_e_sai_na_gravacao() {
        let dir = temp_dir("chave-a-mais");
        let path = dir.join(DB_FILE_NAME);
        fs::write(
            &path,
            r#"{
  "schema": 3,
  "todos": [],
  "trash": [],
  "categories": [],
  "dangling_recovered": 7
}"#,
        )
        .expect("escrever");

        let store = Store::open(&path).expect("um ficheiro com a chave a mais lê-se");
        assert_eq!(
            store.referencias_recuperadas(),
            0,
            "a chave é ignorada: não é dela que sai a contagem"
        );
        store.save().expect("gravar");
        let valor: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("reler")).expect("json");
        assert!(
            valor.get("dangling_recovered").is_none(),
            "uma gravação desta versão não volta a emitir a chave: {valor}"
        );
    }

    /// Uma referência que não resolve não esconde nem perde a tarefa: fica
    /// **visível** e sem categoria, conta-se como referência recuperada **da
    /// leitura**, e o ficheiro em disco **não é tocado** — uma leitura nunca
    /// reescreve dados do utilizador (é o byte-a-byte que prova isto).
    #[test]
    fn referencia_orfa_e_normalizada_sem_tocar_no_ficheiro() {
        let dir = temp_dir("categoria-orfa");
        let path = dir.join(DB_FILE_NAME);
        fs::write(
            &path,
            r#"{
  "schema": 3,
  "todos": [
    {
      "id": "t1",
      "title": "sem categoria conhecida",
      "created_at": "2026-09-19T09:00:00+01:00",
      "category_id": "categoria-que-nao-existe"
    }
  ],
  "trash": [],
  "categories": []
}"#,
        )
        .expect("escrever a referência pendente");
        let antes = fs::read(&path).expect("ler bytes");

        let store = Store::open(&path).expect("abre sem recusar");

        assert_eq!(store.todos().len(), 1, "a tarefa continua visível");
        assert_eq!(store.todos()[0].title, "sem categoria conhecida");
        assert_eq!(
            store.todos()[0].category_id,
            None,
            "a referência pendente saiu"
        );
        assert_eq!(store.referencias_recuperadas(), 1);

        let depois = fs::read(&path).expect("reler bytes");
        assert_eq!(
            antes, depois,
            "uma leitura não reescreve o ficheiro do utilizador"
        );

        // Reler o mesmo ficheiro (ainda pendente) conta outra vez o mesmo: a
        // contagem é desta leitura, não um acumulado em disco (ADR Adenda 1).
        let outra = Store::open(&path).expect("reabrir");
        assert_eq!(outra.referencias_recuperadas(), 1);
    }

    /// A contagem é da **leitura**, e a leitura percorre a lista **e** o lixo:
    /// uma referência pendente em cada dá **2**. Depois de uma gravação que
    /// cure o ficheiro, a abertura seguinte já não conta nada — é o que faz o
    /// aviso nascer em cada abertura enquanto o ficheiro estiver por curar e
    /// calar-se depois.
    #[test]
    fn contagem_soma_lista_e_lixo_e_cala_se_depois_da_gravacao() {
        let dir = temp_dir("categoria-orfa-soma");
        let path = dir.join(DB_FILE_NAME);
        fs::write(
            &path,
            r#"{
  "schema": 3,
  "todos": [
    {
      "id": "t1",
      "title": "na lista",
      "created_at": "2026-09-19T09:00:00+01:00",
      "category_id": "sumiu-1"
    }
  ],
  "trash": [
    {
      "todo": {
        "id": "t2",
        "title": "no lixo",
        "created_at": "2026-09-19T09:00:00+01:00",
        "category_id": "sumiu-2"
      },
      "index": 1,
      "deleted_at": "2026-09-19T10:00:00+01:00",
      "batch": 1
    }
  ],
  "categories": []
}"#,
        )
        .expect("escrever");
        let antes = fs::read(&path).expect("ler bytes");

        let store = Store::open(&path).expect("abre");
        assert_eq!(
            store.referencias_recuperadas(),
            2,
            "uma na lista e uma no lixo: as duas que a leitura normalizou"
        );
        assert_eq!(store.todos().len(), 1, "a tarefa da lista continua visível");
        assert_eq!(store.todos()[0].category_id, None);
        assert_eq!(
            fs::read(&path).expect("reler bytes"),
            antes,
            "ler não grava"
        );

        store.save().expect("gravar a base curada");
        let depois = Store::open(&path).expect("reabrir");
        assert_eq!(
            depois.referencias_recuperadas(),
            0,
            "o ficheiro já está curado: a abertura seguinte não avisa"
        );
    }

    /// O lixo é normalizado também: sem isto, o `u` repunha uma tarefa a apontar
    /// para uma categoria que não existe — o mesmo invariante que a eliminação
    /// de categoria tem de manter na lista e no lixo (ADR §Decisão 4, T2).
    #[test]
    fn referencia_orfa_no_lixo_tambem_e_normalizada() {
        let dir = temp_dir("categoria-orfa-lixo");
        let path = dir.join(DB_FILE_NAME);
        fs::write(
            &path,
            r#"{
  "schema": 3,
  "todos": [],
  "trash": [
    {
      "todo": {
        "id": "t1",
        "title": "no lixo",
        "created_at": "2026-09-19T09:00:00+01:00",
        "category_id": "categoria-que-nao-existe"
      },
      "index": 0,
      "deleted_at": "2026-09-19T10:00:00+01:00",
      "batch": 1
    }
  ],
  "categories": []
}"#,
        )
        .expect("escrever");

        let store = Store::open(&path).expect("abre");
        assert_eq!(store.db().trash.len(), 1, "a entrada do lixo continua lá");
        assert_eq!(store.db().trash[0].todo.category_id, None);
        assert_eq!(store.referencias_recuperadas(), 1);
    }

    /// Um ficheiro de uma geração futura é recusado — e a recusa não lhe toca:
    /// lá dentro está informação que este binário não sabe preservar, e a
    /// gravação seguinte apagá-la-ia (ADR §Decisão 3).
    #[test]
    fn schema_4_recusa_e_nao_reescreve_o_ficheiro() {
        let dir = temp_dir("schema-4");
        let path = dir.join(DB_FILE_NAME);
        fs::write(&path, br#"{"schema": 4, "todos": [], "trash": []}"#).expect("escrever");
        let antes = fs::read(&path).expect("ler bytes");

        let err = Store::open(&path).expect_err("tem de recusar");
        match err {
            StoreError::UnsupportedSchema {
                found, expected, ..
            } => {
                assert_eq!(found, 4);
                assert_eq!(expected, SCHEMA_VERSION);
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
        assert_eq!(fs::read(&path).expect("reler bytes"), antes);
    }

    /// `0` e `1` nunca existiram (o `rtodo` antigo não tinha envelope) e são
    /// recusados como qualquer desconhecido: só o `2` é lido por tolerância.
    #[test]
    fn schemas_0_e_1_sao_recusados() {
        for schema in [0u32, 1] {
            let dir = temp_dir(&format!("schema-{schema}"));
            let path = dir.join(DB_FILE_NAME);
            fs::write(
                &path,
                format!(r#"{{"schema": {schema}, "todos": [], "trash": []}}"#),
            )
            .expect("escrever");

            let err = Store::open(&path).expect_err("tem de recusar");
            assert!(
                matches!(err, StoreError::UnsupportedSchema { found, .. } if found == schema),
                "schema {schema} tinha de ser recusado, veio {err:?}"
            );
        }
    }

    /// Gravar e reler devolve a mesma `Db` — categorias incluídas — e a
    /// contagem de referências recuperadas **não vai para o ficheiro**: uma
    /// releitura de uma base sem referências pendentes conta zero.
    #[test]
    fn round_trip_preserva_as_categorias_sem_persistir_contagem() {
        let dir = temp_dir("categorias-round-trip");
        let path = dir.join(DB_FILE_NAME);
        let trabalho = Category {
            id: CategoryId::new(),
            name: "Trabalho".to_owned(),
        };
        let casa = Category {
            id: CategoryId::new(),
            name: "Casa".to_owned(),
        };

        let esperada = Db {
            todos: vec![
                crate::core::model::Todo::try_new("com categoria")
                    .unwrap()
                    .with_category(Some(trabalho.id.clone())),
                crate::core::model::Todo::try_new("sem categoria").unwrap(),
            ],
            categories: vec![trabalho, casa],
            ..Db::empty()
        };

        let mut store = Store::open(&path).expect("abrir");
        store.save_db(esperada.clone()).expect("gravar");

        let recarregado = Store::open(&path).expect("reabrir");
        assert_eq!(recarregado.db(), &esperada);
        assert_eq!(
            recarregado.referencias_recuperadas(),
            0,
            "nada estava pendente — a contagem é da leitura (ADR Adenda 1)"
        );
    }
}
