//! Modelo de domínio. Sem I/O e sem terminal.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identificador estável de uma tarefa.
///
/// As operações referem-se sempre ao `id`, nunca ao índice na lista: o índice
/// muda a cada reordenação e a cada remoção (foi o defeito central do `rtodo`
/// antigo).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TodoId(String);

impl TodoId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TodoId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TodoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for TodoId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl FromStr for TodoId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_owned()))
    }
}

/// Identificador estável de uma categoria.
///
/// Mesmo molde do [`TodoId`], e pela mesma razão: as operações referem-se ao
/// `id` e o nome é atributo editável (ADR §Decisão 1). É o que torna o
/// *rename* seguro — nenhuma tarefa guarda o nome.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CategoryId(String);

impl CategoryId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for CategoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CategoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for CategoryId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl FromStr for CategoryId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_owned()))
    }
}

/// Uma categoria de tarefas.
///
/// Uma tarefa pertence a **no máximo** uma categoria (ADR §Decisão 1: pertença
/// múltipla seria outro formato). Sem `created_at` — nada o mostra — e sem cor,
/// ícone ou ordem manual: a ordem é a de inserção, como a das tarefas. A
/// unicidade e a limpeza do nome são regra das operações (`core::ops`), não do
/// modelo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
}

/// Prioridade da tarefa. A ordem de declaração é a ordem de ordenação, logo
/// `High > Medium > Low` (útil para ordenar por prioridade decrescente).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    #[default]
    Medium,
    High,
}

impl Priority {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Low => "baixa",
            Self::Medium => "média",
            Self::High => "alta",
        }
    }

    /// Nome como aparece no JSON e no CSV. Tem de coincidir com o
    /// `serde(rename_all = "lowercase")` do `enum` — há um teste que compara
    /// os dois, para o formato em disco não divergir da API.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Aceita `"High"`/`"high"`/`"alta"` (o `rtodo` antigo gravava `"High"`).
    #[must_use]
    pub fn from_legacy(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "low" | "baixa" => Some(Self::Low),
            "medium" | "média" | "media" => Some(Self::Medium),
            "high" | "alta" => Some(Self::High),
            _ => None,
        }
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Erros de validação do modelo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoError {
    EmptyTitle,
}

impl fmt::Display for TodoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTitle => f.write_str("o título não pode estar vazio"),
        }
    }
}

impl std::error::Error for TodoError {}

/// Uma tarefa.
///
/// `due_at` existe no schema mas a edição na UI ficou fora do âmbito da v1
/// (ADR): é preservada no round-trip, não é editável.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub id: TodoId,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub priority: Priority,
    pub created_at: DateTime<Local>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Local>>,
    #[serde(default)]
    pub due_at: Option<NaiveDate>,
    /// Categoria da tarefa, ou `None` para «sem categoria». `#[serde(default)]`
    /// é o que faz um ficheiro da v1.1.0 (sem o campo) ler-se como `None`, tal
    /// como o `2` do envelope (ADR §Decisão 3).
    #[serde(default)]
    pub category_id: Option<CategoryId>,
}

impl Todo {
    /// Cria uma tarefa com `id` novo e `created_at` agora.
    pub fn try_new(title: impl Into<String>) -> Result<Self, TodoError> {
        let title = title.into().trim().to_owned();
        if title.is_empty() {
            return Err(TodoError::EmptyTitle);
        }
        Ok(Self {
            id: TodoId::new(),
            title,
            description: String::new(),
            done: false,
            priority: Priority::default(),
            created_at: Local::now(),
            completed_at: None,
            due_at: None,
            category_id: None,
        })
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub fn with_due_at(mut self, due_at: Option<NaiveDate>) -> Self {
        self.due_at = due_at;
        self
    }

    /// Atribui (`Some`) ou tira (`None`) a categoria. Como o `with_priority`,
    /// não valida contra a `Db`: quem garante que a categoria existe é a
    /// operação que chama isto (`core::ops`), que é quem tem a `Db` à frente.
    #[must_use]
    pub fn with_category(mut self, category_id: Option<CategoryId>) -> Self {
        self.category_id = category_id;
        self
    }

    /// Marca/desmarca a conclusão mantendo `completed_at` coerente — é a única
    /// forma de mexer em `done`, para as duas datas não divergirem.
    pub fn set_done(&mut self, done: bool) {
        if done == self.done {
            return;
        }
        self.done = done;
        self.completed_at = if done { Some(Local::now()) } else { None };
    }

    pub fn toggle(&mut self) {
        self.set_done(!self.done);
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Data do registo, para a coluna de data da lista.
    #[must_use]
    pub fn created_date(&self) -> NaiveDate {
        self.created_at.date_naive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_coincide_com_o_serde() {
        for prioridade in Priority::ALL {
            assert_eq!(
                serde_json::to_value(prioridade).unwrap(),
                serde_json::json!(prioridade.as_str()),
                "o nome em disco tem de ser o mesmo do `as_str()`"
            );
        }
    }

    #[test]
    fn try_new_rejeita_titulo_vazio() {
        assert_eq!(Todo::try_new("   ").unwrap_err(), TodoError::EmptyTitle);
        assert_eq!(Todo::try_new("").unwrap_err(), TodoError::EmptyTitle);
        assert!(TodoError::EmptyTitle.to_string().contains("vazio"));
    }

    #[test]
    fn try_new_limpa_espacos_e_gera_id_unico() {
        let a = Todo::try_new("  café  ").unwrap();
        let b = Todo::try_new("café").unwrap();
        assert_eq!(a.title, "café");
        assert_ne!(a.id, b.id, "cada tarefa tem id próprio");
        assert!(!a.is_done());
        assert!(a.completed_at.is_none());
        assert!(a.due_at.is_none());
        assert_eq!(a.priority, Priority::Medium);
    }

    #[test]
    fn set_done_mantem_completed_at_coerente() {
        let mut todo = Todo::try_new("testar").unwrap();
        todo.set_done(true);
        let primeira = todo.completed_at.expect("concluída tem data");
        todo.set_done(true); // idempotente: não reescreve a data
        assert_eq!(todo.completed_at, Some(primeira));

        todo.set_done(false);
        assert!(todo.completed_at.is_none(), "desmarcar limpa a data");

        todo.toggle();
        assert!(todo.is_done());
        assert!(todo.completed_at.is_some());
    }

    #[test]
    fn prioridade_ordena_da_baixa_para_a_alta() {
        assert!(Priority::Low < Priority::Medium);
        assert!(Priority::Medium < Priority::High);
        let mut lista = [Priority::High, Priority::Low, Priority::Medium];
        lista.sort();
        assert_eq!(lista, [Priority::Low, Priority::Medium, Priority::High]);
        assert_eq!(Priority::ALL.len(), 3);
    }

    #[test]
    fn from_legacy_aceita_o_formato_do_rtodo() {
        assert_eq!(Priority::from_legacy("High"), Some(Priority::High));
        assert_eq!(Priority::from_legacy(" alta "), Some(Priority::High));
        assert_eq!(Priority::from_legacy("MEDIUM"), Some(Priority::Medium));
        assert_eq!(Priority::from_legacy("baixa"), Some(Priority::Low));
        assert_eq!(Priority::from_legacy("urgente"), None);
    }

    #[test]
    fn json_usa_rfc3339_para_instantes_e_iso_para_datas() {
        let mut todo = Todo::try_new("datas").unwrap();
        todo.due_at = NaiveDate::from_ymd_opt(2026, 9, 30);
        todo.priority = Priority::High;

        let json = serde_json::to_value(&todo).unwrap();
        let created = json["created_at"].as_str().expect("created_at é string");
        assert!(
            created.contains('T') && (created.contains('+') || created.ends_with('Z')),
            "created_at tem de ser RFC3339 com offset, veio «{created}»"
        );
        assert_eq!(json["due_at"], serde_json::json!("2026-09-30"));
        assert_eq!(json["priority"], serde_json::json!("high"));
        assert!(json["completed_at"].is_null());

        let recarregado: Todo = serde_json::from_value(json).unwrap();
        assert_eq!(recarregado, todo, "o round-trip preserva o instante exacto");
    }

    #[test]
    fn categoria_tem_id_proprio_e_round_trip_transparente() {
        let a = Category {
            id: CategoryId::new(),
            name: "Trabalho".to_owned(),
        };
        let b = Category {
            id: CategoryId::new(),
            name: "Trabalho".to_owned(),
        };
        assert_ne!(
            a.id, b.id,
            "duas categorias com o mesmo nome têm ids próprios"
        );

        // O `id` é transparente no JSON (uma string), como o do `Todo`.
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "id": a.id.as_str(), "name": "Trabalho" })
        );
        assert_eq!(serde_json::from_value::<Category>(json).unwrap(), a);
    }

    #[test]
    fn categoria_na_tarefa_e_opcional_e_ausente_le_se_como_none() {
        let mut todo = Todo::try_new("com categoria").unwrap();
        assert!(todo.category_id.is_none(), "nasce sem categoria");

        let id = CategoryId::new();
        todo = todo.with_category(Some(id.clone()));
        assert_eq!(todo.category_id, Some(id.clone()));

        let json = serde_json::to_value(&todo).unwrap();
        assert_eq!(json["category_id"], serde_json::json!(id.as_str()));

        // Um ficheiro da v1.1.0 não tem o campo: lê-se como `None`.
        let mut antigo = json.clone();
        antigo.as_object_mut().unwrap().remove("category_id");
        let recarregado: Todo = serde_json::from_value(antigo).unwrap();
        assert_eq!(recarregado.category_id, None);

        assert_eq!(todo.with_category(None).category_id, None, "None tira");
    }
}
