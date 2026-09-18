//! Operações sobre a base de dados.
//!
//! Duas regras estruturais, ambas vindas do ADR:
//!
//! 1. **Tudo é referenciado por `id`.** O índice muda a cada reordenação e a
//!    cada remoção; foi o defeito central do `rtodo` antigo (operações por
//!    índice), e é por isso que cada método aqui recebe [`TodoId`].
//! 2. **O lixo substitui o diálogo de confirmação**, logo é dado e não RAM:
//!    `remove`/`clear_completed` guardam em [`Db::trash`] o suficiente para
//!    repor a tarefa na posição original, e a gravação é imediata. Um
//!    `Ctrl-C` a seguir a «limpar concluídas» não perde nada.

use std::fmt;

use chrono::Local;

use super::model::{Priority, Todo, TodoError, TodoId};
use super::store::{Db, Store, StoreError, Trashed};

/// Limite de entradas no lixo.
///
/// Ao transbordar saem as mais antigas, a contagem acumulada sobe em
/// [`Db::trash_dropped`] e é devolvida a quem chamou — nada sai em silêncio.
pub const TRASH_LIMIT: usize = 100;

/// Erros das operações.
#[derive(Debug)]
pub enum OpsError {
    /// O `id` não existe (a UI estava desactualizada, ou a tarefa já foi
    /// removida noutro sítio).
    NotFound(TodoId),
    EmptyTitle,
    /// Não há nada no lixo para restaurar.
    NothingToRestore,
    Persist(StoreError),
}

impl fmt::Display for OpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "não existe nenhuma tarefa com o id {id}"),
            Self::EmptyTitle => TodoError::EmptyTitle.fmt(f),
            Self::NothingToRestore => f.write_str("não há nada no lixo para restaurar"),
            Self::Persist(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for OpsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persist(err) => Some(err),
            _ => None,
        }
    }
}

impl From<StoreError> for OpsError {
    fn from(err: StoreError) -> Self {
        Self::Persist(err)
    }
}

/// O que aconteceu ao limpar as concluídas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClearOutcome {
    /// Tarefas que foram para o lixo.
    pub removidos: usize,
    /// Entradas antigas que saíram do lixo por o limite ter sido atingido.
    pub saidas_do_lixo: usize,
}

/// Aplica o limite do lixo e devolve quantas entradas saíram.
///
/// Sai sempre do princípio da lista (a mais antiga, porque a lista está em
/// ordem de remoção).
pub(crate) fn trim_trash(db: &mut Db) -> usize {
    if db.trash.len() <= TRASH_LIMIT {
        return 0;
    }
    let saidas = db.trash.len() - TRASH_LIMIT;
    db.trash.drain(..saidas);
    db.trash_dropped = db.trash_dropped.saturating_add(saidas as u64);
    saidas
}

impl Store {
    /// Posição da tarefa, se existir. Só para apresentação — as operações
    /// usam-na internamente, mas nunca é o que se guarda.
    #[must_use]
    pub fn position(&self, id: &TodoId) -> Option<usize> {
        self.todos().iter().position(|todo| &todo.id == id)
    }

    #[must_use]
    pub fn find(&self, id: &TodoId) -> Option<&Todo> {
        self.todos().iter().find(|todo| &todo.id == id)
    }

    pub fn find_mut(&mut self, id: &TodoId) -> Option<&mut Todo> {
        self.db_mut().todos.iter_mut().find(|todo| &todo.id == id)
    }

    /// Identificador do próximo lote de remoção.
    ///
    /// Derivado do lixo existente (máximo + 1) em vez de um contador
    /// persistido: um contador no schema podia colidir com lotes antigos
    /// depois de o lixo ser esvaziado, e um lote só interessa enquanto as suas
    /// entradas existirem.
    fn next_batch(&self) -> u64 {
        self.db()
            .trash
            .iter()
            .map(|entrada| entrada.batch)
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Cria uma tarefa a partir do título, valida e grava.
    pub fn add(&mut self, title: &str) -> Result<TodoId, OpsError> {
        let todo = Todo::try_new(title).map_err(|TodoError::EmptyTitle| OpsError::EmptyTitle)?;
        self.insert(todo)
    }

    /// Insere uma tarefa já construída (import, testes) e grava.
    pub fn insert(&mut self, todo: Todo) -> Result<TodoId, OpsError> {
        let id = todo.id.clone();
        self.db_mut().todos.push(todo);
        self.save()?;
        Ok(id)
    }

    /// Grava mesmo quando o `id` não existe? Não: erro explícito.
    fn mutate<F>(&mut self, id: &TodoId, edit: F) -> Result<(), OpsError>
    where
        F: FnOnce(&mut Todo),
    {
        let todo = self
            .find_mut(id)
            .ok_or_else(|| OpsError::NotFound(id.clone()))?;
        edit(todo);
        self.save()?;
        Ok(())
    }

    pub fn edit_title(&mut self, id: &TodoId, title: &str) -> Result<(), OpsError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(OpsError::EmptyTitle);
        }
        let title = title.to_owned();
        self.mutate(id, |todo| todo.title = title)
    }

    pub fn edit_description(&mut self, id: &TodoId, description: &str) -> Result<(), OpsError> {
        let description = description.to_owned();
        self.mutate(id, |todo| todo.description = description)
    }

    pub fn set_priority(&mut self, id: &TodoId, priority: Priority) -> Result<(), OpsError> {
        self.mutate(id, |todo| todo.priority = priority)
    }

    /// Ciclo baixa → média → alta → baixa. É o que a tecla de prioridade da
    /// TUI faz, e vive no core para ser testável sem terminal.
    pub fn cycle_priority(&mut self, id: &TodoId) -> Result<Priority, OpsError> {
        let actual = self
            .find(id)
            .ok_or_else(|| OpsError::NotFound(id.clone()))?
            .priority;
        let proxima = match actual {
            Priority::Low => Priority::Medium,
            Priority::Medium => Priority::High,
            Priority::High => Priority::Low,
        };
        self.mutate(id, |todo| todo.priority = proxima)?;
        Ok(proxima)
    }

    pub fn set_done(&mut self, id: &TodoId, done: bool) -> Result<bool, OpsError> {
        self.mutate(id, |todo| todo.set_done(done))?;
        Ok(done)
    }

    pub fn toggle_done(&mut self, id: &TodoId) -> Result<bool, OpsError> {
        let actual = self
            .find(id)
            .ok_or_else(|| OpsError::NotFound(id.clone()))?
            .is_done();
        self.set_done(id, !actual)
    }

    /// Remove para o lixo e devolve quantas entradas antigas saíram do lixo
    /// por causa do limite (0 no caso normal).
    pub fn remove(&mut self, id: &TodoId) -> Result<usize, OpsError> {
        let index = self
            .position(id)
            .ok_or_else(|| OpsError::NotFound(id.clone()))?;
        let batch = self.next_batch();
        let db = self.db_mut();
        let todo = db.todos.remove(index);
        db.trash.push(Trashed {
            todo,
            index,
            deleted_at: Local::now(),
            batch,
        });
        let saidas = trim_trash(db);
        self.save()?;
        Ok(saidas)
    }

    /// Remove todas as concluídas num só lote: `u` restaura o lote inteiro.
    pub fn clear_completed(&mut self) -> Result<ClearOutcome, OpsError> {
        let batch = self.next_batch();
        let db = self.db_mut();
        let mut removidos = Vec::new();
        // De trás para a frente: assim cada índice recolhido é o índice
        // original, que é o que o undo precisa para repor a posição.
        for index in (0..db.todos.len()).rev() {
            if db.todos[index].is_done() {
                removidos.push((index, db.todos.remove(index)));
            }
        }
        if removidos.is_empty() {
            return Ok(ClearOutcome {
                removidos: 0,
                saidas_do_lixo: 0,
            });
        }
        for (index, todo) in removidos {
            db.trash.push(Trashed {
                todo,
                index,
                deleted_at: Local::now(),
                batch,
            });
        }
        let saidas_do_lixo = trim_trash(db);
        let removidos = db
            .trash
            .iter()
            .filter(|entrada| entrada.batch == batch)
            .count();
        self.save()?;
        Ok(ClearOutcome {
            removidos,
            saidas_do_lixo,
        })
    }

    /// Número de entradas no lixo.
    #[must_use]
    pub fn trash_len(&self) -> usize {
        self.db().trash.len()
    }

    /// Há algo para restaurar?
    #[must_use]
    pub fn can_restore(&self) -> bool {
        !self.db().trash.is_empty()
    }

    /// Restaura o último lote (uma remoção simples ou um «limpar concluídas»
    /// inteiro) nas posições originais e devolve quantas tarefas voltaram.
    ///
    /// Repor a posição: as entradas são reinseridas por ordem crescente de
    /// índice, com o índice limitado ao tamanho actual da lista — se a lista
    /// entretanto encolheu, a tarefa volta o mais perto possível do sítio
    /// onde estava, e a ordem relativa do lote é sempre preservada.
    pub fn restore_last_batch(&mut self) -> Result<usize, OpsError> {
        let Some(ultimo) = self.db().trash.iter().map(|entrada| entrada.batch).max() else {
            return Err(OpsError::NothingToRestore);
        };

        let db = self.db_mut();
        let mut lote: Vec<Trashed> = db
            .trash
            .iter()
            .filter(|entrada| entrada.batch == ultimo)
            .cloned()
            .collect();
        db.trash.retain(|entrada| entrada.batch != ultimo);
        lote.sort_by_key(|entrada| entrada.index);

        let restaurados = lote.len();
        for entrada in lote {
            let posicao = entrada.index.min(db.todos.len());
            db.todos.insert(posicao, entrada.todo);
        }
        self.save()?;
        Ok(restaurados)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Priority;
    use std::fs;
    use std::path::PathBuf;

    fn store(tag: &str) -> (PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!(
            "todo-ratatui-ops-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("criar diretório");
        let path = dir.join("db.json");
        let store = Store::open(&path).expect("abrir store");
        (path, store)
    }

    fn titulos(store: &Store) -> Vec<String> {
        store
            .todos()
            .iter()
            .map(|todo| todo.title.clone())
            .collect()
    }

    #[test]
    fn add_valida_e_persiste() {
        let (path, mut store) = store("add");
        let id = store.add("  comprar café  ").expect("adicionar");
        assert_eq!(
            store.add("   ").unwrap_err().to_string(),
            "o título não pode estar vazio"
        );
        assert_eq!(store.find(&id).unwrap().title, "comprar café");

        let recarregado = Store::open(&path).expect("reabrir");
        assert_eq!(recarregado.todos().len(), 1, "gravou imediatamente");
        assert_eq!(recarregado.todos()[0].id, id);
    }

    #[test]
    fn operacoes_por_id_ignoram_a_posicao() {
        let (_path, mut store) = store("id");
        let a = store.add("a").unwrap();
        let b = store.add("b").unwrap();
        let c = store.add("c").unwrap();

        store.edit_title(&a, "a editada").unwrap();
        store.toggle_done(&c).unwrap();
        store.remove(&b).unwrap();

        assert_eq!(titulos(&store), ["a editada", "c"]);
        assert!(store.find(&b).is_none());
        // O id continua válido mesmo com a lista reordenada/encolhida.
        store.edit_description(&c, "nota").unwrap();
        assert_eq!(store.find(&c).unwrap().description, "nota");
    }

    #[test]
    fn erro_explicito_quando_o_id_nao_existe() {
        let (_path, mut store) = store("not-found");
        let fantasma = TodoId::new();
        for resultado in [
            store.edit_title(&fantasma, "x").map(|_| ()),
            store.toggle_done(&fantasma).map(|_| ()),
            store.set_priority(&fantasma, Priority::High),
            store.remove(&fantasma).map(|_| ()),
        ] {
            assert!(matches!(resultado, Err(OpsError::NotFound(_))));
        }
    }

    #[test]
    fn prioridade_cicla_e_grava() {
        let (path, mut store) = store("prioridade");
        let id = store.add("ciclo").unwrap();
        assert_eq!(store.find(&id).unwrap().priority, Priority::Medium);
        assert_eq!(store.cycle_priority(&id).unwrap(), Priority::High);
        assert_eq!(store.cycle_priority(&id).unwrap(), Priority::Low);
        assert_eq!(store.cycle_priority(&id).unwrap(), Priority::Medium);

        let recarregado = Store::open(&path).unwrap();
        assert_eq!(recarregado.todos()[0].priority, Priority::Medium);
    }

    #[test]
    fn remove_guarda_indice_e_lote_no_lixo() {
        let (_path, mut store) = store("remove");
        store.add("primeira").unwrap();
        let segunda = store.add("segunda").unwrap();
        store.add("terceira").unwrap();

        assert_eq!(store.remove(&segunda).unwrap(), 0, "lixo não transbordou");
        assert_eq!(store.trash_len(), 1);
        assert!(store.find(&segunda).is_none());

        let guardada = &store.db().trash[0];
        assert_eq!(guardada.index, 1);
        assert_eq!(guardada.todo.title, "segunda");
    }

    #[test]
    fn undo_restaura_na_posicao_original() {
        let (_path, mut store) = store("undo");
        store.add("a").unwrap();
        let b = store.add("b").unwrap();
        store.add("c").unwrap();
        let d = store.add("d").unwrap();

        store.remove(&b).unwrap();
        store.remove(&d).unwrap();
        assert_eq!(titulos(&store), ["a", "c"]);

        // Segundo `u`: desfaz a última remoção (d, índice 3).
        assert_eq!(store.restore_last_batch().unwrap(), 1);
        assert_eq!(titulos(&store), ["a", "c", "d"]);
        // Terceiro `u`: desfaz a remoção de b, que volta ao índice 1.
        assert_eq!(store.restore_last_batch().unwrap(), 1);
        assert_eq!(titulos(&store), ["a", "b", "c", "d"]);
        assert!(!store.can_restore());
        assert!(matches!(
            store.restore_last_batch(),
            Err(OpsError::NothingToRestore)
        ));
    }

    #[test]
    fn undo_sobrevive_a_reabertura_do_ficheiro() {
        let (path, mut store) = store("undo-persistido");
        store.add("a").unwrap();
        let b = store.add("b").unwrap();
        store.remove(&b).unwrap();
        drop(store);

        // Simula um Ctrl-C depois da remoção: processo novo, mesmo ficheiro.
        let mut reaberto = Store::open(&path).expect("reabrir");
        assert_eq!(reaberto.trash_len(), 1, "o lixo está no disco, não em RAM");
        assert_eq!(reaberto.restore_last_batch().unwrap(), 1);
        assert_eq!(titulos(&reaberto), ["a", "b"]);
    }

    #[test]
    fn clear_completed_e_um_lote_unico() {
        let (_path, mut store) = store("clear");
        store.add("feita 1").unwrap();
        let segunda = store.add("por fazer").unwrap();
        store.add("feita 2").unwrap();
        store.add("feita 3").unwrap();
        for id in store
            .todos()
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>()
        {
            if id != segunda {
                store.toggle_done(&id).unwrap();
            }
        }

        let resultado = store.clear_completed().unwrap();
        assert_eq!(resultado.removidos, 3);
        assert_eq!(resultado.saidas_do_lixo, 0);
        assert_eq!(titulos(&store), ["por fazer"]);

        // Um único `u` devolve as três, cada uma na posição original.
        assert_eq!(store.restore_last_batch().unwrap(), 3);
        assert_eq!(
            titulos(&store),
            ["feita 1", "por fazer", "feita 2", "feita 3"]
        );
    }

    #[test]
    fn clear_completed_sem_concluidas_nao_cria_lote() {
        let (_path, mut store) = store("clear-vazio");
        store.add("pendente").unwrap();
        let resultado = store.clear_completed().unwrap();
        assert_eq!(resultado.removidos, 0);
        assert!(!store.can_restore(), "não se cria lote vazio");
    }

    #[test]
    fn lixo_respeita_o_limite_e_conta_o_que_saiu() {
        let (path, mut store) = store("limite");
        let ids: Vec<TodoId> = (0..TRASH_LIMIT + 5)
            .map(|i| store.add(&format!("tarrefa {i}")).unwrap())
            .collect();

        // Cada remoção devolve o que *aquela* remoção fez sair do lixo; a
        // soma é que tem de dar 5.
        let saidas: Vec<usize> = ids.iter().map(|id| store.remove(id).unwrap()).collect();
        assert_eq!(
            saidas[..TRASH_LIMIT].iter().sum::<usize>(),
            0,
            "enquanto o lixo não enche, nada sai"
        );
        assert!(
            saidas[TRASH_LIMIT..].iter().all(|&n| n == 1),
            "depois do limite sai exactamente uma por remoção: {saidas:?}"
        );
        assert_eq!(
            saidas.iter().sum::<usize>(),
            5,
            "a contagem é devolvida a quem removeu"
        );

        let recarregado = Store::open(&path).unwrap();
        assert_eq!(recarregado.db().trash_dropped, 5, "e fica gravada");
        assert_eq!(recarregado.db().trash[0].todo.title, "tarrefa 5");
        assert_eq!(
            recarregado.db().trash.last().unwrap().todo.title,
            format!("tarrefa {}", TRASH_LIMIT + 4)
        );
    }

    #[test]
    fn edit_title_vazio_nao_altera_nada() {
        let (path, mut store) = store("titulo-vazio");
        let id = store.add("original").unwrap();
        assert!(matches!(
            store.edit_title(&id, "   "),
            Err(OpsError::EmptyTitle)
        ));
        assert_eq!(store.find(&id).unwrap().title, "original");
        let em_disco = fs::read_to_string(path).unwrap();
        assert!(em_disco.contains("original"));
    }
}
