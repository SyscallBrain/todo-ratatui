//! Tradução de `KeyEvent` em [`Action`] e os modos de entrada do [`App`].
//!
//! A tabela é a do @designer (`design-system/todo-ratatui.md` §5), que é a
//! autoritativa para este módulo: `Esc` em repouso **não sai** (limpa a busca e
//! o filtro), a vista do lixo é um **contexto próprio** de [`map_key`] (não um
//! valor de `Filter`, para o `c` e o `u` não terem dois significados) e os modos
//! de texto só se abandonam com `Enter`, `Esc` ou `Ctrl+C`.
//!
//! Tudo aqui é testável sem TTY: `KeyEvent::new(…)` constrói-se fora de
//! terminal (verificado).
//!
//! Duas notas sobre a forma, para o que vem a seguir não as redescobrir:
//!
//! * O `KeyEvent` do crossterm 0.29 implementa `PartialEq`/`Eq` (`event.rs`,
//!   `impl PartialEq for KeyEvent`), logo o [`Action`] pode derivá-los e os
//!   testes comparam acções a sério em vez de compararem etiquetas de texto.
//! * O enum do plano não tinha forma de dizer «esta tecla não faz nada»: daí
//!   [`Action::Ignore`], que é o que devolvem um `Release`/`Repeat` e qualquer
//!   tecla não mapeada no modo activo. Sem ele, um `z` teria de devolver *alguma*
//!   acção — e qualquer escolha seria um bug.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::core::Priority;

/// Uma acção. É o único vocabulário que o [`App`] entende: as teclas moram em
/// [`map_key`], o efeito mora em `App::handle`.
///
/// [`App`]: super::app::App
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Sair: `q` em repouso, `Ctrl+C` em qualquer modo.
    Quit,
    /// Navegação: `j`/`↓` e `k`/`↑` (também na vista do lixo).
    Up,
    Down,
    /// `g`/`G` — topo e fim da lista (§5, mudança 3).
    Home,
    End,
    /// Abrir a linha de texto para criar uma tarefa (`a`).
    AddStart,
    /// Abrir a linha de texto com o título da selecionada (`e`).
    EditStart,
    /// Abrir a linha de texto com a descrição da selecionada (`Ctrl+E`).
    EditDescriptionStart,
    /// `Espaço`/`Enter` — concluir ou reabrir a selecionada.
    Toggle,
    /// `t` — concluir todas, ou reabrir todas se já estiverem concluídas.
    ToggleAll,
    /// `d` — mandar a selecionada para o lixo (é o «apagar» da v1).
    Trash,
    /// `u` em repouso repõe o último lote; `Enter` no lixo repõe a selecionada.
    Restore,
    /// `L` — entrar (ou sair) da vista do lixo.
    TrashView,
    /// `c` em repouso — mandar todas as concluídas para o lixo, num só lote.
    ClearCompleted,
    /// `c` no lixo — esvaziar o lixo, com guarda de duas pressões.
    EmptyTrash,
    /// `s` — ciclar a ordem.
    CycleSort,
    /// `f` — ciclar o filtro.
    CycleFilter,
    /// `1`/`2`/`3` — prioridade Alta/Média/Baixa na selecionada.
    SetPriority(Priority),
    /// `/` — abrir a linha de busca.
    SearchStart,
    /// `Esc` em repouso — tirar a mensagem; se não houver, limpar busca e
    /// filtro. Nunca sai da aplicação.
    DismissMessage,
    /// `i` — pedir o caminho de um ficheiro a importar.
    ImportStart,
    /// `x` — pedir o caminho de um ficheiro a exportar.
    ExportStart,
    /// `?` — abrir a ajuda (fecha-se com `?`, `Esc` ou `q`, que o mapa da ajuda
    /// traduz em [`Action::InputCancel`]).
    Help,
    /// `T` — abrir a caixa de temas (§Decisão 9 do ADR). Só na lista: o mapa do
    /// lixo não tem `T`, para a caixa não se abrir por cima da vista do lixo.
    ThemeView,
    /// `j`/`k`/`↓`/`↑` na caixa de temas: mover o cursor, que já traz a
    /// pré-visualização (o delta é `+1`/`-1`, como na lista).
    ThemeMove(isize),
    /// `Enter` na caixa de temas — gravar a preferência e fechar.
    ThemeConfirm,
    /// `Esc` (ou `q`) na caixa de temas — repor o tema de entrada e fechar.
    ThemeCancel,
    /// `C` em repouso — abrir a caixa de categorias (§C do `@designer`).
    ///
    /// Como a caixa de temas: uma sobreposição com **mapa próprio**, e abri-la
    /// não atribui nada (não mexe no filtro, na ordem nem na seleção).
    CategoryView,
    /// `j`/`k`/`↓`/`↑` na caixa de categorias: mover o cursor (`+1`/`-1`). O
    /// cursor para nos extremos, como o da caixa de temas.
    CategoryMove(isize),
    /// `Enter` na caixa de categorias — atribuir a categoria do cursor à tarefa
    /// selecionada e fechar (em `sem categoria`, **tira** a atribuição).
    CategoryAssign,
    /// `a` na caixa — abrir a linha de texto «Nova categoria».
    CategoryNew,
    /// `e` na caixa — abrir a linha de texto «Renomear» com o nome actual.
    CategoryRename,
    /// `d` na caixa — armar a guarda da eliminação; a segunda pressão elimina
    /// (segunda operação do programa sem undo por trás, ADR §5).
    CategoryDelete,
    /// `F` em repouso — ciclar o filtro por categoria (`todas → sem categoria →
    /// … → todas`), um eixo próprio ao lado do `f` (ADR §8).
    CycleCategoryFilter,
    /// Numa linha de texto: a tecla crua vai para o buffer.
    Input(KeyEvent),
    /// `Enter` — confirmar a linha de texto.
    InputConfirm,
    /// `Esc` — cancelar a linha de texto, ou voltar do lixo/da ajuda.
    InputCancel,
    /// A tecla não faz nada aqui. Devolvido por `Release`/`Repeat` e por teclas
    /// não mapeadas — nunca dispara efeito nem desarma a guarda do lixo.
    Ignore,
}

/// Modo de entrada: o que as teclas fazem depende dele, e é ele (e não o
/// `Filter`) que distingue a vista do lixo da lista.
///
/// Os cinco modos de texto partilham o mesmo mapa (`Enter`/`Esc`/texto); a
/// diferença está no que o `Enter` faz. A ajuda e o lixo não são modos de
/// texto: cada um tem mapa próprio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InputMode {
    #[default]
    Normal,
    /// `a` — criar tarefa.
    Adding,
    /// `e` / `Ctrl+E` — editar o título ou a descrição da selecionada.
    Editing,
    /// `/` — buscar.
    Search,
    /// `i` — escrever o caminho de um ficheiro a importar.
    Import,
    /// `x` — escrever o caminho de um ficheiro a exportar.
    Export,
    /// `?` — ajuda sobreposta (só `?`, `q` e `Esc` fazem algo).
    Help,
    /// `T` — caixa de temas sobreposta: só as teclas da caixa fazem algo
    /// (`j`/`k`/`↓`/`↑`, `Enter`, `Esc`, `q` e o `Ctrl+C` de todos os modos).
    Theme,
    /// `C` — caixa de categorias sobreposta: mapa próprio, como a das temas.
    /// Traz o `a`/`e`/`d` da caixa, que **não** são os da lista.
    Category,
    /// `a`/`e` com a caixa de categorias aberta — a linha de texto do nome
    /// (criar ou renomear; qual dos dois é do `App`, como o `editing_description`
    /// do `Editing`). Sai-se dela para a caixa, não para a lista.
    CategoryName,
    /// `L` — vista do lixo: as teclas da lista não valem aqui.
    Trash,
}

impl InputMode {
    /// Modos em que tudo o que não seja `Enter`/`Esc`/`Ctrl+C` é texto.
    #[must_use]
    pub const fn is_text(self) -> bool {
        matches!(
            self,
            Self::Adding
                | Self::Editing
                | Self::Search
                | Self::Import
                | Self::Export
                | Self::CategoryName
        )
    }

    /// Etiqueta da linha de entrada (o T6 pinta-a em `accent`; o frame
    /// `80x24-3-adicionar` mostra `Nova  ` seguido do buffer).
    ///
    /// Em [`InputMode::CategoryName`] a etiqueta da linha 22 é `Nova categoria`
    /// **ou** `Renomear` (§C.4): qual dos dois só o `App` sabe (é o
    /// [`super::app::App`] que sabe se o `e` está a renomear), logo o desenho lê
    /// este valor como o de «nova» e pergunta ao `App` pelo resto.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Adding => "Nova",
            Self::Editing => "Editar",
            Self::Search => "Busca",
            Self::Import => "Importar",
            Self::Export => "Exportar",
            Self::Help => "Ajuda",
            Self::Theme => "Tema",
            Self::Category => "Categoria",
            Self::CategoryName => "Nova categoria",
            Self::Trash => "Lixo",
        }
    }

    /// Texto de marcador quando a linha está vazia (itálico no T6; §4).
    #[must_use]
    pub const fn placeholder(self) -> &'static str {
        match self {
            Self::Adding | Self::Editing => "Título da tarefa…",
            Self::Search => "Buscar…",
            Self::Import => "Caminho do ficheiro a importar…",
            Self::Export => "Caminho do ficheiro a exportar…",
            // A linha 22 de criar/renomear uma categoria (`80x24-20-categorias-erro-vazio`).
            Self::CategoryName => "Nome da categoria…",
            Self::Normal | Self::Help | Self::Theme | Self::Category | Self::Trash => "",
        }
    }
}

/// Traduz uma tecla na acção correspondente ao modo activo.
///
/// `Release`/`Repeat` são descartados: o crossterm entrega os três tipos de
/// evento e só as pressões contam (o `kitty`/Windows mandam `Release`, e sem
/// este filtro cada `Espaço` concluía duas tarefas).
#[must_use]
pub fn map_key(mode: &InputMode, key: KeyEvent) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::Ignore;
    }

    // `Ctrl+C` sai de **qualquer** modo, incluindo os de texto: em raw mode não
    // há sinal, e a barra dos modos de texto promete-o («Ctrl+C sai»).
    if ctrl(key) && key.code == KeyCode::Char('c') {
        return Action::Quit;
    }

    if mode.is_text() {
        return match key.code {
            KeyCode::Enter => Action::InputConfirm,
            KeyCode::Esc => Action::InputCancel,
            _ => Action::Input(key),
        };
    }

    match mode {
        InputMode::Help => map_help(key),
        InputMode::Theme => map_theme(key),
        InputMode::Category => map_category(key),
        InputMode::Trash => map_trash(key),
        // `Normal` — e mais nada, porque os modos de texto saíram acima.
        _ => map_normal(key),
    }
}

fn ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

fn map_normal(key: KeyEvent) -> Action {
    // `Ctrl+E` antes da tabela: com `CONTROL` a tecla é um comando, não a letra.
    if ctrl(key) {
        return match key.code {
            KeyCode::Char('e') => Action::EditDescriptionStart,
            _ => Action::Ignore,
        };
    }
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::Char('g') => Action::Home,
        KeyCode::Char('G') => Action::End,
        KeyCode::Char(' ') | KeyCode::Enter => Action::Toggle,
        KeyCode::Char('a') => Action::AddStart,
        KeyCode::Char('e') => Action::EditStart,
        KeyCode::Char('d') => Action::Trash,
        KeyCode::Char('u') => Action::Restore,
        KeyCode::Char('1') => Action::SetPriority(Priority::High),
        KeyCode::Char('2') => Action::SetPriority(Priority::Medium),
        KeyCode::Char('3') => Action::SetPriority(Priority::Low),
        KeyCode::Char('s') => Action::CycleSort,
        KeyCode::Char('f') => Action::CycleFilter,
        KeyCode::Char('/') => Action::SearchStart,
        KeyCode::Char('t') => Action::ToggleAll,
        KeyCode::Char('c') => Action::ClearCompleted,
        KeyCode::Char('i') => Action::ImportStart,
        KeyCode::Char('x') => Action::ExportStart,
        KeyCode::Char('L') => Action::TrashView,
        // `T` maiúsculo está livre (`t` minúsculo é `ToggleAll`): é a tecla da
        // caixa de temas (§Decisão 9). As maiúsculas são distintas de propósito
        // neste mapa — a mesma regra que separa `g`/`G`.
        KeyCode::Char('T') => Action::ThemeView,
        // `C` e `F` maiúsculos estão livres (§C.0 do `@designer`, medido: 29
        // teclas ocupadas): a caixa de categorias e o eixo da categoria. O `f`
        // minúsculo continua a governar **só** o estado — «pendentes de
        // Trabalho» é o que se ganha por serem dois eixos (ADR §8).
        KeyCode::Char('C') => Action::CategoryView,
        KeyCode::Char('F') => Action::CycleCategoryFilter,
        KeyCode::Char('?') => Action::Help,
        // `Esc` em repouso limpa a busca e o filtro — **não** sai. Sair é só `q`
        // ou `Ctrl+C`: depois de um `d`, o reflexo de carregar em `Esc` não pode
        // fechar o programa.
        KeyCode::Esc => Action::DismissMessage,
        _ => Action::Ignore,
    }
}

/// Mapa da vista do lixo (§5). Só tem as teclas vivas neste contexto — e é por
/// isso que é um mapa próprio: `u` e `c` significam outra coisa na lista, e um
/// `c` com dois sentidos esvaziaria o lixo sem guarda.
fn map_trash(key: KeyEvent) -> Action {
    if ctrl(key) {
        return Action::Ignore;
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::Char('g') => Action::Home,
        KeyCode::Char('G') => Action::End,
        // Aqui `Enter` repõe a **selecionada**; na lista, `u` repõe o último
        // lote. Duas teclas, duas semânticas.
        KeyCode::Enter | KeyCode::Char(' ') => Action::Restore,
        KeyCode::Char('c') => Action::EmptyTrash,
        KeyCode::Esc => Action::InputCancel,
        KeyCode::Char('?') => Action::Help,
        // `a`, `e`, `d`, `u`, `1/2/3`, `t`, `s`, `f`, `i`, `x` e `q` não estão
        // no mapa do lixo: `u` significaria «restaurar o selecionado» e é assim
        // que se perde um item por engano. Sair daqui é `Esc` e depois `q`, ou
        // `Ctrl+C` — que o mapa acima trata em qualquer modo.
        //
        // `C` e `F` também não estão, e é deliberado (§C.2): a vista do lixo não
        // mostra categorias (ADR §11), logo são teclas mortas aqui — o rodapé
        // desta vista não as anuncia.
        _ => Action::Ignore,
    }
}

/// Mapa da caixa de temas (§Decisão 9): navegar com pré-visualização (`j`/`k` e
/// as setas), `Enter` grava, `Esc`/`q` fecham sem gravar.
///
/// É um mapa próprio, como o da ajuda: com a caixa aberta, nada do modo normal
/// vale — nem `a`, nem `e`, nem `d`, nem `/`, nem o `T` outra vez. A lista fica
/// por baixo, à vista, e uma tecla que ali significa «remover» não pode agir
/// sobre ela através da caixa.
fn map_theme(key: KeyEvent) -> Action {
    if ctrl(key) {
        // Só o `Ctrl+C` acima tem significado; qualquer outro `Ctrl+…` não faz
        // nada aqui (é a mesma regra do mapa da ajuda e do lixo).
        return Action::Ignore;
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Action::ThemeMove(1),
        KeyCode::Char('k') | KeyCode::Up => Action::ThemeMove(-1),
        KeyCode::Enter => Action::ThemeConfirm,
        // `q` é «fechar sem gravar»: o `q` de sair da aplicação não vale aqui,
        // senão o reflexo de fechar a caixa saía do programa a meio de uma
        // escolha. Para sair, `Ctrl+C` (ou `Esc` e depois `q`).
        KeyCode::Esc | KeyCode::Char('q') => Action::ThemeCancel,
        _ => Action::Ignore,
    }
}

/// Mapa da caixa de categorias (§C.2): a navegação da caixa, o `Enter` que
/// atribui, o `a`/`e`/`d` que são **dela** (não os da lista) e o fecho sem
/// atribuir.
///
/// É um mapa próprio, como o da ajuda e o das temas: com a caixa aberta, nada
/// da lista vale lá dentro. É o que impede que o `d` que armou a eliminação de
/// uma categoria mande a tarefa selecionada para o lixo por baixo da
/// sobreposição — a lista está à vista atrás dela.
///
/// O `g`/`G` são o topo e o fim da caixa (a caixa tem janela de 10 entradas) e
/// resolvem-se em [`Action::Home`]/[`Action::End`], que o `App` interpreta
/// conforme o modo — como o `Restore` faz na lista e no lixo. Em `sem
/// categoria` os dois são a mesma linha, logo não fazem nada.
fn map_category(key: KeyEvent) -> Action {
    if ctrl(key) {
        // Só o `Ctrl+C` acima tem significado (a regra da ajuda, das temas e do lixo).
        return Action::Ignore;
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Action::CategoryMove(1),
        KeyCode::Char('k') | KeyCode::Up => Action::CategoryMove(-1),
        KeyCode::Char('g') => Action::Home,
        KeyCode::Char('G') => Action::End,
        KeyCode::Enter => Action::CategoryAssign,
        KeyCode::Char('a') => Action::CategoryNew,
        KeyCode::Char('e') => Action::CategoryRename,
        KeyCode::Char('d') => Action::CategoryDelete,
        // `q` fecha sem atribuir, como o `Esc` — o `q` de sair da aplicação não
        // vale aqui, senão o reflexo de fechar a caixa saía do programa a meio
        // de uma escolha (a mesma decisão da caixa de temas).
        KeyCode::Esc | KeyCode::Char('q') => Action::InputCancel,
        _ => Action::Ignore,
    }
}

/// Mapa da ajuda: `?`, `q` e `Esc` fecham, o resto é ignorado (§5).
fn map_help(key: KeyEvent) -> Action {
    if ctrl(key) {
        return Action::Ignore;
    }
    match key.code {
        KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => Action::InputCancel,
        _ => Action::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEventKind, KeyModifiers};

    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn with_kind(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
    }

    fn acao(mode: InputMode, key: KeyEvent) -> Action {
        map_key(&mode, key)
    }

    fn atalho(c: char) -> Action {
        acao(InputMode::Normal, press(KeyCode::Char(c)))
    }

    #[test]
    fn normal_mapeia_a_tabela_do_designer() {
        let esperado = [
            (KeyCode::Char('q'), Action::Quit),
            (KeyCode::Char('j'), Action::Down),
            (KeyCode::Char('k'), Action::Up),
            (KeyCode::Char('g'), Action::Home),
            (KeyCode::Char('G'), Action::End),
            (KeyCode::Char('a'), Action::AddStart),
            (KeyCode::Char('e'), Action::EditStart),
            (KeyCode::Char('d'), Action::Trash),
            (KeyCode::Char('u'), Action::Restore),
            (KeyCode::Char('1'), Action::SetPriority(Priority::High)),
            (KeyCode::Char('2'), Action::SetPriority(Priority::Medium)),
            (KeyCode::Char('3'), Action::SetPriority(Priority::Low)),
            (KeyCode::Char('s'), Action::CycleSort),
            (KeyCode::Char('f'), Action::CycleFilter),
            (KeyCode::Char('/'), Action::SearchStart),
            (KeyCode::Char('t'), Action::ToggleAll),
            (KeyCode::Char('c'), Action::ClearCompleted),
            (KeyCode::Char('i'), Action::ImportStart),
            (KeyCode::Char('x'), Action::ExportStart),
            (KeyCode::Char('L'), Action::TrashView),
            (KeyCode::Char('?'), Action::Help),
        ];
        for (code, esperada) in esperado {
            assert_eq!(acao(InputMode::Normal, press(code)), esperada, "{code:?}");
        }
    }

    #[test]
    fn normal_mapeia_as_teclas_de_navegacao_e_espaco() {
        for (code, esperada) in [
            (KeyCode::Down, Action::Down),
            (KeyCode::Up, Action::Up),
            (KeyCode::Char(' '), Action::Toggle),
            (KeyCode::Enter, Action::Toggle),
        ] {
            assert_eq!(acao(InputMode::Normal, press(code)), esperada, "{code:?}");
        }
    }

    #[test]
    fn esc_em_repouso_limpa_a_mensagem_e_nao_sai() {
        let esc = acao(InputMode::Normal, press(KeyCode::Esc));
        assert_eq!(esc, Action::DismissMessage);
        assert_ne!(esc, Action::Quit, "o reflexo de cancelar não fecha o programa");
    }

    #[test]
    fn ctrl_c_sai_em_todos_os_modos() {
        for mode in [
            InputMode::Normal,
            InputMode::Adding,
            InputMode::Editing,
            InputMode::Search,
            InputMode::Import,
            InputMode::Export,
            InputMode::Help,
            InputMode::Theme,
            InputMode::Category,
            InputMode::CategoryName,
            InputMode::Trash,
        ] {
            assert_eq!(
                acao(mode, press_ctrl(KeyCode::Char('c'))),
                Action::Quit,
                "Ctrl+C em {mode:?}"
            );
        }
    }

    #[test]
    fn ctrl_e_abre_a_descricao() {
        assert_eq!(
            acao(InputMode::Normal, press_ctrl(KeyCode::Char('e'))),
            Action::EditDescriptionStart
        );
        assert_eq!(
            atalho('e'),
            Action::EditStart,
            "sem `Ctrl` a tecla é o título, não a descrição"
        );
    }

    #[test]
    fn modos_de_texto_mandam_a_tecla_crua_para_o_buffer() {
        for mode in [
            InputMode::Adding,
            InputMode::Editing,
            InputMode::Search,
            InputMode::Import,
            InputMode::Export,
            // A linha do nome da categoria é texto como as outras cinco (§C.2):
            // lá dentro, `q`, `j`, `d`, `C` e `F` são letras.
            InputMode::CategoryName,
        ] {
            assert_eq!(acao(mode, press(KeyCode::Enter)), Action::InputConfirm, "{mode:?}");
            assert_eq!(acao(mode, press(KeyCode::Esc)), Action::InputCancel, "{mode:?}");
            // `q`, `d`, `j` e `?` são texto: um `q` a meio de um título não sai,
            // e um `?` não abre a ajuda.
            for c in ['q', 'd', 'j', '1', '?', 'L', ' '] {
                assert_eq!(
                    acao(mode, press(KeyCode::Char(c))),
                    Action::Input(press(KeyCode::Char(c))),
                    "«{c}» em {mode:?} tem de ser texto"
                );
            }
            assert_eq!(
                acao(mode, press(KeyCode::Backspace)),
                Action::Input(press(KeyCode::Backspace))
            );
            assert_eq!(
                acao(mode, press_ctrl(KeyCode::Char('u'))),
                Action::Input(press_ctrl(KeyCode::Char('u'))),
                "Ctrl+U vai para o buffer: quem o interpreta é o `App`"
            );
        }
    }

    #[test]
    fn ajuda_so_fecha_com_tres_teclas() {
        for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('?')] {
            assert_eq!(acao(InputMode::Help, press(code)), Action::InputCancel, "{code:?}");
        }
        for c in ['j', 'k', 'd', 'a', 'h', 'c'] {
            assert_eq!(
                acao(InputMode::Help, press(KeyCode::Char(c))),
                Action::Ignore,
                "«{c}» não faz nada com a ajuda aberta"
            );
        }
    }

    /// Critério 1 do cartão T5: a tecla nova existe, não colide com nenhuma da
    /// tabela do modo normal, e o `t` minúsculo continua a alternar todas.
    #[test]
    fn t_maiusculo_abre_a_caixa_e_nao_colide_com_a_tabela() {
        assert_eq!(atalho('T'), Action::ThemeView);
        assert_eq!(
            atalho('t'),
            Action::ToggleAll,
            "`t` minúsculo continua a alternar todas"
        );

        // Percorre a tabela toda: só o `T` maiúsculo vale `ThemeView`.
        let mut teclas: Vec<char> = ('a'..='z')
            .chain('A'..='Z')
            .chain('0'..='9')
            .filter(|c| *c != 'T')
            .collect();
        teclas.extend([
            '/', '?', ' ', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '-', '_', '=', '+',
            '[', ']', '{', '}', '\\', '|', '\'', '"', '`', '~', '<', '>', ',', '.', ';', ':',
        ]);
        for c in teclas {
            assert_ne!(
                atalho(c),
                Action::ThemeView,
                "«{c}» não pode abrir a caixa de temas"
            );
        }
        for code in [
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Tab,
            KeyCode::F(1),
        ] {
            assert_ne!(
                acao(InputMode::Normal, press(code)),
                Action::ThemeView,
                "{code:?} não abre a caixa de temas"
            );
        }
    }

    #[test]
    fn caixa_de_temas_tem_mapa_proprio() {
        let esperado = [
            (KeyCode::Char('j'), Action::ThemeMove(1)),
            (KeyCode::Down, Action::ThemeMove(1)),
            (KeyCode::Char('k'), Action::ThemeMove(-1)),
            (KeyCode::Up, Action::ThemeMove(-1)),
            (KeyCode::Enter, Action::ThemeConfirm),
            (KeyCode::Esc, Action::ThemeCancel),
            (KeyCode::Char('q'), Action::ThemeCancel),
        ];
        for (code, esperada) in esperado {
            assert_eq!(acao(InputMode::Theme, press(code)), esperada, "{code:?}");
        }
        // Nada do modo normal atravessa a caixa — `d` sobretudo: a lista está
        // à vista por baixo da sobreposição.
        for c in [
            'a', 'e', 'd', 'u', 't', 'c', 's', 'f', 'i', 'x', 'L', 'T', '?', 'g', 'G', '1', ' ',
            '/', 'z',
        ] {
            assert_eq!(
                acao(InputMode::Theme, press(KeyCode::Char(c))),
                Action::Ignore,
                "«{c}» não vale com a caixa aberta"
            );
        }
    }

    /// §C.0: `C` e `F` estão livres (medido: 29 teclas ocupadas) e não roubam o
    /// significado a nenhuma da tabela — o `f` minúsculo continua a governar o
    /// estado e o `c` minúsculo continua a limpar as concluídas.
    #[test]
    fn c_e_f_nao_colidem_com_a_tabela_do_modo_normal() {
        assert_eq!(atalho('C'), Action::CategoryView);
        assert_eq!(atalho('F'), Action::CycleCategoryFilter);
        assert_eq!(atalho('f'), Action::CycleFilter, "`f` é só o estado");
        assert_eq!(atalho('c'), Action::ClearCompleted);

        for c in ('a'..='z').chain('A'..='Z').chain('0'..='9') {
            if c != 'C' {
                assert_ne!(atalho(c), Action::CategoryView, "«{c}» não abre a caixa");
            }
            if c != 'F' {
                assert_ne!(
                    atalho(c),
                    Action::CycleCategoryFilter,
                    "«{c}» não cicla a categoria"
                );
            }
        }
        for code in [
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Tab,
            KeyCode::F(2),
        ] {
            assert_ne!(
                acao(InputMode::Normal, press(code)),
                Action::CategoryView,
                "{code:?}"
            );
            assert_ne!(
                acao(InputMode::Normal, press(code)),
                Action::CycleCategoryFilter,
                "{code:?}"
            );
        }
    }

    /// §C.2: a caixa de categorias tem mapa próprio — e o `a`/`e`/`d` de lá
    /// dentro são os **dela** (a lista está à vista, por baixo).
    #[test]
    fn caixa_de_categorias_tem_mapa_proprio() {
        let esperado = [
            (KeyCode::Char('j'), Action::CategoryMove(1)),
            (KeyCode::Down, Action::CategoryMove(1)),
            (KeyCode::Char('k'), Action::CategoryMove(-1)),
            (KeyCode::Up, Action::CategoryMove(-1)),
            (KeyCode::Char('g'), Action::Home),
            (KeyCode::Char('G'), Action::End),
            (KeyCode::Enter, Action::CategoryAssign),
            (KeyCode::Char('a'), Action::CategoryNew),
            (KeyCode::Char('e'), Action::CategoryRename),
            (KeyCode::Char('d'), Action::CategoryDelete),
            (KeyCode::Esc, Action::InputCancel),
            (KeyCode::Char('q'), Action::InputCancel),
        ];
        for (code, esperada) in esperado {
            assert_eq!(acao(InputMode::Category, press(code)), esperada, "{code:?}");
        }
        // Nada da lista atravessa a caixa — `s`, `f`, `1`/`2`/`3` e `d`
        // sobretudo: um `d` que ali dentro eliminasse a categoria e cá fora
        // mandasse a tarefa ao lixo era o defeito clássico das sobreposições.
        for c in [
            's', 'f', '1', '2', '3', 't', 'c', 'u', 'i', 'x', 'L', 'T', 'C', 'F', '?', '/', 'z',
        ] {
            assert_eq!(
                acao(InputMode::Category, press(KeyCode::Char(c))),
                Action::Ignore,
                "«{c}» não vale com a caixa de categorias aberta"
            );
        }
    }

    /// §C.2: a vista do lixo não tem categorias — `C` e `F` são teclas mortas lá
    /// dentro (o rodapé da vista não as anuncia).
    #[test]
    fn as_teclas_das_categorias_sao_mortas_no_lixo() {
        assert_eq!(
            acao(InputMode::Trash, press(KeyCode::Char('C'))),
            Action::Ignore
        );
        assert_eq!(
            acao(InputMode::Trash, press(KeyCode::Char('F'))),
            Action::Ignore
        );
    }

    /// A linha do nome é um modo de texto (as teclas cruas vão para o buffer) e
    /// a caixa é navegação: a etiqueta e o marcador da linha 22 são os de §C.4.
    #[test]
    fn o_nome_da_categoria_e_texto_e_a_caixa_nao() {
        assert!(InputMode::CategoryName.is_text());
        assert!(!InputMode::Category.is_text(), "a caixa é navegação");
        assert_eq!(InputMode::CategoryName.label(), "Nova categoria");
        assert_eq!(InputMode::CategoryName.placeholder(), "Nome da categoria…");
        assert_eq!(
            InputMode::Category.placeholder(),
            "",
            "a caixa não tem linha 22"
        );
    }

    #[test]
    fn lixo_tem_mapa_proprio() {
        let esperado = [
            (KeyCode::Char('j'), Action::Down),
            (KeyCode::Char('k'), Action::Up),
            (KeyCode::Char('g'), Action::Home),
            (KeyCode::Char('G'), Action::End),
            (KeyCode::Enter, Action::Restore),
            (KeyCode::Char(' '), Action::Restore),
            (KeyCode::Char('c'), Action::EmptyTrash),
            (KeyCode::Esc, Action::InputCancel),
            (KeyCode::Char('?'), Action::Help),
        ];
        for (code, esperada) in esperado {
            assert_eq!(acao(InputMode::Trash, press(code)), esperada, "{code:?}");
        }
        // As teclas da lista morrem à porta do lixo — `u` e `c` sobretudo, mas
        // também `d` (não se manda ao lixo o que já lá está).
        for c in ['u', 'a', 'e', 'd', '1', 't', 's', 'f', 'i', 'x', 'q'] {
            assert_eq!(
                acao(InputMode::Trash, press(KeyCode::Char(c))),
                Action::Ignore,
                "«{c}» não vale no lixo"
            );
        }
    }

    #[test]
    fn release_e_repeat_nao_disparam_nada() {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            for mode in [InputMode::Normal, InputMode::Trash, InputMode::Adding] {
                assert_eq!(
                    acao(mode, with_kind(KeyCode::Char('d'), kind)),
                    Action::Ignore,
                    "{kind:?} em {mode:?}"
                );
            }
        }
    }

    #[test]
    fn teclas_desconhecidas_dao_ignore() {
        // `Ignore` é a ausência de acção: sem ele uma tecla livre teria de
        // devolver *alguma* acção, e qualquer escolha seria um bug.
        assert_eq!(atalho('z'), Action::Ignore);
        assert_eq!(atalho('Q'), Action::Ignore, "`Q` não é `q`: maiúsculas são distintas");
        assert_eq!(acao(InputMode::Normal, press(KeyCode::F(1))), Action::Ignore);
        assert_eq!(
            acao(InputMode::Normal, press_ctrl(KeyCode::Char('z'))),
            Action::Ignore,
            "só `Ctrl+E` e `Ctrl+C` têm significado em repouso"
        );
        assert_eq!(
            acao(InputMode::Normal, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT)),
            Action::Down,
            "um modificador inesperado não torna a tecla invisível"
        );
    }
}
