//! Consultas de leitura sobre a base de dados.
//!
//! Filtrar, buscar e **ordenar** vivem aqui: o ponto 2 do ADR exige que toda a
//! lógica testável sem TTY fique no `core`, e as três são funções puras sobre o
//! modelo. O `tui` fica com as teclas e com o desenho (o texto do cabeçalho é
//! dele). A ordem natural da `Db` é a ordem de inserção, que o undo preserva.

use std::cmp::Reverse;

use super::model::{Category, CategoryId, Todo, TodoId};
use super::store::Store;
use super::store::Trashed;

/// Que tarefas mostrar.
///
/// Não há variante para o lixo: a vista do lixo é um **contexto de teclas**
/// (`tui::event::InputMode::Trash`), servida por [`Store::trash_entries`]. Duas
/// representações do mesmo estado é como o `c` no lixo esvaziava sem guarda
/// (ADR, adenda 1, ponto 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    /// Só as pendentes.
    Active,
    Done,
}

impl Filter {
    pub const ALL: [Self; 3] = [Self::All, Self::Active, Self::Done];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "todas",
            Self::Active => "pendentes",
            Self::Done => "concluídas",
        }
    }
}

/// Filtro por categoria: eixo próprio da vista, ao lado do [`Filter`] de
/// estado (ADR §Decisão 8).
///
/// Não é uma variante do [`Filter`]: dois eixos num só `enum` tornariam
/// inexprimível o caso que interessa — «as pendentes de Trabalho».
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CategoryFilter {
    /// Todas as tarefas, com categoria ou sem ela.
    #[default]
    Todas,
    /// Só as que não têm categoria — o `None`, e também uma referência que não
    /// resolva em categoria nenhuma (tratada como «sem categoria» em todo o
    /// `core`).
    SemCategoria,
    /// Só as de uma categoria.
    Uma(CategoryId),
}

impl CategoryFilter {
    /// Etiqueta para a linha 2, no molde das do [`Filter`] e do [`SortKey`].
    ///
    /// Para [`Self::Uma`] é genérica: o `nome` da categoria filtrada sai de
    /// [`Store::category`] — um `&'static str` não o pode devolver.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Todas => "todas",
            Self::SemCategoria => "sem categoria",
            Self::Uma(_) => "categoria",
        }
    }

    /// A categoria filtrada, se o filtro for [`Self::Uma`].
    #[must_use]
    pub fn id(&self) -> Option<&CategoryId> {
        match self {
            Self::Uma(id) => Some(id),
            Self::Todas | Self::SemCategoria => None,
        }
    }
}

/// Ordem de apresentação da lista. O `s` percorre [`Self::ALL`] pela ordem de
/// declaração e volta ao princípio.
///
/// As etiquetas são curtas porque entram na barra de estado (`ordem: …`). A
/// única fixada por um golden é `"prioridade"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    /// Criação, da mais antiga para a mais recente.
    #[default]
    CreatedAsc,
    /// Criação, da mais recente para a mais antiga.
    CreatedDesc,
    /// Prioridade, da alta para a baixa.
    Priority,
    /// Pendentes primeiro, concluídas no fim.
    Status,
    /// Prazo, do mais próximo para o mais distante; sem prazo no fim.
    Due,
    /// Categoria, pela ordem de inserção das categorias; sem categoria no fim.
    Category,
}

impl SortKey {
    /// Ordem do ciclo do `s` — a ordem de declaração do `enum`.
    pub const ALL: [Self; 6] = [
        Self::CreatedAsc,
        Self::CreatedDesc,
        Self::Priority,
        Self::Status,
        Self::Due,
        Self::Category,
    ];

    /// Próxima ordem do ciclo; a última volta à primeira.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::CreatedAsc => Self::CreatedDesc,
            Self::CreatedDesc => Self::Priority,
            Self::Priority => Self::Status,
            Self::Status => Self::Due,
            Self::Due => Self::Category,
            Self::Category => Self::CreatedAsc,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::CreatedAsc => "mais antigas",
            Self::CreatedDesc => "mais recentes",
            Self::Priority => "prioridade",
            Self::Status => "estado",
            Self::Due => "prazo",
            Self::Category => "categoria",
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
        }
    }

    /// Entradas do lixo, da mais recente para a mais antiga — a ordem em que a
    /// vista do lixo as mostra e o `u` as desfaz.
    #[must_use]
    pub fn trash_entries(&self) -> Vec<&Trashed> {
        self.db().trash.iter().rev().collect()
    }

    /// Categorias conhecidas, pela ordem de inserção — que é a ordem em que a
    /// caixa as mostra (a categoria não tem ordem manual, ADR §Decisão 1).
    #[must_use]
    pub fn categories(&self) -> &[Category] {
        &self.db().categories
    }

    /// A categoria com este `id`, se existir.
    #[must_use]
    pub fn category(&self, id: &CategoryId) -> Option<&Category> {
        self.categories()
            .iter()
            .find(|categoria| &categoria.id == id)
    }

    /// A categoria de uma tarefa, resolvida pelo `id`.
    ///
    /// `None` tanto para «sem categoria» como para uma referência que não
    /// resolva: o invariante diz que a segunda não existe depois de uma leitura
    /// (`Store::load` normaliza-a), mas a consulta não depende disso para estar
    /// certa — e é o que faz o filtro e a ordenação concordarem.
    #[must_use]
    pub fn category_of(&self, todo: &Todo) -> Option<&Category> {
        todo.category_id.as_ref().and_then(|id| self.category(id))
    }

    /// Uma entrada por categoria conhecida, para a caixa.
    ///
    /// As tarefas contadas são as das **duas vistas** (lista e lixo): uma
    /// tarefa no lixo continua atribuída, e é isso que faz a soma desta lista
    /// ser o total de tarefas do programa — nenhuma fora, nenhuma duas vezes.
    ///
    /// O primeiro elemento é sempre o «sem categoria» (`None`), que é a
    /// primeira linha da caixa; as categorias seguem pela ordem de inserção e
    /// aparecem **mesmo sem tarefas** (com `0`) — a linha existe na caixa, logo
    /// a contagem também. Uma referência pendente conta como «sem categoria».
    #[must_use]
    pub fn counts_por_categoria(&self) -> Vec<(Option<&Category>, usize)> {
        // A chave é o `id` (o que a tarefa guarda) e é resolvida para a
        // categoria no fim.
        let mut contagens: Vec<(Option<&CategoryId>, usize)> = std::iter::once((None, 0)).collect();
        contagens.extend(
            self.categories()
                .iter()
                .map(|categoria| (Some(&categoria.id), 0)),
        );

        for todo in self
            .todos()
            .iter()
            .chain(self.db().trash.iter().map(|entrada| &entrada.todo))
        {
            let alvo = todo.category_id.as_ref();
            // Índice 0 é o «sem categoria», onde caem as referências pendentes.
            let posicao = contagens
                .iter()
                .position(|(id, _)| *id == alvo)
                .unwrap_or(0);
            contagens[posicao].1 += 1;
        }

        contagens
            .into_iter()
            .map(|(id, n)| (id.and_then(|id| self.category(id)), n))
            .collect()
    }

    /// Busca case-insensitive sobre o título **e** a descrição, na ordem
    /// natural (de inserção).
    ///
    /// A descrição entra porque também entra pelo import e pela UI (`Ctrl+E`):
    /// procurar só pelo título deixaria invisível metade do texto que o
    /// utilizador escreveu. Busca vazia devolve lista vazia — quem decide
    /// ignorar a busca nesse caso é a [`Store::view`].
    #[must_use]
    pub fn search(&self, needle: &str) -> Vec<TodoId> {
        if needle.is_empty() {
            return Vec::new();
        }
        let alvo = needle.to_lowercase();
        self.todos()
            .iter()
            .filter(|todo| contem(todo, &alvo))
            .map(|todo| todo.id.clone())
            .collect()
    }

    /// Vista da lista: filtro → busca → categoria → ordenação, devolvendo
    /// **ids**.
    ///
    /// Ids e não índices: o `App` vai possuir este `Vec`, e um `Vec<usize>`
    /// obrigaria o mesmo campo a indexar duas listas diferentes (a lista e o
    /// lixo) — dois espaços de índices no mesmo tipo é como se repõe a tarefa
    /// errada. O id é estável e serve as duas vistas.
    ///
    /// Os três eixos compõem-se e nenhum muda a semântica do outro: a busca
    /// continua a ser sobre o título e a descrição, o `filter` continua a ser
    /// sobre o estado e a `categoria` é um eixo próprio (ADR §Decisão 8) — é o
    /// que permite «pendentes de Trabalho».
    ///
    /// É só leitura: não altera a `Db`.
    #[must_use]
    pub fn view(
        &self,
        filter: Filter,
        sort: SortKey,
        categoria: CategoryFilter,
        query: &str,
    ) -> Vec<TodoId> {
        let mut todos = self.filtered(filter);
        if !query.is_empty() {
            let alvo = query.to_lowercase();
            todos.retain(|todo| contem(todo, &alvo));
        }
        match &categoria {
            CategoryFilter::Todas => {}
            CategoryFilter::SemCategoria => {
                todos.retain(|todo| self.category_of(todo).is_none());
            }
            CategoryFilter::Uma(id) => {
                todos.retain(|todo| self.category_of(todo).is_some_and(|c| &c.id == id));
            }
        }
        sort_todos(&mut todos, sort, self.categories());
        todos.iter().map(|todo| todo.id.clone()).collect()
    }
}

/// Comparação da busca: o `alvo` já vem em minúsculas.
fn contem(todo: &Todo, alvo: &str) -> bool {
    todo.title.to_lowercase().contains(alvo) || todo.description.to_lowercase().contains(alvo)
}

/// Ordenação estável: empates ficam pela ordem de inserção.
///
/// `categorias` só serve o [`SortKey::Category`] e vem pela ordem de inserção
/// (a mesma da caixa e da lista devolvida por [`Store::categories`]).
fn sort_todos(todos: &mut [&Todo], sort: SortKey, categorias: &[Category]) {
    match sort {
        SortKey::CreatedAsc => todos.sort_by_key(|todo| todo.created_at),
        SortKey::CreatedDesc => todos.sort_by_key(|todo| Reverse(todo.created_at)),
        // `Priority` ordena Low < Medium < High, logo decrescente é High→Low.
        SortKey::Priority => todos.sort_by_key(|todo| Reverse(todo.priority)),
        // Pendentes primeiro; o `false < true` faz isso sem inverter nada.
        SortKey::Status => todos.sort_by_key(|todo| todo.is_done()),
        // `None` no fim (`is_none()` é chave primária), estável nos dois
        // grupos: entre as tarefas sem prazo fica a ordem de inserção.
        SortKey::Due => todos.sort_by_key(|todo| (todo.due_at.is_none(), todo.due_at)),
        // Mesmo molde do `Due`, com a categoria no lugar do prazo. A chave é a
        // **posição da categoria na ordem de inserção** — a ordem que o
        // utilizador vê na caixa — e não o `id`: com `uuid` v4 os grupos
        // sairiam por uma ordem arbitrária, que não corresponde a nada. Sem
        // categoria (ou com uma referência que não resolve) vai para o fim,
        // pela ordem de inserção.
        SortKey::Category => todos.sort_by_key(|todo| {
            let posicao = todo
                .category_id
                .as_ref()
                .and_then(|id| categorias.iter().position(|categoria| &categoria.id == id));
            (posicao.is_none(), posicao)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Priority;
    use chrono::{Duration, Local, NaiveDate};
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

    /// Tarefa com criação há `minutos` minutos — datas explícitas, para a
    /// ordenação por criação não depender da resolução do relógio.
    fn todo(titulo: &str, minutos: i64) -> Todo {
        let mut todo = Todo::try_new(titulo).expect("título válido");
        todo.created_at = Local::now() - Duration::minutes(minutos);
        todo
    }

    fn titulos(store: &Store, ids: &[TodoId]) -> Vec<String> {
        ids.iter()
            .map(|id| {
                store
                    .find(id)
                    .expect("id da vista existe no store")
                    .title
                    .clone()
            })
            .collect()
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
        assert_eq!(
            store
                .trash_entries()
                .iter()
                .map(|e| e.todo.title.as_str())
                .collect::<Vec<_>>(),
            ["removida"],
            "o lixo não é um `Filter`: é a vista do `InputMode::Trash`"
        );
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
        assert_eq!(Filter::All.label(), "todas");
        assert_eq!(
            Filter::Active.label(),
            "pendentes",
            "o golden diz «filtro: pendentes» (ADR, adenda 1)"
        );
        assert_eq!(Filter::Done.label(), "concluídas");
        assert_eq!(Filter::ALL, [Filter::All, Filter::Active, Filter::Done]);
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

    #[test]
    fn busca_e_case_insensitive_e_apanha_a_descricao() {
        let (_path, mut store) = store("busca");
        let cafe = store
            .insert(todo("Comprar Café", 30).with_description("grão da Etiópia"))
            .unwrap();
        let netflix = store.insert(todo("Ver série", 20)).unwrap();
        let _outra = store.insert(todo("Pagar contas", 10)).unwrap();

        assert_eq!(store.search("café"), vec![cafe.clone()], "apanha o título");
        assert_eq!(
            store.search("CAFÉ"),
            vec![cafe.clone()],
            "a busca é case-insensitive nos dois lados"
        );
        assert_eq!(
            store.search("etiópia"),
            vec![cafe.clone()],
            "a descrição também conta"
        );
        assert_eq!(store.search("GRÃO"), vec![cafe], "descrição em maiúsculas");
        assert_eq!(store.search("série"), vec![netflix]);
        assert!(store.search("inexistente").is_empty());
        assert!(
            store.search("").is_empty(),
            "busca vazia devolve lista vazia (quem a ignora é o `view`)"
        );
    }

    #[test]
    fn view_combina_filtro_busca_e_ordem_sem_alterar_a_db() {
        let (_path, mut store) = store("view");
        let antiga = store
            .insert(todo("café antigo", 30).with_priority(Priority::Low))
            .unwrap();
        let media = store
            .insert(todo("café médio", 20).with_priority(Priority::Medium))
            .unwrap();
        let alta = store
            .insert(todo("café novo", 10).with_priority(Priority::High))
            .unwrap();
        let feita = store.insert(todo("café feito", 5)).unwrap();
        store.toggle_done(&feita).unwrap();
        // Pendente e **sem** «café»: existe para a busca ter o que excluir, e por
        // isso conta nas vistas de `Filter::Active`. Prioridade explícita para a
        // asserção de prioridade não depender do `Priority::default()`.
        let sem_cafe = store
            .insert(todo("netflix", 1).with_priority(Priority::Low))
            .unwrap();

        let antes = store.db().clone();

        // Filtro: a concluída fica de fora, a «netflix» fica dentro.
        assert_eq!(
            store.view(
                Filter::Active,
                SortKey::CreatedAsc,
                CategoryFilter::Todas,
                ""
            ),
            [
                antiga.clone(),
                media.clone(),
                alta.clone(),
                sem_cafe.clone()
            ],
            "filtro + ordem natural"
        );
        // Busca: só os que têm «café» no título ou na descrição.
        assert_eq!(
            store.view(
                Filter::All,
                SortKey::CreatedAsc,
                CategoryFilter::Todas,
                "CAFÉ"
            ),
            [antiga.clone(), media.clone(), alta.clone(), feita.clone()],
            "a busca é case-insensitive e deixa «netflix» de fora"
        );
        // Ordem: prioridade alta primeiro; o empate fica pela ordem de inserção.
        assert_eq!(
            store.view(Filter::Active, SortKey::Priority, CategoryFilter::Todas, ""),
            [
                alta.clone(),
                media.clone(),
                antiga.clone(),
                sem_cafe.clone()
            ],
            "`prioridade` ordena High → Low"
        );
        // Ordem: criação decrescente.
        assert_eq!(
            store.view(
                Filter::Active,
                SortKey::CreatedDesc,
                CategoryFilter::Todas,
                ""
            ),
            [
                sem_cafe.clone(),
                alta.clone(),
                media.clone(),
                antiga.clone()
            ]
        );
        // Ordem: estado (pendentes primeiro).
        assert_eq!(
            store.view(Filter::All, SortKey::Status, CategoryFilter::Todas, ""),
            [
                antiga.clone(),
                media.clone(),
                alta.clone(),
                sem_cafe.clone(),
                feita.clone()
            ]
        );
        // Filtro + busca sem resultados: «netflix» está pendente, logo não
        // sobrevive ao filtro `Done`.
        assert!(
            store
                .view(
                    Filter::Done,
                    SortKey::Priority,
                    CategoryFilter::Todas,
                    "netflix"
                )
                .is_empty()
        );

        assert_eq!(
            *store.db(),
            antes,
            "`view` é só leitura: a `Db` fica exactamente como estava"
        );
    }

    #[test]
    fn view_ordena_por_prazo_com_as_sem_data_no_fim() {
        let (_path, mut store) = store("prazo");
        let sem_prazo_a = store.insert(todo("sem prazo a", 30)).unwrap();
        let proximo = store
            .insert(todo("amanhã", 20).with_due_at(NaiveDate::from_ymd_opt(2026, 9, 19)))
            .unwrap();
        let sem_prazo_b = store.insert(todo("sem prazo b", 10)).unwrap();
        let longe = store
            .insert(
                todo("para o mês que vem", 1).with_due_at(NaiveDate::from_ymd_opt(2026, 10, 30)),
            )
            .unwrap();

        assert_eq!(
            store.view(Filter::All, SortKey::Due, CategoryFilter::Todas, ""),
            [proximo, longe, sem_prazo_a, sem_prazo_b],
            "as sem prazo ficam no fim, pela ordem natural (estável)"
        );
    }

    #[test]
    fn sort_key_cicla_pela_ordem_de_all_e_volta_ao_principio() {
        assert_eq!(SortKey::ALL.len(), 6);
        assert_eq!(SortKey::Priority.label(), "prioridade");
        assert_eq!(SortKey::Category.label(), "categoria");
        for (i, ordem) in SortKey::ALL.iter().enumerate() {
            let seguinte = SortKey::ALL[(i + 1) % SortKey::ALL.len()];
            assert_eq!(
                ordem.next(),
                seguinte,
                "`next()` tem de coincidir com a ordem de `ALL`"
            );
            assert!(!ordem.label().is_empty(), "todas as ordens têm etiqueta");
        }
        assert_eq!(
            SortKey::Category.next(),
            SortKey::CreatedAsc,
            "o ciclo fecha"
        );
        assert_eq!(SortKey::Due.next(), SortKey::Category);
        assert_eq!(SortKey::default(), SortKey::CreatedAsc);
    }

    #[test]
    fn titulos_da_vista_por_id() {
        let (_path, mut store) = store("titulos");
        let a = store.insert(todo("primeira", 10)).unwrap();
        let b = store.insert(todo("segunda", 5)).unwrap();
        let vista = store.view(Filter::All, SortKey::CreatedDesc, CategoryFilter::Todas, "");
        assert_eq!(titulos(&store, &vista), ["segunda", "primeira"]);
        assert_eq!(vista, [b, a]);
    }

    #[test]
    fn rotulos_do_filtro_de_categoria() {
        let (_path, mut store) = store("filtro-categoria-rotulos");
        let trabalho = store.add_category("Trabalho").unwrap();

        assert_eq!(CategoryFilter::default(), CategoryFilter::Todas);
        assert_eq!(CategoryFilter::Todas.label(), "todas");
        assert_eq!(CategoryFilter::SemCategoria.label(), "sem categoria");
        assert_eq!(CategoryFilter::Uma(trabalho.clone()).label(), "categoria");
        // A etiqueta de `Uma` é genérica de propósito: o **nome** da categoria
        // sai do `Store`, que é o único que o conhece.
        assert_eq!(store.category(&trabalho).unwrap().name, "Trabalho");
        assert_eq!(CategoryFilter::Uma(trabalho.clone()).id(), Some(&trabalho));
        assert!(CategoryFilter::Todas.id().is_none());
        assert!(CategoryFilter::SemCategoria.id().is_none());
    }

    #[test]
    fn view_filtra_por_categoria_sem_alterar_a_db() {
        let (_path, mut store) = store("view-categoria");
        let trabalho = store.add_category("Trabalho").unwrap();
        let casa = store.add_category("Casa").unwrap();
        let relatorio = store
            .insert(todo("relatório", 40).with_category(Some(trabalho.clone())))
            .unwrap();
        let reuniao = store
            .insert(todo("reunião", 30).with_category(Some(trabalho.clone())))
            .unwrap();
        let compras = store
            .insert(todo("compras", 20).with_category(Some(casa.clone())))
            .unwrap();
        let solta = store.insert(todo("sem categoria", 10)).unwrap();
        store.toggle_done(&reuniao).unwrap();

        let antes = store.db().clone();

        assert_eq!(
            store.view(Filter::All, SortKey::CreatedAsc, CategoryFilter::Todas, ""),
            [
                relatorio.clone(),
                reuniao.clone(),
                compras.clone(),
                solta.clone()
            ],
            "`Todas` não filtra nada"
        );
        assert_eq!(
            store.view(
                Filter::All,
                SortKey::CreatedAsc,
                CategoryFilter::SemCategoria,
                ""
            ),
            vec![solta.clone()],
            "«sem categoria» são só as que não têm atribuição"
        );
        assert_eq!(
            store.view(
                Filter::All,
                SortKey::CreatedAsc,
                CategoryFilter::Uma(trabalho.clone()),
                ""
            ),
            [relatorio.clone(), reuniao.clone()]
        );
        assert_eq!(
            store.view(
                Filter::All,
                SortKey::CreatedAsc,
                CategoryFilter::Uma(casa.clone()),
                ""
            ),
            vec![compras.clone()]
        );
        // Os dois eixos são independentes: «concluídas de Trabalho».
        assert_eq!(
            store.view(
                Filter::Done,
                SortKey::CreatedAsc,
                CategoryFilter::Uma(trabalho.clone()),
                ""
            ),
            vec![reuniao.clone()]
        );
        // E a busca filtra **dentro** da categoria, não em vez dela.
        assert_eq!(
            store.view(
                Filter::All,
                SortKey::CreatedAsc,
                CategoryFilter::Uma(trabalho.clone()),
                "REUNIÃO"
            ),
            vec![reuniao.clone()]
        );
        assert!(
            store
                .view(
                    Filter::All,
                    SortKey::CreatedAsc,
                    CategoryFilter::Uma(casa.clone()),
                    "relatório"
                )
                .is_empty(),
            "a busca não atravessa categorias"
        );

        // `category_of` é a resolução que o filtro usa.
        assert_eq!(
            store
                .category_of(store.find(&relatorio).unwrap())
                .map(|categoria| &categoria.id),
            Some(&trabalho)
        );
        assert!(store.category_of(store.find(&solta).unwrap()).is_none());
        assert!(
            store.category(&CategoryId::new()).is_none(),
            "um id que não existe não é categoria"
        );
        assert_eq!(store.categories().len(), 2, "pela ordem de inserção");
        assert_eq!(store.categories()[0].id, trabalho);
        assert_eq!(store.categories()[1].id, casa);

        assert_eq!(
            *store.db(),
            antes,
            "`view` é só leitura: a `Db` fica exactamente como estava"
        );
    }

    #[test]
    fn view_ordena_por_categoria_com_as_sem_categoria_no_fim() {
        let (_path, mut store) = store("ordem-categoria");
        let casa = store.add_category("Casa").unwrap();
        let trabalho = store.add_category("Trabalho").unwrap();

        // Inserção entrelaçada de propósito: se a ordenação fosse por criação
        // (ou pelo `id` da categoria) a ordem saída seria outra.
        let t1 = store
            .insert(todo("t1", 60).with_category(Some(trabalho.clone())))
            .unwrap();
        let s1 = store.insert(todo("s1", 50)).unwrap();
        let c1 = store
            .insert(todo("c1", 40).with_category(Some(casa.clone())))
            .unwrap();
        let t2 = store
            .insert(todo("t2", 30).with_category(Some(trabalho.clone())))
            .unwrap();
        let s2 = store.insert(todo("s2", 20)).unwrap();
        let c2 = store
            .insert(todo("c2", 10).with_category(Some(casa.clone())))
            .unwrap();

        assert_eq!(
            store.view(Filter::All, SortKey::Category, CategoryFilter::Todas, ""),
            [c1, c2, t1, t2, s1, s2],
            "agrupa pela ordem de inserção das categorias (Casa, depois Trabalho), \
             preserva a ordem de inserção dentro de cada grupo e põe as sem \
             categoria no fim"
        );
    }

    #[test]
    fn counts_por_categoria_somam_o_total_das_duas_vistas() {
        let (_path, mut store) = store("contagens-categoria");
        let trabalho = store.add_category("Trabalho").unwrap();
        let casa = store.add_category("Casa").unwrap();
        let vazia = store.add_category("Vazia").unwrap();

        store
            .insert(todo("a", 50).with_category(Some(trabalho.clone())))
            .unwrap();
        store
            .insert(todo("b", 40).with_category(Some(trabalho.clone())))
            .unwrap();
        let no_lixo = store
            .insert(todo("c", 30).with_category(Some(trabalho.clone())))
            .unwrap();
        store
            .insert(todo("d", 20).with_category(Some(casa.clone())))
            .unwrap();
        store.insert(todo("e", 10)).unwrap();
        // Uma tarefa **no lixo** com categoria: continua atribuída, logo conta.
        store.remove(&no_lixo).unwrap();

        let contagens = store.counts_por_categoria();
        assert_eq!(contagens.len(), 4, "«sem categoria» + as três categorias");
        assert_eq!(contagens[0].0, None, "«sem categoria» é a primeira linha");
        assert_eq!(contagens[0].1, 1);
        assert_eq!(
            contagens[1].0.map(|categoria| categoria.name.as_str()),
            Some("Trabalho")
        );
        assert_eq!(contagens[1].1, 3, "2 na lista + 1 no lixo");
        assert_eq!(
            contagens[2].0.map(|categoria| categoria.name.as_str()),
            Some("Casa")
        );
        assert_eq!(contagens[2].1, 1);
        assert_eq!(
            contagens[3].0.map(|categoria| categoria.id.as_str()),
            Some(vazia.as_str()),
            "uma categoria sem tarefas aparece com 0: a linha existe na caixa"
        );
        assert_eq!(contagens[3].1, 0);
        assert_eq!(
            contagens.iter().map(|(_, n)| n).sum::<usize>(),
            store.todos().len() + store.trash_len(),
            "a soma é o total das duas vistas: nada fora, nada contado duas vezes"
        );
    }

    #[test]
    fn counts_por_categoria_sem_categorias_sobra_a_linha_sem_categoria() {
        // Uma base que nunca teve categorias não devolve uma lista vazia:
        // sobra a linha «sem categoria» com o total — é a primeira linha da
        // caixa, e a caixa abre numa base vazia (ADR §Decisão 7).
        let (_path, mut store) = store("contagens-sem-categoria");
        store.insert(todo("x", 2)).unwrap();
        store.insert(todo("y", 1)).unwrap();

        let contagens = store.counts_por_categoria();
        assert_eq!(contagens.len(), 1);
        assert_eq!(contagens[0], (None, 2));
    }
}
