//! Estado da aplicação e efeito de cada acção.
//!
//! O [`App`] é a única peça que liga as teclas aos dados: [`map_key`] diz *que*
//! acção a tecla vale no modo activo, e `App::handle` é o único sítio onde essa
//! acção toca no `core`. Nada aqui desenha (o render é do T6) nem lê eventos: o
//! dono do `App` é o loop de `tui::run`.
//!
//! Tudo é testável sem TTY: o [`Store`] dos testes vive num directório
//! temporário e as teclas são `KeyEvent::new(…)`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::core::{
    Category, CategoryFilter, CategoryId, Config, Counts, Filter, OpsError, Priority, SortKey,
    Store, Todo, TodoId, export_csv_to_path, import_from_path,
};

use super::event::{Action, InputMode, map_key};
use super::theme::{CATALOGO, ModoCor, Theme};
// O corte em colunas é o mesmo do desenho (`ui::cortar`): um só sítio mede
// `café` como quatro colunas, e a mensagem da eliminação corta o **nome**, nunca
// a contagem (§C.4).
use super::ui::cortar;

/// Janela da guarda do `c` no lixo: a segunda pressão tem de vir dentro deste
/// tempo, e qualquer outra acção desarma (§5, mudança 7).
///
/// É a única operação da v1 sem undo por trás — a guarda existe por isso, não
/// por gosto de confirmações.
pub const ARM_WINDOW: Duration = Duration::from_secs(5);

/// Quanto tempo uma mensagem de acção fica na linha 22 antes de sair sozinha
/// (§4 do desenho).
///
/// Só as mensagens de acção a respeitam: o undo disponível e o aviso de
/// transbordo nascem `sticky` e ficam até outra acção os substituir.
pub const MESSAGE_TTL: Duration = Duration::from_secs(3);

/// Quantas colunas do nome de uma categoria cabem numa mensagem de acção da
/// linha 22 (§C.4, medido: 79 colunas úteis).
///
/// A regra da §C.4 é cortar **o nome** e nunca a contagem — é ela que diz o
/// preço da operação. Com 30 colunas, `Eliminada «Backups do servidor
/// doméstico…» · 3 tarefas ficaram sem categoria` mede 76/79, que é exactamente
/// o caso que o `@designer` mediu.
pub const NOME_NA_MENSAGEM: usize = 30;

/// Marca da mensagem de remoção, que é a que fica enquanto houver undo (§4).
///
/// É uma constante partilhada porque o T6 tem de reconhecê-la para reduzir a
/// barra de ajuda a `u desfazer  ? ajuda` (frame `80x24-5-undo`): o `Status` é
/// texto, e a barra do desfazer é o único sítio onde o desenho depende do
/// *conteúdo* da mensagem. Sem a constante, o literal estaria em dois ficheiros.
pub const UNDO_HINT: &str = "u para desfazer";

/// Filtro seguinte do ciclo (`f`).
///
/// `Filter::Trash` **não** entra: no todo-ratatui o lixo é uma vista própria
/// (`L`), não um valor de filtro — se entrasse, o `d` e o `c` ficavam
/// alcançáveis dentro do lixo com o significado da lista.
fn next_filter(filter: Filter) -> Filter {
    match filter {
        Filter::Active => Filter::Done,
        Filter::Done => Filter::All,
        // `All` — e o `Filter::Trash` do `core` anterior, que o cartão T3b
        // retira. O ciclo do `f` nunca mostra o lixo (§5: o lixo é a vista do
        // `L`), e o `_` deixa isto a compilar contra as duas versões da
        // `Filter` **sem** voltar a nomear a variante que sai — nomeá-la era
        // reintroduzir no `tui` exactamente a decisão que o T3b desfaz.
        _ => Filter::Active,
    }
}

/// Índice do tema no [`CATALOGO`] — a posição do cursor da caixa de temas.
///
/// Um tema que não esteja no catálogo (hoje impossível, mas o [`Theme`] é
/// `pub`) não pode pôr o cursor fora dos limites: cai em `0`, o primeiro da
/// lista, e o `Enter` a partir daí grava o que está debaixo do cursor.
fn posicao_no_catalogo(tema: &Theme) -> usize {
    CATALOGO
        .iter()
        .position(|candidato| candidato.slug == tema.slug)
        .unwrap_or(0)
}

/// O que a guarda de duas pressões tem armado (ADR §Decisão 6).
///
/// Era um `Option<Instant>` só, e um sinalizador com dois significados é como
/// se esvazia o lixo ao eliminar uma categoria: as duas operações sem undo por
/// trás (`c` no lixo, `d` na caixa) têm de saber **qual** das duas está armada
/// para que a outra não confirme o que o utilizador não pediu.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Guarda {
    /// `c` no lixo — esvaziar tudo (a primeira operação sem undo da v1).
    EsvaziarLixo,
    /// `d` na caixa — eliminar esta categoria e tirar a atribuição das tarefas
    /// dela (a segunda: recriar a categoria é uma linha, a atribuição não volta).
    EliminarCategoria(CategoryId),
}

/// Nome cortado para caber numa mensagem de acção da linha 22 (§C.4).
///
/// `pub(crate)` porque o desenho também o usa: a linha de estado da caixa diz o
/// preço da eliminação (`Eliminar «…»? 3 tarefas ficam sem categoria`) e o nome
/// corta-se da mesma maneira nos dois sítios — a regra do `cortar` é uma só.
pub(crate) fn nome_na_mensagem(nome: &str) -> String {
    cortar(nome.trim(), NOME_NA_MENSAGEM)
}

/// `Eliminada «X» · N tarefas ficaram sem categoria` (§C.4) — o número é o preço
/// de a eliminação não ser reversível, e por isso nunca é ele que se corta.
fn mensagem_eliminada(nome: &str, afectadas: usize) -> String {
    let nome = nome_na_mensagem(nome);
    if afectadas == 1 {
        format!("Eliminada «{nome}» · 1 tarefa ficou sem categoria")
    } else {
        format!("Eliminada «{nome}» · {afectadas} tarefas ficaram sem categoria")
    }
}

/// O que a linha 22 mostra (§4). A precedência — erro > mensagem > descrição do
/// selecionado > vazio — é do T6; aqui só se guarda o que há para mostrar, e o
/// prazo de cada mensagem.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Idle,
    /// Mensagem de acção («Removida «X»    u para desfazer»). Sai sozinha ao
    /// fim de [`MESSAGE_TTL`] — ou na tecla seguinte, o que vier primeiro.
    Message {
        texto: String,
        /// Instante em que a mensagem nasceu: é o que [`App::tick`] compara com
        /// o instante injectado.
        desde: Instant,
    },
    /// Mensagem que **não** expira: o undo disponível e o aviso de transbordo
    /// (§4). O sinalizador é explícito — quem cria a mensagem é que sabe o
    /// prazo — e não um `contains(UNDO_HINT)` sobre o texto.
    Sticky(String),
    /// Erro: vermelho e com o prefixo «Erro:» no T6. Nunca expira sozinho.
    Error(String),
}

impl Status {
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Message { texto, .. } | Self::Sticky(texto) | Self::Error(texto) => Some(texto),
        }
    }

    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Mensagem de acção: expira em [`MESSAGE_TTL`].
    pub fn message(texto: impl Into<String>) -> Self {
        Self::Message {
            texto: texto.into(),
            desde: Instant::now(),
        }
    }

    /// Mensagem que fica enquanto for verdade: undo disponível ou aviso de
    /// transbordo (§4).
    pub fn sticky(texto: impl Into<String>) -> Self {
        Self::Sticky(texto.into())
    }

    pub fn error(texto: impl Into<String>) -> Self {
        Self::Error(texto.into())
    }
}

/// Estado completo da aplicação.
pub struct App {
    store: Store,
    /// Índice da linha selecionada **na vista** (não na `Db`): a `List` do T6
    /// consome-o directamente.
    pub list_state: ListState,
    pub mode: InputMode,
    pub status: Status,
    pub filter: Filter,
    /// Busca confirmada (`/` + `Enter`). `Esc` em repouso limpa-a.
    pub query: String,
    /// Ordem de apresentação da lista (`s` cicla-a; a linha 2 do T6 mostra-a
    /// em `ordem: …`). Arranca em [`SortKey::Priority`] porque é o que o golden
    /// `80x24-1-normal` mostra — e **não** em [`SortKey::default`], que é
    /// `CreatedAsc` (`mais antigas`).
    ///
    /// O que é decidido no `tui` é a divisão **pendentes / concluídas**: o
    /// golden põe as concluídas no fim e o `SortKey` do `core` é puro (ordena
    /// as `M` concluídas acima das `L` pendentes). A ordem **dentro** de cada
    /// grupo continua a ser a do `core::query` — ver [`App::visible`].
    pub sort: SortKey,
    /// O tema com que o ecrã é desenhado. É a **única** fonte de cor do `tui`:
    /// o `ui` lê os papéis daqui e o `App` não guarda cores nenhumas.
    ///
    /// `&'static` porque o catálogo é `const` no binário (o tema é dado, não
    /// estado): trocar de tema é reapontar a referência (T5), nunca copiar
    /// paletas. Por omissão é o [`Theme::default`] — Tokyo Night —, que é o que
    /// mantém `App::new(store)` com uma assinatura só (o arranque passa a
    /// escolha por cima, quando a tem).
    pub theme: &'static Theme,
    /// Ficheiro de preferências onde a caixa de temas grava o *slug* (T5).
    ///
    /// `None` quando não há caminho conhecido — o `App` dos testes, ou um
    /// sistema sem `XDG_CONFIG_HOME` nem `HOME`: aí o tema escolhe-se na mesma
    /// nesta sessão e o `Enter` da caixa fecha **sem** tentar gravar, em vez de
    /// inventar um caminho no `~/.config` real.
    pub config_path: Option<PathBuf>,
    /// Cursor da caixa de temas (`T`): índice na [`CATALOGO`].
    ///
    /// Estado de navegação — e `pub` porque o desenho o lê (a caixa marca a
    /// linha do cursor e o tema em uso).
    pub theme_cursor: usize,
    /// Tema que estava aplicado quando a caixa abriu: é o que o `Esc` repõe
    /// (§Decisão 9). Por referência, como o `theme` — o catálogo é `const`.
    theme_entrada: &'static Theme,
    /// O modo de cor desta sessão (T4), **já resolvido**: um `Auto` passa por
    /// [`ModoCor::detectar`] no construtor, porque a caixa de temas anuncia
    /// `rgb` ou `ansi` (§T.5) e um valor farejado a cada desenho faria o ecrã
    /// depender do ambiente em que o desenho corre.
    ///
    /// Existe por causa de uma linha da caixa: em `ansi` o ecrã mostra o
    /// `classico` e a caixa tem de o dizer — sem ela, navegar por quatro temas
    /// num terminal de 16 cores parecia a caixa avariada (ADR §Decisão 3).
    pub modo_cor: ModoCor,
    buffer: Vec<char>,
    cursor: usize,
    /// Em `Editing`: o `Enter` grava a descrição em vez do título.
    editing_description: bool,
    /// Cursor da caixa de categorias (`C`): índice na lista de entradas, onde
    /// **0 é sempre `sem categoria`** (o `None`, não uma categoria) e as
    /// categorias seguem pela ordem de inserção (§C.1).
    ///
    /// `pub` como o `theme_cursor`: o desenho lê-o para pôr o `▶`.
    pub categoria_cursor: usize,
    /// Filtro por categoria da lista (`F`) — eixo próprio, ao lado do
    /// [`App::filter`] de estado. Cada um restringe o seu (ADR §8): é o que
    /// permite «pendentes de Trabalho».
    pub category_filter: CategoryFilter,
    /// Em `CategoryName`: o `Enter` renomeia a categoria do cursor em vez de
    /// criar uma nova — o mesmo molde do [`App::editing_description`] no modo
    /// `Editing` (dois destinos numa linha de texto).
    renomear_categoria: bool,
    /// Modo para onde voltar quando a sobreposição fechar — a ajuda, a caixa de
    /// temas e a caixa de categorias abrem-se por cima da vista e guardam aqui
    /// de onde vieram.
    return_mode: InputMode,
    /// A guarda de duas pressões: **o que** está armado e até quando (ADR §6).
    armado: Option<(Guarda, Instant)>,
    quitting: bool,
}

impl App {
    /// Arranca com a lista na ordem por omissão, a primeira tarefa selecionada
    /// e o tema por omissão — e **sem** ficheiro de preferências.
    ///
    /// É o construtor dos testes: continua a ter a assinatura que os três
    /// sítios de construção da v1.0.1 esperam (`src/tui/mod.rs`,
    /// `src/tui/app.rs`, `tests/render.rs`) e não tem por onde gravar, logo
    /// nenhum teste escreve no `~/.config` real. O arranque a sério usa
    /// [`App::com_tema`].
    ///
    /// Sem sessão não há modo de cor para consultar: fica `rgb`, o modo do
    /// golden `80x24-14-temas`. Um valor fixo (e não `Auto`, que farejaria o
    /// ambiente a cada desenho) é o que torna o desenho previsível num teste.
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self::com_tema(store, Theme::default(), None, ModoCor::Rgb)
    }

    /// O construtor do arranque: o tema, o caminho do `config.json` e o modo de
    /// cor já resolvidos por [`super::arranque::resolver`] (T4).
    ///
    /// O [`App`] guarda-os porque a caixa de temas do T5 precisa dos dois
    /// primeiros para gravar — e só ela grava: nem `--theme` nem a variável de
    /// ambiente escrevem no ficheiro (ADR §Decisão 2) — e o desenho do T6
    /// precisa do terceiro para a linha `modo de cor:` da caixa. O modo entra
    /// aqui como o arranque o decidiu (`Auto` incluído) e é guardado
    /// **detectado**: é o modo que está em vigor que a caixa anuncia.
    ///
    /// O cursor da caixa abre na posição do tema **aplicado**, e não na primeira
    /// do catálogo: é o que faz o `Enter` sem navegar não mudar nada.
    #[must_use]
    pub fn com_tema(
        store: Store,
        theme: &'static Theme,
        config_path: Option<PathBuf>,
        modo_cor: ModoCor,
    ) -> Self {
        let mut app = Self {
            store,
            theme,
            config_path,
            modo_cor: modo_cor.detectar(),
            theme_cursor: posicao_no_catalogo(theme),
            theme_entrada: theme,
            list_state: ListState::default(),
            mode: InputMode::Normal,
            status: Status::Idle,
            filter: Filter::default(),
            query: String::new(),
            sort: SortKey::Priority,
            buffer: Vec::new(),
            cursor: 0,
            editing_description: false,
            categoria_cursor: 0,
            category_filter: CategoryFilter::Todas,
            renomear_categoria: false,
            return_mode: InputMode::Normal,
            armado: None,
            quitting: false,
        };
        app.clamp_selection();
        app
    }

    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// `true` depois de `q` ou `Ctrl+C`: o loop do T7 sai.
    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.quitting
    }

    /// Linha de entrada em construção (o T6 desenha-a com o cursor no fim).
    #[must_use]
    pub fn input(&self) -> String {
        self.buffer.iter().collect()
    }

    /// Posição do cursor na linha, em caracteres.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// A guarda do `c` no lixo está armada (e dentro do prazo)?
    ///
    /// Pergunta **pelo alvo**: com a guarda a ter alvo (ADR §6), um `armed()`
    /// que respondesse «há alguma guarda armada» diria ao rodapé do lixo para
    /// anunciar `c outra vez confirma` com a eliminação de uma categoria armada.
    #[must_use]
    pub fn armed(&self) -> bool {
        matches!(&self.armado, Some((Guarda::EsvaziarLixo, fim)) if Instant::now() < *fim)
    }

    /// A categoria armada para eliminação (`d` uma vez), se a janela ainda
    /// correr.
    ///
    /// É o que o desenho da caixa precisa para a linha de estado `Eliminar
    /// «Trabalho»? N tarefas ficam sem categoria` (§C.4): o texto é do desenho, o
    /// **alvo** é do `App`.
    #[must_use]
    pub fn categoria_armada(&self) -> Option<&CategoryId> {
        match &self.armado {
            Some((Guarda::EliminarCategoria(id), fim)) if Instant::now() < *fim => Some(id),
            _ => None,
        }
    }

    /// Em `CategoryName`: o `Enter` vai renomear (e não criar) — é o que diz ao
    /// desenho se a linha 22 se etiqueta `Renomear` ou `Nova categoria` (§C.4).
    #[must_use]
    pub const fn renomeando_categoria(&self) -> bool {
        self.renomear_categoria
    }

    /// O contexto por baixo da sobreposição: com a ajuda aberta, é a vista de
    /// onde ela foi aberta, não `Help`.
    ///
    /// É o que diz ao desenho que teclas listar dentro da caixa e qual é a
    /// barra do rodapé (§6: «na vista do lixo mostra só as teclas dessa vista»).
    #[must_use]
    pub const fn context(&self) -> InputMode {
        if matches!(self.mode, InputMode::Help) {
            self.return_mode
        } else {
            self.mode
        }
    }

    #[must_use]
    pub fn counts(&self) -> Counts {
        self.store.counts()
    }

    /// O tema **gravado** — o que estava aplicado quando a caixa de temas abriu.
    ///
    /// É o que a caixa marca com `em uso` e o que o `Esc` repõe (§T.5): enquanto
    /// se navega, [`App::theme`] é o candidato a pré-visualizar e este fica
    /// parado. Fora da caixa os dois coincidem.
    #[must_use]
    pub const fn tema_gravado(&self) -> &'static Theme {
        self.theme_entrada
    }

    /// Que tarefas a vista mostra, já filtradas, buscadas e ordenadas.
    ///
    /// Este é o índice de `list_state`: o que aqui aparece na posição `i` é o
    /// que está selecionado quando `list_state.selected() == Some(i)`.
    ///
    /// A ordem vem do `core::query::view` ([`SortKey`]), com uma decisão de
    /// apresentação por cima: com `Filter::All` as **concluídas ficam no fim**
    /// (frame `80x24-1-normal`), cada grupo pela ordem escolhida. Faz-se com
    /// dois `view` — um por grupo — e não com um comparador próprio: a ordem
    /// dentro do grupo continua a ser a do `core`.
    #[must_use]
    pub fn visible(&self) -> Vec<&Todo> {
        // A vista do lixo é um modo (`L`), não um valor de `Filter`: as
        // entradas saem de `Store::trash_entries()`, da mais recente para a
        // mais antiga — e essa ordem não depende do `sort`.
        if matches!(self.mode, InputMode::Trash) {
            return self
                .store
                .trash_entries()
                .into_iter()
                .map(|entrada| &entrada.todo)
                .collect();
        }
        let ids = match self.filter {
            // Pendentes primeiro, concluídas no fim: dois grupos, cada um pela
            // ordem escolhida (o `view` é estável). A categoria atravessa as
            // **duas** chamadas: com `pendentes de Trabalho` o eixo vale nos dois
            // grupos, e passar `Todas` a uma delas fazia as concluídas de outra
            // categoria aparecerem na lista.
            Filter::All => {
                let mut ids = self.store.view(
                    Filter::Active,
                    self.sort,
                    self.category_filter.clone(),
                    &self.query,
                );
                ids.extend(self.store.view(
                    Filter::Done,
                    self.sort,
                    self.category_filter.clone(),
                    &self.query,
                ));
                ids
            }
            // Estes dois filtros já são um grupo homogéneo: não há o que dividir.
            filter => self
                .store
                .view(filter, self.sort, self.category_filter.clone(), &self.query),
        };
        ids.iter().filter_map(|id| self.store.find(id)).collect()
    }

    /// Tarefa selecionada, na vista.
    #[must_use]
    pub fn selected(&self) -> Option<&Todo> {
        let todos = self.visible();
        self.list_state.selected().and_then(|i| todos.get(i).copied())
    }

    /// Traduz a tecla e aplica a acção. Devolve a acção, para os testes e para
    /// o registo do T7.
    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        let action = map_key(&self.mode, key);
        self.handle(action);
        action
    }

    /// Expira a mensagem de acção ao fim de [`MESSAGE_TTL`] (§4).
    ///
    /// O instante entra por parâmetro — não se lê o relógio aqui dentro — para
    /// o prazo poder ser testado sem TTY. O `tick` **não** grava nem marca nada
    /// sujo: a gravação continua a ser só do [`App::handle`] e o redesenho do
    /// loop, que desenha no topo de cada iteração.
    pub fn tick(&mut self, agora: Instant) {
        let expirada = match &self.status {
            Status::Message { desde, .. } => agora.saturating_duration_since(*desde) >= MESSAGE_TTL,
            // O undo e o aviso de transbordo ficam; o erro sai só com `Esc`.
            Status::Idle | Status::Sticky(_) | Status::Error(_) => false,
        };
        if expirada {
            self.status = Status::Idle;
        }
    }

    /// Aplica uma acção. É o único ponto que toca na `Db`.
    pub fn handle(&mut self, action: Action) {
        // A guarda tem **alvo** (ADR §6): só a acção que confirma aquilo que está
        // armado a mantém armada; qualquer outra desarma (§5, mudança 7). A regra
        // não pode ser «tudo excepto `EmptyTrash`» — com o `d` da caixa a valer
        // uma segunda pressão, ele desarmava a guarda que ele próprio acabara de
        // armar e a eliminação nunca chegava a confirmar-se. O `Ignore` também não
        // desarma: um `Release`/`Repeat` ou uma tecla não mapeada não pode desfazer
        // o que o utilizador acabou de armar.
        if action != Action::Ignore && !self.mesmo_alvo_armado(action) {
            self.armado = None;
        }

        if self.mode.is_text() {
            self.handle_text(action);
            return;
        }

        match action {
            Action::Quit => self.quitting = true,
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::Home => {
                // Na caixa de categorias, topo é a primeira entrada; a lista
                // não é tocada (o mapa da caixa manda `g`/`G` para aqui).
                if matches!(self.mode, InputMode::Category) {
                    self.categoria_cursor = 0;
                } else {
                    self.select_index(0);
                }
            }
            Action::End => {
                if matches!(self.mode, InputMode::Category) {
                    self.categoria_cursor = self.entradas_da_caixa().saturating_sub(1);
                } else {
                    let ultimo = self.visible().len().saturating_sub(1);
                    self.select_index(ultimo);
                }
            }
            Action::AddStart => self.open_line(InputMode::Adding, String::new(), false),
            Action::EditStart => {
                if let Some(titulo) = self.selected().map(|todo| todo.title.clone()) {
                    self.open_line(InputMode::Editing, titulo, false);
                } else {
                    self.status = Status::error("Erro: não há nenhuma tarefa selecionada");
                }
            }
            Action::EditDescriptionStart => {
                if let Some(descricao) = self.selected().map(|todo| todo.description.clone()) {
                    self.open_line(InputMode::Editing, descricao, true);
                } else {
                    self.status = Status::error("Erro: não há nenhuma tarefa selecionada");
                }
            }
            Action::Toggle => self.toggle_selected(),
            Action::ToggleAll => self.toggle_all(),
            Action::Trash => self.trash_selected(),
            Action::Restore => {
                if matches!(self.mode, InputMode::Trash) {
                    self.restore_selected();
                } else {
                    self.restore_last_batch();
                }
            }
            Action::TrashView => self.toggle_trash_view(),
            Action::ClearCompleted => self.clear_completed(),
            Action::EmptyTrash => self.confirm_or_empty_trash(),
            Action::CycleSort => {
                // A ordem em si é `core::query::SortKey`; o que este cartão
                // faz é ligar-lhe a tecla (até aqui era um no-op declarado) e
                // redesenhar a vista pela nova ordem.
                self.sort = self.sort.next();
                self.clamp_selection();
            }
            Action::CycleFilter => {
                self.filter = next_filter(self.filter);
                self.clamp_selection();
            }
            Action::SearchStart => self.open_line(InputMode::Search, String::new(), false),
            Action::ImportStart => self.open_line(InputMode::Import, String::new(), false),
            Action::ExportStart => self.open_line(InputMode::Export, String::new(), false),
            Action::DismissMessage => self.dismiss(),
            Action::Help => {
                self.return_mode = self.mode;
                self.mode = InputMode::Help;
            }
            Action::InputCancel => {
                // A ajuda e a caixa de categorias voltam para a vista de onde
                // abriram; do lixo sai-se sempre para a lista.
                self.mode = match self.mode {
                    InputMode::Help | InputMode::Category => self.return_mode,
                    _ => InputMode::Normal,
                };
                self.return_mode = InputMode::Normal;
            }
            Action::ThemeView => self.abrir_caixa_de_temas(),
            Action::ThemeMove(delta) => self.mover_na_caixa_de_temas(delta),
            Action::ThemeConfirm => self.confirmar_tema(),
            Action::ThemeCancel => self.cancelar_tema(),
            Action::CategoryView => self.abrir_caixa_de_categorias(),
            Action::CategoryMove(delta) => self.mover_na_caixa_de_categorias(delta),
            Action::CategoryAssign => self.atribuir_categoria(),
            Action::CategoryNew => self.abrir_linha_do_nome(false),
            Action::CategoryRename => self.abrir_linha_do_nome(true),
            Action::CategoryDelete => self.confirmar_ou_eliminar_categoria(),
            Action::CycleCategoryFilter => self.ciclar_filtro_de_categoria(),
            Action::SetPriority(priority) => self.set_priority(priority),
            // `Input`/`InputConfirm` só existem em modos de texto (tratados
            // acima) e `Ignore` é a ausência de acção.
            Action::Input(_) | Action::InputConfirm | Action::Ignore => {}
        }
    }

    /// Efeito das acções quando há uma linha de texto aberta.
    fn handle_text(&mut self, action: Action) {
        match action {
            Action::Quit => self.quitting = true,
            Action::Input(key) => self.edit_buffer(key),
            Action::InputConfirm => self.confirm_line(),
            Action::InputCancel => {
                // Cancelar o nome devolve à **caixa** (é de lá que a linha
                // abriu), como o cancelar de uma tarefa devolve à lista: a caixa
                // mantém o cursor onde estava (§C.2).
                if matches!(self.mode, InputMode::CategoryName) {
                    self.voltar_a_caixa();
                } else {
                    self.close_line();
                }
            }
            _ => {}
        }
    }

    /// A acção é a **confirmação** daquilo que está armado (mesmo `d` da caixa,
    /// mesmo `c` do lixo)?
    ///
    /// É esta pergunta — e não uma lista de excepções — que decide o desarme:
    /// qualquer acção que não confirme o alvo armado desarma-o, e o alvo armado
    /// não é desarmado pela própria tecla que o confirmou.
    fn mesmo_alvo_armado(&self, action: Action) -> bool {
        let Some((armado, _)) = &self.armado else {
            return false;
        };
        match armado {
            Guarda::EsvaziarLixo => action == Action::EmptyTrash,
            Guarda::EliminarCategoria(_) => action == Action::CategoryDelete,
        }
    }

    // ------------------------------------------------------------ navegação

    fn move_selection(&mut self, delta: isize) {
        if self.visible().is_empty() {
            self.list_state.select(None);
            return;
        }
        let actual = self.list_state.selected().unwrap_or(0) as isize;
        let ultimo = self.visible().len() as isize - 1;
        self.select_index((actual + delta).clamp(0, ultimo) as usize);
    }

    fn select_index(&mut self, indice: usize) {
        if self.visible().is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(indice.min(self.visible().len() - 1)));
        }
    }

    /// Põe a seleção na linha da tarefa indicada, se ela estiver na vista.
    ///
    /// É o `a` a cumprir o que a §5 promete: a seleção segue o `TodoId`
    /// devolvido por `Store::add`, e não o índice final, porque com a ordem
    /// activa a tarefa nova pode não ser a última da vista. A lista rola até à
    /// seleção por si: o desenho recalcula a janela visível a partir dela.
    fn select_todo(&mut self, id: &TodoId) {
        let posicao = self.visible().iter().position(|todo| todo.id == *id);
        match posicao {
            Some(indice) => self.list_state.select(Some(indice)),
            // A tarefa nova pode estar escondida pelo filtro activo (p. ex.
            // `concluídas`): a seleção não pode ficar pendurada fora da vista.
            None => self.clamp_selection(),
        }
    }

    /// A seleção nunca pode ficar pendurada fora da vista: depois de remover,
    /// filtrar ou mudar de vista, volta para dentro dos limites.
    fn clamp_selection(&mut self) {
        let len = self.visible().len();
        if len == 0 {
            self.list_state.select(None);
            return;
        }
        match self.list_state.selected() {
            Some(i) if i < len => {}
            Some(_) => self.list_state.select(Some(len - 1)),
            None => self.list_state.select(Some(0)),
        }
    }

    fn selected_id(&self) -> Option<TodoId> {
        self.selected().map(|todo| todo.id.clone())
    }

    // -------------------------------------------------------------- acções

    /// Erro de uma operação que mexe na lista: escreve a faixa da linha 22 e
    /// volta a pôr a seleção dentro dos limites.
    ///
    /// O `clamp` não é decoração. Quando a gravação falha, a mutação já foi
    /// aplicada **em memória** (é o «estado sujo mantido para nova tentativa»
    /// da adenda 2 do ADR): uma remoção encurta a lista na mesma e, sem isto, a
    /// seleção ficava pendurada fora da vista — a partir daí a TUI respondia
    /// «não há nenhuma tarefa selecionada» a tudo, mesmo com linhas no ecrã.
    fn falha(&mut self, err: OpsError) {
        self.status = erro(err, self.store.path());
        self.clamp_selection();
    }

    fn toggle_selected(&mut self) {
        let Some(id) = self.selected_id() else {
            self.status = Status::error("Erro: não há nenhuma tarefa selecionada");
            return;
        };
        let titulo = self
            .store
            .find(&id)
            .map(|todo| todo.title.clone())
            .unwrap_or_default();
        match self.store.toggle_done(&id) {
            Ok(true) => {
                self.status = Status::message(format!("Concluída «{titulo}»"));
                self.clamp_selection();
            }
            Ok(false) => {
                self.status = Status::message(format!("Reaberta «{titulo}»"));
                self.clamp_selection();
            }
            Err(err) => self.falha(err),
        }
    }

    /// `t`: conclui todas, ou reabre todas se já estiverem concluídas.
    ///
    /// Faz N gravações (uma por tarefa) porque o `core::ops` não tem
    /// `set_all_done` — é a operação mais cara da TUI e o sítio natural para
    /// um `ops` novo (ver notas do T5).
    fn toggle_all(&mut self) {
        if self.store.todos().is_empty() {
            self.status = Status::message("Sem tarefas para alternar");
            return;
        }
        let concluir = !self.store.todos().iter().all(|todo| todo.is_done());
        let ids: Vec<TodoId> = self
            .store
            .todos()
            .iter()
            .map(|todo| todo.id.clone())
            .collect();
        for id in &ids {
            if let Err(err) = self.store.set_done(id, concluir) {
                self.falha(err);
                return;
            }
        }
        let total = ids.len();
        self.status = if concluir {
            Status::message(format!("{total} tarefas concluídas"))
        } else {
            Status::message(format!("{total} tarefas reabertas"))
        };
        self.clamp_selection();
    }

    fn set_priority(&mut self, priority: Priority) {
        let Some(id) = self.selected_id() else {
            self.status = Status::error("Erro: não há nenhuma tarefa selecionada");
            return;
        };
        let titulo = self
            .store
            .find(&id)
            .map(|todo| todo.title.clone())
            .unwrap_or_default();
        match self.store.set_priority(&id, priority) {
            Ok(()) => {
                self.status = Status::message(format!(
                    "Prioridade {} em «{titulo}»",
                    priority.label()
                ));
                self.clamp_selection();
            }
            Err(err) => self.falha(err),
        }
    }

    fn trash_selected(&mut self) {
        let Some(id) = self.selected_id() else {
            self.status = Status::error("Erro: não há nenhuma tarefa selecionada");
            return;
        };
        let titulo = self
            .store
            .find(&id)
            .map(|todo| todo.title.clone())
            .unwrap_or_default();
        match self.store.remove(&id) {
            Ok(0) => {
                // A mensagem de remoção fica enquanto houver undo (§4).
                self.status = Status::sticky(format!("Removida «{titulo}»    {UNDO_HINT}"));
                self.clamp_selection();
            }
            Ok(saidas) => {
                // Transbordo: nada sai em silêncio, e o aviso também não expira
                // — é o único caminho em que algo saiu sem o utilizador pedir.
                self.status = Status::sticky(format!(
                    "Aviso: {saidas} entradas antigas saíram do lixo (limite {})  ·  L ver o lixo",
                    crate::core::TRASH_LIMIT
                ));
                self.clamp_selection();
            }
            Err(err) => self.falha(err),
        }
    }

    fn clear_completed(&mut self) {
        match self.store.clear_completed() {
            Ok(resultado) if resultado.removidos == 0 => {
                self.status = Status::message("Não há concluídas para limpar");
            }
            Ok(resultado) if resultado.saidas_do_lixo > 0 => {
                // O aviso de transbordo ganha à mensagem da acção: é o único
                // caminho em que algo saiu sem o utilizador pedir — e fica.
                self.status = Status::sticky(format!(
                    "Aviso: {} entradas antigas saíram do lixo (limite {})  ·  L ver o lixo",
                    resultado.saidas_do_lixo,
                    crate::core::TRASH_LIMIT
                ));
                self.clamp_selection();
            }
            Ok(resultado) => {
                // «u para desfazer»: enquanto o lote estiver no lixo, fica (§4).
                self.status = Status::sticky(format!(
                    "{} concluídas para o lixo  ·  u para desfazer",
                    resultado.removidos
                ));
                self.clamp_selection();
            }
            Err(err) => self.falha(err),
        }
    }

    fn restore_last_batch(&mut self) {
        let quando = self
            .store
            .trash_entries()
            .first()
            .map(|entrada| entrada.deleted_at);
        match self.store.restore_last_batch() {
            Ok(n) => {
                let verbo = if n == 1 { "Reposta" } else { "Repostas" };
                let lote = quando.map_or_else(String::new, |fim| {
                    format!(" (lote de {})", fim.format("%H:%M"))
                });
                let seguinte = if self.store.can_restore() {
                    "  ·  u repõe o lote anterior"
                } else {
                    ""
                };
                let texto = format!("{verbo} {n} tarefas{lote}{seguinte}");
                // Com outro lote por repor a mensagem anuncia um undo: fica
                // enquanto ele existir; sem isso é uma mensagem de acção.
                self.status = if seguinte.is_empty() {
                    Status::message(texto)
                } else {
                    Status::sticky(texto)
                };
                self.clamp_selection();
            }
            Err(OpsError::NothingToRestore) => {
                self.status = Status::message("Lixo vazio  ·  nada para repor");
            }
            Err(err) => self.falha(err),
        }
    }

    /// Repõe a entrada **selecionada** do lixo (o `Enter` da vista do lixo).
    ///
    /// A reposição em si é `core::ops::Store::restore(id)` — o `index` original
    /// e a posição de destino são regra do `core`, testada sem terminal (a
    /// lacuna que o T5 declarou aqui foi fechada pelo T3b).
    fn restore_selected(&mut self) {
        let alvo = self
            .list_state
            .selected()
            .and_then(|i| self.store.trash_entries().get(i).copied());
        let Some(entrada) = alvo else {
            self.status = Status::message("Lixo vazio  ·  nada para repor");
            return;
        };
        let id = entrada.todo.id.clone();
        let titulo = entrada.todo.title.clone();

        match self.store.restore(&id) {
            Ok(posicao) => {
                self.status = Status::message(format!(
                    "Restaurada «{titulo}» para a posição {} da lista",
                    posicao + 1
                ));
            }
            Err(err) => self.falha(err),
        }
        self.clamp_selection();
    }

    /// Guarda de duas pressões do `c` no lixo: a primeira arma, a segunda
    /// esvazia. É a única operação da v1 sem undo (§4).
    fn confirm_or_empty_trash(&mut self) {
        let total = self.store.trash_len();
        if total == 0 {
            self.status = Status::message("Lixo vazio  ·  nada para esvaziar");
            return;
        }
        if self.armed() {
            self.empty_trash();
        } else {
            self.armado = Some((Guarda::EsvaziarLixo, Instant::now() + ARM_WINDOW));
            self.status = Status::message(format!(
                "Esvaziar o lixo? {total} entradas, sem volta atrás  ·  c outra vez confirma"
            ));
        }
    }

    /// Esvazia o lixo (a segunda pressão do `c`).
    ///
    /// O trabalho é `core::ops::Store::empty_trash()`, que já grava e devolve
    /// quantas entradas saíram — a TUI só compõe a mensagem. `trash_dropped`
    /// fica como está: é um contador acumulado do que o limite deitou fora,
    /// não do que está no lixo.
    fn empty_trash(&mut self) {
        match self.store.empty_trash() {
            Ok(entradas) => {
                self.status = Status::message(format!("Lixo esvaziado ({entradas} entradas)"));
            }
            Err(err) => self.falha(err),
        }
        self.armado = None;
        self.clamp_selection();
    }

    fn toggle_trash_view(&mut self) {
        self.mode = if matches!(self.mode, InputMode::Trash) {
            InputMode::Normal
        } else {
            InputMode::Trash
        };
        self.clamp_selection();
    }

    /// `Esc` em repouso: primeiro tira a mensagem, depois limpa busca, filtro e
    /// categoria. Nunca sai — sair é só `q` ou `Ctrl+C` (§5, mudança 1).
    fn dismiss(&mut self) {
        if self.status == Status::Idle {
            self.query.clear();
            self.filter = Filter::All;
            // O eixo da categoria é limpo com os outros dois (ADR §8): o `Esc`
            // em repouso põe a lista inteira, não «quase toda».
            self.category_filter = CategoryFilter::Todas;
            self.clamp_selection();
        } else {
            self.status = Status::Idle;
        }
    }

    // ------------------------------------------------ caixa de temas (§Decisão 9)

    /// `T` — abre a caixa no tema que está aplicado.
    ///
    /// O cursor abre na posição do tema em uso (e não na primeira do catálogo):
    /// a caixa mostra onde se está, e o `Enter` sem navegar não muda nada. O
    /// modo de onde a caixa abriu fica guardado para ela saber onde voltar, como
    /// na ajuda.
    ///
    /// Em `ansi` o tema em uso é o `classico` (o [`ModoCor::resolver`] já o
    /// trocou no arranque), logo é ele que fica debaixo do cursor — e é o que
    /// um `Enter` sem navegar grava, porque é o que está a ser desenhado
    /// (ADR §Adenda A1.3).
    fn abrir_caixa_de_temas(&mut self) {
        self.theme_entrada = self.theme;
        self.theme_cursor = posicao_no_catalogo(self.theme);
        self.return_mode = self.mode;
        self.mode = InputMode::Theme;
    }

    /// `j`/`k`/`↓`/`↑` na caixa: move o cursor e aplica logo o tema candidato.
    ///
    /// A pré-visualização é o próprio [`App::theme`] a apontar para o candidato
    /// — não há cópia da paleta nem um segundo estado de cor. O cursor não dá a
    /// volta: para na primeira e na última posição, como a seleção da lista.
    /// Nada aqui toca no disco: gravar é só o `Enter`.
    ///
    /// O candidato passa pelo [`ModoCor::resolver`] como o tema do arranque
    /// (ADR §Adenda A1.4): em `ansi`, um tema com fundo dá o `classico`, e é
    /// isso que o ecrã e a caixa desenham. Sem esta passagem, navegar num
    /// terminal de 16 cores escrevia `48;2;…` e `38;2;…` — medido — com a
    /// própria caixa a dizer «modo de cor: ansi». O [`App::theme_cursor`]
    /// continua a marcar a **escolha** (é o que o `Enter` grava): em `ansi`
    /// pode ser um Tokyo Night, pedido para quando houver truecolor.
    fn mover_na_caixa_de_temas(&mut self, delta: isize) {
        if !matches!(self.mode, InputMode::Theme) {
            return;
        }
        let ultimo = CATALOGO.len() as isize - 1;
        let novo = (self.theme_cursor as isize + delta).clamp(0, ultimo) as usize;
        self.theme_cursor = novo;
        self.theme = self.modo_cor.resolver(&CATALOGO[novo]);
    }

    /// `Enter` — grava a preferência e fecha.
    ///
    /// O *slug* gravado é o do **cursor** — a escolha, que em `ansi` pode ser um
    /// tema Tokyo Night pedido para quando houver truecolor — e o tema aplicado
    /// é esse mesmo candidato passado pelo [`ModoCor::resolver`] (ADR §Adenda
    /// A1.4): sem isso, o `Enter` num terminal de 16 cores punha o ecrã a
    /// escrever RGB depois de a caixa fechar, com o `ansi` a valer só até à
    /// primeira tecla.
    ///
    /// Sem caminho de config conhecido (o [`App`] dos testes, ou um sistema sem
    /// `XDG_CONFIG_HOME` nem `HOME`) fecha com o tema aplicado **sem** tentar
    /// gravar: não se inventa um caminho no `~/.config` real.
    ///
    /// Falhar a gravar não é fatal (ADR §Decisão 8): o tema fica nesta sessão, a
    /// linha 22 diz **onde** não se gravou, a lista não é tocada e não há
    /// `panic!`. É a mesma política da falha a gravar o `db.json`.
    fn confirmar_tema(&mut self) {
        if !matches!(self.mode, InputMode::Theme) {
            return;
        }
        let escolhido = CATALOGO[self.theme_cursor];
        self.theme = self.modo_cor.resolver(&CATALOGO[self.theme_cursor]);
        self.fechar_caixa_de_temas();

        let Some(caminho) = self.config_path.clone() else {
            // Sem ficheiro não há o que gravar nem o que dizer: a escolha vale
            // nesta sessão e o próximo arranque volta ao que o arranque decidir.
            return;
        };
        let preferencia = Config {
            theme: Some(escolhido.slug.to_owned()),
        };
        match preferencia.save(&caminho) {
            Ok(()) => self.status = Status::message(format!("Tema «{}» gravado", escolhido.nome)),
            Err(_) => {
                self.status = Status::error(format!(
                    "Erro: não gravou {} — tema aplicado só nesta sessão",
                    caminho.display()
                ));
            }
        }
    }

    /// `Esc`/`q` — fecha e repõe **exactamente** o tema de entrada.
    ///
    /// É a outra metade da §Decisão 9: o que se viu a navegar não fica, e o
    /// ficheiro não é tocado porque nada foi gravado. `q` faz o mesmo que o
    /// `Esc` — «fechar sem gravar» não pode deixar uma pré-visualização aplicada
    /// que ninguém pediu para guardar. (`Ctrl+C` sai da aplicação, como em todos
    /// os modos.)
    fn cancelar_tema(&mut self) {
        if !matches!(self.mode, InputMode::Theme) {
            return;
        }
        self.theme = self.theme_entrada;
        self.fechar_caixa_de_temas();
    }

    /// Fecha a caixa e volta ao modo de onde ela abriu (`T` só existe na lista,
    /// mas a volta fica pelo mesmo caminho da ajuda).
    fn fechar_caixa_de_temas(&mut self) {
        self.mode = self.return_mode;
        self.return_mode = InputMode::Normal;
        self.theme_cursor = posicao_no_catalogo(self.theme);
    }

    // ------------------------------------- caixa de categorias (§C do @designer)

    /// Número de linhas da caixa: `sem categoria` mais as categorias.
    ///
    /// É sempre ≥ 1 — o `None` é uma linha como as outras (§C.1) —, logo o
    /// cursor nunca precisa de um `Option`: ao contrário da seleção da lista,
    /// «sem categorias» não é «sem linha nenhuma».
    fn entradas_da_caixa(&self) -> usize {
        self.store.categories().len() + 1
    }

    /// A categoria debaixo do cursor — `None` quando ele está em `sem categoria`
    /// (a 1.ª linha), que não é uma categoria e não se renomeia nem se elimina.
    fn categoria_no_cursor(&self) -> Option<&Category> {
        self.store
            .categories()
            .get(self.categoria_cursor.checked_sub(1)?)
    }

    /// O cursor da caixa nunca fica pendurado fora das entradas — molde do
    /// [`App::clamp_selection`], para a caixa.
    ///
    /// Sem isto, eliminar a categoria debaixo do cursor deixava o índice a
    /// apontar para uma linha que já não existe: o `Enter` a seguir atribuía a
    /// categoria errada (ou nenhuma) sem o utilizador perceber porquê.
    fn clamp_categoria_cursor(&mut self) {
        let ultimo = self.entradas_da_caixa().saturating_sub(1);
        if self.categoria_cursor > ultimo {
            self.categoria_cursor = ultimo;
        }
    }

    /// `C` — abre a caixa de categorias.
    ///
    /// O cursor abre **na primeira linha** (`sem categoria`), que é o que o
    /// frame `80x24-15-categorias` fixa (§C.5): a linha de estado diz o que a
    /// tarefa selecionada tem agora, e é isso que impede atribuir o que já lá
    /// está. Abre-se mesmo sem tarefas (ADR §7) — é a única forma de criar
    /// categorias numa base vazia.
    ///
    /// Só na lista: na vista do lixo o mapa não tem `C` (§C.2), e esta guarda
    /// mantém a promessa mesmo para quem chame a acção por outro caminho.
    fn abrir_caixa_de_categorias(&mut self) {
        if !matches!(self.mode, InputMode::Normal) {
            return;
        }
        self.categoria_cursor = 0;
        self.return_mode = self.mode;
        self.mode = InputMode::Category;
    }

    /// Fecha a caixa e volta à vista de onde ela abriu — sem atribuir nada.
    fn fechar_caixa_de_categorias(&mut self) {
        self.mode = self.return_mode;
        self.return_mode = InputMode::Normal;
        self.clamp_categoria_cursor();
        self.clamp_selection();
    }

    /// `j`/`k`/`↓`/`↑` na caixa: move o cursor, que **para** nos extremos (a
    /// lista não roda em ciclo, como a das temas).
    fn mover_na_caixa_de_categorias(&mut self, delta: isize) {
        if !matches!(self.mode, InputMode::Category) {
            return;
        }
        let ultimo = self.entradas_da_caixa() as isize - 1;
        self.categoria_cursor = (self.categoria_cursor as isize + delta).clamp(0, ultimo) as usize;
    }

    /// `Enter` na caixa: atribui a categoria do cursor à tarefa selecionada e
    /// fecha — ou **tira** a atribuição, se o cursor estiver em `sem categoria`
    /// (§C.2).
    ///
    /// Uma atribuição que não muda nada **não é gravada**: o ficheiro fica
    /// exactamente como estava e a mensagem di-lo («já estava em…»). Gravar o
    /// mesmo por cima escrevia o `db.json` e o `.bak` sem nada ter mudado — e a
    /// linha de estado da caixa existe precisamente para se ver o que já lá está.
    fn atribuir_categoria(&mut self) {
        if !matches!(self.mode, InputMode::Category) {
            return;
        }
        let Some(tarefa) = self.selected_id() else {
            // A caixa abre sem tarefas (ADR §7): o `Enter` não tem alvo e di-lo.
            self.status = Status::error("Erro: não há nenhuma tarefa selecionada");
            return;
        };
        let alvo = self
            .categoria_no_cursor()
            .map(|categoria| categoria.id.clone());
        let nome = self
            .categoria_no_cursor()
            .map(|categoria| nome_na_mensagem(&categoria.name));
        let actual = self
            .store
            .find(&tarefa)
            .and_then(|todo| todo.category_id.clone());

        if actual == alvo {
            self.fechar_caixa_de_categorias();
            self.status = Status::message(match nome {
                Some(nome) => format!("Já estava em «{nome}»"),
                None => "Já estava sem categoria".to_owned(),
            });
            return;
        }
        match self.store.assign_category(&tarefa, alvo.as_ref()) {
            Ok(()) => {
                self.fechar_caixa_de_categorias();
                self.status = Status::message(match nome {
                    Some(nome) => format!("Atribuída a «{nome}»"),
                    None => "Atribuição de categoria removida".to_owned(),
                });
            }
            Err(err) => self.falha(err),
        }
    }

    /// `a` (criar) e `e` (renomear): abre a linha do nome, na linha 22, **por
    /// cima da caixa**.
    ///
    /// Em `sem categoria` o `e` não faz nada: o `None` não se renomeia (§C.2), e
    /// é por isso que o rodapé de lá não anuncia o `e`. Em `e` o buffer abre com
    /// o nome actual — o cursor da caixa fica na linha que se vai gravar.
    fn abrir_linha_do_nome(&mut self, renomear: bool) {
        if !matches!(self.mode, InputMode::Category) {
            return;
        }
        let inicial = if renomear {
            match self.categoria_no_cursor() {
                Some(categoria) => categoria.name.clone(),
                None => return,
            }
        } else {
            String::new()
        };
        self.renomear_categoria = renomear;
        self.open_line(InputMode::CategoryName, inicial, false);
        // A linha abriu de dentro da caixa: é para lá que o `Enter` e o `Esc`
        // voltam (o `return_mode` é o que o faz, como na ajuda e nas temas).
        self.return_mode = InputMode::Category;
    }

    /// Volta à caixa depois de gravar (ou de cancelar) o nome, com o cursor onde
    /// estava (§C.2: a caixa não perde o cursor no modo de texto).
    fn voltar_a_caixa(&mut self) {
        self.mode = self.return_mode;
        self.return_mode = InputMode::Normal;
        self.buffer.clear();
        self.cursor = 0;
        self.editing_description = false;
        self.renomear_categoria = false;
        self.clamp_categoria_cursor();
    }

    /// `d` na caixa: a primeira pressão arma a guarda (5 s), a segunda elimina.
    ///
    /// Em `sem categoria` — e sem categorias nenhumas — não faz nada: não há o
    /// que eliminar. A guarda é **daquela** categoria: armar numa e carregar em
    /// `d` noutra re-arma na nova, e nunca elimina a que não foi anunciada.
    fn confirmar_ou_eliminar_categoria(&mut self) {
        if !matches!(self.mode, InputMode::Category) {
            return;
        }
        let Some(id) = self.categoria_no_cursor().map(|c| c.id.clone()) else {
            return;
        };
        if self.categoria_armada() == Some(&id) {
            self.eliminar_categoria(&id);
        } else {
            self.armado = Some((Guarda::EliminarCategoria(id), Instant::now() + ARM_WINDOW));
        }
    }

    /// Elimina a categoria (a segunda pressão do `d`).
    ///
    /// O trabalho é `Store::delete_category`, que tira a atribuição das tarefas
    /// da lista **e do lixo** e devolve quantas ficaram sem categoria: a TUI
    /// compõe a mensagem e diz o preço (§C.4), nunca o esconde.
    ///
    /// Se a categoria eliminada era a do filtro, o filtro volta a `Todas`: senão
    /// a lista aparecia vazia sem explicação (ADR §8 e §C.3).
    fn eliminar_categoria(&mut self, id: &CategoryId) {
        let nome = self
            .store
            .category(id)
            .map(|categoria| categoria.name.clone())
            .unwrap_or_default();
        match self.store.delete_category(id) {
            Ok(afectadas) => {
                if self.category_filter.id() == Some(id) {
                    self.category_filter = CategoryFilter::Todas;
                }
                self.armado = None;
                self.clamp_categoria_cursor();
                self.clamp_selection();
                self.status = Status::message(mensagem_eliminada(&nome, afectadas));
            }
            Err(err) => self.falha(err),
        }
    }

    /// `F` em repouso: cicla o filtro de categoria — `todas → sem categoria →
    /// <as categorias, pela ordem de inserção> → todas` (ADR §8, §C.9).
    ///
    /// O ciclo é sobre as categorias que existem **agora**: uma categoria
    /// eliminada não deixa o filtro pendurado nela (a comparação com a lista é o
    /// que o repõe em `todas`, mesmo que a eliminação tenha vindo de outro
    /// caminho).
    fn ciclar_filtro_de_categoria(&mut self) {
        if !matches!(self.mode, InputMode::Normal) {
            return;
        }
        let categorias: Vec<CategoryId> = self
            .store
            .categories()
            .iter()
            .map(|categoria| categoria.id.clone())
            .collect();
        self.category_filter = match &self.category_filter {
            CategoryFilter::Todas => CategoryFilter::SemCategoria,
            CategoryFilter::SemCategoria => categorias
                .first()
                .cloned()
                .map_or(CategoryFilter::Todas, CategoryFilter::Uma),
            CategoryFilter::Uma(id) => categorias
                .iter()
                .position(|candidata| candidata == id)
                .and_then(|posicao| categorias.get(posicao + 1))
                .cloned()
                .map_or(CategoryFilter::Todas, CategoryFilter::Uma),
        };
        self.clamp_selection();
    }

    /// Erro de nome, na linha de estado da caixa (§C.4): diz **porquê** e o
    /// texto escrito **não se perde** (a linha continua aberta).
    ///
    /// A mensagem cita o nome **escrito** e não o que lá está: o frame
    /// `80x24-19-categorias-erro` tem `trabalho` no buffer e «trabalho» no erro,
    /// embora a categoria existente se chame `Trabalho` — a colisão ignora as
    /// maiúsculas (ADR §2) e o que se cita é o que o utilizador escreveu.
    fn erro_de_nome(&mut self, err: OpsError, escrito: &str) {
        self.status = match err {
            OpsError::EmptyName => Status::error("Erro: o nome não pode ficar vazio"),
            OpsError::DuplicateName(_) => Status::error(format!(
                "Erro: já existe uma categoria chamada «{}»",
                nome_na_mensagem(escrito)
            )),
            outro => erro(outro, self.store.path()),
        };
    }

    // -------------------------------------------------------- linha de texto

    fn open_line(&mut self, mode: InputMode, texto: String, descricao: bool) {
        self.mode = mode;
        self.buffer = texto.chars().collect();
        self.cursor = self.buffer.len();
        self.editing_description = descricao;
    }

    fn close_line(&mut self) {
        self.mode = InputMode::Normal;
        self.buffer.clear();
        self.cursor = 0;
        self.editing_description = false;
    }

    /// Teclas de edição da linha (§5: `Backspace`/`Delete`/`←`/`→`/`Home`/
    /// `End` editam, `Ctrl+U` limpa a linha, o resto é texto no cursor).
    fn edit_buffer(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('u') {
                self.buffer.clear();
                self.cursor = 0;
            }
            return;
        }

        match key.code {
            KeyCode::Char(c) => {
                self.buffer.insert(self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.buffer.remove(self.cursor - 1);
                    self.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.buffer.len() {
                    self.buffer.remove(self.cursor);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.buffer.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.buffer.len(),
            _ => {}
        }
    }

    fn confirm_line(&mut self) {
        let texto: String = self.input();
        match self.mode {
            InputMode::Adding => {
                if texto.trim().is_empty() {
                    // Fica na linha e diz porquê: o `Enter` em vazio não pode
                    // desaparecer em silêncio.
                    self.status = Status::error("Erro: o título não pode estar vazio");
                    return;
                }
                match self.store.add(&texto) {
                    Ok(id) => {
                        self.close_line();
                        // A seleção segue o id que a `Db` acabou de devolver, e
                        // não o índice final (§5).
                        self.select_todo(&id);
                        self.status = Status::message(format!("Adicionada «{}»", texto.trim()));
                    }
                    Err(err) => self.falha(err),
                }
            }
            InputMode::Editing => {
                let Some(id) = self.selected_id() else {
                    self.close_line();
                    return;
                };
                let resultado = if self.editing_description {
                    self.store.edit_description(&id, &texto)
                } else if texto.trim().is_empty() {
                    Err(OpsError::EmptyTitle)
                } else {
                    self.store.edit_title(&id, &texto)
                };
                match resultado {
                    Ok(()) => {
                        self.close_line();
                        self.status = Status::message("Tarefa editada");
                    }
                    Err(err) => self.falha(err),
                }
            }
            InputMode::Search => {
                self.query = texto.trim().to_owned();
                self.close_line();
                self.clamp_selection();
                let resultados = self.visible().len();
                self.status = if self.query.is_empty() {
                    Status::Idle
                } else {
                    Status::message(format!("{resultados} resultados"))
                };
            }
            InputMode::Import => {
                let caminho = texto.trim().to_owned();
                self.close_line();
                if caminho.is_empty() {
                    return;
                }
                match import_from_path(&mut self.store, Path::new(&caminho)) {
                    Ok(relatorio) => {
                        let prefixo = if relatorio.sem_alteracoes() {
                            "Importação sem alterações"
                        } else {
                            "Importado"
                        };
                        self.status = Status::message(format!("{prefixo}: {}", relatorio.resumo()));
                        self.clamp_selection();
                    }
                    Err(err) => self.status = Status::error(format!("Erro: {err}")),
                }
            }
            InputMode::Export => {
                let caminho = texto.trim().to_owned();
                self.close_line();
                if caminho.is_empty() {
                    return;
                }
                // A `Db` inteira e não só as tarefas: o nome da categoria vive
                // nas categorias (T3).
                match export_csv_to_path(self.store.db(), Path::new(&caminho)) {
                    Ok(n) => {
                        self.status =
                            Status::message(format!("Exportadas {n} tarefas para {caminho}"));
                    }
                    Err(err) => self.status = Status::error(format!("Erro: {err}")),
                }
            }
            InputMode::CategoryName => {
                if self.renomear_categoria {
                    let Some(id) = self.categoria_no_cursor().map(|c| c.id.clone()) else {
                        // A categoria do cursor deixou de existir: fecha a linha
                        // sem inventar um alvo (pela caixa não acontece).
                        self.voltar_a_caixa();
                        return;
                    };
                    let antigo = self
                        .store
                        .category(&id)
                        .map(|categoria| categoria.name.clone())
                        .unwrap_or_default();
                    match self.store.rename_category(&id, &texto) {
                        Ok(()) => {
                            self.voltar_a_caixa();
                            self.status = Status::message(format!(
                                "Renomeada «{}» para «{}»",
                                nome_na_mensagem(&antigo),
                                nome_na_mensagem(&texto)
                            ));
                        }
                        Err(err) => self.erro_de_nome(err, &texto),
                    }
                } else {
                    match self.store.add_category(&texto) {
                        Ok(_id) => {
                            self.voltar_a_caixa();
                            self.status =
                                Status::message(format!("Criada «{}»", nome_na_mensagem(&texto)));
                        }
                        Err(err) => self.erro_de_nome(err, &texto),
                    }
                }
            }
            // Normal, Help e Trash não têm linha (o `map_key` não abre nenhuma).
            _ => self.close_line(),
        }
    }
}

/// Mensagem de erro a partir de um erro do `core` (e do caminho da base de
/// dados, que é o que o utilizador procura).
///
/// O prefixo «Erro:» é do desenho (§4: o vermelho nunca aparece sozinho) e um
/// erro de gravação diz sempre duas coisas: **onde** não se gravou e que a
/// lista em memória não perdeu nada — é o que o golden `80x24-6-erro` fixa. O
/// `Display` do erro continua a existir para quem o queira em `stderr`
/// (arranque recusado, T7); na linha 22 o que cabe é o caminho e a garantia.
fn erro(err: OpsError, caminho: &Path) -> Status {
    if let OpsError::Persist(_) = &err {
        return Status::error(format!(
            "Erro: não gravou {} — lista intacta",
            caminho.display()
        ));
    }
    Status::error(format!("Erro: {err}"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use chrono::Local;
    use crossterm::event::{KeyEventKind, KeyModifiers};

    use super::*;

    fn app(tag: &str, titulos: &[&str]) -> (PathBuf, App) {
        let dir = std::env::temp_dir().join(format!(
            "todo-ratatui-app-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("criar diretório de teste");
        let path = dir.join("db.json");
        let mut store = Store::open(&path).expect("abrir store");
        for titulo in titulos {
            store.add(titulo).expect("adicionar");
        }
        (path, App::new(store))
    }

    fn carrega(app: &mut App, codigo: KeyCode) {
        app.on_key(KeyEvent::new(codigo, KeyModifiers::NONE));
    }

    fn tecla(app: &mut App, c: char) {
        carrega(app, KeyCode::Char(c));
    }

    fn escreve(app: &mut App, texto: &str) {
        for c in texto.chars() {
            tecla(app, c);
        }
    }

    fn entra(app: &mut App) {
        carrega(app, KeyCode::Enter);
    }

    fn escapa(app: &mut App) {
        carrega(app, KeyCode::Esc);
    }

    /// Ordem da `Db` (não da vista): é o que os critérios do T5 medem.
    fn na_db(app: &App) -> Vec<String> {
        app.store().todos().iter().map(|t| t.title.clone()).collect()
    }

    fn na_vista(app: &App) -> Vec<String> {
        app.visible().iter().map(|t| t.title.clone()).collect()
    }

    fn concluir(app: &mut App, titulo: &str) {
        let id = app
            .store()
            .todos()
            .iter()
            .find(|t| t.title == titulo)
            .expect("tarefa existe")
            .id
            .clone();
        app.store.toggle_done(&id).expect("concluir");
    }

    /// Um `App` com categorias já criadas — pela porta do `core`, que é a única
    /// que o `App` usa (`add_category`).
    fn app_com_categorias(
        tag: &str,
        titulos: &[&str],
        categorias: &[&str],
    ) -> (PathBuf, App, Vec<CategoryId>) {
        let (path, mut app) = app(tag, titulos);
        let ids = categorias
            .iter()
            .map(|nome| app.store.add_category(nome).expect("criar categoria"))
            .collect();
        (path, app, ids)
    }

    /// O `id` da tarefa com este título, na `Db`.
    fn id_de(app: &App, titulo: &str) -> TodoId {
        app.store()
            .todos()
            .iter()
            .find(|t| t.title == titulo)
            .expect("tarefa existe")
            .id
            .clone()
    }

    /// Atribui a categoria à tarefa, pela porta do `core` (o caminho do `App`).
    fn atribui(app: &mut App, titulo: &str, categoria: &CategoryId) {
        let id = id_de(app, titulo);
        app.store
            .assign_category(&id, Some(categoria))
            .expect("atribuir");
    }

    /// Nomes das categorias, pela ordem de inserção.
    fn categorias_na_db(app: &App) -> Vec<String> {
        app.store()
            .categories()
            .iter()
            .map(|categoria| categoria.name.clone())
            .collect()
    }

    #[test]
    fn d_seguido_de_u_repoe_a_tarefa() {
        let (_path, mut app) = app("d-u", &["a", "b", "c"]);
        assert_eq!(app.selected().expect("há selecionada").title, "a");

        tecla(&mut app, 'j');
        assert_eq!(app.selected().unwrap().title, "b");

        tecla(&mut app, 'd');
        assert_eq!(na_db(&app), ["a", "c"]);
        assert_eq!(app.store().trash_len(), 1);
        assert!(
            app.status.text().unwrap().contains("Removida «b»"),
            "a mensagem diz o que saiu: {:?}",
            app.status
        );

        tecla(&mut app, 'u');
        assert_eq!(na_db(&app), ["a", "b", "c"], "o undo repõe no índice original");
        assert_eq!(app.store().trash_len(), 0);
        assert!(app.status.text().unwrap().contains("Reposta 1 tarefa"));
    }

    #[test]
    fn abrir_adding_e_cancelar_nao_altera_a_db() {
        let (path, mut app) = app("adding-cancel", &["a"]);
        let antes = fs::read(&path).expect("ler ficheiro");

        tecla(&mut app, 'a');
        assert_eq!(app.mode, InputMode::Adding);
        escreve(&mut app, "café");
        assert_eq!(app.input(), "café");

        escapa(&mut app);
        assert_eq!(app.mode, InputMode::Normal);
        assert_eq!(app.input(), "");
        assert_eq!(na_db(&app), ["a"], "cancelar não cria nada");
        assert_eq!(
            fs::read(&path).expect("reler ficheiro"),
            antes,
            "e não escreve uma única vez no ficheiro"
        );
    }

    #[test]
    fn c_sobre_tres_concluidas_vai_para_o_lixo_e_u_repoe_nos_indices_originais() {
        let (_path, mut app) = app(
            "clear-completed",
            &["feita 1", "por fazer", "feita 2", "feita 3"],
        );
        for titulo in ["feita 1", "feita 2", "feita 3"] {
            concluir(&mut app, titulo);
        }

        tecla(&mut app, 'c');
        assert_eq!(na_db(&app), ["por fazer"]);
        assert_eq!(app.store().trash_len(), 3, "as três num só lote");
        let indices: Vec<usize> = app.store().db().trash.iter().map(|e| e.index).collect();
        assert_eq!(
            indices,
            [3, 2, 0],
            "removidas de trás para a frente: cada índice é o original"
        );

        tecla(&mut app, 'u');
        assert_eq!(
            na_db(&app),
            ["feita 1", "por fazer", "feita 2", "feita 3"],
            "um `u` repõe o lote inteiro nas posições originais"
        );
        assert_eq!(app.store().trash_len(), 0);
    }

    #[test]
    fn adicionar_grava_a_tarefa() {
        let (path, mut app) = app("adding", &[]);
        tecla(&mut app, 'a');
        escreve(&mut app, "Comprar café em grão");
        entra(&mut app);

        assert_eq!(app.mode, InputMode::Normal);
        assert_eq!(na_db(&app), ["Comprar café em grão"]);
        assert_eq!(app.selected().unwrap().title, "Comprar café em grão");

        let reaberto = Store::open(&path).expect("reabrir");
        assert_eq!(reaberto.todos().len(), 1, "gravou na confirmação");
    }

    #[test]
    fn linha_de_texto_tem_cursor() {
        let (_path, mut app) = app("cursor", &[]);
        tecla(&mut app, 'a');
        escreve(&mut app, "abc");
        assert_eq!(app.cursor(), 3);

        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
        assert_eq!(app.input(), "abXc", "escreve na posição do cursor");
        assert_eq!(app.cursor(), 3);

        app.on_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.input(), "abX");
        app.on_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.input(), "abX", "no início não há nada para apagar");

        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.input(), "", "Ctrl+U limpa a linha");
    }

    #[test]
    fn esc_em_repouso_tira_a_mensagem_e_depois_limpa_busca_e_filtro() {
        let (_path, mut app) = app("esc", &["café", "netflix"]);
        tecla(&mut app, 'f');
        assert_eq!(app.filter, Filter::Active);

        tecla(&mut app, '/');
        escreve(&mut app, "café");
        entra(&mut app);
        assert_eq!(app.mode, InputMode::Normal);
        assert_eq!(app.query, "café");
        assert_eq!(na_vista(&app), ["café"]);

        tecla(&mut app, 'd'); // deixa mensagem de acção
        assert!(app.status.text().is_some());

        escapa(&mut app);
        assert_eq!(app.status, Status::Idle, "o primeiro `Esc` tira a mensagem");
        assert_eq!(app.query, "café", "e não toca na busca");
        assert_eq!(app.filter, Filter::Active);

        escapa(&mut app);
        assert!(app.query.is_empty(), "o segundo limpa a busca");
        assert_eq!(app.filter, Filter::All);
        assert!(!app.should_quit(), "`Esc` nunca sai da aplicação");
    }

    #[test]
    fn filtro_nunca_entra_na_vista_do_lixo() {
        let (_path, mut app) = app("ciclo-filtro", &[]);
        let mut vistos = Vec::new();
        for _ in 0..4 {
            tecla(&mut app, 'f');
            vistos.push(app.filter);
        }
        assert_eq!(
            vistos,
            [Filter::Active, Filter::Done, Filter::All, Filter::Active],
            "o ciclo é All → Active → Done; a vista do lixo é o `L`"
        );
    }

    #[test]
    fn esvaziar_o_lixo_pede_duas_pressoes() {
        let (_path, mut app) = app("lixo-guarda", &["a", "b"]);
        tecla(&mut app, 'd');
        tecla(&mut app, 'd');
        assert_eq!(app.store().trash_len(), 2);

        tecla(&mut app, 'L');
        assert_eq!(app.mode, InputMode::Trash);
        assert_eq!(na_vista(&app), ["b", "a"], "o lixo mostra a remoção recente primeiro");

        tecla(&mut app, 'c');
        assert_eq!(app.store().trash_len(), 2, "a primeira pressão só arma");
        assert!(app.armed());
        assert!(app.status.text().unwrap().contains("c outra vez confirma"));

        tecla(&mut app, 'j'); // qualquer outra acção desarma
        assert!(!app.armed());

        tecla(&mut app, 'c');
        assert!(app.armed());
        tecla(&mut app, 'c');
        assert_eq!(app.store().trash_len(), 0, "a segunda pressão esvazia");
        assert!(!app.armed());
        assert!(app.status.text().unwrap().contains("Lixo esvaziado"));
    }

    #[test]
    fn enter_no_lixo_repoe_a_selecionada_na_posicao_original() {
        let (_path, mut app) = app("lixo-restaurar", &["a", "b", "c", "d"]);
        tecla(&mut app, 'j'); // "b"
        tecla(&mut app, 'd');
        tecla(&mut app, 'j'); // "c" -> "d"
        tecla(&mut app, 'd');
        assert_eq!(na_db(&app), ["a", "c"]);

        tecla(&mut app, 'L');
        assert_eq!(na_vista(&app), ["d", "b"]);
        tecla(&mut app, 'j'); // seleciona "b"
        assert_eq!(app.selected().unwrap().title, "b");
        entra(&mut app);

        assert_eq!(na_db(&app), ["a", "b", "c"], "«b» volta ao índice 1");
        assert_eq!(app.store().trash_len(), 1, "«d» fica no lixo");
        assert!(app.status.text().unwrap().contains("posição 2"));
        assert_eq!(app.mode, InputMode::Trash, "restaurar não sai da vista do lixo");
    }

    #[test]
    fn prioridade_e_ordem() {
        let (_path, mut app) = app("ordem", &["primeira", "segunda"]);
        tecla(&mut app, '1');
        assert_eq!(
            app.store().todos()[0].priority,
            Priority::High,
            "«1» é prioridade alta na selecionada"
        );
        assert_eq!(na_vista(&app), ["primeira", "segunda"]);

        // `s` é no-op até o `core::query::SortKey` entrar (T3b): a vista fica
        // pela ordem natural, e é isso que este teste prende.
        tecla(&mut app, 's');
        assert_eq!(na_vista(&app), ["primeira", "segunda"]);
    }

    #[test]
    fn alternar_todas_vai_e_volta() {
        let (_path, mut app) = app("alternar-todas", &["a", "b"]);
        tecla(&mut app, 't');
        assert!(app.store().todos().iter().all(Todo::is_done));
        tecla(&mut app, 't');
        assert!(app.store().todos().iter().all(|todo| !todo.is_done()));
    }

    #[test]
    fn selecao_nunca_fica_fora_da_lista() {
        let (_path, mut app) = app("selecao", &["a", "b"]);
        tecla(&mut app, 'j');
        assert_eq!(app.list_state.selected(), Some(1));

        tecla(&mut app, 'd'); // remove a última
        assert_eq!(na_db(&app), ["a"]);
        assert_eq!(app.list_state.selected(), Some(0), "a seleção volta para dentro");

        tecla(&mut app, 'd'); // remove a única
        assert!(na_db(&app).is_empty());
        assert_eq!(app.list_state.selected(), None);
        tecla(&mut app, 'j');
        assert_eq!(app.list_state.selected(), None, "sem lista não há seleção");
    }

    #[test]
    fn exportar_e_importar_por_caminho() {
        // A variável chama-se `tui` (e não `app`) porque o helper deste módulo
        // se chama `app`: sem isso, o segundo `app(…)` era uma chamada a um
        // `App`.
        let (path, mut tui) = app("import-export", &["a", "b"]);
        let dir = path.parent().expect("directório da base de dados").to_path_buf();
        let csv = dir.join("tarefas.csv");

        tecla(&mut tui, 'x');
        assert_eq!(tui.mode, InputMode::Export);
        escreve(&mut tui, &csv.to_string_lossy());
        entra(&mut tui);
        assert!(csv.is_file(), "o export escreveu o ficheiro");
        assert!(tui.status.text().unwrap().contains("Exportadas 2 tarefas"));

        let (_outro_path, mut outro) = app("import-destino", &[]);
        tecla(&mut outro, 'i');
        assert_eq!(outro.mode, InputMode::Import);
        escreve(&mut outro, &csv.to_string_lossy());
        entra(&mut outro);
        assert_eq!(na_db(&outro), ["a", "b"], "o import traz as duas tarefas");
        assert!(outro.status.text().unwrap().contains("Importado"));
    }

    #[test]
    fn ajuda_volta_para_a_vista_de_onde_veio() {
        let (_path, mut app) = app("ajuda", &["a"]);
        tecla(&mut app, 'L');
        assert_eq!(app.mode, InputMode::Trash);

        tecla(&mut app, '?');
        assert_eq!(app.mode, InputMode::Help);
        escapa(&mut app);
        assert_eq!(app.mode, InputMode::Trash, "a ajuda não fecha a vista do lixo");

        escapa(&mut app);
        assert_eq!(app.mode, InputMode::Normal, "e o `Esc` seguinte volta à lista");
    }

    #[test]
    fn q_sai_e_esc_nao() {
        let (_path, mut app) = app("sair", &["a"]);
        escapa(&mut app);
        assert!(!app.should_quit());
        tecla(&mut app, 'q');
        assert!(app.should_quit());
    }

    #[test]
    fn release_e_repeat_nao_chegam_a_mexer_na_db() {
        let (_path, mut app) = app("release", &["a"]);
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            app.on_key(KeyEvent::new_with_kind(
                KeyCode::Char('d'),
                KeyModifiers::NONE,
                kind,
            ));
        }
        assert_eq!(na_db(&app), ["a"]);
        assert_eq!(app.store().trash_len(), 0);
    }

    /// A gravação falhada não pode deixar a TUI num estado em que já não
    /// responde: a mutação fica em memória, a faixa diz o que se passou e o
    /// ficheiro não é tocado.
    #[test]
    fn gravacao_falhada_mostra_a_faixa_e_nao_deixa_a_selecao_pendurada() {
        use std::os::unix::fs::PermissionsExt;

        let (path, mut app) = app("gravacao-falhada", &["a", "b", "c"]);
        let dir = path
            .parent()
            .expect("directório da base de dados")
            .to_path_buf();
        let antes = fs::read(&path).expect("ler o ficheiro");

        // O `save` cria o `db.json.tmp` ao lado do `db.json`: sem escrita no
        // directório, a gravação falha antes de tocar no ficheiro bom.
        let mut perm = fs::metadata(&dir).expect("metadata").permissions();
        perm.set_mode(0o500);
        fs::set_permissions(&dir, perm.clone()).expect("tirar a escrita ao directório");

        tecla(&mut app, 'j');
        tecla(&mut app, 'j');
        tecla(&mut app, 'd');

        perm.set_mode(0o700);
        fs::set_permissions(&dir, perm).expect("devolver a escrita ao directório");

        let texto = app.status.text().expect("há mensagem").to_owned();
        assert!(app.status.is_error(), "devia ser erro: {texto}");
        assert!(
            texto.contains("não gravou") && texto.contains("lista intacta"),
            "a faixa diz onde não se gravou e que a lista está intacta: {texto}"
        );
        assert_eq!(
            na_vista(&app),
            ["a", "b"],
            "a mutação fica em memória (é o estado sujo a repetir)"
        );
        assert_eq!(
            app.list_state.selected(),
            Some(1),
            "a seleção volta para dentro dos limites"
        );
        assert_eq!(
            fs::read(&path).expect("reler o ficheiro"),
            antes,
            "o ficheiro em disco não foi tocado"
        );

        // E a TUI continua a responder à linha que está selecionada.
        tecla(&mut app, '1');
        assert_eq!(app.store().todos()[1].priority, Priority::High);
    }

    /// `a` + `Enter`: a seleção segue o id que o `add` devolveu, mesmo quando a
    /// tarefa nova **não** é a última da vista (aqui entra a meio, entre uma
    /// pendente e uma concluída).
    #[test]
    fn a_selecao_passa_para_a_tarefa_nova() {
        let (_path, mut app) = app("a-seleciona", &["primeira", "segunda"]);
        concluir(&mut app, "primeira");
        assert_eq!(na_vista(&app), ["segunda", "primeira"]);

        tecla(&mut app, 'a');
        escreve(&mut app, "café");
        entra(&mut app);

        assert_eq!(
            na_vista(&app),
            ["segunda", "café", "primeira"],
            "a nova entra entre a pendente e a concluída"
        );
        assert_eq!(
            app.selected().map(|todo| todo.title.as_str()),
            Some("café"),
            "e é ela que fica selecionada"
        );
        assert_eq!(
            app.list_state.selected(),
            Some(1),
            "a seleção é a linha dela, não o fim da lista"
        );
    }

    /// A mensagem de acção sai sozinha ao fim de 3 s (§4) — sem esperar pela
    /// tecla seguinte.
    #[test]
    fn a_mensagem_de_accao_expira_aos_tres_segundos() {
        let (_path, mut app) = app("tick-mensagem", &["a", "b"]);
        let t0 = Instant::now();

        tecla(&mut app, 't'); // «2 tarefas concluídas»
        assert!(app.status.text().is_some(), "a acção deixa mensagem");

        app.tick(t0 + MESSAGE_TTL - Duration::from_millis(500));
        assert!(
            app.status.text().is_some(),
            "dentro do prazo a mensagem fica: {:?}",
            app.status
        );

        app.tick(t0 + MESSAGE_TTL + Duration::from_millis(200));
        assert_eq!(
            app.status,
            Status::Idle,
            "aos 3 s a mensagem de acção sai sozinha"
        );
    }

    /// O undo disponível e o aviso de transbordo são `sticky`: ficam (§4).
    #[test]
    fn o_undo_nao_expira() {
        let (_path, mut app) = app("tick-sticky", &["a", "b"]);
        let t0 = Instant::now();

        tecla(&mut app, 'd');
        assert!(
            app.status
                .text()
                .is_some_and(|texto| texto.contains(UNDO_HINT)),
            "a remoção deixa a mensagem do undo: {:?}",
            app.status
        );
        app.tick(t0 + Duration::from_secs(600));
        assert!(
            app.status
                .text()
                .is_some_and(|texto| texto.contains("Removida")),
            "enquanto houver undo a mensagem fica: {:?}",
            app.status
        );
    }

    /// O lixo cheio: a remoção seguinte faz transbordar as entradas antigas e o
    /// aviso é permanente (§4).
    #[test]
    fn o_aviso_de_transbordo_nao_expira() {
        use crate::core::{TRASH_LIMIT, Trashed};

        // A variável chama-se `tui` e não `app` porque o helper deste módulo se
        // chama `app`: sem isso, a segunda chamada era a um `App` (E0618).
        let (_path, mut tui) = app("tick-transbordo", &["unica"]);
        {
            let agora = Local::now();
            let db = tui.store.db_mut();
            db.trash = (0..TRASH_LIMIT)
                .map(|i| Trashed {
                    todo: Todo::try_new(format!("antiga {i}")).expect("título"),
                    index: i,
                    deleted_at: agora,
                    batch: 0,
                })
                .collect();
        }
        let t0 = Instant::now();

        tecla(&mut tui, 'd');
        assert_eq!(tui.store().trash_len(), TRASH_LIMIT, "o lixo fica cheio");
        assert!(
            tui.status
                .text()
                .is_some_and(|texto| texto.contains("Aviso: 1 entradas")),
            "nada sai em silêncio: {:?}",
            tui.status
        );

        tui.tick(t0 + Duration::from_secs(600));
        assert!(
            tui.status
                .text()
                .is_some_and(|texto| texto.contains("Aviso:")),
            "o aviso de transbordo é permanente: {:?}",
            tui.status
        );
    }

    /// O `tick` não é um segundo caminho de escrita: expira a mensagem e mais
    /// nada (não marca nada sujo, não grava).
    #[test]
    fn o_tick_nao_toca_nos_dados_nem_no_ficheiro() {
        let (path, mut app) = app("tick-sem-gravacao", &["a", "b"]);
        tecla(&mut app, 't');
        let antes = fs::read(&path).expect("ler o ficheiro");
        let lista = na_db(&app);

        app.tick(Instant::now() + Duration::from_secs(600));

        assert_eq!(app.status, Status::Idle);
        assert_eq!(na_db(&app), lista, "o `tick` não mexe na lista");
        assert_eq!(
            fs::read(&path).expect("reler o ficheiro"),
            antes,
            "nem grava: a gravação continua reservada ao `App::handle`"
        );
    }

    // ------------------------------------------ caixa de categorias (§C do T4)

    /// Critério 2: `C` abre a caixa e, com ela aberta, as teclas da lista **não**
    /// fazem o que fazem na lista — a mais perigosa é o `d`.
    #[test]
    fn c_abre_a_caixa_e_com_ela_aberta_a_lista_nao_governa() {
        let (_path, mut app, _ids) = app_com_categorias("c-abre", &["a", "b"], &["Trabalho"]);
        let (filtro, ordem) = (app.filter, app.sort);
        let prioridade = app.store().todos()[0].priority;
        let titulos = na_db(&app);

        tecla(&mut app, 'C');
        assert_eq!(app.mode, InputMode::Category);
        assert_eq!(
            app.categoria_cursor, 0,
            "abre na 1.ª linha, que é `sem categoria` (§C.5)"
        );

        // O `d` da caixa arma a eliminação da categoria; não manda a tarefa
        // selecionada para o lixo (a lista está à vista por baixo).
        tecla(&mut app, 'd');
        assert_eq!(na_db(&app), titulos, "nenhuma tarefa foi para o lixo");
        assert_eq!(app.store().trash_len(), 0);
        assert_eq!(
            app.categoria_armada(),
            None,
            "o cursor está em `sem categoria`: não há o que armar"
        );
        assert_eq!(app.mode, InputMode::Category, "a caixa fica aberta");

        for tecla_da_lista in ['s', 'f', '1', '2', '3', 't', 'c', 'u', 'L', '?'] {
            tecla(&mut app, tecla_da_lista);
        }
        assert_eq!(app.sort, ordem, "«s» não ciclou a ordem");
        assert_eq!(app.filter, filtro, "«f» não ciclou o estado");
        assert_eq!(app.store().todos()[0].priority, prioridade, "«1» não mexeu");
        assert_eq!(app.store().trash_len(), 0, "«c» não limpou nada");
        assert_eq!(na_db(&app), titulos);
        assert_eq!(app.mode, InputMode::Category, "e a caixa continua aberta");
    }

    /// Critério 3: `j`/`k`/`↓`/`↑` movem o cursor e **não passam dos limites**,
    /// nos dois extremos.
    #[test]
    fn o_cursor_da_caixa_move_e_nao_passa_dos_limites() {
        let (_path, mut app, _ids) = app_com_categorias("cursor-caixa", &[], &["A", "B"]);
        tecla(&mut app, 'C');
        assert_eq!(app.categoria_cursor, 0);

        tecla(&mut app, 'k');
        assert_eq!(app.categoria_cursor, 0, "no princípio fica");
        carrega(&mut app, KeyCode::Up);
        assert_eq!(app.categoria_cursor, 0);

        tecla(&mut app, 'j');
        assert_eq!(app.categoria_cursor, 1);
        carrega(&mut app, KeyCode::Down);
        assert_eq!(app.categoria_cursor, 2, "«B» é a última das 3 entradas");
        tecla(&mut app, 'j');
        assert_eq!(app.categoria_cursor, 2, "no fim fica");
        carrega(&mut app, KeyCode::Down);
        assert_eq!(app.categoria_cursor, 2);

        // `g`/`G`: topo e fim (§C.2), e a lista não se mexe.
        tecla(&mut app, 'g');
        assert_eq!(app.categoria_cursor, 0);
        tecla(&mut app, 'G');
        assert_eq!(app.categoria_cursor, 2);
        assert_eq!(app.list_state.selected(), None, "a lista não tem nada");
    }

    /// Critério 4: `Enter` numa categoria grava a atribuição **e fecha**.
    #[test]
    fn enter_atribui_a_categoria_do_cursor_e_fecha() {
        let (path, mut app, ids) =
            app_com_categorias("enter-atribui", &["a", "b"], &["Trabalho", "Casa"]);
        tecla(&mut app, 'C');
        tecla(&mut app, 'j'); // «Trabalho»
        assert_eq!(app.categoria_cursor, 1);

        entra(&mut app);
        assert_eq!(app.mode, InputMode::Normal, "o `Enter` fecha a caixa");
        assert_eq!(app.store().todos()[0].category_id.as_ref(), Some(&ids[0]));
        assert_eq!(app.store().todos()[1].category_id, None, "só a selecionada");

        let reaberto = Store::open(&path).expect("reabrir");
        assert_eq!(
            reaberto.todos()[0].category_id.as_ref(),
            Some(&ids[0]),
            "gravou na confirmação"
        );
    }

    /// Critério 4: `Enter` em `sem categoria` **tira** a atribuição.
    #[test]
    fn enter_em_sem_categoria_tira_a_atribuicao() {
        let (path, mut app, ids) = app_com_categorias("enter-tira", &["a"], &["Trabalho"]);
        atribui(&mut app, "a", &ids[0]);

        tecla(&mut app, 'C');
        assert_eq!(app.categoria_cursor, 0, "abre em `sem categoria`");
        entra(&mut app);

        assert_eq!(app.mode, InputMode::Normal);
        assert_eq!(app.store().todos()[0].category_id, None);
        assert_eq!(
            Store::open(&path).expect("reabrir").todos()[0].category_id,
            None,
            "a remoção da atribuição chegou ao ficheiro"
        );
    }

    /// Critério 4: os dois fechos que **não** atribuem nada deixam a `Db` igual —
    /// nem uma gravação a mais.
    #[test]
    fn fechar_a_caixa_sem_mudar_nada_nao_toca_no_ficheiro() {
        let (path, mut app, _ids) = app_com_categorias("fecha-sem-gravar", &["a"], &["Trabalho"]);
        let antes = fs::read(&path).expect("ler o ficheiro");

        // (1) `Enter` em `sem categoria` numa tarefa que já está sem categoria.
        tecla(&mut app, 'C');
        entra(&mut app);
        assert_eq!(app.mode, InputMode::Normal);
        assert_eq!(
            fs::read(&path).expect("reler o ficheiro"),
            antes,
            "atribuir o que já lá estava não escreve nada"
        );
        assert!(app.status.text().unwrap().contains("Já estava"));

        // (2) `Esc` com o cursor numa categoria: fecha sem atribuir.
        tecla(&mut app, 'C');
        tecla(&mut app, 'j');
        escapa(&mut app);
        assert_eq!(app.mode, InputMode::Normal, "o `Esc` fecha a caixa");
        assert_eq!(app.store().todos()[0].category_id, None);
        assert_eq!(fs::read(&path).expect("reler o ficheiro"), antes);
    }

    /// Critério 5: `a` → linha de texto → `Enter` cria a categoria e **volta à
    /// caixa**.
    #[test]
    fn a_na_caixa_cria_uma_categoria_e_volta_a_caixa() {
        let (_path, mut app, _ids) = app_com_categorias("a-cria", &["a"], &[]);
        tecla(&mut app, 'C');
        tecla(&mut app, 'a');
        assert_eq!(app.mode, InputMode::CategoryName);
        assert!(
            !app.renomeando_categoria(),
            "`a` cria; é o `e` que renomeia"
        );

        escreve(&mut app, "Trabalho");
        entra(&mut app);

        assert_eq!(app.mode, InputMode::Category, "voltou à caixa");
        assert_eq!(categorias_na_db(&app), ["Trabalho"]);
        assert_eq!(na_db(&app), ["a"], "não criou nenhuma tarefa");
        assert!(app.status.text().unwrap().contains("Criada «Trabalho»"));
    }

    /// Critério 5: nome vazio → erro visível, nada criado, e a linha continua
    /// aberta com o que lá estava (o texto não se perde).
    #[test]
    fn nome_vazio_da_erro_e_a_linha_continua_aberta() {
        let (_path, mut app, _ids) = app_com_categorias("nome-vazio", &["a"], &[]);
        tecla(&mut app, 'C');
        tecla(&mut app, 'a');
        entra(&mut app);

        assert_eq!(app.mode, InputMode::CategoryName, "a linha fica aberta");
        assert!(app.status.is_error());
        assert_eq!(
            app.status.text().unwrap(),
            "Erro: o nome não pode ficar vazio"
        );
        assert!(categorias_na_db(&app).is_empty(), "nada foi criado");

        // E o que se escreve a seguir não perde nada.
        escreve(&mut app, "Trabalho");
        entra(&mut app);
        assert_eq!(categorias_na_db(&app), ["Trabalho"]);
    }

    /// Critério 5: nome repetido (`Trabalho` vs `trabalho`) → erro **e** nada é
    /// criado; o texto escrito fica no buffer e o ficheiro não é tocado.
    #[test]
    fn nome_repetido_ignora_maiusculas_e_nao_perde_o_texto() {
        let (path, mut app, _ids) = app_com_categorias("nome-repetido", &["a"], &["Trabalho"]);
        let antes = fs::read(&path).expect("ler o ficheiro");

        tecla(&mut app, 'C');
        tecla(&mut app, 'a');
        escreve(&mut app, "trabalho");
        entra(&mut app);

        assert_eq!(categorias_na_db(&app), ["Trabalho"], "nada foi criado");
        assert_eq!(app.mode, InputMode::CategoryName, "a linha fica aberta");
        assert_eq!(app.input(), "trabalho", "o texto escrito não se perde");
        assert_eq!(
            app.status.text().unwrap(),
            "Erro: já existe uma categoria chamada «trabalho»"
        );
        assert_eq!(
            fs::read(&path).expect("reler o ficheiro"),
            antes,
            "a recusa não escreve no ficheiro"
        );

        // `Esc` cancela e volta à caixa (o buffer não é gravado).
        escapa(&mut app);
        assert_eq!(app.mode, InputMode::Category);
        assert_eq!(categorias_na_db(&app), ["Trabalho"]);
    }

    /// Critério 6: `e` renomeia mantendo o `id` — a atribuição das tarefas não
    /// muda (asserção sobre a `Db`).
    #[test]
    fn e_renomeia_mantendo_o_id_e_a_atribuicao() {
        let (path, mut app, ids) = app_com_categorias("renomear", &["a"], &["Trabalho"]);
        atribui(&mut app, "a", &ids[0]);

        tecla(&mut app, 'C');
        tecla(&mut app, 'j'); // «Trabalho»
        tecla(&mut app, 'e');
        assert_eq!(app.mode, InputMode::CategoryName);
        assert!(app.renomeando_categoria(), "é o `e`, não o `a`");
        assert_eq!(app.input(), "Trabalho", "o buffer abre com o nome actual");

        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        escreve(&mut app, "Casa");
        entra(&mut app);

        assert_eq!(app.mode, InputMode::Category, "voltou à caixa");
        assert_eq!(categorias_na_db(&app), ["Casa"]);
        assert_eq!(app.store().categories()[0].id, ids[0], "o id não mudou");
        assert_eq!(
            app.store().todos()[0].category_id.as_ref(),
            Some(&ids[0]),
            "a atribuição continua a apontar para a mesma categoria"
        );
        let reaberto = Store::open(&path).expect("reabrir");
        assert_eq!(reaberto.categories()[0].name, "Casa");
        assert_eq!(reaberto.todos()[0].category_id.as_ref(), Some(&ids[0]));
        assert!(
            app.status
                .text()
                .unwrap()
                .contains("Renomeada «Trabalho» para «Casa»")
        );
    }

    /// §C.2: em `sem categoria` nem o `e` nem o `d` fazem nada — o `None` não se
    /// renomeia nem se elimina.
    #[test]
    fn em_sem_categoria_o_e_e_o_d_nao_fazem_nada() {
        let (_path, mut app, _ids) = app_com_categorias("sem-categoria", &["a"], &["Trabalho"]);
        tecla(&mut app, 'C');
        tecla(&mut app, 'e');
        assert_eq!(
            app.mode,
            InputMode::Category,
            "`e` em `sem categoria` não abre linha nenhuma"
        );
        tecla(&mut app, 'd');
        assert_eq!(app.categoria_armada(), None);
        assert_eq!(categorias_na_db(&app), ["Trabalho"], "nada foi eliminado");
    }

    /// Critério 7: **a guarda tem alvo**. Armar a eliminação de uma categoria e
    /// disparar o `c` do lixo não esvazia nada; armar o `c` do lixo e disparar o
    /// `d` da caixa deixa o lixo intacto.
    ///
    /// As duas acções chegam pelo `handle` (e não por teclas) porque é o `handle`
    /// que decide o desarme — é lá que vivia o sinalizador partilhado que o ADR
    /// §6 existe para matar.
    #[test]
    fn a_guarda_tem_alvo_o_c_do_lixo_e_o_d_da_caixa_nao_se_confundem() {
        // (1) armada a eliminação da categoria, o `c` do lixo não esvazia nada.
        let (_path, mut app, ids) = app_com_categorias("guarda-lixo", &["a", "b"], &["Trabalho"]);
        tecla(&mut app, 'd'); // «a» vai para o lixo
        assert_eq!(app.store().trash_len(), 1);
        tecla(&mut app, 'C');
        tecla(&mut app, 'j'); // «Trabalho»
        app.handle(Action::CategoryDelete);
        assert_eq!(
            app.categoria_armada(),
            Some(&ids[0]),
            "o `d` armou o alvo dele"
        );

        app.handle(Action::EmptyTrash);
        assert_eq!(app.store().trash_len(), 1, "o lixo não foi esvaziado");
        assert_eq!(categorias_na_db(&app), ["Trabalho"], "nada foi eliminado");
        assert_eq!(
            app.categoria_armada(),
            None,
            "o `c` desarmou a guarda da categoria (outro alvo)"
        );
        assert!(
            app.armed(),
            "e armou a dele: a primeira pressão não esvazia"
        );

        // (2) armado o `c` do lixo, o `d` da caixa deixa o lixo intacto.
        let (_outro, mut app, ids) = app_com_categorias("guarda-caixa", &["a", "b"], &["Trabalho"]);
        tecla(&mut app, 'd');
        tecla(&mut app, 'C');
        tecla(&mut app, 'j');
        app.handle(Action::EmptyTrash);
        assert!(app.armed(), "o `c` armou a guarda do lixo");

        app.handle(Action::CategoryDelete);
        assert_eq!(app.store().trash_len(), 1, "o lixo ficou intacto");
        assert_eq!(categorias_na_db(&app), ["Trabalho"], "e nada foi eliminado");
        assert_eq!(
            app.categoria_armada(),
            Some(&ids[0]),
            "o `d` armou o alvo **dele**"
        );

        // A confirmação do mesmo alvo continua a funcionar (e o lixo continua lá).
        app.handle(Action::CategoryDelete);
        assert!(categorias_na_db(&app).is_empty(), "a segunda `d` elimina");
        assert_eq!(app.store().trash_len(), 1, "o lixo não foi tocado por isso");
    }

    /// Critério 8: eliminar a categoria que está a ser filtrada volta o filtro a
    /// `Todas` e `visible()` devolve a lista inteira.
    #[test]
    fn eliminar_a_categoria_filtrada_volta_o_filtro_a_todas() {
        let (_path, mut app, ids) =
            app_com_categorias("filtro-pendurado", &["a", "b", "c"], &["Trabalho"]);
        atribui(&mut app, "a", &ids[0]);

        tecla(&mut app, 'F'); // sem categoria
        tecla(&mut app, 'F'); // Trabalho
        assert_eq!(app.category_filter, CategoryFilter::Uma(ids[0].clone()));
        assert_eq!(na_vista(&app), ["a"], "só a tarefa da categoria");

        tecla(&mut app, 'C');
        tecla(&mut app, 'j'); // «Trabalho»
        tecla(&mut app, 'd');
        tecla(&mut app, 'd'); // elimina

        assert_eq!(
            app.category_filter,
            CategoryFilter::Todas,
            "o filtro volta a `Todas` (não fica pendurado numa categoria que já não existe)"
        );
        assert_eq!(na_vista(&app), ["a", "b", "c"], "`visible()` devolve tudo");
        assert_eq!(
            app.categoria_cursor, 0,
            "o cursor clampa para a única entrada"
        );
        assert_eq!(
            app.status.text().unwrap(),
            "Eliminada «Trabalho» · 1 tarefa ficou sem categoria"
        );
    }

    /// A eliminação diz **quantas** tarefas ficaram sem categoria — e o `core`
    /// conta também as que estão no lixo (uma entrada do lixo continua atribuída).
    #[test]
    fn a_eliminacao_conta_as_tarefas_das_duas_listas() {
        let (_path, mut app, ids) =
            app_com_categorias("elimina-conta", &["a", "b", "c"], &["Trabalho"]);
        atribui(&mut app, "a", &ids[0]);
        atribui(&mut app, "b", &ids[0]);
        atribui(&mut app, "c", &ids[0]);
        tecla(&mut app, 'd'); // «a» (a selecionada) vai para o lixo, atribuída

        tecla(&mut app, 'C');
        tecla(&mut app, 'j');
        tecla(&mut app, 'd');
        tecla(&mut app, 'd');

        assert_eq!(
            app.status.text().unwrap(),
            "Eliminada «Trabalho» · 3 tarefas ficaram sem categoria"
        );
        assert_eq!(app.store().todos()[0].category_id, None);
        assert_eq!(app.store().todos()[1].category_id, None);
        assert_eq!(
            app.store().db().trash[0].todo.category_id,
            None,
            "o lixo também perde a referência (senão voltava pendurada)"
        );
    }

    /// Critério 9: `F` cicla `Todas → SemCategoria → <categorias, pela ordem de
    /// inserção> → Todas`, sem mexer no `f` (estado).
    #[test]
    fn f_cicla_as_categorias_pela_ordem_de_insercao() {
        let (_path, mut app, ids) =
            app_com_categorias("ciclo-f", &["a"], &["Trabalho", "Casa", "Café"]);
        let estado = app.filter;

        let mut vistos = Vec::new();
        for _ in 0..5 {
            tecla(&mut app, 'F');
            vistos.push(app.category_filter.clone());
        }
        assert_eq!(
            vistos,
            [
                CategoryFilter::SemCategoria,
                CategoryFilter::Uma(ids[0].clone()),
                CategoryFilter::Uma(ids[1].clone()),
                CategoryFilter::Uma(ids[2].clone()),
                CategoryFilter::Todas,
            ]
        );
        assert_eq!(app.filter, estado, "`F` não mexe no estado");
    }

    /// Critério 9: `Esc` em repouso limpa busca **e** filtro de estado **e**
    /// filtro de categoria.
    #[test]
    fn esc_em_repouso_limpa_a_busca_o_estado_e_a_categoria() {
        let (_path, mut app, ids) = app_com_categorias("esc-tudo", &["a", "b"], &["Trabalho"]);
        atribui(&mut app, "a", &ids[0]);

        tecla(&mut app, 'F'); // sem categoria
        tecla(&mut app, 'F'); // Trabalho
        tecla(&mut app, 'f'); // estado: pendentes
        tecla(&mut app, '/');
        escreve(&mut app, "a");
        entra(&mut app);
        assert_eq!(app.category_filter, CategoryFilter::Uma(ids[0].clone()));
        assert_ne!(app.filter, Filter::All);
        assert_eq!(app.query, "a");

        escapa(&mut app); // tira a mensagem dos resultados
        escapa(&mut app); // limpa busca + estado + categoria

        assert!(app.query.is_empty());
        assert_eq!(app.filter, Filter::All);
        assert_eq!(app.category_filter, CategoryFilter::Todas);
        assert!(!app.should_quit());
    }

    /// O eixo da categoria atravessa **as duas** chamadas de `view` do
    /// `Filter::All` (pendentes e concluídas): se só uma o recebesse, a lista
    /// mostrava tarefas de outra categoria.
    #[test]
    fn a_categoria_atravessa_as_duas_chamadas_da_vista() {
        let (_path, mut app, ids) = app_com_categorias(
            "duas-chamadas",
            &["pendente", "feita", "outra", "outra feita"],
            &["Trabalho", "Casa"],
        );
        atribui(&mut app, "pendente", &ids[0]);
        atribui(&mut app, "feita", &ids[0]);
        atribui(&mut app, "outra", &ids[1]);
        atribui(&mut app, "outra feita", &ids[1]);
        concluir(&mut app, "feita");
        concluir(&mut app, "outra feita");
        assert_eq!(
            na_vista(&app),
            ["pendente", "outra", "feita", "outra feita"],
            "pendentes primeiro, concluídas no fim"
        );

        tecla(&mut app, 'F'); // sem categoria
        tecla(&mut app, 'F'); // Trabalho
        assert_eq!(app.filter, Filter::All, "o eixo do estado não mexeu");
        assert_eq!(
            na_vista(&app),
            ["pendente", "feita"],
            "uma de cada grupo — as duas chamadas levam o filtro"
        );
    }

    /// A caixa abre **sem tarefas** (ADR §7) e o `a` funciona lá; o `Enter` não
    /// tem alvo e di-lo, sem fechar a caixa.
    #[test]
    fn a_caixa_abre_sem_tarefas_e_o_a_cria_categorias() {
        let (_path, mut app, _ids) = app_com_categorias("caixa-vazia", &[], &[]);
        tecla(&mut app, 'C');
        assert_eq!(app.mode, InputMode::Category);
        assert_eq!(app.categoria_cursor, 0, "a única entrada é `sem categoria`");

        tecla(&mut app, 'a');
        escreve(&mut app, "Trabalho");
        entra(&mut app);
        assert_eq!(app.mode, InputMode::Category, "voltou à caixa");
        assert_eq!(categorias_na_db(&app), ["Trabalho"]);

        entra(&mut app);
        assert!(app.status.is_error(), "o `Enter` sem alvo não é silencioso");
        assert!(
            app.status
                .text()
                .unwrap()
                .contains("não há nenhuma tarefa selecionada")
        );
        assert_eq!(app.mode, InputMode::Category, "a caixa fica aberta");
    }

    /// Critério 10: eliminar a categoria debaixo do cursor deixa o cursor dentro
    /// dos limites — sem panic e sem apontar para uma entrada que não existe.
    #[test]
    fn eliminar_a_categoria_do_cursor_deixa_o_cursor_dentro_dos_limites() {
        let (_path, mut app, _ids) = app_com_categorias("clamp-cursor", &[], &["A", "B", "C"]);
        tecla(&mut app, 'C');
        tecla(&mut app, 'G');
        assert_eq!(app.categoria_cursor, 3, "a última entrada é «C»");

        tecla(&mut app, 'd');
        tecla(&mut app, 'd');
        assert_eq!(categorias_na_db(&app), ["A", "B"]);
        assert_eq!(
            app.categoria_cursor, 2,
            "o cursor clampa para a última entrada"
        );

        // Continua a apontar para uma categoria a sério (não para o vazio).
        tecla(&mut app, 'e');
        assert_eq!(app.input(), "B");
        escapa(&mut app);

        // Esvaziar a caixa: o cursor para em `sem categoria` (índice 0).
        for _ in 0..2 {
            tecla(&mut app, 'g');
            tecla(&mut app, 'j'); // a 1.ª categoria
            tecla(&mut app, 'd');
            tecla(&mut app, 'd');
        }
        assert!(categorias_na_db(&app).is_empty());
        assert_eq!(app.categoria_cursor, 0);
        assert_eq!(
            app.mode,
            InputMode::Category,
            "a caixa continua a responder"
        );
        tecla(&mut app, 'j');
        assert_eq!(app.categoria_cursor, 0, "só há `sem categoria`");
        tecla(&mut app, 'k');
        assert_eq!(app.categoria_cursor, 0);
    }

    /// §C.2: na vista do lixo as teclas das categorias são **mortas** — não se
    /// abre a caixa por cima do lixo nem se filtra a lista de lá.
    #[test]
    fn no_lixo_nao_se_abre_a_caixa_nem_se_filtra_categoria() {
        let (_path, mut app, _ids) = app_com_categorias("lixo-mortas", &["a"], &["Trabalho"]);
        tecla(&mut app, 'L');
        assert_eq!(app.mode, InputMode::Trash);

        tecla(&mut app, 'C');
        assert_eq!(app.mode, InputMode::Trash, "`C` não abre a caixa no lixo");
        tecla(&mut app, 'F');
        assert_eq!(
            app.category_filter,
            CategoryFilter::Todas,
            "`F` não filtra a partir do lixo"
        );
    }
}
