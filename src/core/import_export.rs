//! Import e export: CSV e JSON (envelope novo, array de [`Todo`] ou array
//! legado do `rtodo`).
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

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate};
use serde::Deserialize;

use super::atomic;
use super::model::{Category, CategoryId, Priority, Todo, TodoId};
use super::ops::trim_trash;
use super::store::{Db, SCHEMA_VERSION, Store, StoreError, Trashed};

/// Colunas do CSV, **na ordem dos campos de [`Todo`]** (a ordem é irrelevante
/// à leitura, que compara conjuntos, mas mantém o ficheiro legível).
///
/// A última é a categoria, que o CSV identifica pelo **nome** — é o formato
/// que se lê e se escreve à mão, e um `uuid` não diz nada a quem abre a folha
/// de cálculo (ADR §Decisão 10).
pub const CSV_COLUMNS: [&str; 9] = [
    "id",
    "title",
    "description",
    "done",
    "priority",
    "created_at",
    "completed_at",
    "due_at",
    "category",
];

/// O cabeçalho da v1.1.0: o mesmo, sem a coluna `category`.
///
/// Continua a ser aceite à leitura — um ficheiro exportado pela v1.1.0 importa,
/// com tudo sem categoria. Recusá-lo partiria o único caminho de troca de dados
/// do programa sem ganho nenhum (ADR §Decisão 10); a mensagem de erro mostra os
/// **dois** cabeçalhos, porque é isso que o utilizador lê e tem de acertar.
pub const CSV_COLUMNS_V11: [&str; 8] = [
    "id",
    "title",
    "description",
    "done",
    "priority",
    "created_at",
    "completed_at",
    "due_at",
];

/// Gerações de envelope que o import aceita: a actual e a anterior.
///
/// A mesma regra do [`Store`] (lá `SCHEMAS_ACEITES`, privada) e pela mesma
/// razão: um `db.json` gravado pela v1.1.0 (`schema: 2`) **tem** de importar —
/// é o caminho de recuperação de um backup antigo (ADR §Decisão 3). As duas
/// constantes dizem o mesmo a partir do mesmo [`SCHEMA_VERSION`].
const SCHEMAS_ACEITES: [u32; 2] = [SCHEMA_VERSION - 1, SCHEMA_VERSION];

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
                "«{}» não tem o cabeçalho esperado. Esperava as 9 colunas ({}) ou as 8 \
                 da v1.1.0 ({}). Encontrei: {}. \
                 Um CSV sem cabeçalho é recusado de propósito: a primeira linha de \
                 dados seria consumida como cabeçalho e a primeira tarefa perdia-se \
                 em silêncio",
                path.display(),
                CSV_COLUMNS.join(", "),
                CSV_COLUMNS_V11.join(", "),
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
                "«{}» tem schema {encontrado}, e este binário só sabe ler {esperado} \
                 (ou a geração anterior, {})",
                path.display(),
                SCHEMAS_ACEITES[0]
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
    /// Registos lidos do ficheiro. Conta **só** tarefas e lixo: as categorias
    /// têm contadores próprios (e não são tarefas).
    pub lidos: usize,
    /// Tarefas novas inseridas.
    pub inseridos: usize,
    /// Registos ignorados por o `id` já existir (na base de dados ou no
    /// próprio ficheiro).
    pub duplicados: usize,
    /// Entradas de lixo importadas.
    pub lixo: usize,
    /// Tarefas do legado que vieram sem data legível (`created_at` = agora).
    /// No formato novo é sempre 0: o `created_at` é obrigatório lá.
    pub sem_data: usize,
    /// Categorias que o ficheiro trouxe e que não existiam: foram criadas.
    pub categorias_criadas: usize,
    /// Categorias que o ficheiro trouxe com um **nome que já existia**
    /// (*case-insensitive*) ou com um `id` que já existia: não entraram, e as
    /// tarefas passaram a apontar à que cá está. Conta **categorias**, não
    /// tarefas: a mesma categoria pedida por dez tarefas é uma só.
    pub categorias_reaproveitadas: usize,
    /// Tarefas que vinham com um `category_id` que não resolve — nem numa
    /// categoria do ficheiro, nem numa da base: entraram **sem categoria** e
    /// são contadas aqui (é o mesmo caso que o `store` conta em
    /// `dangling_recovered`). Um campo vazio ou ausente não conta: não houve
    /// referência nenhuma para falhar.
    pub sem_categoria: usize,
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
        // Como o `lixo` e o `sem_data`: só aparece o que aconteceu.
        if self.categorias_criadas > 0 {
            partes.push(format!("{} categorias criadas", self.categorias_criadas));
        }
        if self.categorias_reaproveitadas > 0 {
            partes.push(format!(
                "{} categorias reaproveitadas",
                self.categorias_reaproveitadas
            ));
        }
        if self.sem_categoria > 0 {
            partes.push(format!("{} sem categoria", self.sem_categoria));
        }
        partes.join(", ")
    }

    /// Nada foi alterado (tudo duplicado e nenhuma categoria nova).
    ///
    /// As categorias contam: um envelope que só trouxe categorias alterou a
    /// base, e dizer «sem alterações» seria falso.
    #[must_use]
    pub fn sem_alteracoes(&self) -> bool {
        self.inseridos == 0 && self.lixo == 0 && self.categorias_criadas == 0
    }
}

// ---------------------------------------------------------------- export

/// Exporta as tarefas **da lista** para CSV, criando o ficheiro de forma
/// atómica (`tmp` + `rename`).
///
/// O lixo não sai no CSV (continua a ser a lista o que se exporta), e a coluna
/// `category` leva o **nome** da categoria — uma tarefa sem categoria (ou com
/// uma referência que não resolve) sai com o campo vazio. É preciso a `Db`
/// inteira e não só as tarefas: o nome vive nas categorias.
pub fn export_csv_to_path(db: &Db, path: &Path) -> Result<usize, ExportError> {
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(|source| ExportError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    let tmp = atomic::tmp_path_for(path);

    let escrito = {
        // `criar_privado` e não `fs::File::create`: o nome do temporário é
        // derivado do destino (logo previsível) e o `create` seguia um symlink
        // ou um hard link lá plantado, além de deixar o modo ao `umask`
        // (SA-02/SA-03).
        let ficheiro = atomic::criar_privado(&tmp).map_err(|source| ExportError::Io {
            path: tmp.clone(),
            source,
        })?;
        write_csv(io::BufWriter::new(ficheiro), db).map_err(|source| ExportError::Csv {
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

fn write_csv<W: io::Write>(writer: W, db: &Db) -> Result<usize, csv::Error> {
    // `has_headers(false)`: o cabeçalho é escrito à mão a partir de
    // `CSV_COLUMNS`, que fica a ser a única fonte de verdade do formato.
    let mut writer = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(writer);
    writer.write_record(CSV_COLUMNS)?;
    for todo in &db.todos {
        writer.write_record(row(todo, &db.categories))?;
    }
    writer.flush()?;
    Ok(db.todos.len())
}

/// A linha de uma tarefa, com o **nome** da categoria no fim (vazio quando não
/// há categoria, ou quando o `category_id` não resolve em nenhuma — o
/// invariante diz que não acontece depois de um `load`, mas o export não
/// depende disso).
fn row(todo: &Todo, categorias: &[Category]) -> [String; 9] {
    let categoria = todo
        .category_id
        .as_ref()
        .and_then(|id| categorias.iter().find(|categoria| &categoria.id == id))
        .map(|categoria| categoria.name.clone())
        .unwrap_or_default();
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
        categoria,
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

/// Importa um CSV com cabeçalho — o de 9 colunas ou o de 8 da v1.1.0.
///
/// A coluna `category` leva o **nome**: um nome que não exista cria a
/// categoria, um que já exista (*case-insensitive*, como em `core::ops`) é
/// reaproveitado e o `category_id` das tarefas lidas passa a ser o `id`
/// **local** — é este passo que impede duas categorias com o mesmo nome depois
/// de um import. Campo vazio (ou ausente, no cabeçalho de 8) é «sem categoria».
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
        let (candidato, sem_data_desta) = registo.into_candidato(path, linha_fallback)?;
        if sem_data_desta {
            sem_data += 1;
        }
        candidatos.push(candidato);
    }

    let lidos = candidatos.len();
    // O CSV não traz lista de categorias: só nomes, nas linhas das tarefas.
    merge(store, candidatos, Vec::new(), Vec::new(), lidos, sem_data)
}

/// Importa JSON: o envelope novo (`{schema, todos, trash, categories}`) ou um
/// array.
///
/// No envelope as `categories` entram **antes** das tarefas, com dedupe por
/// `id` e reaproveitamento por nome (como no CSV): um `category_id` que não
/// resolva deixa a tarefa **sem categoria** e conta em
/// [`ImportReport::sem_categoria`] — a tarefa nunca se perde por causa de uma
/// categoria (ADR §Decisão 4). O `schema` `2` (a geração da v1.1.0) importa.
///
/// No array o formato é decidido **por registo**: um registo com qualquer dos
/// campos que só o formato novo tem é lido como [`Todo`] (com a mesma validade
/// do envelope e o `id` preservado); sem nenhum deles é lido como registo legado
/// do `rtodo` (`{title, description, done, time, date}`). Os dois podem conviver
/// no mesmo ficheiro.
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
                if registo_do_formato_novo(&registo) {
                    // O formato novo traz `created_at` validado pelo próprio
                    // `Todo`: aqui não há «data que não se leu».
                    candidatos.push(Candidato::do_registo(todo_do_formato_novo(
                        &registo, path, linha,
                    )?));
                    continue;
                }
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
                candidatos.push(Candidato::do_registo(todo));
            }
            let lidos = candidatos.len();
            // Um array não traz categorias: os `category_id` dele só podem
            // resolver contra as que a base já tem.
            merge(store, candidatos, Vec::new(), Vec::new(), lidos, sem_data)
        }
        serde_json::Value::Object(ref mapa) if mapa.contains_key("schema") => {
            let db: Db = serde_json::from_value(value).map_err(|source| ImportError::Json {
                path: path.to_path_buf(),
                source,
            })?;
            // O envelope `2` (o que a v1.1.0 gravava) importa: é o caminho de
            // recuperação de um backup antigo, e os `#[serde(default)]` fazem
            // as categorias e os `category_id` nascer vazios.
            if !SCHEMAS_ACEITES.contains(&db.schema) {
                return Err(ImportError::Schema {
                    path: path.to_path_buf(),
                    encontrado: db.schema,
                    esperado: SCHEMA_VERSION,
                });
            }
            let lidos = db.todos.len() + db.trash.len();
            // As categorias do ficheiro entram **antes** das tarefas: é o que
            // faz um `category_id` do próprio ficheiro resolver.
            merge(
                store,
                db.todos.into_iter().map(Candidato::do_registo).collect(),
                db.categories,
                db.trash,
                lidos,
                0,
            )
        }
        _ => Err(ImportError::NotEnveloped {
            path: path.to_path_buf(),
        }),
    }
}

/// O cabeçalho é comparado como **conjunto exacto**, e não por posição — é isso
/// que faz um CSV com as colunas noutra ordem importar. Aceita os dois
/// conjuntos: o de 9 colunas e o de 8 da v1.1.0 (ADR §Decisão 10).
fn cabecalho_esperado(encontrado: &[String]) -> bool {
    [&CSV_COLUMNS[..], &CSV_COLUMNS_V11[..]]
        .iter()
        .any(|esperado| mesmo_conjunto(esperado, encontrado))
}

/// `encontrado` é o mesmo conjunto de colunas que `esperado` (ordem indiferente).
fn mesmo_conjunto(esperado: &[&str], encontrado: &[String]) -> bool {
    if encontrado.len() != esperado.len() {
        return false;
    }
    let mut esperado = esperado.to_vec();
    esperado.sort_unstable();
    let mut visto: Vec<&str> = encontrado.iter().map(String::as_str).collect();
    visto.sort_unstable();
    esperado == visto
}

/// A chave de comparação de um nome de categoria: sem espaços à volta e em
/// minúsculas — a mesma regra de `core::ops::validar_nome` e a da busca
/// (`query::contem`). É o que impede «Trabalho» e «trabalho» de conviverem.
fn chave_do_nome(nome: &str) -> String {
    nome.trim().to_lowercase()
}

/// Um candidato a importar: a tarefa e a categoria que o ficheiro lhe pede.
///
/// A categoria vem como **pedido** e não como referência porque quem a resolve
/// é o [`merge`]: as categorias novas só nascem depois de se saber que tarefas
/// entram (um CSV reimportado não inventa categorias para tarefas que já cá
/// estavam), e a resolução escreve o `category_id` já com o `id` **local**.
#[derive(Debug)]
struct Candidato {
    todo: Todo,
    categoria: Option<PedidoCategoria>,
}

impl Candidato {
    /// Tira o `category_id` do registo e passa-o a pedido por `id`: um `id` do
    /// ficheiro só vale depois de resolvido contra as categorias da base (mais
    /// as que o próprio ficheiro trouxe).
    fn do_registo(mut todo: Todo) -> Self {
        let categoria = todo.category_id.take().map(PedidoCategoria::Id);
        Self { todo, categoria }
    }
}

/// Como o ficheiro identifica a categoria de uma tarefa.
#[derive(Debug)]
enum PedidoCategoria {
    /// CSV: pelo **nome** (é o formato que se lê e se escreve à mão).
    Nome(String),
    /// JSON: pelo `category_id`, que pode ter sido reaproveitado por nome.
    Id(CategoryId),
}

/// Integra as categorias que o ficheiro trouxe, devolvendo o mapa
/// `id do ficheiro → id local`.
///
/// Uma categoria que chegue com um **nome que já existe** (*case-insensitive*)
/// ou com um `id` que já existe **não entra**: é a mesma que cá está, e as
/// tarefas passam a apontar-lhe — é este passo que mantém verdadeiro o
/// invariante do ADR §Decisão 2 depois de um import. O dedupe por `id` vale
/// também dentro do próprio ficheiro: a segunda ocorrência é reaproveitada
/// pela primeira.
fn integrar_categorias(
    db: &mut Db,
    entrantes: Vec<Category>,
    relatorio: &mut ImportReport,
) -> HashMap<CategoryId, CategoryId> {
    let mut remap = HashMap::new();
    for categoria in entrantes {
        // A comparação resolve-se antes de mexer na `Db`: um `if let` a
        // segurar o empréstimo da lista impediria o `push` do ramo seguinte.
        let existente = db
            .categories
            .iter()
            .find(|local| {
                local.id == categoria.id
                    || chave_do_nome(&local.name) == chave_do_nome(&categoria.name)
            })
            .map(|local| local.id.clone());
        match existente {
            Some(id) => {
                remap.insert(categoria.id, id);
                relatorio.categorias_reaproveitadas += 1;
            }
            None => {
                remap.insert(categoria.id.clone(), categoria.id.clone());
                db.categories.push(categoria);
                relatorio.categorias_criadas += 1;
            }
        }
    }
    remap
}

/// Insere os candidatos, ignorando os `id` que já existem (ordem preservada), e
/// integra as categorias: as do ficheiro (envelope) antes, as que os nomes do
/// CSV pedem depois — só para as tarefas que entram de facto.
///
/// A `Db` de trabalho é uma **cópia**: a gravação é a última operação, logo um
/// erro a meio não deixa tarefas nem categorias «meio importadas».
fn merge(
    store: &mut Store,
    candidatos: Vec<Candidato>,
    categorias: Vec<Category>,
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
    for candidato in candidatos {
        // `insert` devolve `false` tanto para um id já na base de dados como
        // para um id repetido dentro do próprio ficheiro.
        if !ids.insert(candidato.todo.id.clone()) {
            relatorio.duplicados += 1;
            continue;
        }
        novos.push(candidato);
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

    let mut db = store.db().clone();
    let remap = integrar_categorias(&mut db, categorias, &mut relatorio);

    // Nomes já resolvidos: uma categoria criada por uma tarefa deste import é
    // reaproveitada pelas seguintes **sem voltar a contar** (e dez tarefas na
    // mesma categoria que já cá estava são uma só categoria reaproveitada).
    let mut por_nome: HashMap<String, CategoryId> = db
        .categories
        .iter()
        .map(|local| (chave_do_nome(&local.name), local.id.clone()))
        .collect();
    let mut contados: HashSet<String> = HashSet::new();
    for candidato in &mut novos {
        let Some(pedido) = candidato.categoria.take() else {
            continue;
        };
        match pedido {
            PedidoCategoria::Nome(nome) => {
                let chave = chave_do_nome(&nome);
                let (id, criada_agora) = match por_nome.get(&chave) {
                    // Já existe (cá ou criada por uma tarefa anterior deste
                    // import): a tarefa aponta-lhe.
                    Some(id) => (id.clone(), false),
                    None => {
                        let nova = Category {
                            id: CategoryId::new(),
                            name: nome.trim().to_owned(),
                        };
                        let id = nova.id.clone();
                        db.categories.push(nova);
                        por_nome.insert(chave.clone(), id.clone());
                        relatorio.categorias_criadas += 1;
                        // Marca-a como já contabilizada: uma segunda tarefa com
                        // este nome não pode contar como «reaproveitada» (o que
                        // este import criou já foi contado em `criadas`).
                        contados.insert(chave.clone());
                        (id, true)
                    }
                };
                // «Reaproveitada» = a categoria já cá estava **antes** deste
                // import, e conta-se uma vez por categoria e não por tarefa.
                if !criada_agora && contados.insert(chave) {
                    relatorio.categorias_reaproveitadas += 1;
                }
                candidato.todo.category_id = Some(id);
            }
            PedidoCategoria::Id(id) => {
                // O `remap` diz qual é o `id` local de um `id` do ficheiro que
                // tenha sido reaproveitado por nome.
                let local = remap.get(&id).unwrap_or(&id);
                match db
                    .categories
                    .iter()
                    .find(|local_cat| &local_cat.id == local)
                {
                    Some(categoria) => candidato.todo.category_id = Some(categoria.id.clone()),
                    // A referência não resolve: a tarefa entra **sem
                    // categoria** e conta — nunca se perde (ADR §Decisão 4).
                    None => relatorio.sem_categoria += 1,
                }
            }
        }
    }

    // Nada mudou (tudo duplicado e nenhuma categoria nova): não se toca no
    // ficheiro — nem se roda o `.bak`. O trabalho foi todo sobre a cópia.
    if novos.is_empty() && lixo_novo.is_empty() && relatorio.categorias_criadas == 0 {
        return Ok(relatorio);
    }

    db.todos
        .extend(novos.into_iter().map(|candidato| candidato.todo));
    db.trash.extend(lixo_novo);
    let antes = db.trash.len();
    trim_trash(&mut db); // o import também respeita o limite do lixo
    // `saturating_sub` e não `-=`: um import que não traz lixo nenhum e cuja
    // `Db` já tivesse o lixo acima do limite não tem de onde descontar.
    relatorio.lixo = relatorio.lixo.saturating_sub(antes - db.trash.len());

    store.save_db(db).map_err(ImportError::Persist)?;
    Ok(relatorio)
}

// ------------------------------------------------- registo do formato novo

/// Campos que **só** o formato novo tem: é por eles que se reconhece um
/// registo de [`Todo`] dentro de um array.
///
/// `title`, `description` e `done` são comuns aos dois formatos e por isso não
/// discriminam — `done` no conjunto, em particular, mandaria **todo** o registo
/// legado para o ramo novo, porque o `rtodo` também o gravava
/// (`state/rtodo_ro/src/todos.rs:26-32` no ADR). `category_id` só existe no
/// formato novo: um registo que o traga é do formato novo, e o erro que sai
/// dele (`id` em falta) é o erro certo.
const CAMPOS_DO_FORMATO_NOVO: [&str; 6] = [
    "id",
    "priority",
    "created_at",
    "completed_at",
    "due_at",
    "category_id",
];

/// Um registo é do formato novo quando traz **qualquer** campo que só ele tem.
fn registo_do_formato_novo(registo: &serde_json::Value) -> bool {
    registo
        .as_object()
        .is_some_and(|mapa| CAMPOS_DO_FORMATO_NOVO.iter().any(|c| mapa.contains_key(*c)))
}

/// Lê um registo do formato novo com a **mesma** validade do envelope: é
/// `serde_json::from_value::<Todo>`, o mesmo caminho que o `Db` usa.
///
/// Sem tolerâncias extra: um array nu mais permissivo do que o envelope seriam
/// duas regras de validade para o mesmo tipo. O `id` que lá estiver é
/// preservado — é o que faz o dedupe do [`merge`] voltar a casar num array.
fn todo_do_formato_novo(
    registo: &serde_json::Value,
    path: &Path,
    linha: u64,
) -> Result<Todo, ImportError> {
    serde_json::from_value::<Todo>(registo.clone()).map_err(|source| ImportError::Row {
        path: path.to_path_buf(),
        linha,
        campo: campo_do_erro(registo, &source),
        detalhe: source.to_string(),
    })
}

/// A que campo pertence o erro do `serde_json`.
///
/// O `serde_json` só nomeia o campo nos erros de campo ausente («missing field
/// `id`»); nos de valor errado diz apenas «invalid type: null, expected a
/// string». Por isso, quando a mensagem não nomeia o campo, o valor é
/// revalidado campo a campo, na ordem de [`Todo`] — o primeiro que não passa é
/// o culpado, e o `serde` segue essa mesma ordem ao ler a struct.
fn campo_do_erro(registo: &serde_json::Value, erro: &serde_json::Error) -> &'static str {
    let Some(mapa) = registo.as_object() else {
        return "registo novo";
    };
    let mensagem = erro.to_string();
    for campo in CSV_COLUMNS {
        if mensagem.contains(&format!("missing field `{campo}`")) {
            return campo;
        }
    }
    for campo in CSV_COLUMNS {
        if mapa
            .get(campo)
            .is_some_and(|valor| !campo_valido(campo, valor))
        {
            return campo;
        }
    }
    // A categoria é o último campo que o `Todo` declara, logo é o último a ser
    // revalidado — a mesma ordem que o `serde` segue ao ler a struct. No JSON
    // chama-se `category_id` (é por esse nome que o `Todo` o declara e por ele
    // que o `serde` o nomeia); `category` é o nome da **coluna** no CSV.
    if mapa
        .get("category_id")
        .is_some_and(|valor| !campo_valido("category_id", valor))
    {
        return "category_id";
    }
    "registo novo"
}

/// O valor passa a validação do campo, com o tipo que [`Todo`] declara.
fn campo_valido(campo: &str, valor: &serde_json::Value) -> bool {
    match campo {
        "id" => serde_json::from_value::<TodoId>(valor.clone()).is_ok(),
        "title" | "description" => valor.is_string(),
        "done" => valor.is_boolean(),
        "priority" => serde_json::from_value::<Priority>(valor.clone()).is_ok(),
        "created_at" => serde_json::from_value::<DateTime<Local>>(valor.clone()).is_ok(),
        "completed_at" => serde_json::from_value::<Option<DateTime<Local>>>(valor.clone()).is_ok(),
        "due_at" => serde_json::from_value::<Option<NaiveDate>>(valor.clone()).is_ok(),
        // O mesmo campo com os dois nomes: `category` é a coluna do CSV,
        // `category_id` é o campo do `Todo` (e do JSON).
        "category" | "category_id" => {
            serde_json::from_value::<Option<CategoryId>>(valor.clone()).is_ok()
        }
        _ => true,
    }
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
    /// Nome da categoria. Ausente no cabeçalho de 8 colunas da v1.1.0 (o
    /// `serde(default)` lê-o como vazio, que é «sem categoria») e vazio numa
    /// tarefa sem categoria.
    #[serde(default)]
    category: String,
}

impl CsvRow {
    /// Devolve o candidato e se veio **sem data** (`created_at` vazio).
    ///
    /// O nome da categoria fica como **pedido**, não como `category_id`: quem
    /// o resolve (criando ou reaproveitando) é o [`merge`], que tem a `Db` à
    /// frente e só o faz para as tarefas que entram.
    fn into_candidato(self, path: &Path, linha: u64) -> Result<(Candidato, bool), ImportError> {
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

        let nome = self.category.trim().to_owned();
        Ok((
            Candidato {
                todo: Todo {
                    id: TodoId::from(id.to_owned()),
                    title: title.to_owned(),
                    description: self.description,
                    done: self.done,
                    priority,
                    created_at,
                    completed_at,
                    due_at,
                    category_id: None,
                },
                // Vazio (ou ausente) é «sem categoria»: não há pedido nenhum
                // para resolver, logo também não conta em `sem_categoria`.
                categoria: (!nome.is_empty()).then_some(PedidoCategoria::Nome(nome)),
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
                category_id: None,
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
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
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

        assert_eq!(export_csv_to_path(store.db(), &path).unwrap(), 1);

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
        export_csv_to_path(store.db(), &path).unwrap();

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
        assert!(
            mensagem.contains(&CSV_COLUMNS.join(", ")),
            "a mensagem tem de mostrar o cabeçalho de 9 colunas: {mensagem}"
        );
        assert!(
            mensagem.contains(&CSV_COLUMNS_V11.join(", ")),
            "e também o de 8 colunas da v1.1.0, que continua a ser aceite: {mensagem}"
        );
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
        export_csv_to_path(origem.db(), &path).unwrap();

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
        export_csv_to_path(origem.db(), &csv).unwrap();

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
            ..ImportReport::default()
        };
        assert_eq!(
            relatorio.resumo(),
            "10 lidos, 7 inseridos, 3 duplicados ignorados, 1 no lixo, 2 sem data legível"
        );
        assert!(!relatorio.sem_alteracoes());
        assert!(ImportReport::default().sem_alteracoes());
    }

    /// A linha que o utilizador lê quando o import trouxe categorias: cada
    /// contador só aparece quando não é zero, como o `lixo` e o `sem_data`.
    #[test]
    fn relatorio_do_import_conta_as_categorias() {
        let relatorio = ImportReport {
            lidos: 3,
            inseridos: 3,
            categorias_criadas: 1,
            categorias_reaproveitadas: 1,
            sem_categoria: 2,
            ..ImportReport::default()
        };
        assert_eq!(
            relatorio.resumo(),
            "3 lidos, 3 inseridos, 0 duplicados ignorados, 1 categorias criadas, \
             1 categorias reaproveitadas, 2 sem categoria"
        );
        assert!(!relatorio.sem_alteracoes());

        // Só as categorias (um envelope com categorias e zero tarefas novas)
        // já é uma alteração: dizer «sem alterações» seria falso.
        let so_categorias = ImportReport {
            categorias_criadas: 1,
            ..ImportReport::default()
        };
        assert!(!so_categorias.sem_alteracoes());
    }

    #[test]
    fn id_no_csv_e_estavel_entre_exportacoes() {
        let (_, mut store) = nova_loja("id-estavel");
        let id = store.add("estável").unwrap();
        let dir = dir("id-estavel-destino");
        let path = dir.join("a.csv");
        export_csv_to_path(store.db(), &path).unwrap();

        let (_, mut destino) = nova_loja("id-estavel-db");
        import_csv_from_path(&mut destino, &path).unwrap();
        let importado = &destino.todos()[0];
        assert_eq!(importado.id, id);
        assert_eq!(TodoId::from_str(id.as_str()).unwrap(), importado.id);
        // E o lixo continua vazio depois de um import sem lixo.
        assert!(destino.trash_entries().is_empty());
    }

    // ------------------------------------------------ array do formato novo

    /// O que `jq '.todos' db.json` devolve, escrito num ficheiro.
    fn array_do_formato_novo(store: &Store) -> String {
        serde_json::to_string_pretty(store.todos()).expect("serializar as tarefas")
    }

    #[test]
    fn import_json_array_do_formato_novo_preserva_id_prioridade_e_data() {
        let (_, mut origem) = nova_loja("array-origem");
        let id = origem.add("uma do formato novo").unwrap();
        origem.set_priority(&id, Priority::High).unwrap();
        let original = origem.find(&id).unwrap().clone();

        let dir = dir("array");
        let path = escrever(&dir, "arr.json", &array_do_formato_novo(&origem));

        let (_, mut destino) = nova_loja("array-db");
        let relatorio = import_json_from_path(&mut destino, &path).unwrap();

        assert_eq!(relatorio.lidos, 1);
        assert_eq!(relatorio.inseridos, 1);
        assert_eq!(relatorio.duplicados, 0);
        assert_eq!(
            relatorio.sem_data, 0,
            "o formato novo traz a data validada: não há «sem data legível» aqui"
        );

        let importado = destino.find(&id).expect("voltou com o mesmo id");
        assert_eq!(importado, &original, "round-trip campo a campo");
        assert_eq!(importado.priority, Priority::High);
        assert_eq!(importado.created_at, original.created_at);
    }

    #[test]
    fn import_json_array_do_formato_novo_duas_vezes_nao_duplica() {
        let (_, mut origem) = nova_loja("array-dedupe-origem");
        origem.add("primeira").unwrap();
        origem.add("segunda").unwrap();
        let dir = dir("array-dedupe");
        let path = escrever(&dir, "arr.json", &array_do_formato_novo(&origem));

        let (_, mut destino) = nova_loja("array-dedupe-db");
        let primeiro = import_json_from_path(&mut destino, &path).unwrap();
        assert_eq!(primeiro.lidos, 2);
        assert_eq!(primeiro.inseridos, 2);
        assert_eq!(primeiro.duplicados, 0);

        let segundo = import_json_from_path(&mut destino, &path).unwrap();
        assert_eq!(segundo.lidos, 2);
        assert_eq!(segundo.inseridos, 0);
        assert_eq!(segundo.duplicados, 2, "os ids preservados voltam a casar");
        assert!(segundo.sem_alteracoes());
        assert_eq!(destino.todos().len(), 2, "sem extend cego");
    }

    #[test]
    fn import_json_array_do_formato_novo_sem_id_da_erro_a_nomear_o_campo() {
        let dir = dir("array-sem-id");
        let path = escrever(
            &dir,
            "sem-id.json",
            r#"[{"title":"sem id","description":"","done":false,"priority":"high","created_at":"2026-09-18T10:00:00+01:00"}]"#,
        );

        let (_, mut store) = nova_loja("array-sem-id-db");
        let erro = import_json_from_path(&mut store, &path).expect_err("tinha de falhar");

        match &erro {
            ImportError::Row {
                linha,
                campo,
                path: ficheiro,
                ..
            } => {
                assert_eq!(*linha, 1);
                assert_eq!(*campo, "id", "o erro nomeia o campo que falta");
                assert_eq!(ficheiro, &path, "o erro traz o ficheiro certo");
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
        assert!(erro.to_string().contains("sem-id.json"), "mensagem: {erro}");
        assert_eq!(store.todos().len(), 0, "nunca aceitação muda");
    }

    #[test]
    fn import_json_array_sem_created_at_da_erro_a_nomear_o_campo() {
        let dir = dir("array-sem-data");
        let path = escrever(
            &dir,
            "sem-data.json",
            r#"[{"id":"a1","title":"sem data","description":"","done":false,"priority":"high"}]"#,
        );

        let (_, mut store) = nova_loja("array-sem-data-db");
        let erro = import_json_from_path(&mut store, &path).expect_err("tinha de falhar");

        match &erro {
            ImportError::Row { linha, campo, .. } => {
                assert_eq!(*linha, 1);
                assert_eq!(*campo, "created_at");
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
        assert_eq!(store.todos().len(), 0);
    }

    #[test]
    fn import_json_array_com_description_nula_da_erro_a_nomear_o_campo() {
        let dir = dir("array-descricao-nula");
        let path = escrever(
            &dir,
            "nula.json",
            r#"[{"id":"a1","title":"descrição nula","description":null,"done":false,"priority":"high","created_at":"2026-09-18T10:00:00+01:00"}]"#,
        );

        let (_, mut store) = nova_loja("array-descricao-nula-db");
        let erro = import_json_from_path(&mut store, &path).expect_err("tinha de falhar");

        match &erro {
            ImportError::Row { linha, campo, .. } => {
                assert_eq!(*linha, 1);
                assert_eq!(*campo, "description");
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
        assert_eq!(store.todos().len(), 0);
    }

    #[test]
    fn a_validade_do_array_novo_e_a_do_envelope() {
        // O que o envelope recusa (é o próprio `Todo` a recusar), o array nu
        // também recusa: não há duas regras de validade para o mesmo tipo.
        let maus = [
            json!({"id":"a1","title":"t","description":null,"done":false,"priority":"high","created_at":"2026-09-18T10:00:00+01:00"}),
            json!({"id":"a1","title":"t","description":"","done":false,"priority":"high"}),
            json!({"id":"a1","title":"t","description":"","done":false,"priority":"urgente","created_at":"2026-09-18T10:00:00+01:00"}),
            json!({"id":"a1","title":"t","description":"","done":"sim","priority":"high","created_at":"2026-09-18T10:00:00+01:00"}),
            json!({"title":"t","description":"","done":false,"priority":"high","created_at":"2026-09-18T10:00:00+01:00"}),
        ];

        let dir = dir("array-validade");
        let (_, mut store) = nova_loja("array-validade-db");
        for (n, mau) in maus.iter().enumerate() {
            assert!(
                serde_json::from_value::<Todo>(mau.clone()).is_err(),
                "o envelope recusava isto: {mau}"
            );
            let path = escrever(&dir, &format!("mau-{n}.json"), &format!("[{mau}]"));
            let erro = import_json_from_path(&mut store, &path)
                .expect_err("o array tem de recusar o que o envelope recusa");
            assert!(matches!(erro, ImportError::Row { .. }), "erro foi {erro:?}");
        }

        // E o caminho real do envelope, com o primeiro caso mau lá dentro.
        let envelope = format!(
            r#"{{"schema":{SCHEMA_VERSION},"todos":[{}],"trash":[]}}"#,
            maus[0]
        );
        let path = escrever(&dir, "envelope-mau.json", &envelope);
        let erro = import_json_from_path(&mut store, &path).expect_err("envelope mau");
        assert!(
            matches!(erro, ImportError::Json { .. }),
            "erro foi {erro:?}"
        );

        assert_eq!(
            store.todos().len(),
            0,
            "nada entrou por nenhum dos caminhos"
        );
    }

    #[test]
    fn um_array_com_os_dois_formatos_e_lido_registo_a_registo() {
        let dir = dir("array-misto");
        let path = escrever(
            &dir,
            "misto.json",
            r#"[
                {"title":"antiga","description":"do rtodo","done":false,"time":"High","date":""},
                {"id":"nova-1","title":"nova","description":"","done":false,"priority":"low","created_at":"2026-09-18T10:00:00+01:00"}
            ]"#,
        );

        let (_, mut store) = nova_loja("array-misto-db");
        let relatorio = import_json_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.lidos, 2);
        assert_eq!(relatorio.inseridos, 2);
        assert_eq!(
            relatorio.sem_data, 1,
            "só o registo legado conta em «sem data legível»"
        );

        let nova = store
            .find(&TodoId::from("nova-1".to_owned()))
            .expect("o id do formato novo é preservado");
        assert_eq!(nova.title, "nova");
        assert_eq!(nova.priority, Priority::Low);

        let antiga = store
            .todos()
            .iter()
            .find(|t| t.title == "antiga")
            .expect("o registo legado continua a ser lido como legado");
        assert_eq!(antiga.priority, Priority::High, "o `time` do rtodo mapeia");
        assert_ne!(antiga.id.as_str(), "nova-1", "o legado ganha id novo");
    }

    // ------------------------------------------------------------- SA-02/03

    /// Os bits de permissão do caminho, sem o tipo (`0o600`, …).
    fn modo(path: &Path) -> u32 {
        fs::metadata(path).expect("metadados").permissions().mode() & 0o777
    }

    #[test]
    fn o_csv_exportado_nasce_privado_e_sem_temporario() {
        let (_, mut store) = nova_loja("csv-privado");
        store.add("privada").unwrap();
        let dir = dir("csv-privado-destino");
        let path = dir.join("saida.csv");

        export_csv_to_path(store.db(), &path).expect("exportar");

        assert_eq!(modo(&path), 0o600, "o CSV exportado nasce 0600");
        assert!(
            !crate::core::atomic::tmp_path_for(&path).exists(),
            "ficou o temporário do export"
        );
    }

    /// O temporário do export é `<destino>.tmp`, logo previsível: um symlink lá
    /// plantado não pode redireccionar a escrita (o `File::create` de antes
    /// atravessava-o).
    #[test]
    fn symlink_no_temporario_do_csv_nao_redirecciona_a_escrita() {
        let (_, mut store) = nova_loja("csv-symlink");
        store.add("tarefa").unwrap();
        let dir = dir("csv-symlink-destino");
        let path = dir.join("saida.csv");
        let vitima = dir.join("vitima.txt");
        fs::write(&vitima, "conteudo da vitima").expect("escrever a vítima");
        std::os::unix::fs::symlink(&vitima, crate::core::atomic::tmp_path_for(&path))
            .expect("plantar o symlink");

        export_csv_to_path(store.db(), &path).expect("o export tem de suceder");

        assert_eq!(
            fs::read_to_string(&vitima).expect("ler a vítima"),
            "conteudo da vitima",
            "a escrita atravessou o symlink"
        );
        assert!(
            fs::read_to_string(&path)
                .expect("ler o CSV")
                .contains("tarefa"),
            "o CSV ficou com a tarefa"
        );
    }

    // -------------------------------------------------------- categorias (T3)

    /// O cabeçalho exportado tem as **9** colunas e a última é `category`;
    /// exportar→importar numa base vazia não duplica nada e mantém a
    /// atribuição (critério 2).
    #[test]
    fn csv_tem_9_colunas_a_ultima_e_a_categoria_e_o_round_trip_nao_duplica() {
        let (_, mut origem) = nova_loja("cat-export");
        let trabalho = origem.add_category("Trabalho").unwrap();
        let com = origem.add("com categoria").unwrap();
        origem.assign_category(&com, Some(&trabalho)).unwrap();
        origem.add("sem categoria").unwrap();

        let dir = dir("cat-export-destino");
        let path = dir.join("backup.csv");
        assert_eq!(export_csv_to_path(origem.db(), &path).unwrap(), 2);

        let conteudo = fs::read_to_string(&path).unwrap();
        let mut linhas = conteudo.lines();
        let cabecalho = linhas.next().unwrap();
        assert_eq!(cabecalho, CSV_COLUMNS.join(","));
        assert_eq!(cabecalho.split(',').count(), 9, "9 colunas");
        assert_eq!(
            cabecalho.split(',').next_back().unwrap(),
            "category",
            "a categoria é a última coluna"
        );
        assert_eq!(
            linhas.next().unwrap().split(',').next_back().unwrap(),
            "Trabalho",
            "a coluna leva o **nome** da categoria"
        );
        assert_eq!(
            linhas.next().unwrap().split(',').next_back().unwrap(),
            "",
            "tarefa sem categoria sai com o campo vazio"
        );

        // Numa base vazia: entra tudo, nada duplica, a atribuição mantém-se.
        let (_, mut destino) = nova_loja("cat-export-db");
        let relatorio = import_csv_from_path(&mut destino, &path).unwrap();
        assert_eq!(relatorio.lidos, 2);
        assert_eq!(relatorio.inseridos, 2);
        assert_eq!(relatorio.duplicados, 0);
        assert_eq!(relatorio.categorias_criadas, 1, "a categoria viaja no CSV");
        assert_eq!(relatorio.categorias_reaproveitadas, 0);
        assert_eq!(relatorio.sem_categoria, 0);
        assert_eq!(destino.categories().len(), 1);
        let importada = destino.find(&com).expect("a tarefa voltou com o mesmo id");
        assert_eq!(
            destino.category_of(importada).map(|c| c.name.as_str()),
            Some("Trabalho")
        );
        let sem_categoria = destino
            .todos()
            .iter()
            .find(|todo| todo.title == "sem categoria")
            .unwrap();
        assert!(destino.category_of(sem_categoria).is_none());

        // Um segundo import do mesmo ficheiro não duplica nada — nem as
        // categorias. E como nenhuma tarefa entrou, não houve nomes a
        // resolver: o relatório não fala de categorias (nada mudou).
        let segundo = import_csv_from_path(&mut destino, &path).unwrap();
        assert_eq!(segundo.inseridos, 0);
        assert_eq!(segundo.duplicados, 2);
        assert_eq!(segundo.categorias_criadas, 0);
        assert_eq!(segundo.categorias_reaproveitadas, 0);
        assert!(segundo.sem_alteracoes());
        assert_eq!(destino.todos().len(), 2);
        assert_eq!(destino.categories().len(), 1, "não se duplicou a categoria");
    }

    /// O CSV de 8 colunas que a v1.1.0 exportava importa, sem erro, e todas as
    /// tarefas ficam sem categoria (critério 3).
    #[test]
    fn csv_da_v1_1_com_8_colunas_importa_e_fica_tudo_sem_categoria() {
        let dir = dir("cat-v11");
        let path = escrever(
            &dir,
            "v11.csv",
            "id,title,description,done,priority,created_at,completed_at,due_at\n\
             v1,antiga,desc,false,high,2026-09-18T10:00:00+01:00,,\n\
             v2,outra,,true,low,2026-09-18T11:00:00+01:00,2026-09-18T12:00:00+01:00,\n",
        );

        let (_, mut store) = nova_loja("cat-v11-db");
        let relatorio = import_csv_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.lidos, 2);
        assert_eq!(relatorio.inseridos, 2);
        assert_eq!(
            relatorio.categorias_criadas, 0,
            "o de 8 colunas não traz categoria"
        );
        assert_eq!(
            relatorio.sem_categoria, 0,
            "campo ausente não é uma referência que falha"
        );
        assert!(store.categories().is_empty());
        assert!(store.todos().iter().all(|todo| todo.category_id.is_none()));
    }

    /// A comparação do cabeçalho é por **conjunto exacto**: um CSV de 9 colunas
    /// noutra ordem importa (critério 4) — e continua a ser assim depois de a
    /// coluna nova entrar.
    #[test]
    fn csv_de_9_colunas_noutra_ordem_importa() {
        let dir = dir("cat-ordem");
        let path = escrever(
            &dir,
            "ordem.csv",
            "category,due_at,id,title,priority,done,created_at,completed_at,description\n\
             Viagens,,v1,com categoria,high,false,2026-09-18T10:00:00+01:00,,nota\n\
             ,,v2,outra,low,false,2026-09-18T10:00:00+01:00,,\n",
        );

        let (_, mut store) = nova_loja("cat-ordem-db");
        let relatorio = import_csv_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.lidos, 2);
        assert_eq!(relatorio.inseridos, 2);
        assert_eq!(relatorio.categorias_criadas, 1);
        assert_eq!(relatorio.sem_categoria, 0);

        let com = store.find(&TodoId::from("v1".to_owned())).unwrap();
        assert_eq!(com.description, "nota", "as colunas foram lidas pelo nome");
        assert_eq!(
            store.category_of(com).map(|c| c.name.as_str()),
            Some("Viagens")
        );
        let sem = store.find(&TodoId::from("v2".to_owned())).unwrap();
        assert!(sem.category_id.is_none());
    }

    /// Uma base que já tem `trabalho` e um CSV com `Trabalho`: **0 criadas, 1
    /// reaproveitada**, e todas as tarefas apontam à que cá está (critério 5).
    /// O contador conta a **categoria**, não as tarefas.
    #[test]
    fn csv_reaproveita_a_categoria_existente_por_nome_case_insensitive() {
        let (_, mut store) = nova_loja("cat-reaproveita");
        let existente = store.add_category("trabalho").unwrap();

        let dir = dir("cat-reaproveita-origem");
        let path = escrever(
            &dir,
            "cat.csv",
            "id,title,description,done,priority,created_at,completed_at,due_at,category\n\
             a1,uma,,false,medium,2026-09-18T10:00:00+01:00,,,  Trabalho  \n\
             a2,duas,,false,medium,2026-09-18T10:00:00+01:00,,,TRABALHO\n",
        );
        let relatorio = import_csv_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.inseridos, 2);
        assert_eq!(
            relatorio.categorias_criadas, 0,
            "não se duplica a categoria"
        );
        assert_eq!(
            relatorio.categorias_reaproveitadas, 1,
            "duas tarefas na mesma categoria são uma só categoria reaproveitada"
        );
        assert_eq!(store.categories().len(), 1);
        for todo in store.todos() {
            assert_eq!(
                todo.category_id.as_ref(),
                Some(&existente),
                "todas as tarefas apontam à categoria que cá está"
            );
        }
    }

    /// Um **nome novo** no CSV cria a categoria, conta-a e atribui-a; e um nome
    /// vazio não cria nada.
    #[test]
    fn csv_com_nome_novo_cria_a_categoria() {
        let dir = dir("cat-nova");
        let path = escrever(
            &dir,
            "nova.csv",
            "id,title,description,done,priority,created_at,completed_at,due_at,category\n\
             a1,uma,,false,medium,2026-09-18T10:00:00+01:00,,,Casa\n\
             a2,duas,,false,medium,2026-09-18T10:00:00+01:00,,,Casa\n\
             a3,três,,false,medium,2026-09-18T10:00:00+01:00,,,   \n",
        );
        let (_, mut store) = nova_loja("cat-nova-db");
        let relatorio = import_csv_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.inseridos, 3);
        assert_eq!(relatorio.categorias_criadas, 1, "o mesmo nome só cria uma");
        assert_eq!(relatorio.categorias_reaproveitadas, 0);
        assert_eq!(relatorio.sem_categoria, 0);
        assert_eq!(store.categories().len(), 1);
        let casa = &store.categories()[0];
        assert_eq!(casa.name, "Casa");
        assert_eq!(
            store
                .find(&TodoId::from("a1".to_owned()))
                .unwrap()
                .category_id
                .as_ref(),
            Some(&casa.id)
        );
        assert_eq!(
            store
                .find(&TodoId::from("a3".to_owned()))
                .unwrap()
                .category_id,
            None,
            "nome só com espaços é «sem categoria»"
        );
    }

    /// Um array de tarefas com um `category_id` que não resolve: a tarefa entra
    /// **sem categoria** e o resumo di-lo (critério 6). A tarefa não se perde.
    #[test]
    fn array_com_category_id_desconhecido_entra_sem_categoria() {
        let dir = dir("cat-orfao");
        let path = escrever(
            &dir,
            "orfao.json",
            r#"[{"id":"a1","title":"com id pendente","description":"","done":false,"priority":"high","created_at":"2026-09-18T10:00:00+01:00","category_id":"nao-existe"}]"#,
        );

        let (_, mut store) = nova_loja("cat-orfao-db");
        let relatorio = import_json_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.inseridos, 1);
        assert_eq!(relatorio.sem_categoria, 1);
        assert_eq!(relatorio.categorias_criadas, 0);
        assert!(store.categories().is_empty());
        let tarefa = store
            .find(&TodoId::from("a1".to_owned()))
            .expect("a tarefa entra na mesma: nunca se perde por causa da categoria");
        assert!(tarefa.category_id.is_none());
        assert!(
            relatorio.resumo().contains("1 sem categoria"),
            "resumo: {}",
            relatorio.resumo()
        );
    }

    /// Um array com um `category_id` que **resolve** numa categoria da base
    /// mantém a atribuição (e não conta nada).
    #[test]
    fn array_com_category_id_da_base_mantem_a_atribuicao() {
        let (_, mut store) = nova_loja("cat-orfao-bom-db");
        let existente = store.add_category("Casa").unwrap();

        let dir = dir("cat-orfao-bom");
        let path = escrever(
            &dir,
            "bom.json",
            &format!(
                r#"[{{"id":"a1","title":"com categoria","description":"","done":false,"priority":"high","created_at":"2026-09-18T10:00:00+01:00","category_id":"{}"}}]"#,
                existente.as_str()
            ),
        );
        let relatorio = import_json_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.inseridos, 1);
        assert_eq!(relatorio.sem_categoria, 0);
        assert_eq!(relatorio.categorias_criadas, 0);
        assert_eq!(
            store
                .find(&TodoId::from("a1".to_owned()))
                .unwrap()
                .category_id
                .as_ref(),
            Some(&existente)
        );
    }

    /// Envelope `schema: 3` com categorias: exportar→importar preserva a
    /// atribuição e não duplica; `schema: 2` (o da v1.1.0) importa; `schema: 9`
    /// dá erro **e deixa a `Db` exactamente como estava** (critério 7).
    #[test]
    fn envelope_com_categorias_round_trip_e_schema_futuro_sem_tocar_na_db() {
        let (origem_path, mut origem) = nova_loja("cat-envelope-origem");
        let trabalho = origem.add_category("Trabalho").unwrap();
        let id = origem.add("com categoria").unwrap();
        origem.assign_category(&id, Some(&trabalho)).unwrap();
        origem.save().unwrap();

        let dir = dir("cat-envelope");
        let envelope = dir.join("db.json");
        fs::copy(&origem_path, &envelope).unwrap();

        let (_, mut destino) = nova_loja("cat-envelope-db");
        let relatorio = import_json_from_path(&mut destino, &envelope).unwrap();
        assert_eq!(relatorio.lidos, 1);
        assert_eq!(relatorio.inseridos, 1);
        assert_eq!(relatorio.categorias_criadas, 1);
        assert_eq!(relatorio.categorias_reaproveitadas, 0);
        assert_eq!(relatorio.sem_categoria, 0);
        let importada = destino.find(&id).expect("a tarefa voltou com o mesmo id");
        assert_eq!(
            destino.category_of(importada).map(|c| c.name.as_str()),
            Some("Trabalho")
        );

        // O mesmo envelope outra vez: nada muda.
        let segundo = import_json_from_path(&mut destino, &envelope).unwrap();
        assert_eq!(segundo.inseridos, 0);
        assert_eq!(segundo.duplicados, 1);
        assert_eq!(segundo.categorias_criadas, 0);
        assert_eq!(segundo.categorias_reaproveitadas, 1);
        assert!(segundo.sem_alteracoes());
        assert_eq!(
            destino.db().categories.len(),
            1,
            "não se duplicou a categoria"
        );

        // O `schema: 2` da v1.1.0 importa: as tarefas ficam sem categoria.
        let v11 = dir.join("v11.json");
        fs::write(
            &v11,
            r#"{"schema":2,"todos":[{"id":"b1","title":"da v1.1.0","created_at":"2026-09-18T10:00:00+01:00"}],"trash":[]}"#,
        )
        .unwrap();
        let relatorio = import_json_from_path(&mut destino, &v11).unwrap();
        assert_eq!(relatorio.inseridos, 1);
        assert_eq!(relatorio.sem_categoria, 0);
        let antiga = destino.find(&TodoId::from("b1".to_owned())).unwrap();
        assert!(antiga.category_id.is_none());

        // Um `schema` que não sabemos ler: erro, e a `Db` fica como estava
        // (nem as categorias do ficheiro entram).
        let antes = destino.db().clone();
        let futuro = dir.join("futuro.json");
        fs::write(
            &futuro,
            r#"{"schema":9,"todos":[],"trash":[],"categories":[{"id":"c9","name":"Futuro"}]}"#,
        )
        .unwrap();
        let erro = import_json_from_path(&mut destino, &futuro).expect_err("tinha de falhar");
        match &erro {
            ImportError::Schema {
                encontrado,
                esperado,
                ..
            } => {
                assert_eq!(*encontrado, 9);
                assert_eq!(*esperado, SCHEMA_VERSION);
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
        assert_eq!(destino.db(), &antes, "a Db não mudou nada");
        assert_eq!(
            destino.db().categories.len(),
            1,
            "a categoria do ficheiro não entrou"
        );
    }

    /// O envelope traz uma categoria com o **mesmo nome** de uma que cá está
    /// (mas outro `id`): não entra, e a tarefa do ficheiro passa a apontar à
    /// existente — é isto que impede duas categorias com o mesmo nome.
    #[test]
    fn envelope_reaproveita_por_nome_e_reescreve_o_category_id_da_tarefa() {
        let (_, mut destino) = nova_loja("cat-envelope-nome-db");
        let existente = destino.add_category("trabalho").unwrap();

        let dir = dir("cat-envelope-nome");
        let path = escrever(
            &dir,
            "envelope.json",
            r#"{
                "schema": 3,
                "categories": [{"id":"do-ficheiro","name":"Trabalho"}],
                "todos": [{"id":"a1","title":"t","created_at":"2026-09-18T10:00:00+01:00","category_id":"do-ficheiro"}],
                "trash": []
            }"#,
        );
        let relatorio = import_json_from_path(&mut destino, &path).unwrap();

        assert_eq!(relatorio.inseridos, 1);
        assert_eq!(relatorio.categorias_criadas, 0, "o nome já existia");
        assert_eq!(relatorio.categorias_reaproveitadas, 1);
        assert_eq!(
            relatorio.sem_categoria, 0,
            "a referência resolveu: noutro id"
        );
        assert_eq!(destino.db().categories.len(), 1);
        assert_eq!(
            destino
                .find(&TodoId::from("a1".to_owned()))
                .unwrap()
                .category_id
                .as_ref(),
            Some(&existente),
            "a tarefa aponta à categoria que cá está"
        );
    }

    /// Um envelope com um `category_id` que não resolve — nem numa categoria
    /// do ficheiro, nem numa da base: a tarefa entra sem categoria, contada.
    #[test]
    fn envelope_com_category_id_que_nao_resolve_entra_sem_categoria() {
        let dir = dir("cat-envelope-orfao");
        let path = escrever(
            &dir,
            "orfao.json",
            r#"{"schema":3,"todos":[{"id":"a1","title":"t","created_at":"2026-09-18T10:00:00+01:00","category_id":"nao-existe"}],"trash":[],"categories":[]}"#,
        );

        let (_, mut store) = nova_loja("cat-envelope-orfao-db");
        let relatorio = import_json_from_path(&mut store, &path).unwrap();

        assert_eq!(relatorio.inseridos, 1);
        assert_eq!(relatorio.sem_categoria, 1);
        assert!(store.todos()[0].category_id.is_none());
        assert!(relatorio.resumo().contains("1 sem categoria"));
    }

    /// Um import que falhe a meio não deixa tarefas **nem categorias** meio
    /// importadas: a `Db` em memória fica como estava (critério 7).
    #[test]
    fn import_que_falha_a_meio_nao_cria_categorias() {
        let dir = dir("cat-meio");
        let path = escrever(
            &dir,
            "meio.csv",
            "id,title,description,done,priority,created_at,completed_at,due_at,category\n\
             a1,boa,,false,medium,2026-09-18T10:00:00+01:00,,,Nova\n\
             a2,má,,false,urgente,2026-09-18T10:00:00+01:00,,,Nova\n",
        );

        let (_, mut store) = nova_loja("cat-meio-db");
        let erro = import_csv_from_path(&mut store, &path).expect_err("tinha de falhar");
        match &erro {
            ImportError::Row { linha, campo, .. } => {
                assert_eq!(*linha, 3, "linha 1 é o cabeçalho");
                assert_eq!(*campo, "priority");
            }
            outro => panic!("erro inesperado: {outro:?}"),
        }
        assert!(store.todos().is_empty(), "nada entrou a meio");
        assert!(
            store.categories().is_empty(),
            "nenhuma categoria ficou meio criada"
        );
    }

    /// A linha que o utilizador lê no fim de um import real: uma categoria
    /// criada, uma reaproveitada (a base já tinha `trabalho`), o resto das
    /// tarefas sem categoria. A frase tem de ser esta.
    #[test]
    fn resumo_do_import_com_uma_criada_e_uma_reaproveitada() {
        let (_, mut store) = nova_loja("cat-resumo");
        store.add_category("trabalho").unwrap();

        let dir = dir("cat-resumo-origem");
        let path = escrever(
            &dir,
            "cat.csv",
            "id,title,description,done,priority,created_at,completed_at,due_at,category\n\
             a1,casa,,false,medium,2026-09-18T10:00:00+01:00,,,Casa\n\
             a2,emprego,,false,medium,2026-09-18T10:00:00+01:00,,,Trabalho\n\
             a3,avulsa,,false,medium,2026-09-18T10:00:00+01:00,,,\n",
        );

        let relatorio = import_csv_from_path(&mut store, &path).unwrap();
        assert_eq!(
            relatorio.resumo(),
            "3 lidos, 3 inseridos, 0 duplicados ignorados, 1 categorias criadas, \
             1 categorias reaproveitadas"
        );
        assert_eq!(
            store.categories().len(),
            2,
            "a criada entra, a outra não se repete"
        );
    }

    /// Uma tarefa com um `category_id` que não resolve não faz o export rebentar
    /// nem inventar um nome: sai com o campo vazio.
    #[test]
    fn export_de_referencia_pendente_sai_com_o_campo_vazio() {
        let (_path, _store) = nova_loja("cat-export-pendente");
        let mut db = Db::empty();
        db.categories.push(Category {
            id: CategoryId::new(),
            name: "Trabalho".to_owned(),
        });
        db.todos.push(
            Todo::try_new("pendente")
                .unwrap()
                .with_category(Some(CategoryId::new())),
        );

        let dir = dir("cat-export-pendente-destino");
        let path = dir.join("saida.csv");
        export_csv_to_path(&db, &path).expect("o export não pode rebentar");

        let conteudo = fs::read_to_string(&path).unwrap();
        let linha = conteudo.lines().nth(1).unwrap();
        assert_eq!(linha.split(',').next_back().unwrap(), "");
    }
}
