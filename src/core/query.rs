//! Consultas de leitura sobre a base de dados.
//!
//! Sem terminal e sem ordenação: a **ordem** é decisão de apresentação (o
//! @designer fecha-a no T5/T6); aqui só se decide *o que* entra na vista.
//! A ordem natural é a ordem de inserção, que o undo preserva.

use super::model::Todo;
use super::store::Store;
use super::store::Trashed;

/// Que tarefas mostrar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    /// Por fazer.
    Active,
    Done,
    /// Vista do lixo, com as mais recentes primeiro (a última remoção é a
    /// primeira a ser mostrada, que é a que o `u` restaura).
    Trash,
}

impl Filter {
    pub const ALL: [Self; 4] = [Self::All, Self::Active, Self::Done, Self::Trash];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "todas",
            Self::Active => "por fazer",
            Self::Done => "concluídas",
            Self::Trash => "lixo",
        }
    }

    /// Rótulo da barra de estado, com a contagem.
    #[must_use]
    pub fn title(self, counts: Counts) -> String {
        match self {
            Self::All => format!("{} todas", counts.total()),
            Self::Active => format!("{} por fazer", counts.active),
            Self::Done => format!("{} concluídas", counts.done),
            Self::Trash => format!("{} no lixo", counts.trash),
        }
    }
}

/// Contagens para a barra de estado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub active: usize,
    pub done: usize,
    pub trash: usize,
}

impl Counts {
    #[must_use]
    pub const fn total(&self) -> usize {
        self.active + self.done
    }
}

impl Store {
    #[must_use]
    pub fn counts(&self) -> Counts {
        let (mut active, mut done) = (0, 0);
        for todo in self.todos() {
            if todo.is_done() {
                done += 1;
            } else {
                active += 1;
            }
        }
        Counts {
            active,
            done,
            trash: self.trash_len(),
        }
    }

    /// Tarefas da vista, por ordem natural (de inserção).
    #[must_use]
    pub fn filtered(&self, filter: Filter) -> Vec<&Todo> {
        match filter {
            Filter::All => self.todos().iter().collect(),
            Filter::Active => self.todos().iter().filter(|t| !t.is_done()).collect(),
            Filter::Done => self.todos().iter().filter(|t| t.is_done()).collect(),
            Filter::Trash => self
                .db()
                .trash
                .iter()
                .rev()
                .map(|entrada| &entrada.todo)
                .collect(),
        }
    }

    /// Entradas do lixo, da mais recente para a mais antiga — a ordem em que a
    /// vista do lixo as mostra e o `u` as desfaz.
    #[must_use]
    pub fn trash_entries(&self) -> Vec<&Trashed> {
        self.db().trash.iter().rev().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn store(tag: &str) -> (PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!(
            "todo-ratatui-query-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("criar diretório");
        let path = dir.join("db.json");
        let store = Store::open(&path).expect("abrir");
        (path, store)
    }

    #[test]
    fn filtros_veem_o_que_devem() {
        let (_path, mut store) = store("filtros");
        let a = store.add("activa").unwrap();
        let b = store.add("feita").unwrap();
        let c = store.add("removida").unwrap();
        store.toggle_done(&b).unwrap();
        store.remove(&c).unwrap();

        let titulos = |filter| -> Vec<String> {
            store
                .filtered(filter)
                .iter()
                .map(|t| t.title.clone())
                .collect()
        };
        assert_eq!(titulos(Filter::All), ["activa", "feita"]);
        assert_eq!(titulos(Filter::Active), ["activa"]);
        assert_eq!(titulos(Filter::Done), ["feita"]);
        assert_eq!(titulos(Filter::Trash), ["removida"]);
        assert!(store.find(&a).is_some());
    }

    #[test]
    fn contagens_e_rotulos_da_barra_de_estado() {
        let (_path, mut store) = store("contagens");
        let a = store.add("uma").unwrap();
        store.add("duas").unwrap();
        store.toggle_done(&a).unwrap();

        let counts = store.counts();
        assert_eq!(counts.active, 1);
        assert_eq!(counts.done, 1);
        assert_eq!(counts.trash, 0);
        assert_eq!(counts.total(), 2);
        assert_eq!(Filter::All.title(counts), "2 todas");
        assert_eq!(Filter::Active.title(counts), "1 por fazer");
        assert_eq!(Filter::Done.title(counts), "1 concluídas");
        assert_eq!(Filter::Trash.title(counts), "0 no lixo");
        assert_eq!(Filter::ALL.len(), 4);
    }

    #[test]
    fn lixo_mostra_a_remocao_mais_recente_primeiro() {
        let (_path, mut store) = store("ordem-lixo");
        let a = store.add("antiga").unwrap();
        let b = store.add("recente").unwrap();
        store.remove(&a).unwrap();
        store.remove(&b).unwrap();

        let titulos: Vec<&str> = store
            .trash_entries()
            .iter()
            .map(|e| e.todo.title.as_str())
            .collect();
        assert_eq!(titulos, ["recente", "antiga"]);
    }
}
