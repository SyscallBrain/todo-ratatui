//! Estado da aplicação e efeito de cada acção.
//!
//! O [`App`] é a única peça que liga as teclas aos dados: [`map_key`] diz *que*
//! acção a tecla vale no modo activo, e `App::handle` é o único sítio onde essa
//! acção toca no `core`. Nada aqui desenha (o render é do T6) nem lê eventos: o
//! dono do `App` é o loop de `tui::run`.
//!
//! Tudo é testável sem TTY: o [`Store`] dos testes vive num directório
//! temporário e as teclas são `KeyEvent::new(…)`.

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::core::{
    Counts, Filter, OpsError, Priority, SortKey, Store, Todo, TodoId, export_csv_to_path,
    import_from_path,
};

use super::event::{Action, InputMode, map_key};

/// Janela da guarda do `c` no lixo: a segunda pressão tem de vir dentro deste
/// tempo, e qualquer outra acção desarma (§5, mudança 7).
///
/// É a única operação da v1 sem undo por trás — a guarda existe por isso, não
/// por gosto de confirmações.
pub const ARM_WINDOW: Duration = Duration::from_secs(5);

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

/// O que a linha 22 mostra (§4). A precedência — erro > mensagem > descrição do
/// selecionado > vazio — é do T6; aqui só se guarda o que há para mostrar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Idle,
    /// Mensagem de acção («Removida «X»    u para desfazer»).
    Message(String),
    /// Erro: vermelho e com o prefixo «Erro:» no T6. Nunca expira sozinho.
    Error(String),
}

impl Status {
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Message(text) | Self::Error(text) => Some(text),
        }
    }

    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }

    fn error(text: impl Into<String>) -> Self {
        Self::Error(text.into())
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
    buffer: Vec<char>,
    cursor: usize,
    /// Em `Editing`: o `Enter` grava a descrição em vez do título.
    editing_description: bool,
    /// Modo para onde voltar quando a ajuda fechar (a ajuda sobrepõe-se à
    /// vista, e é a única sobreposição que não se fecha sozinha).
    return_mode: InputMode,
    /// Fim da janela da guarda do `c` no lixo.
    armed_until: Option<Instant>,
    quitting: bool,
}

impl App {
    /// Arranca com a lista na ordem por omissão e a primeira tarefa
    /// selecionada.
    #[must_use]
    pub fn new(store: Store) -> Self {
        let mut app = Self {
            store,
            list_state: ListState::default(),
            mode: InputMode::Normal,
            status: Status::Idle,
            filter: Filter::default(),
            query: String::new(),
            sort: SortKey::Priority,
            buffer: Vec::new(),
            cursor: 0,
            editing_description: false,
            return_mode: InputMode::Normal,
            armed_until: None,
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
    #[must_use]
    pub fn armed(&self) -> bool {
        self.armed_until.is_some_and(|fim| Instant::now() < fim)
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
            // ordem escolhida (o `view` é estável).
            Filter::All => {
                let mut ids = self.store.view(Filter::Active, self.sort, &self.query);
                ids.extend(self.store.view(Filter::Done, self.sort, &self.query));
                ids
            }
            // Estes dois filtros já são um grupo homogéneo: não há o que dividir.
            filter => self.store.view(filter, self.sort, &self.query),
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

    /// Aplica uma acção. É o único ponto que toca na `Db`.
    pub fn handle(&mut self, action: Action) {
        // Qualquer acção que não seja o `c` do lixo desarma a guarda (§5,
        // mudança 7) — mas o `Ignore` não: um `Release`/`Repeat` ou uma tecla
        // não mapeada não pode desarmar o que o utilizador acabou de armar.
        if action != Action::EmptyTrash && action != Action::Ignore {
            self.armed_until = None;
        }

        if self.mode.is_text() {
            self.handle_text(action);
            return;
        }

        match action {
            Action::Quit => self.quitting = true,
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::Home => self.select_index(0),
            Action::End => {
                let ultimo = self.visible().len().saturating_sub(1);
                self.select_index(ultimo);
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
                // A ajuda volta para a vista de onde foi aberta; do lixo sai-se
                // sempre para a lista.
                self.mode = if matches!(self.mode, InputMode::Help) {
                    self.return_mode
                } else {
                    InputMode::Normal
                };
                self.return_mode = InputMode::Normal;
            }
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
            Action::InputCancel => self.close_line(),
            _ => {}
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
                self.status = Status::message(format!("Removida «{titulo}»    {UNDO_HINT}"));
                self.clamp_selection();
            }
            Ok(saidas) => {
                self.status = Status::message(format!(
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
                // caminho em que algo saiu sem o utilizador pedir.
                self.status = Status::message(format!(
                    "Aviso: {} entradas antigas saíram do lixo (limite {})  ·  L ver o lixo",
                    resultado.saidas_do_lixo,
                    crate::core::TRASH_LIMIT
                ));
                self.clamp_selection();
            }
            Ok(resultado) => {
                self.status = Status::message(format!(
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
                self.status = Status::message(format!("{verbo} {n} tarefas{lote}{seguinte}"));
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
            self.armed_until = Some(Instant::now() + ARM_WINDOW);
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
        self.armed_until = None;
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

    /// `Esc` em repouso: primeiro tira a mensagem, depois limpa busca e filtro.
    /// Nunca sai — sair é só `q` ou `Ctrl+C` (§5, mudança 1).
    fn dismiss(&mut self) {
        if self.status == Status::Idle {
            self.query.clear();
            self.filter = Filter::All;
            self.clamp_selection();
        } else {
            self.status = Status::Idle;
        }
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
                    Ok(_) => {
                        self.close_line();
                        self.clamp_selection();
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
                match export_csv_to_path(self.store.todos(), Path::new(&caminho)) {
                    Ok(n) => {
                        self.status =
                            Status::message(format!("Exportadas {n} tarefas para {caminho}"));
                    }
                    Err(err) => self.status = Status::error(format!("Erro: {err}")),
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
}
