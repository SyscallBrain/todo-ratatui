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

use super::model::{Category, CategoryId, Priority, Todo, TodoError, TodoId};
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
    /// Categoria inexistente — o mesmo caso de [`Self::NotFound`], para o outro
    /// identificador. Não é uma variante `NotFound(String)`: o erro continua a
    /// dizer de que espécie é o `id` que falhou, e o `TodoId` do [`Self::NotFound`]
    /// não serve para uma categoria.
    CategoryNotFound(CategoryId),
    /// Nome de categoria vazio (depois do `trim`).
    EmptyName,
    /// Já existe uma categoria com este nome (comparação *case-insensitive*).
    /// Carrega o **nome já existente**, que é o que explica a colisão.
    DuplicateName(String),
    /// Não há nada no lixo para restaurar.
    NothingToRestore,
    Persist(StoreError),
}

impl fmt::Display for OpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "não existe nenhuma tarefa com o id {id}"),
            Self::EmptyTitle => TodoError::EmptyTitle.fmt(f),
            Self::CategoryNotFound(id) => {
                write!(f, "não existe nenhuma categoria com o id {id}")
            }
            Self::EmptyName => f.write_str("o nome da categoria não pode estar vazio"),
            Self::DuplicateName(name) => {
                write!(f, "já existe uma categoria com o nome «{name}»")
            }
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

    /// Esvazia o lixo e grava; devolve quantas entradas saíram.
    ///
    /// Não toca em `todos` e **não** mexe em [`Db::trash_dropped`]: esse é o
    /// contador acumulado do transbordo (o que o limite de 100 deitou fora),
    /// não o que está no lixo. Lixo vazio é `Ok(0)` e não grava nada.
    pub fn empty_trash(&mut self) -> Result<usize, OpsError> {
        let entradas = self.db().trash.len();
        if entradas == 0 {
            return Ok(0);
        }
        self.db_mut().trash.clear();
        self.save()?;
        Ok(entradas)
    }

    /// Repõe a tarefa indicada na posição original e devolve onde ficou
    /// (índice na lista).
    ///
    /// É o `Enter` da vista do lixo: actua sobre **uma** entrada, com o mesmo
    /// critério de reposição do [`Store::restore_last_batch`] (índice limitado
    /// ao tamanho actual, ordem relativa preservada). Repor uma tarefa de um
    /// lote de várias não apaga o resto do lote — a pilha do `u` continua a ser
    /// por lote. `id` que não está no lixo é erro, nunca panic.
    pub fn restore(&mut self, id: &TodoId) -> Result<usize, OpsError> {
        let posicao_no_lixo = self
            .db()
            .trash
            .iter()
            .position(|entrada| &entrada.todo.id == id)
            .ok_or_else(|| OpsError::NotFound(id.clone()))?;

        let db = self.db_mut();
        let entrada = db.trash.remove(posicao_no_lixo);
        let posicao = entrada.index.min(db.todos.len());
        db.todos.insert(posicao, entrada.todo);
        self.save()?;
        Ok(posicao)
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

    /// A categoria com este `id`, para escrita.
    ///
    /// Espelha o [`Store::find_mut`] das tarefas: as operações de categoria
    /// referem-se ao `id`, nunca ao índice na lista (que muda com uma
    /// eliminação).
    fn categoria_mut(&mut self, id: &CategoryId) -> Option<&mut Category> {
        self.db_mut()
            .categories
            .iter_mut()
            .find(|categoria| &categoria.id == id)
    }

    /// Valida o nome de uma categoria e devolve-o já limpo (`trim`).
    ///
    /// `excepto` é a categoria que pode ficar com o nome — a que está a ser
    /// renomeada. Sem esse parâmetro, corrigir a capitalização de «casa» para
    /// «Casa» seria recusado por «o nome já existir», quando é o mesmo objecto.
    fn validar_nome(&self, name: &str, excepto: Option<&CategoryId>) -> Result<String, OpsError> {
        let nome = name.trim();
        if nome.is_empty() {
            return Err(OpsError::EmptyName);
        }
        // A regra é a da busca: minúsculas dos dois lados (`query::contem`).
        let alvo = nome.to_lowercase();
        let repetida = self.categories().iter().find(|categoria| {
            categoria.name.to_lowercase() == alvo && excepto != Some(&categoria.id)
        });
        match repetida {
            Some(categoria) => Err(OpsError::DuplicateName(categoria.name.clone())),
            None => Ok(nome.to_owned()),
        }
    }

    /// Cria uma categoria com este nome e grava; devolve o `id` novo.
    ///
    /// `&mut self` (e não `&self`) porque a criação grava, como o
    /// [`Store::add`]: uma categoria em RAM que não chegasse ao ficheiro
    /// desapareceria no arranque seguinte, deixando tarefas com referências
    /// pendentes.
    pub fn add_category(&mut self, name: &str) -> Result<CategoryId, OpsError> {
        let nome = self.validar_nome(name, None)?;
        let categoria = Category {
            id: CategoryId::new(),
            name: nome,
        };
        let id = categoria.id.clone();
        self.db_mut().categories.push(categoria);
        self.save()?;
        Ok(id)
    }

    /// Renomeia uma categoria e grava.
    ///
    /// O `id` **não** muda e não há nada a cascatear: nenhuma tarefa guarda o
    /// nome da categoria (ADR §Decisão 1 e 2) — é exactamente isso que torna o
    /// *rename* barato.
    pub fn rename_category(&mut self, id: &CategoryId, name: &str) -> Result<(), OpsError> {
        let nome = self.validar_nome(name, Some(id))?;
        let categoria = self
            .categoria_mut(id)
            .ok_or_else(|| OpsError::CategoryNotFound(id.clone()))?;
        categoria.name = nome;
        self.save()?;
        Ok(())
    }

    /// Elimina a categoria e tira a atribuição de **todas** as tarefas que
    /// apontavam para ela, devolvendo o total afectado (lista + lixo).
    ///
    /// As tarefas ficam — sem categoria: elimina-se um metadado, não trabalho
    /// (ADR §Decisão 5). O lixo conta para o total e é limpo como a lista:
    /// uma entrada do lixo que ficasse a apontar para a categoria eliminada
    /// voltaria à lista, pelo `u`, com uma referência pendente.
    pub fn delete_category(&mut self, id: &CategoryId) -> Result<usize, OpsError> {
        let posicao = self
            .db()
            .categories
            .iter()
            .position(|categoria| &categoria.id == id)
            .ok_or_else(|| OpsError::CategoryNotFound(id.clone()))?;

        let db = self.db_mut();
        db.categories.remove(posicao);
        // Uma só passagem pelas **duas** listas: é aqui que é fácil limpar a
        // lista e esquecer o lixo.
        let mut afectadas = 0;
        for todo in db
            .todos
            .iter_mut()
            .chain(db.trash.iter_mut().map(|entrada| &mut entrada.todo))
        {
            if todo.category_id.as_ref() == Some(id) {
                todo.category_id = None;
                afectadas += 1;
            }
        }
        self.save()?;
        Ok(afectadas)
    }

    /// Atribui a categoria à tarefa — ou tira-lhe a atribuição, com `None` — e
    /// grava.
    ///
    /// Valida **antes** de tocar na `Db`: uma categoria inexistente (ou uma
    /// tarefa inexistente) é erro e a `Db` fica exactamente como estava, sem
    /// meia atribuição.
    pub fn assign_category(
        &mut self,
        todo: &TodoId,
        categoria: Option<&CategoryId>,
    ) -> Result<(), OpsError> {
        // Uma categoria que não existe: erro e `Db` intacta. O `if let` com
        // `&&` é a forma que o clippy pede (edition 2024 tem *let chains*).
        if let Some(id) = categoria
            && self.category(id).is_none()
        {
            return Err(OpsError::CategoryNotFound(id.clone()));
        }
        let alvo = categoria.cloned();
        self.mutate(todo, |todo| todo.category_id = alvo)
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

    #[test]
    fn empty_trash_esvazia_o_lixo_e_devolve_quantas_sairam() {
        let (path, mut store) = store("esvaziar");
        assert_eq!(store.empty_trash().unwrap(), 0, "lixo vazio é `Ok(0)`");
        assert_eq!(store.empty_trash().unwrap(), 0, "e não é erro repetir");

        let a = store.add("a").unwrap();
        let b = store.add("b").unwrap();
        store.remove(&a).unwrap();
        store.remove(&b).unwrap();
        assert_eq!(store.trash_len(), 2);

        assert_eq!(store.empty_trash().unwrap(), 2);
        assert_eq!(store.trash_len(), 0);
        assert!(
            titulos(&store).is_empty(),
            "esvaziar o lixo não toca em `todos`"
        );
        assert!(!store.can_restore());

        let recarregado = Store::open(&path).unwrap();
        assert_eq!(recarregado.trash_len(), 0, "esvaziar ficou gravado");
    }

    #[test]
    fn empty_trash_nao_repoe_a_contagem_do_transbordo() {
        let (_path, mut store) = store("esvaziar-contagem");
        let id = store.add("a").unwrap();
        store.remove(&id).unwrap();
        store.db_mut().trash_dropped = 7;

        assert_eq!(store.empty_trash().unwrap(), 1);
        assert_eq!(
            store.db().trash_dropped,
            7,
            "`trash_dropped` é o acumulado do limite, não o que está no lixo"
        );
    }

    #[test]
    fn restore_repoe_uma_tarefa_na_posicao_original_e_sobrevive_ao_reload() {
        let (path, mut store) = store("restore-id");
        store.add("a").unwrap();
        let b = store.add("b").unwrap();
        store.add("c").unwrap();
        let d = store.add("d").unwrap();
        // A ordem das remoções importa: o índice guardado é o da posição na
        // lista no momento da remoção.
        store.remove(&d).unwrap(); // índice 3
        store.remove(&b).unwrap(); // índice 1
        assert_eq!(titulos(&store), ["a", "c"]);

        // Repõe a que se indica, não a última: «b» volta ao índice 1.
        assert_eq!(store.restore(&b).unwrap(), 1);
        assert_eq!(titulos(&store), ["a", "b", "c"]);
        assert_eq!(store.trash_len(), 1, "«d» continua no lixo");
        drop(store);

        // Sobrevive a um Ctrl-C: processo novo, mesmo ficheiro.
        let mut reaberto = Store::open(&path).unwrap();
        assert_eq!(titulos(&reaberto), ["a", "b", "c"]);
        assert_eq!(reaberto.trash_len(), 1);
        assert_eq!(reaberto.restore(&d).unwrap(), 3);
        assert_eq!(titulos(&reaberto), ["a", "b", "c", "d"]);
        assert!(!reaberto.can_restore());
    }

    #[test]
    fn restore_de_id_que_nao_esta_no_lixo_e_erro_sem_panic() {
        let (_path, mut store) = store("restore-erro");
        let na_lista = store.add("na lista").unwrap();

        // Existe na lista mas não no lixo: não é restauro, é erro.
        let erro = store.restore(&na_lista).expect_err("não está no lixo");
        assert!(matches!(erro, OpsError::NotFound(_)));
        assert!(erro.to_string().contains("não existe nenhuma tarefa"));

        let erro = store.restore(&TodoId::new()).expect_err("id inventado");
        assert!(matches!(erro, OpsError::NotFound(_)));

        let id = store.add("removida").unwrap();
        store.remove(&id).unwrap();
        store.restore(&id).unwrap();
        assert!(
            matches!(store.restore(&id), Err(OpsError::NotFound(_))),
            "repor duas vezes o mesmo id é erro, não panic"
        );
        assert_eq!(titulos(&store), ["na lista", "removida"]);
    }

    #[test]
    fn restore_tira_so_a_entrada_indicada_do_lote() {
        let (_path, mut store) = store("restore-lote");
        let primeira = store.add("feita 1").unwrap();
        store.add("pendente").unwrap();
        let ultima = store.add("feita 2").unwrap();
        store.toggle_done(&primeira).unwrap();
        store.toggle_done(&ultima).unwrap();

        assert_eq!(store.clear_completed().unwrap().removidos, 2);
        assert_eq!(titulos(&store), ["pendente"]);

        // Uma do lote volta sozinha, na posição original (índice 0).
        assert_eq!(store.restore(&primeira).unwrap(), 0);
        assert_eq!(titulos(&store), ["feita 1", "pendente"]);
        assert_eq!(store.trash_len(), 1, "a outra do lote fica no lixo");

        // O `u` continua a repor o lote — aqui, o que dele resta.
        assert_eq!(store.restore_last_batch().unwrap(), 1);
        assert_eq!(titulos(&store), ["feita 1", "pendente", "feita 2"]);
    }

    /// Quantas referências à categoria `id` existem na `Db` **inteira** — a
    /// lista e o lixo. É a asserção que o invariante da eliminação pede: sobre
    /// a `Db` toda, não sobre uma amostra (a metade esquecida é sempre a do
    /// lixo).
    fn referencias_a(db: &Db, id: &CategoryId) -> usize {
        db.todos
            .iter()
            .chain(db.trash.iter().map(|entrada| &entrada.todo))
            .filter(|todo| todo.category_id.as_ref() == Some(id))
            .count()
    }

    #[test]
    fn categoria_valida_o_nome_e_persiste() {
        let (path, mut store) = store("categoria-add");
        let trabalho = store.add_category("  Trabalho  ").expect("criar");
        assert_eq!(
            store.category(&trabalho).unwrap().name,
            "Trabalho",
            "o nome é limpo (`trim`)"
        );

        let erro = store.add_category("   ").expect_err("nome vazio");
        assert!(matches!(erro, OpsError::EmptyName));
        assert_eq!(erro.to_string(), "o nome da categoria não pode estar vazio");
        assert!(matches!(store.add_category(""), Err(OpsError::EmptyName)));

        let erro = store.add_category("trabalho").expect_err("nome repetido");
        assert!(matches!(erro, OpsError::DuplicateName(_)));
        assert!(
            erro.to_string().contains("Trabalho"),
            "a mensagem diz o nome em causa: {erro}"
        );
        assert_eq!(
            store.categories().len(),
            1,
            "nenhum dos erros criou categoria"
        );
        assert_eq!(store.categories()[0].name, "Trabalho", "nem lhe tocou");

        let recarregado = Store::open(&path).expect("reabrir");
        assert_eq!(recarregado.categories().len(), 1, "gravou imediatamente");
        assert_eq!(recarregado.categories()[0].id, trabalho);
        assert_eq!(recarregado.categories()[0].name, "Trabalho");
    }

    #[test]
    fn rename_mantem_o_id_e_recusa_nome_repetido() {
        let (path, mut store) = store("categoria-rename");
        let trabalho = store.add_category("Trabalho").unwrap();
        let casa = store.add_category("Casa").unwrap();
        let tarefa = store.add("comprar café").unwrap();
        store.assign_category(&tarefa, Some(&trabalho)).unwrap();

        store
            .rename_category(&trabalho, "  Trabalho pessoal  ")
            .expect("renomear");
        assert_eq!(
            store.category(&trabalho).unwrap().name,
            "Trabalho pessoal",
            "o `id` não muda: a consulta pelo id antigo continua a responder"
        );
        assert_eq!(
            store.find(&tarefa).unwrap().category_id.as_ref(),
            Some(&trabalho),
            "não há nome nenhum dentro da tarefa, logo não há cascata"
        );
        assert_eq!(
            store.categories()[0].name,
            "Trabalho pessoal",
            "o rename não reordena as categorias"
        );

        // Um nome que já existe **noutra** categoria é recusado, e nada muda.
        let erro = store
            .rename_category(&casa, "trabalho PESSOAL")
            .expect_err("nome repetido");
        assert!(matches!(erro, OpsError::DuplicateName(_)));
        assert!(erro.to_string().contains("Trabalho pessoal"));
        assert_eq!(store.category(&casa).unwrap().name, "Casa");

        // O nome da **própria** categoria, noutra capitalização, é permitido:
        // é a colisão consigo mesma que o `excepto` exclui.
        store.rename_category(&casa, "CASA").expect("corrigir");
        assert_eq!(store.category(&casa).unwrap().name, "CASA");

        assert!(matches!(
            store.rename_category(&casa, "   "),
            Err(OpsError::EmptyName)
        ));
        assert_eq!(
            store.category(&casa).unwrap().name,
            "CASA",
            "o erro não alterou"
        );

        let antes = store.db().clone();
        let fantasma = CategoryId::new();
        let erro = store
            .rename_category(&fantasma, "Nova")
            .expect_err("categoria inexistente");
        assert!(matches!(erro, OpsError::CategoryNotFound(_)));
        assert!(
            erro.to_string().contains("não existe nenhuma categoria"),
            "mensagem própria, no molde do erro de tarefa: {erro}"
        );
        assert_eq!(
            *store.db(),
            antes,
            "categoria inexistente não altera a `Db`"
        );

        let recarregado = Store::open(&path).unwrap();
        assert_eq!(
            recarregado.category(&trabalho).unwrap().name,
            "Trabalho pessoal"
        );
        assert_eq!(recarregado.category(&casa).unwrap().name, "CASA");
    }

    #[test]
    fn delete_category_limpa_as_referencias_da_lista_e_do_lixo() {
        let (path, mut store) = store("categoria-delete");
        let trabalho = store.add_category("Trabalho").unwrap();
        let casa = store.add_category("Casa").unwrap();

        // Três na categoria, na lista, e uma no lixo — mais uma noutra
        // categoria e outra sem categoria: as duas últimas provam que a
        // limpeza é selectiva.
        for titulo in ["a", "b", "c"] {
            let id = store.add(titulo).unwrap();
            store.assign_category(&id, Some(&trabalho)).unwrap();
        }
        let no_lixo = store.add("d").unwrap();
        store.assign_category(&no_lixo, Some(&trabalho)).unwrap();
        store.remove(&no_lixo).unwrap();
        let outra = store.add("e").unwrap();
        store.assign_category(&outra, Some(&casa)).unwrap();
        let sem = store.add("f").unwrap();

        assert_eq!(
            store.delete_category(&trabalho).expect("eliminar"),
            4,
            "3 na lista + 1 no lixo"
        );

        assert!(store.category(&trabalho).is_none(), "a categoria saiu");
        assert_eq!(store.categories().len(), 1, "e só ela saiu");
        assert_eq!(store.categories()[0].id, casa);
        // Asserção sobre a `Db` **toda**: nenhuma referência ao id eliminado
        // sobra em `todos` nem em `trash`.
        assert_eq!(
            referencias_a(store.db(), &trabalho),
            0,
            "sobrou uma referência à categoria eliminada: {:?}",
            store.db()
        );
        assert_eq!(
            titulos(&store),
            ["a", "b", "c", "e", "f"],
            "nenhuma tarefa se perdeu"
        );
        assert_eq!(
            store.find(&outra).unwrap().category_id.as_ref(),
            Some(&casa),
            "a outra categoria não foi tocada"
        );
        assert!(store.find(&sem).unwrap().category_id.is_none());
        assert_eq!(store.trash_len(), 1, "o lixo não foi esvaziado");
        assert_eq!(store.db().trash[0].todo.title, "d");
        assert!(
            store.db().trash[0].todo.category_id.is_none(),
            "a entrada do lixo perdeu a atribuição (senão o `u` repunha uma \
             referência pendente)"
        );

        // Uma categoria inexistente é erro, e a `Db` fica como estava.
        let antes = store.db().clone();
        let erro = store.delete_category(&trabalho).expect_err("já não existe");
        assert!(matches!(erro, OpsError::CategoryNotFound(_)));
        assert_eq!(*store.db(), antes);

        drop(store);
        let recarregado = Store::open(&path).unwrap();
        assert_eq!(
            referencias_a(recarregado.db(), &trabalho),
            0,
            "e a limpeza ficou gravada"
        );
        assert!(recarregado.category(&trabalho).is_none());
        assert_eq!(recarregado.todos().len(), 5);
        assert_eq!(recarregado.trash_len(), 1);
    }

    #[test]
    fn assign_category_valida_antes_de_tocar_na_db() {
        let (path, mut store) = store("categoria-assign");
        let trabalho = store.add_category("Trabalho").unwrap();
        let tarefa = store.add("comprar café").unwrap();
        let outra = store.add("pagar contas").unwrap();

        store.assign_category(&tarefa, Some(&trabalho)).unwrap();
        assert_eq!(
            store.find(&tarefa).unwrap().category_id.as_ref(),
            Some(&trabalho)
        );
        assert!(store.find(&outra).unwrap().category_id.is_none());

        // `None` tira a atribuição.
        store.assign_category(&tarefa, None).unwrap();
        assert!(store.find(&tarefa).unwrap().category_id.is_none());

        // Volta a atribuir, para a prova de persistência no fim.
        store.assign_category(&tarefa, Some(&trabalho)).unwrap();

        // Categoria inexistente: erro **e** `Db` igual antes/depois (nada de
        // meia atribuição).
        let antes = store.db().clone();
        let erro = store
            .assign_category(&tarefa, Some(&CategoryId::new()))
            .expect_err("categoria inexistente");
        assert!(matches!(erro, OpsError::CategoryNotFound(_)));
        assert_eq!(*store.db(), antes, "o erro não deixou meia atribuição");
        assert_eq!(
            store.find(&tarefa).unwrap().category_id.as_ref(),
            Some(&trabalho),
            "e não desatribuiu o que estava atribuído"
        );

        // Tarefa inexistente: mesmo contrato.
        let erro = store
            .assign_category(&TodoId::new(), Some(&trabalho))
            .expect_err("tarefa inexistente");
        assert!(matches!(erro, OpsError::NotFound(_)));
        assert_eq!(*store.db(), antes);

        let recarregado = Store::open(&path).unwrap();
        assert_eq!(
            recarregado.find(&tarefa).unwrap().category_id.as_ref(),
            Some(&trabalho),
            "a atribuição ficou gravada"
        );
    }
}
