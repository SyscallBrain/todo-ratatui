//! Import e export: CSV e JSON (envelope novo ou array legado do `rtodo`).
//!
//! Três requisitos fechados na revisão pré-T0, todos com teste próprio:
//!
//! 1. **Import sem panic**, com erro que diz ficheiro e linha. O `csv::Error`
//!    não transporta o caminho, por isso o erro é composto aqui.
//! 2. **CSV sem cabeçalho é erro explícito.** O cabeçalho é comparado com o
//!    esperado *antes* de deserializar: sem esta comparação a primeira linha de
//!    dados seria engolida como cabeçalho e a primeira tarefa desaparecia em
//!    silêncio (era o que o `rtodo` antigo fazia, além do panic em linha curta).
//! 3. **Dedupe por `id` com contagem**, em vez do `extend` cego que duplicava
//!    tudo a cada importação repetida.
//!
//! Uma decisão de coerência: um valor inválido num campo (prioridade
//! desconhecida, data que não se parseia) é **erro com linha**, não conversão
//! silenciosa. A excepção é o legado sem data legível, que conta em
//! [`ImportReport::sem_data`] porque aí a falta é do ficheiro antigo.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate};
use serde::Deserialize;

use super::model::{Priority, Todo, TodoId};
use super::ops::trim_trash;
use super::store::{Db, SCHEMA_VERSION, Store, StoreError, Trashed};

/// Colunas do CSV, **na ordem dos campos de [`Todo`]** (a ordem é irrelevante
/// à leitura, que compara conjuntos, mas mantém o ficheiro legível).
pub const CSV_COLUMNS: [&str; 8] = [
    "id",
    "title",
    "description",
    "done",
    "priority",
    "created_at",
    "completed_at",
    "due_at",
];

/// Erros ao exportar.
#[derive(Debug)]
pub enum ExportError {
    Io { path: PathBuf, source: io::Error },
    Csv { path: PathBuf, source: csv::Error },
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "falha de I/O em «{}»: {source}", path.display())
            }
            Self::Csv { path, source } => {
                write!(f, "erro a escrever CSV em «{}»: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Csv { source, .. } => Some(source),
        }
    }
}

/// Erros ao importar. Todos identificam o ficheiro; os de dados identificam
/// também a linha.
#[derive(Debug)]
pub enum ImportError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    /// Erro de CSV já com o caminho composto por nós (o `csv::Error` não o traz).
    Csv {
        path: PathBuf,
        linha: Option<u64>,
        registo: Option<u64>,
        source: csv::Error,
    },
    /// Cabeçalho ausente ou diferente do esperado.
    Header {
        path: PathBuf,
        encontrado: Vec<String>,
    },
    /// Campo inválido numa linha concreta.
    Row {
        path: PathBuf,
        linha: u64,
        campo: &'static str,
        detalhe: String,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// Objeto JSON sem o campo `schema`.
    NotEnveloped {
        path: PathBuf,
    },
    /// Envelope com uma versão de schema que não sabemos ler.
    Schema {
        path: PathBuf,
        encontrado: u32,
        esperado: u32,
    },
    Persist(StoreError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "falha de I/O em «{}»: {source}", path.display())
            }
            Self::Csv {
                path,
                linha,
                registo,
                source,
            } => {
                write!(f, "«{}»", path.display())?;
                match (linha, registo) {
                    (Some(linha), Some(registo)) => {
                        write!(f, ", linha {linha} (registo {registo})")?
                    }
                    (Some(linha), None) => write!(f, ", linha {linha}")?,
                    _ => write!(f, " (posição desconhecida)")?,
                }
                write!(f, ": {source}")
            }
            Self::Header { path, encontrado } => write!(
                f,
                "«{}» não tem o cabeçalho esperado ({}). Encontrei: {}. \
                 Um CSV sem cabeçalho é recusado de propósito: a primeira linha de \
                 dados seria consumida como cabeçalho e a primeira tarefa perdia-se \
                 em silêncio",
                path.display(),
                CSV_COLUMNS.join(", "),
                if encontrado.is_empty() {
                    "nenhuma coluna".to_owned()
                } else {
                    encontrado.join(", ")
                }
            ),
            Self::Row {
                path,
                linha,
                campo,
                detalhe,
            } => write!(
                f,
                "«{}», linha {linha}: campo `{campo}` inválido ({detalhe})",
                path.display()
            ),
            Self::Json { path, source } => {
                write!(f, "JSON inválido em «{}»: {source}", path.display())
            }
            Self::NotEnveloped { path } => write!(
                f,
                "«{}» é um objeto JSON sem o campo `schema`: não é um envelope nem \
                 o array legado do rtodo",
                path.display()
            ),
            Self::Schema {
                path,
                encontrado,
                esperado,
            } => write!(
                f,
                "«{}» tem schema {encontrado}, e este binário só sabe ler {esperado}",
                path.display()
            ),
            Self::Persist(err) => write!(f, "não consegui gravar depois do import: {err}"),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Csv { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::Persist(err) => Some(err),
            _ => None,
        }
    }
}

impl From<StoreError> for ImportError {
    fn from(err: StoreError) -> Self {
        Self::Persist(err)
    }
}

/// O que uma importação fez. É mostrado na barra de estado — o import nunca
/// altera nada em silêncio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Registos lidos do ficheiro.
    pub lidos: usize,
    /// Tarefas novas inseridas.
    pub inseridos: usize,
    /// Registos ignorados por o `id` já existir (na base de dados ou no
    /// próprio ficheiro).
    pub duplicados: usize,
    /// Entradas de lixo importadas.
    pub lixo: usize,
    /// Tarefas do legado que vieram sem data legível (`created_at` = agora).
    pub sem_data: usize,
}

impl ImportReport {
    #[must_use]
    pub fn resumo(&self) -> String {
        let mut partes = vec![format!(
            "{} lidos, {} inseridos, {} duplicados ignorados",
            self.lidos, self.inseridos, self.duplicados
        )];
        if self.lixo > 0 {
            partes.push(format!("{} no lixo", self.lixo));
        }
        if self.sem_data > 0 {
            partes.push(format!("{} sem data legível", self.sem_data));
        }
        partes.join(", ")
    }

    /// Nada foi alterado (tudo duplicado).
    #[must_use]
    pub fn sem_alteracoes(&self) -> bool {
        self.inseridos == 0 && self.lixo == 0
    }
}

// ---------------------------------------------------------------- export

/// Exporta as tarefas para CSV, criando o ficheiro de forma atómica
/// (`tmp` + `rename`).
pub fn export_csv_to_path(todos: &[Todo], path: &Path) -> Result<usize, ExportError> {
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(|source| ExportError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    let tmp = path.with_file_name(format!(
        "{}.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "export.csv".to_owned())
    ));

    let escrito = {
        let ficheiro = fs::File::create(&tmp).map_err(|source| ExportError::Io {
            path: tmp.clone(),
            source,
        })?;
        write_csv(io::BufWriter::new(ficheiro), todos).map_err(|source| ExportError::Csv {
            path: tmp.clone(),
            source,
        })?
    };

    fs::rename(&tmp, path).map_err(|source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(escrito)
}

fn write_csv<W: io::Write>(writer: W, todos: &[Todo]) -> Result<usize, csv::Error> {
    // `has_headers(false)`: o cabeçalho é escrito à mão a partir de
    // `CSV_COLUMNS`, que fica a ser a única fonte de verdade do formato.
    let mut writer = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(writer);
    writer.write_record(CSV_COLUMNS)?;
    for todo in todos {
        writer.write_record(row(todo))?;
    }
    writer.flush()?;
    Ok(todos.len())
}

fn row(todo: &Todo) -> [String; 8] {
    [
        todo.id.to_string(),
        todo.title.clone(),
        todo.description.clone(),
        todo.done.to_string(),
        todo.priority.as_str().to_owned(),
        todo.created_at.to_rfc3339(),
        todo.completed_at
            .map(|d| d.to_rfc3339())
            .unwrap_or_default(),
        todo.due_at
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default(),
    ]
}

// ---------------------------------------------------------------- import

/// Importa pela extensão: `.csv` → CSV, tudo o resto → JSON.
pub fn import_from_path(store: &mut Store, path: &Path) -> Result<ImportReport, ImportError> {
    let csv = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"));
    if csv {
        import_csv_from_path(store, path)
    } else {
        import_json_from_path(store, path)
    }
}

/// Importa um CSV com cabeçalho.
pub fn import_csv_from_path(store: &mut Store, path: &Path) -> Result<ImportReport, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .from_path(path)
        .map_err(|source| ImportError::Csv {
            path: path.to_path_buf(),
            linha: None,
            registo: None,
            source,
        })?;

    // Requisito 2: comparar o cabeçalho ANTES de deserializar.
    let cabecalho = reader.headers().map_err(|source| ImportError::Csv {
        path: path.to_path_buf(),
        linha: None,
        registo: None,
        source,
    })?;
    let encontrado: Vec<String> = cabecalho.iter().map(str::to_owned).collect();
    if !cabecalho_esperado(&encontrado) {
        return Err(ImportError::Header {
            path: path.to_path_buf(),
            encontrado,
        });
    }

    let mut candidatos = Vec::new();
    let mut sem_data = 0;
    for (indice, resultado) in reader.deserialize::<CsvRow>().enumerate() {
        // Linha 1 = cabeçalho, logo o registo `n` está na linha `n + 2`.
        let linha_fallback = indice as u64 + 2;
        let registo = resultado.map_err(|source| {
            let posicao = source.position();
            ImportError::Csv {
                path: path.to_path_buf(),
                linha: Some(posicao.map_or(linha_fallback, |p| p.line())),
                registo: posicao.map(|p| p.record()),
                source,
            }
        })?;
        let (todo, sem_data_desta) = registo.into_todo(path, linha_fallback)?;
        if sem_data_desta {
            sem_data += 1;
        }
        candidatos.push(todo);
    }

    let lidos = candidatos.len();
    merge(store, candidatos, Vec::new(), lidos, sem_data)
}

/// Importa JSON: o envelope novo (`{schema, todos, trash}`) ou o array legado
/// do `rtodo` (`[{title, description, done, time, date}]`).
pub fn import_json_from_path(store: &mut Store, path: &Path) -> Result<ImportReport, ImportError> {
    let bytes = fs::read(path).map_err(|source| ImportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|source| ImportError::Json {
            path: path.to_path_buf(),
            source,
        })?;

    match value {
        serde_json::Value::Array(registos) => {
            let mut candidatos = Vec::with_capacity(registos.len());
            let mut sem_data = 0;
            for (indice, registo) in registos.into_iter().enumerate() {
                let linha = indice as u64 + 1;
                let legado: LegacyTodo =
                    serde_json::from_value(registo).map_err(|source| ImportError::Row {
                        path: path.to_path_buf(),
                        linha,
                        campo: "registo legado",
                        detalhe: source.to_string(),
                    })?;
                let (todo, sem_data_deste) = legado.into_todo(path, linha)?;
                if sem_data_deste {
                    sem_data += 1;
                }
                candidatos.push(todo);
            }
            let lidos = candidatos.len();
            merge(store, candidatos, Vec::new(), lidos, sem_data)
        }
        serde_json::Value::Object(ref mapa) if mapa.contains_key("schema") => {
            let db: Db = serde_json::from_value(value).map_err(|source| ImportError::Json {
                path: path.to_path_buf(),
                source,
            })?;
            if db.schema != SCHEMA_VERSION {
                return Err(ImportError::Schema {
                    path: path.to_path_buf(),
                    encontrado: db.schema,
                    esperado: SCHEMA_VERSION,
                });
            }
            let lidos = db.todos.len() + db.trash.len();
            merge(store, db.todos, db.trash, lidos, 0)
        }
        _ => Err(ImportError::NotEnveloped {
            path: path.to_path_buf(),
        }),
    }
}

fn cabecalho_esperado(encontrado: &[String]) -> bool {
    if encontrado.len() != CSV_COLUMNS.len() {
        return false;
    }
    let mut esperado: Vec<&str> = CSV_COLUMNS.to_vec();
    esperado.sort_unstable();
    let mut visto: Vec<&str> = encontrado.iter().map(String::as_str).collect();
    visto.sort_unstable();
    esperado == visto
}

/// Insere os candidatos, ignorando os `id` que já existem (ordem preservada).
fn merge(
    store: &mut Store,
    candidatos: Vec<Todo>,
    lixo: Vec<Trashed>,
    lidos: usize,
    sem_data: usize,
) -> Result<ImportReport, ImportError> {
    let mut ids: HashSet<TodoId> = store.todos().iter().map(|todo| todo.id.clone()).collect();
    let mut ids_lixo: HashSet<TodoId> = store
        .db()
        .trash
        .iter()
        .map(|entrada| entrada.todo.id.clone())
        .collect();

    let mut relatorio = ImportReport {
        lidos,
        sem_data,
        ..ImportReport::default()
    };

    let mut novos = Vec::new();
    for todo in candidatos {
        // `insert` devolve `false` tanto para um id já na base de dados como
        // para um id repetido dentro do próprio ficheiro.
        if !ids.insert(todo.id.clone()) {
            relatorio.duplicados += 1;
            continue;
        }
        novos.push(todo);
    }

    let mut lixo_novo = Vec::new();
    for entrada in lixo {
        if !ids_lixo.insert(entrada.todo.id.clone()) {
            relatorio.duplicados += 1;
            continue;
        }
        lixo_novo.push(entrada);
    }

    relatorio.inseridos = novos.len();
    relatorio.lixo = lixo_novo.len();

    if novos.is_empty() && lixo_novo.is_empty() {
        return Ok(relatorio); // nada mudou: não se toca no ficheiro
    }

    let mut db = store.db().clone();
    db.todos.extend(novos);
    db.trash.extend(lixo_novo);
    let antes = db.trash.len();
    trim_trash(&mut db); // o import também respeita o limite do lixo
    relatorio.lixo -= antes - db.trash.len();

    store.save_db(db).map_err(ImportError::Persist)?;
    Ok(relatorio)
}

// ---------------------------------------------------------------- linha CSV

#[derive(Debug, Deserialize)]
struct CsvRow {
    id: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    priority: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    completed_at: String,
    #[serde(default)]
    due_at: String,
}

impl CsvRow {
    /// Devolve a tarefa e se veio **sem data** (`created_at` vazio).
    fn into_todo(self, path: &Path, linha: u64) -> Result<(Todo, bool), ImportError> {
        let erro = |campo: &'static str, detalhe: String| ImportError::Row {
            path: path.to_path_buf(),
            linha,
            campo,
            detalhe,
        };

        let id = self.id.trim();
        if id.is_empty() {
            return Err(erro("id", "vazio".to_owned()));
        }
        let title = self.title.trim();
        if title.is_empty() {
            return Err(erro("title", "vazio".to_owned()));
        }
        let priority = if self.priority.trim().is_empty() {
            Priority::Medium
        } else {
            Priority::from_legacy(&self.priority).ok_or_else(|| {
                erro(
                    "priority",
                    format!("valor desconhecido: «{}»", self.priority),
                )
            })?
        };

        let (created_at, sem_data) = match self.created_at.trim() {
            "" => (Local::now(), true),
            texto => (
                parse_instante(texto)
                    .ok_or_else(|| erro("created_at", format!("data ilegível: «{texto}»")))?,
                false,
            ),
        };
        let completed_at = match self.completed_at.trim() {
            "" => None,
            texto => Some(
                parse_instante(texto)
                    .ok_or_else(|| erro("completed_at", format!("data ilegível: «{texto}»")))?,
            ),
        };
        let due_at = match self.due_at.trim() {
            "" => None,
            texto => Some(
                NaiveDate::parse_from_str(texto, "%Y-%m-%d")
                    .map_err(|err| erro("due_at", format!("data ilegível: «{texto}» ({err})")))?,
            ),
        };

        Ok((
            Todo {
                id: TodoId::from(id.to_owned()),
                title: title.to_owned(),
                description: self.description,
                done: self.done,
                priority,
                created_at,
                completed_at,
                due_at,
            },
            sem_data,
        ))
    }
}

/// Aceita RFC3339 (o que exportamos) e também uma data simples `YYYY-MM-DD`
/// (o que o `rtodo` antigo gravava), interpretada à meia-noite local.
fn parse_instante(texto: &str) -> Option<DateTime<Local>> {
    let texto = texto.trim();
    if let Ok(instante) = DateTime::parse_from_rfc3339(texto) {
        return Some(instante.with_timezone(&Local));
    }
    let data = NaiveDate::parse_from_str(texto, "%Y-%m-%d").ok()?;
    data.and_hms_opt(0, 0, 0)
        .and_then(|meia_noite| meia_noite.and_local_timezone(Local).earliest())
}

// ---------------------------------------------------------------- legado

/// Registo do `rtodo` antigo: `time` era a prioridade e `date` a data, como
/// texto, e não havia `id` nenhum.
#[derive(Debug, Deserialize)]
struct LegacyTodo {
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    date: String,
}

impl LegacyTodo {
    fn into_todo(self, path: &Path, linha: u64) -> Result<(Todo, bool), ImportError> {
        let erro = |campo: &'static str, detalhe: String| ImportError::Row {
            path: path.to_path_buf(),
            linha,
            campo,
            detalhe,
        };

        let title = self.title.trim();
        if title.is_empty() {
            return Err(erro("title", "vazio".to_owned()));
        }
        let priority = match self.time.as_deref().map(str::trim) {
            None | Some("") => Priority::Medium,
            Some(texto) => Priority::from_legacy(texto)
                .ok_or_else(|| erro("time", format!("prioridade desconhecida: «{texto}»")))?,
        };
        let (created_at, sem_data) = match self.date.trim() {
            "" => (Local::now(), true),
            texto => (
                parse_instante(texto)
                    .ok_or_else(|| erro("date", format!("data ilegível: «{texto}»")))?,
                false,
            ),
        };

        // Sem `id` no legado: gera-se um. O `completed_at` fica a `None` mesmo
        // quando `done` é `true`, porque a data real é desconhecida e inventá-la
        // seria pior do que a ausência.
        Ok((
            Todo {
                id: TodoId::new(),
                title: title.to_owned(),
                description: self.description,
                done: self.done,
                priority,
                created_at,
                completed_at: None,
                due_at: None,
            },
            sem_data,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Store;
    use crate::core::ops::TRASH_LIMIT;
    use std::str::FromStr;
    use uuid::Uuid;

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "todo-ratatui-io-{tag}-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("criar diretório");
        dir
    }

    fn nova_loja(tag: &str) -> (PathBuf, Store) {
        let dir = dir(tag);
        let path = dir.join("db.json");
        let store = Store::open(&path).expect("abrir store");
        (path, store)
    }

    fn escrever(dir: &Path, nome: &str, conteudo: &str) -> PathBuf {
        let path = dir.join(nome);
        fs::write(&path, conteudo).expect("escrever ficheiro de teste");
        path
    }

    #[test]
    fn export_escreve_o_cabecalho_esperado() {
        let (_, mut store) = nova_loja("cabecalho");
        store.add("uma").unwrap();
        let dir = dir("cabecalho-destino");
        let path = dir.join("saida.csv");

        assert_eq!(export_csv_to_path(store.todos(), &path).unwrap(), 1);

        let conteudo = fs::read_to_string(&path).unwrap();
        let mut linhas = conteudo.lines();
        assert_eq!(linhas.next().unwrap(), CSV_COLUMNS.join(","));
        assert!(linhas.next().unwrap().contains("uma"));
        assert!(
            !dir.join("saida.csv.tmp").exists(),
            "não ficaram temporários"
        );
    }

    #[test]
    fn round_trip_csv_com_campos_dificeis() {
        let (_, mut store) = nova_loja("round-trip");
        let id = store.add("título com vírgula, e acentos à solta").unwrap();
        store
            .edit_description(&id, "descrição com \"aspas\"\ne nova linha; café ☕")
            .unwrap();
        store.set_priority(&id, Priority::High).unwrap();
        store.toggle_done(&id).unwrap();
        store
            .insert(
                Todo::try_new("com data limite")
                    .unwrap()
                    .with_due_at(NaiveDate::from_ymd_opt(2026, 9, 30)),
            )
            .unwrap();
        let original = store.find(&id).unwrap().clone();

        let dir = dir("round-trip-destino");
        let path = dir.join("backup.csv");
        export_csv_to_path(store.todos(), &path).unwrap();

        let (_, mut destino) = nova_loja("round-trip-destino-db");
        let relatorio = import_csv_from_path(&mut destino, &path).unwrap();
        assert_eq!(relatorio.lidos, 2);
        assert_eq!(relatorio.inseridos, 2);
        assert_eq!(relatorio.duplicados, 0);
        assert_eq!(relatorio.sem_data, 0);

        let recarregado = destino.find(&id).expect("a tarefa voltou com o mesmo id");
        assert_eq!(recarregado, &original, "round-trip campo a campo");
        assert!(recarregado.is_done() && recarregado.completed_at.is_some());
        assert_eq!(
            destino
                .todos()
                .iter()
                .find(|t| t.title == "com data limite")
                .unwrap()
                .due_at,
            NaiveDate::from_ymd_opt(2026, 9, 30)
        );
    }

    #[test]
    fn csv_sem_cabecalho_da_erro_e_nao_engole_a_primeira_tarefa() {
        let dir = dir("sem-cabecalho");
        let path = escrever(
            &dir,
            "sem-cabecalho.csv",
            "01HXYZ,tarefa perdida,desc,false,high,2026-09-18T10:00:00+01:00,,\n",
        );

        let (_, mut store) = nova_loja("sem-cabecalho-db");
        let erro = import_csv_from_path(&mut store, &path).expect_err("tinha de falhar");

        assert!(
            matches!(erro, ImportError::Header { .. }),
            "erro foi {erro:?}"
        );
        let mensagem = erro.to_string();
        assert!(
            mensagem.contains(path.to_str().unwrap()),
            "sem caminho na mensagem: {mensagem}"
        );
        assert!(
            mensagem.contains("01HXYZ"),
            "queria o cabeçalho encontrado na mensagem: {mensagem}"
        );
        assert!(mensagem.contains("primeira tarefa"), "mensagem: {mensagem}");
        assert_eq!(store.todos().len(), 0, "não se importou nada");
    }

    #[test]
    fn csv_com_linha_curta_da_erro_com_ficheiro_e_linha_sem_panic() {
        let dir = dir("linha-curta");
        let path = escrever(
            &dir,
            "curto.csv",
            "id,title,description,done,priority,created_at,completed_at,due_at\n\
             a1,boa,desc,false,high,2026-09-18T10:00:00+01:00,,\n\
             a2,curta\n",
        );

        let (_, mut store) = nova_loja("linha-curta-db");
        let erro = import_csv_from_path(&mut store, &path).expect_err("tinha de falhar");

        match &erro {
            ImportError::Csv {
                linha,
                path: ficheiro_do_erro,
                ..
            } => {
                assert_eq!(ficheiro_do_erro, &path, "o erro traz o ficheiro certo");
                assert_eq!(*linha, Some(3), "a linha do registo curto é a 3");
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
        // O requisito é exactamente este: ficheiro + linha na mensagem, sem panic.
        let mensagem = erro.to_string();
        assert!(mensagem.contains("curto.csv"), "mensagem: {mensagem}");
        assert!(mensagem.contains("linha 3"), "mensagem: {mensagem}");
        assert_eq!(store.todos().len(), 0, "nada foi importado a meio");
    }

    #[test]
    fn import_duas_vezes_nao_duplica() {
        let (_, mut origem) = nova_loja("dedupe-origem");
        origem.add("primeira").unwrap();
        origem.add("segunda").unwrap();
        let dir = dir("dedupe");
        let path = dir.join("dados.csv");
        export_csv_to_path(origem.todos(), &path).unwrap();

        let (_, mut destino) = nova_loja("dedupe-destino");
        let primeiro = import_csv_from_path(&mut destino, &path).unwrap();
        assert_eq!(primeiro.inseridos, 2);
        assert_eq!(primeiro.duplicados, 0);

        let segundo = import_csv_from_path(&mut destino, &path).unwrap();
        assert_eq!(segundo.lidos, 2);
        assert_eq!(segundo.inseridos, 0);
        assert_eq!(segundo.duplicados, 2, "a contagem diz o que foi ignorado");
        assert!(segundo.sem_alteracoes());
        assert_eq!(destino.todos().len(), 2, "sem extend cego");
    }

    #[test]
    fn import_ignora_repetidos_dentro_do_proprio_ficheiro() {
        let dir = dir("repetidos");
        let path = escrever(
            &dir,
            "repetidos.csv",
            "id,title,description,done,priority,created_at,completed_at,due_at\n\
             mesmo,primeira,,false,medium,2026-09-18T10:00:00+01:00,,\n\
             mesmo,repetida,,false,medium,2026-09-18T10:00:00+01:00,,\n",
        );
        let (_, mut store) = nova_loja("repetidos-db");
        let relatorio = import_csv_from_path(&mut store, &path).unwrap();
        assert_eq!(relatorio.inseridos, 1);
        assert_eq!(relatorio.duplicados, 1);
        assert_eq!(store.todos().len(), 1);
        assert_eq!(store.todos()[0].title, "primeira");
    }

    #[test]
    fn prioridade_invalida_da_erro_com_linha_e_caminho() {
        let dir = dir("prioridade-má");
        let path = escrever(
            &dir,
            "má.csv",
            "id,title,description,done,priority,created_at,completed_at,due_at\n\
             p1,tarefa,,false,urgente,2026-09-18T10:00:00+01:00,,\n",
        );
        let (_, mut store) = nova_loja("prioridade-má-db");
        let erro = import_csv_from_path(&mut store, &path).expect_err("tinha de falhar");
        match &erro {
            ImportError::Row { linha, campo, .. } => {
                assert_eq!(*linha, 2);
                assert_eq!(*campo, "priority");
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
        assert!(erro.to_string().contains("urgente"));
    }

    #[test]
    fn import_json_legado_do_rtodo() {
        let dir = dir("legado");
        let path = escrever(
            &dir,
            "db-legado.json",
            r#"[
                {"title":"antiga","description":"do rtodo","done":false,"time":"High","date":"2023-01-05"},
                {"title":"sem data","description":"","done":true,"time":null,"date":""}
            ]"#,
        );

        let (_, mut store) = nova_loja("legado-db");
        let relatorio = import_json_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.lidos, 2);
        assert_eq!(relatorio.inseridos, 2);
        assert_eq!(relatorio.duplicados, 0);
        assert_eq!(relatorio.sem_data, 1, "uma veio sem data legível");

        let antiga = store.todos().iter().find(|t| t.title == "antiga").unwrap();
        assert_eq!(antiga.priority, Priority::High);
        assert_eq!(antiga.description, "do rtodo");
        assert_eq!(
            antiga.created_at.date_naive(),
            NaiveDate::from_ymd_opt(2023, 1, 5).unwrap()
        );
        assert!(!antiga.is_done() && antiga.completed_at.is_none());

        let sem_data = store
            .todos()
            .iter()
            .find(|t| t.title == "sem data")
            .unwrap();
        assert!(sem_data.is_done());
        assert_eq!(sem_data.priority, Priority::Medium);
        assert!(sem_data.completed_at.is_none(), "não se inventa a data");
        assert!(!antiga.id.as_str().is_empty(), "o legado ganha id novo");
    }

    #[test]
    fn import_json_envelope_funde_por_id() {
        let (_, mut origem) = nova_loja("envelope-origem");
        let ja_existe = origem.add("já cá está").unwrap();
        origem.add("também vai").unwrap();
        let dir = dir("envelope");
        let path = dir.join("db.json");
        origem.save().unwrap();
        fs::copy(origem.path(), &path).unwrap();

        let (_, mut destino) = nova_loja("envelope-destino");
        destino.add("local").unwrap();
        destino
            .insert(origem.find(&ja_existe).unwrap().clone())
            .unwrap();

        let relatorio = import_json_from_path(&mut destino, &path).unwrap();
        assert_eq!(relatorio.lidos, 2);
        assert_eq!(relatorio.inseridos, 1, "só a que faltava");
        assert_eq!(relatorio.duplicados, 1);
        assert_eq!(destino.todos().len(), 3);
    }

    #[test]
    fn import_json_sem_envelope_da_erro() {
        let dir = dir("sem-envelope");
        let path = escrever(&dir, "solto.json", r#"{"todos":[]}"#);
        let (_, mut store) = nova_loja("sem-envelope-db");
        let erro = import_json_from_path(&mut store, &path).expect_err("tinha de falhar");
        assert!(matches!(erro, ImportError::NotEnveloped { .. }));
    }

    #[test]
    fn import_json_schema_desconhecido_da_erro() {
        let dir = dir("schema");
        let path = escrever(&dir, "futuro.json", r#"{"schema":99,"todos":[]}"#);
        let (_, mut store) = nova_loja("schema-db");
        let erro = import_json_from_path(&mut store, &path).expect_err("tinha de falhar");
        match &erro {
            ImportError::Schema {
                encontrado,
                esperado,
                ..
            } => {
                assert_eq!(*encontrado, 99);
                assert_eq!(*esperado, SCHEMA_VERSION);
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
    }

    #[test]
    fn import_respeita_o_limite_do_lixo() {
        let dir = dir("lixo-limite");
        let path = dir.join("db.json");
        let mut db = Db::empty();
        for i in 0..TRASH_LIMIT + 20 {
            db.trash.push(Trashed {
                todo: Todo::try_new(format!("lixo {i}")).unwrap(),
                index: i,
                deleted_at: Local::now(),
                batch: 1,
            });
        }
        fs::write(&path, serde_json::to_vec(&db).unwrap()).unwrap();

        let (_, mut store) = nova_loja("lixo-limite-db");
        let relatorio = import_json_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.lidos, TRASH_LIMIT + 20);
        assert_eq!(relatorio.lixo, TRASH_LIMIT, "o lixo não passa do limite");
        assert_eq!(store.db().trash.len(), TRASH_LIMIT);
        assert_eq!(store.db().trash_dropped, 20, "e a contagem é gravada");
    }

    #[test]
    fn import_despacha_pela_extensao() {
        let dir = dir("despacho");
        let (_, mut origem) = nova_loja("despacho-origem");
        origem.add("csv").unwrap();
        let csv = dir.join("dados.CSV");
        export_csv_to_path(origem.todos(), &csv).unwrap();

        let (_, mut destino) = nova_loja("despacho-db");
        assert_eq!(import_from_path(&mut destino, &csv).unwrap().inseridos, 1);

        let json = dir.join("legado.json");
        fs::write(
            &json,
            r#"[{"title":"json","description":"","done":false,"time":"Low","date":"2024-02-02"}]"#,
        )
        .unwrap();
        assert_eq!(import_from_path(&mut destino, &json).unwrap().inseridos, 1);
        assert_eq!(destino.todos().len(), 2);
    }

    #[test]
    fn relatorio_do_import_e_legivel() {
        let relatorio = ImportReport {
            lidos: 10,
            inseridos: 7,
            duplicados: 3,
            lixo: 1,
            sem_data: 2,
        };
        assert_eq!(
            relatorio.resumo(),
            "10 lidos, 7 inseridos, 3 duplicados ignorados, 1 no lixo, 2 sem data legível"
        );
        assert!(!relatorio.sem_alteracoes());
        assert!(ImportReport::default().sem_alteracoes());
    }

    #[test]
    fn id_no_csv_e_estavel_entre_exportacoes() {
        let (_, mut store) = nova_loja("id-estavel");
        let id = store.add("estável").unwrap();
        let dir = dir("id-estavel-destino");
        let path = dir.join("a.csv");
        export_csv_to_path(store.todos(), &path).unwrap();

        let (_, mut destino) = nova_loja("id-estavel-db");
        import_csv_from_path(&mut destino, &path).unwrap();
        let importado = &destino.todos()[0];
        assert_eq!(importado.id, id);
        assert_eq!(TodoId::from_str(id.as_str()).unwrap(), importado.id);
        // E o lixo continua vazio depois de um import sem lixo.
        assert!(destino.trash_entries().is_empty());
    }
}
