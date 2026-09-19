//! Desenho do ecrã: do [`App`] para o `Buffer`, sem estado próprio.
//!
//! A grelha é a do `@designer` (`design-system/todo-ratatui.md` §2–§4) e os
//! testes de `tests/render.rs` comparam-na com os 15 *golden files* de
//! `tests/frames/`, linha a linha. Nada aqui é uma segunda fonte de verdade:
//! o que a spec fixa (colunas, cortes, barras de ajuda) está aqui uma única vez.
//!
//! Três regras que a spec mediu e que não são estilo:
//!
//! * **Sem molduras**: só réguas horizontais a toda a largura (§2). O painel de
//!   detalhe só existe a partir de `96 × 28`; abaixo disso a descrição do
//!   selecionado passa para a linha 22.
//! * **A linha 22 tem `largura - 1` colunas úteis** (79 em 80): o caminho de
//!   casa abrevia-se a `~` e corta-se **pela esquerda** quando não cabe — uma
//!   mensagem cortada a meio é bug de desenho, não detalhe (§8, armadilha 3).
//! * **Nada de `tick`**: o ecrã desenha-se em evento. A única coisa mutável é a
//!   [`ListState`] do `App`, e mesmo essa é consumida por cópia — o `ui` é uma
//!   função pura de `&App` (§6).
//!
//! [`ListState`]: ratatui::widgets::ListState

use std::path::Path;

use chrono::{Local, NaiveDate};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::core::{Priority, TRASH_LIMIT, Todo, Trashed};

use super::app::{App, Status, UNDO_HINT};
use super::event::InputMode;
use super::theme::{CATALOGO, ModoCor, Theme};

// ------------------------------------------------------------------- cor (§1)

// Os nove papéis de cor já não vivem aqui: vivem em `super::theme`, e o
// desenho lê-os de `app.theme`. Não há neste ficheiro uma única cor literal
// (§1 do desenho, decisão 5 do ADR) — é isso que faz o tema escolhido valer
// para o ecrã todo, em vez de conviver com restos da v1.0.1.
//
// As funções que não recebem o `App` (réguas, a linha de uma tarefa, o
// cabeçalho do painel) recebem o `&Theme` como primeiro argumento: a mesma
// fonte de cor, por outro caminho.

// ------------------------------------------------------------------ grelha (§2)

const LARGURA_MINIMA: u16 = 40;
const ALTURA_MINIMA: u16 = 10;
const LARGURA_PAINEL: u16 = 96;
const ALTURA_PAINEL_MINIMA: u16 = 28;
/// Cabeçalho + régua + linha de estado.
const ALTURA_CABECALHO: u16 = 3;
/// Régua + linha de mensagem + barra de ajuda.
const ALTURA_RODAPE: u16 = 3;
/// Régua «selecionada» + 4 linhas de detalhe.
const ALTURA_PAINEL: u16 = 5;
/// Coluna da data: `W-12`, alinhada à direita, com uma de folga em `W-13`.
const RESERVA_DIREITA: u16 = 21;
/// Título começa em `x = 8`: `▶ ` (2) + `[x] ` (4) + `H ` (2).
const COLUNA_TITULO: usize = 8;
/// Coluna do valor no painel de detalhe (`x = 14`; a chave está em `x = 2`).
const COLUNA_DETALHE: usize = 14;
/// Termo de busca cortado a 20 colunas na linha 2.
const LARGURA_BUSCA: usize = 20;
const CAIXA_AJUDA: (u16, u16) = (60, 15);
const COLUNA_AJUDA_ESQ: usize = 2;
const COLUNA_AJUDA_DIR: usize = 26;
/// A caixa de temas (§T.5) **é** o rectângulo da ajuda: mesma medida, mesmo
/// `Clear`, mesma coluna de conteúdo. Não é uma segunda medida a poder divergir.
const CAIXA_TEMAS: (u16, u16) = CAIXA_AJUDA;
/// Coluna onde começa o texto dentro de uma caixa sobreposta — as mesmas duas
/// colunas da [`COLUNA_AJUDA_ESQ`] (uma parede e um espaço em branco).
const COLUNA_CAIXA: usize = 2;

const ETIQUETA_DATA: &str = "criada";
const ETIQUETA_REMOVIDA: &str = "removida";

// ------------------------------------------------- barras de ajuda (§4, §5)

const BARRA: &str = "a nova  e editar  Espaço concluir  d remover  u desfazer  / buscar  ? ajuda";
const BARRA_BUSCA: &str =
    "a nova  e editar  Espaço concluir  d remover  u desfazer  Esc limpar  ? ajuda";
const BARRA_INPUT: &str = "Enter guarda  Esc cancela  Ctrl+C sai";
const BARRA_LIXO: &str = "Enter restaura  c esvaziar o lixo  Esc volta  ? ajuda";
/// Com o lixo vazio só resta sair dele: as teclas da lista vazia seriam teclas
/// mortas nesta vista (§5, frame `80x24-13-lixo-vazio`).
const BARRA_LIXO_VAZIO: &str = "Esc volta à lista  ? ajuda";
const BARRA_ARMADO: &str = "c confirmar  Esc cancela";
const BARRA_ERRO: &str = "Esc  limpar a mensagem    q  sair";
/// A mensagem de remoção fica enquanto houver undo: a barra reduz-se à única
/// acção que resta (frame `80x24-5-undo`).
const BARRA_UNDO: &str = "u desfazer  ? ajuda";
const BARRA_VAZIO: &str = "a nova  ? ajuda";
const BARRA_SEM_RESULTADOS: &str = "Esc limpar  a nova  ? ajuda";
const BARRA_AJUDA: &str = "? ou Esc fecha a ajuda";
/// A barra da caixa de temas (§T.5). É a única barra da caixa: as teclas da
/// lista estão desactivadas enquanto ela está aberta (o modo `Theme` tem mapa
/// próprio), e o que se pode fazer lá dentro é sair.
const BARRA_TEMAS: &str = "Esc volta ao tema de entrada";

/// Teclas da ajuda: a lista principal (`80x24-7-ajuda`).
///
/// As duas teclas de vista (`L`, `T`) ficam lado a lado, antes das de ficheiro
/// (§T.6): a caixa tem 13 linhas interiores e nenhuma livre, por isso o `T`
/// entrou sem a caixa crescer — o espaçador que separava `i importar · x
/// exportar` da linha de saída é que saiu.
const TECLAS_AJUDA: [(&str, &str); 13] = [
    ("", ""),
    ("Navegação", "Tarefas"),
    ("j / ↓  seguinte", "a  nova tarefa"),
    ("k / ↑  anterior", "e  editar título"),
    ("g / G  topo / fim", "Espaço  concluir"),
    ("1 2 3  prioridade", "d  remover"),
    ("s  ordenar", "u  desfazer"),
    ("f  filtrar", "t  alternar todas"),
    ("/  buscar", "c  limpar concluídas"),
    ("Esc  limpar busca", "L  ver o lixo"),
    ("", "T  tema"),
    ("", "i  importar · x  exportar"),
    ("Esc  fechar a ajuda", "q  sair"),
];

/// Teclas da ajuda na vista do lixo (§6): só as teclas vivas neste contexto —
/// `a`, `e`, `d`, `u`, `1/2/3`, `t`, `s`, `f`, `i` e `x` não estão cá.
const TECLAS_AJUDA_LIXO: [(&str, &str); 13] = [
    ("", ""),
    ("Navegação", "Lixo"),
    ("j / ↓  seguinte", "Enter  restaurar"),
    ("k / ↑  anterior", "Espaço  restaurar"),
    ("g / G  topo / fim", "c  esvaziar o lixo"),
    ("", "Esc  voltar à lista"),
    ("", ""),
    ("", ""),
    ("", ""),
    ("", ""),
    ("", ""),
    ("", ""),
    ("Esc  fechar a ajuda", "q  sair"),
];

// -------------------------------------------------------------------- ecrã

/// Pinta `area` com o fundo do tema, e com o `fg` do papel `fg` — para o texto
/// que não declara cor nenhuma (um `Span::raw`, os preenchimentos) ficar com a
/// cor do tema e não com a do terminal.
///
/// O `Cell::set_style` do `ratatui` **junta** o estilo (só troca o que o estilo
/// traz), pelo que a pintura sobrevive a tudo o que é desenhado depois: os
/// spans do desenho não declaram fundo, portanto não o apagam. É esta pintura
/// que faz os rácios de contraste de §T.3 valerem em qualquer terminal e não só
/// naquele em que o desenho foi afinado. Um tema sem `bg` — o `classico`, o
/// único — não pinta nada: mantém o fundo do terminal, como a v1.0.1.
fn pinta_o_fundo(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(bg) = app.theme.bg {
        frame.buffer_mut().set_style(
            area,
            Style::new().bg(bg).fg(app.theme.fg.fg.unwrap_or_default()),
        );
    }
}

/// Desenha o ecrã completo a partir de `&App`.
///
/// Função pura: não guarda nada entre chamadas. O `ListState` do `App` é
/// consumido por cópia (é `Copy`), pelo que o deslocamento da lista é
/// recalculado a cada desenho a partir da seleção — que é a única coisa que o
/// `App` promete manter.
///
/// A cor vem **toda** de `app.theme` (é a única fonte, §1): a função pinta o
/// fundo do tema na área e lê os nove papéis para o resto do desenho.
pub fn ui(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // O fundo primeiro, e antes de tudo o resto (§4 do ADR): o que se desenha a
    // seguir não traz fundo, portanto herda este.
    pinta_o_fundo(frame, app, area);

    if area.width < LARGURA_MINIMA || area.height < ALTURA_MINIMA {
        // Terminal pequeno: mensagem única, sem desenho parcial (§2).
        frame.render_widget(
            Paragraph::new(format!(
                "Terminal demasiado pequeno ({}×{}); mínimo {LARGURA_MINIMA}×{ALTURA_MINIMA}",
                area.width, area.height
            )),
            area,
        );
        return;
    }

    let linha = |y: u16| Rect::new(area.x, area.y + y, area.width, 1);

    frame.render_widget(Paragraph::new(cabecalho(app, area.width)), linha(0));
    frame.render_widget(Paragraph::new(regua(app.theme, area.width)), linha(1));
    frame.render_widget(Paragraph::new(linha_de_estado(app, area.width)), linha(2));

    let com_painel = area.width >= LARGURA_PAINEL && area.height >= ALTURA_PAINEL_MINIMA;
    let altura_lista =
        area.height - ALTURA_CABECALHO - ALTURA_RODAPE - if com_painel { ALTURA_PAINEL } else { 0 };

    let corpo = Rect::new(area.x, area.y + ALTURA_CABECALHO, area.width, altura_lista);
    if app.visible().is_empty() {
        frame.render_widget(Paragraph::new(vazio(app, area.width, altura_lista)), corpo);
    } else {
        // `ListState` é `Copy`: o desenho não fica com estado do `App`.
        let mut estado = app.list_state;
        frame.render_stateful_widget(lista(app, area.width), corpo, &mut estado);
    }

    if com_painel {
        let topo = ALTURA_CABECALHO + altura_lista;
        frame.render_widget(
            Paragraph::new(regua_da_selecionada(app.theme, area.width)),
            linha(topo),
        );
        frame.render_widget(
            Paragraph::new(painel(app, area.width)),
            Rect::new(area.x, area.y + topo + 1, area.width, ALTURA_PAINEL - 1),
        );
    }

    let base = area.height - ALTURA_RODAPE;
    frame.render_widget(Paragraph::new(regua(app.theme, area.width)), linha(base));
    frame.render_widget(
        Paragraph::new(mensagem(app, area.width, com_painel)),
        linha(base + 1),
    );
    frame.render_widget(Paragraph::new(barra_de_ajuda(app)), linha(base + 2));

    // O cursor só existe nos modos de texto (§4): em repouso fica escondido.
    if let Some(coluna) = coluna_do_cursor(app) {
        frame.set_cursor_position(Position::new(area.x + coluna, area.y + base + 1));
    }

    if matches!(app.mode, InputMode::Help) {
        desenha_ajuda(frame, app, area);
    }

    if matches!(app.mode, InputMode::Theme) {
        desenha_temas(frame, app, area);
    }
}

// ------------------------------------------------------------------ cabeçalho

fn cabecalho(app: &App, largura: u16) -> Line<'static> {
    let direita = if matches!(app.mode, InputMode::Trash) {
        format!("lixo: {} de {TRASH_LIMIT}", app.counts().trash)
    } else {
        let contas = app.counts();
        let visiveis = app.visible().len();
        let total = contas.total();
        // Sem filtro nem busca não se repete o total: «9 tarefas» e não «9 de 9».
        if visiveis == total {
            format!(
                "{total} tarefas · {} pendentes · {} concluídas",
                contas.active, contas.done
            )
        } else {
            format!(
                "{visiveis} de {total} tarefas · {} pendentes · {} concluídas",
                contas.active, contas.done
            )
        }
    };
    barra(
        Span::styled("todo-ratatui", app.theme.accent),
        Span::raw(direita),
        largura,
    )
}

/// Linha 2: o que está escondido fica sempre visível (§4) — filtro, ordem,
/// termo de busca e o indicador do lixo.
fn linha_de_estado(app: &App, largura: u16) -> Line<'static> {
    if matches!(app.mode, InputMode::Trash) {
        // O rótulo é o cabeçalho da coluna da data: com o lixo vazio não há
        // coluna nenhuma para nomear (§4, frame `80x24-13-lixo-vazio`).
        let etiqueta = if app.store().trash_len() == 0 {
            Span::raw("")
        } else {
            Span::raw(ETIQUETA_REMOVIDA)
        };
        return barra(
            Span::raw("vista: lixo · ordem: removidas primeiro"),
            etiqueta,
            largura,
        );
    }

    let mut spans = vec![Span::raw(format!(
        "filtro: {} · ordem: {}",
        app.filter.label(),
        app.sort.label()
    ))];
    if !app.query.is_empty() {
        spans.push(Span::raw(format!(
            " · busca \"{}\"",
            cortar(&app.query, LARGURA_BUSCA)
        )));
    }
    let lixo = app.counts().trash;
    if lixo > 0 {
        // `(cheio)` é persistente enquanto for verdade: o lixo a transbordar não
        // pode sair de vista com a mensagem (§4).
        let (texto, estilo) = if lixo >= TRASH_LIMIT {
            (format!(" · lixo: {lixo} (cheio)"), app.theme.high)
        } else {
            (format!(" · lixo: {lixo} (L)"), app.theme.fg)
        };
        spans.push(Span::styled(texto, estilo));
    }
    let etiqueta = Span::raw(ETIQUETA_DATA);
    let usado: usize = spans.iter().map(Span::width).sum::<usize>() + etiqueta.width();
    spans.push(Span::raw(
        " ".repeat(usize::from(largura).saturating_sub(usado)),
    ));
    spans.push(etiqueta);
    Line::from(spans)
}

/// Uma linha com o conteúdo colado à esquerda e à direita.
fn barra(esquerda: Span<'static>, direita: Span<'static>, largura: u16) -> Line<'static> {
    let usado = esquerda.width() + direita.width();
    let espaco = usize::from(largura).saturating_sub(usado);
    Line::from(vec![esquerda, Span::raw(" ".repeat(espaco)), direita])
}

fn regua(tema: &Theme, largura: u16) -> Line<'static> {
    Line::from(Span::styled("─".repeat(usize::from(largura)), tema.rule))
}

/// Régua com etiqueta à esquerda — a do painel de detalhe (`─ selecionada ─…`).
fn regua_da_selecionada(tema: &Theme, largura: u16) -> Line<'static> {
    regua_com_etiqueta(tema, "─ selecionada ", largura)
}

/// Régua de `largura` colunas com uma etiqueta à cabeça (`─ etiqueta ─────…`),
/// que traz o `─` e o espaço final já incluídos.
fn regua_com_etiqueta(tema: &Theme, etiqueta: &str, largura: u16) -> Line<'static> {
    let resto = usize::from(largura).saturating_sub(etiqueta.width());
    Line::from(Span::styled(
        format!("{etiqueta}{}", "─".repeat(resto)),
        tema.rule,
    ))
}

// ---------------------------------------------------------------------- lista

/// O que a linha de uma tarefa mostra (§2) — venha ela da lista, da vista do
/// lixo ou da amostra da caixa de temas.
///
/// Existe para a matemática de colunas de §2 viver num sítio só. A amostra da
/// caixa (§T.5) não é um `Todo`: os títulos são reais mas curtos e as idades são
/// fixas (uma idade relativa a hoje mudaria o desenho de um dia para o outro) —
/// o que a amostra mostra são os papéis de cor, e desenha-se com estas colunas.
struct Tarefa<'a> {
    titulo: &'a str,
    prioridade: Priority,
    feita: bool,
    /// A coluna da direita, já composta: a idade, ou `✓ há N d` nas concluídas.
    coluna: &'a str,
    /// O papel da coluna: `fg`, excepto nas concluídas, que usam `done`.
    estilo_coluna: Style,
}

/// A linha de uma tarefa da lista.
///
/// É a função que torna os testes de render possíveis (§6): mesma matemática de
/// colunas de §2 — `▶ ` em 0, `[x] ` em 2, `H ` em 6, título em 8 cortado a
/// `largura - 21` e a data alinhada à direita — e preenchimento até `largura`,
/// porque é isso que faz a barra da linha selecionada ser sólida até ao fim.
#[must_use]
pub fn row_line(tema: &Theme, todo: &Todo, selecionado: bool, largura: u16) -> Line<'static> {
    let (coluna, estilo) = if todo.is_done() {
        // Nas concluídas a coluna é a idade de **conclusão**, com o `✓` a
        // desfazer a ambiguidade (§2).
        let quando = todo
            .completed_at
            .map_or_else(hoje, |quando| quando.date_naive());
        (format!("✓ {}", idade(quando, hoje())), tema.done)
    } else {
        (idade(todo.created_at.date_naive(), hoje()), tema.fg)
    };
    linha_de_tarefa(
        tema,
        &Tarefa {
            titulo: &todo.title,
            prioridade: todo.priority,
            feita: todo.is_done(),
            coluna: &coluna,
            estilo_coluna: estilo,
        },
        selecionado,
        largura,
    )
}

/// Linha de uma entrada do lixo: mesma grelha, e a coluna da direita é sempre a
/// data de **remoção** — nunca o `✓` de conclusão.
fn linha_do_lixo(
    tema: &Theme,
    entrada: &Trashed,
    selecionado: bool,
    largura: u16,
) -> Line<'static> {
    let coluna = idade(entrada.deleted_at.date_naive(), hoje());
    linha_de_tarefa(
        tema,
        &Tarefa {
            titulo: &entrada.todo.title,
            prioridade: entrada.todo.priority,
            feita: entrada.todo.is_done(),
            coluna: &coluna,
            estilo_coluna: tema.fg,
        },
        selecionado,
        largura,
    )
}

fn linha_de_tarefa(
    tema: &Theme,
    tarefa: &Tarefa<'_>,
    selecionado: bool,
    largura: u16,
) -> Line<'static> {
    let titulo = cortar(
        tarefa.titulo,
        usize::from(largura.saturating_sub(RESERVA_DIREITA)),
    );
    let marca = marca_de_prioridade(tarefa.prioridade);
    let prefixo = if selecionado { "▶ " } else { "  " };
    let caixa = if tarefa.feita { "[x] " } else { "[ ] " };
    let usado = COLUNA_TITULO + titulo.width() + tarefa.coluna.width();
    let espaco = usize::from(largura).saturating_sub(usado);

    if selecionado {
        // A barra é uniforme: os spans usam só o papel `fg` (§3).
        return Line::from(Span::styled(
            format!(
                "{prefixo}{caixa}{marca} {titulo}{}{}",
                " ".repeat(espaco),
                tarefa.coluna
            ),
            tema.selec,
        ));
    }

    Line::from(vec![
        Span::raw(prefixo.to_owned()),
        Span::styled(
            caixa.to_owned(),
            if tarefa.feita { tema.done } else { tema.fg },
        ),
        Span::styled(
            format!("{marca} "),
            if tarefa.prioridade == Priority::High {
                tema.high
            } else {
                tema.fg
            },
        ),
        Span::styled(titulo, if tarefa.feita { tema.riscado } else { tema.fg }),
        Span::raw(" ".repeat(espaco)),
        Span::styled(tarefa.coluna.to_owned(), tarefa.estilo_coluna),
    ])
}

fn lista(app: &App, largura: u16) -> List<'static> {
    let selecionado = app.list_state.selected();
    let itens: Vec<ListItem<'static>> = if matches!(app.mode, InputMode::Trash) {
        app.store()
            .trash_entries()
            .into_iter()
            .enumerate()
            .map(|(i, entrada)| {
                ListItem::new(linha_do_lixo(
                    app.theme,
                    entrada,
                    selecionado == Some(i),
                    largura,
                ))
            })
            .collect()
    } else {
        app.visible()
            .into_iter()
            .enumerate()
            .map(|(i, todo)| {
                ListItem::new(row_line(app.theme, todo, selecionado == Some(i), largura))
            })
            .collect()
    };
    // O realce cobre a linha toda (é o `row_area` do `List`), e é por isso que
    // não há aqui um segundo preenchimento: a barra sólida vem daqui.
    List::new(itens).highlight_style(app.theme.selec)
}

/// Está-se na vista do lixo e o lixo está vazio (frame `80x24-13-lixo-vazio`)?
///
/// É o estado que decide o corpo e o rodapé: dentro do lixo o contexto é o
/// lixo, logo o vazio do lixo tem **precedência** sobre os vazios da lista
/// (§4).
fn lixo_vazio(app: &App) -> bool {
    matches!(app.mode, InputMode::Trash) && app.store().trash_len() == 0
}

/// Centro da lista quando não há nada para mostrar (§4): «Lixo vazio.» na vista
/// do lixo, «Sem tarefas.» ou o texto que nomeia o filtro e a busca activos —
/// três estados diferentes.
fn vazio(app: &App, largura: u16, altura: u16) -> Vec<Line<'static>> {
    let (primeira, segunda) = if lixo_vazio(app) {
        // Não é o vazio da lista com outro texto: nomear um filtro que aqui
        // não está activo e anunciar o `a` seria anunciar uma tecla morta (§5).
        (
            "Lixo vazio.".to_owned(),
            "O que removeres na lista fica aqui e repõe-se com Enter.".to_owned(),
        )
    } else if app.store().todos().is_empty() {
        (
            "Sem tarefas.".to_owned(),
            "a  adicionar a primeira".to_owned(),
        )
    } else {
        (
            format!(
                "Nenhuma tarefa corresponde a: filtro {} · busca \"{}\"",
                app.filter.label(),
                cortar(&app.query, LARGURA_BUSCA)
            ),
            "Esc  limpa a busca e o filtro     a  cria uma nova".to_owned(),
        )
    };

    let meio = usize::from(altura) / 2;
    let mut linhas = vec![Line::default(); usize::from(altura)];
    if let Some(alvo) = meio.checked_sub(1).and_then(|i| linhas.get_mut(i)) {
        *alvo = centrada(&primeira, largura);
    }
    if let Some(alvo) = linhas.get_mut(meio + 1) {
        *alvo = centrada(&segunda, largura);
    }
    linhas
}

fn centrada(texto: &str, largura: u16) -> Line<'static> {
    let espaco = usize::from(largura).saturating_sub(texto.width()) / 2;
    Line::from(format!("{}{texto}", " ".repeat(espaco)))
}

// ------------------------------------------------------------ painel (§2, §4)

/// As 4 linhas do painel (a régua «selecionada» é desenhada à parte).
fn painel(app: &App, largura: u16) -> Vec<Line<'static>> {
    let Some(todo) = app.selected() else {
        return vec![Line::default(); usize::from(ALTURA_PAINEL - 1)];
    };
    let disponivel = usize::from(largura.saturating_sub(COLUNA_DETALHE as u16 + 2));
    let quando = "%Y-%m-%d %H:%M";
    let conclusao = todo
        .completed_at
        .map_or_else(|| "—".to_owned(), |fim| fim.format(quando).to_string());
    let prazo = todo.due_at.map_or_else(
        || "—".to_owned(),
        |data| data.format("%Y-%m-%d").to_string(),
    );
    let linhas = [
        ("Título", todo.title.clone()),
        ("Descrição", todo.description.clone()),
        (
            "Prioridade",
            format!(
                "{} ({})    ·    criada {}    ·    concluída {conclusao}",
                com_maiuscula(todo.priority.label()),
                marca_de_prioridade(todo.priority),
                todo.created_at.format(quando)
            ),
        ),
        ("Prazo", format!("{prazo}    ·    id {}", todo.id)),
    ];

    linhas
        .into_iter()
        .map(|(chave, valor)| linha_do_painel(app.theme, chave, &cortar(&valor, disponivel)))
        .collect()
}

fn linha_do_painel(tema: &Theme, chave: &str, valor: &str) -> Line<'static> {
    let usado = 2 + chave.width();
    Line::from(vec![
        Span::raw("  "),
        Span::styled(chave.to_owned(), tema.accent),
        Span::raw(" ".repeat(COLUNA_DETALHE.saturating_sub(usado))),
        Span::raw(valor.to_owned()),
    ])
}

// ------------------------------------------------------- linha 22 (estado/input)

/// Linha 22 (§4): erro > mensagem de acção > entrada de texto > pré-visualização
/// do lixo > descrição do selecionado > vazio.
///
/// Com o painel de detalhe visível a descrição **não** se repete aqui — ela
/// está no painel; é abaixo de 96×28 que ela «passa para a linha 22» (§2).
fn mensagem(app: &App, largura: u16, com_painel: bool) -> Line<'static> {
    let util = usize::from(largura).saturating_sub(1);
    if app.mode.is_text() {
        return entrada(app);
    }
    match &app.status {
        Status::Error(texto) => Line::from(Span::styled(abreviar(texto, util), app.theme.err)),
        // Uma mensagem que não expira desenha-se como qualquer outra: o
        // `sticky` decide o prazo, não o estilo (§4).
        Status::Message { texto, .. } | Status::Sticky(texto) => {
            Line::from(Span::styled(abreviar(texto, util), app.theme.fg))
        }
        Status::Idle if matches!(app.mode, InputMode::Trash) => pre_visualizacao_do_lixo(app, util),
        Status::Idle if com_painel => Line::default(),
        Status::Idle => match app.selected() {
            Some(todo) if !todo.description.is_empty() => Line::from(Span::styled(
                abreviar(&format!("Descrição: {}", todo.description), util),
                app.theme.fg,
            )),
            _ => Line::default(),
        },
    }
}

/// `Nova  buffer` com o cursor do terminal no fim — o `▏` dos goldens é só do
/// mockup: no `Buffer` essa célula não tem nada (§8, armadilha 1).
fn entrada(app: &App) -> Line<'static> {
    let mut spans = vec![
        Span::styled(app.mode.label().to_owned(), app.theme.accent),
        Span::raw("  "),
    ];
    let buffer = app.input();
    if buffer.is_empty() {
        spans.push(Span::styled(
            app.mode.placeholder().to_owned(),
            app.theme.placeholder,
        ));
    } else {
        spans.push(Span::raw(buffer));
    }
    Line::from(spans)
}

/// Coluna do cursor, ou `None` fora dos modos de texto (§4).
///
/// Medida com a largura de caracteres (a etiqueta + o buffer antes do cursor),
/// não com `chars().count()`.
fn coluna_do_cursor(app: &App) -> Option<u16> {
    if !app.mode.is_text() {
        return None;
    }
    let antes: String = app.input().chars().take(app.cursor()).collect();
    u16::try_from(app.mode.label().width() + 2 + antes.width()).ok()
}

/// O que o `Enter` faria com a entrada selecionada do lixo — a informação que
/// se quer antes de carregar nele (frame `80x24-10-lixo`).
fn pre_visualizacao_do_lixo(app: &App, util: usize) -> Line<'static> {
    let posicao = app.list_state.selected().and_then(|i| {
        app.store()
            .trash_entries()
            .get(i)
            .map(|entrada| entrada.index.min(app.store().todos().len()) + 1)
    });
    match (posicao, app.selected()) {
        (Some(posicao), Some(todo)) => Line::from(Span::styled(
            abreviar(
                &format!(
                    "Enter restaura «{}» para a posição {posicao} da lista",
                    todo.title
                ),
                util,
            ),
            app.theme.fg,
        )),
        _ => Line::default(),
    }
}

/// Barra do rodapé: só as teclas vivas no contexto actual (§5) — na vista do
/// lixo, por exemplo, `d` e `a` não estão cá.
fn barra_de_ajuda(app: &App) -> Line<'static> {
    let texto = if matches!(app.mode, InputMode::Help) {
        BARRA_AJUDA
    } else if matches!(app.mode, InputMode::Theme) {
        // A caixa de temas é uma sobreposição como a ajuda: as teclas da lista
        // estão desactivadas lá dentro, e a barra só pode dizer o que lá se faz.
        BARRA_TEMAS
    } else if app.mode.is_text() {
        BARRA_INPUT
    } else if matches!(app.mode, InputMode::Trash) {
        // O lixo vazio decide **antes** de `BARRA_LIXO`: ali `Enter`, `Espaço`
        // e `c` são teclas mortas (§5).
        if lixo_vazio(app) {
            BARRA_LIXO_VAZIO
        } else if app.armed() {
            BARRA_ARMADO
        } else {
            BARRA_LIXO
        }
    } else if app.status.is_error() {
        // O erro não expira: a barra diz como sair dele.
        BARRA_ERRO
    } else if app
        .status
        .text()
        .is_some_and(|texto| texto.contains(UNDO_HINT))
    {
        BARRA_UNDO
    } else if app.store().todos().is_empty() {
        BARRA_VAZIO
    } else if app.visible().is_empty() {
        BARRA_SEM_RESULTADOS
    } else if app.query.is_empty() {
        BARRA
    } else {
        BARRA_BUSCA
    };
    Line::from(Span::styled(texto, app.theme.fg))
}

// ------------------------------------------------------------------ ajuda (§4)

/// Caixa de ajuda sobreposta: `Clear` **antes** da caixa (senão a lista fica
/// por baixo) e as teclas do contexto actual, não da lista (§6).
fn desenha_ajuda(frame: &mut Frame, app: &App, area: Rect) {
    let largura = CAIXA_AJUDA.0.min(area.width);
    let altura = CAIXA_AJUDA.1.min(area.height);
    let caixa = Rect::new(
        area.x + (area.width - largura) / 2,
        area.y + (area.height - altura) / 2,
        largura,
        altura,
    );
    frame.render_widget(Clear, caixa);
    // O `Clear` repõe as células a `Reset` — é isso que apaga a lista por baixo
    // — e, com isso, tira-lhes o fundo do tema. Repinta-se a caixa para o tema
    // continuar a valer dentro da sobreposição: sem isto, a ajuda era um
    // rectângulo no fundo do terminal, no meio de um ecrã com fundo nosso.
    pinta_o_fundo(frame, app, caixa);
    frame.render_widget(Paragraph::new(linhas_da_ajuda(app, largura, altura)), caixa);
}

fn linhas_da_ajuda(app: &App, largura: u16, altura: u16) -> Vec<Line<'static>> {
    let colunas = usize::from(largura);
    let teclas = if matches!(app.context(), InputMode::Trash) {
        &TECLAS_AJUDA_LIXO
    } else {
        &TECLAS_AJUDA
    };

    let mut linhas = Vec::with_capacity(usize::from(altura));

    // Topo: `╭── ajuda ─…─╮` (na vista do lixo a caixa é a mesma; o que muda é a
    // lista de teclas lá dentro).
    linhas.push(moldura_do_topo(app.theme, " ajuda ", colunas));

    for indice in 0..usize::from(altura).saturating_sub(2) {
        let mut celulas = vec![('│', app.theme.rule)];
        celulas.extend(std::iter::repeat_n(
            (' ', app.theme.fg),
            colunas.saturating_sub(2),
        ));
        celulas.push(('│', app.theme.rule));
        celulas.truncate(colunas);
        if let Some((esquerda, direita)) = teclas.get(indice) {
            // Os nomes de secção (linha 1) são o único texto em `accent`.
            let estilo = if indice == 1 {
                app.theme.accent
            } else {
                app.theme.fg
            };
            escreve(&mut celulas, COLUNA_AJUDA_ESQ, esquerda, estilo);
            escreve(&mut celulas, COLUNA_AJUDA_DIR, direita, estilo);
        }
        linhas.push(celulas_para_linha(celulas));
    }

    linhas.push(moldura_da_base(app.theme, None, colunas));

    linhas
}

/// Primeira linha de uma caixa sobreposta: `╭── <etiqueta> ─…─╮`.
///
/// A etiqueta é o nome da caixa e o único texto em `accent` da moldura (o resto
/// é a régua do tema).
fn moldura_do_topo(tema: &Theme, etiqueta: &str, colunas: usize) -> Line<'static> {
    let mut celulas = vec![('╭', tema.rule), ('─', tema.rule), ('─', tema.rule)];
    celulas.extend(etiqueta.chars().map(|caracter| (caracter, tema.accent)));
    while celulas.len() < colunas.saturating_sub(1) {
        celulas.push(('─', tema.rule));
    }
    celulas.push(('╮', tema.rule));
    celulas.truncate(colunas);
    celulas_para_linha(celulas)
}

/// Última linha de uma caixa sobreposta: `╰───…──╯` e, quando a caixa tem barra
/// de teclas (a de temas, §T.5), o texto dela a começar na [`COLUNA_CAIXA`], por
/// cima dos `─`.
fn moldura_da_base(tema: &Theme, etiqueta: Option<&str>, colunas: usize) -> Line<'static> {
    let mut celulas = vec![('╰', tema.rule)];
    while celulas.len() < colunas.saturating_sub(1) {
        celulas.push(('─', tema.rule));
    }
    celulas.push(('╯', tema.rule));
    celulas.truncate(colunas);
    if let Some(etiqueta) = etiqueta {
        escreve(&mut celulas, COLUNA_CAIXA, etiqueta, tema.accent);
    }
    celulas_para_linha(celulas)
}

/// Põe `conteudo` dentro das paredes da caixa: `│` na primeira e na última
/// coluna, o texto a começar na [`COLUNA_CAIXA`] (como na ajuda) e o resto do
/// interior preenchido — a parede da direita não pode depender de o conteúdo
/// chegar até lá.
fn com_paredes(tema: &Theme, conteudo: &Line<'static>, colunas: usize) -> Line<'static> {
    // A parede e a coluna em branco: a primeira célula de texto é a
    // `COLUNA_CAIXA` da linha.
    let mut celulas = vec![('│', tema.rule), (' ', tema.fg)];
    for span in &conteudo.spans {
        celulas.extend(span.content.chars().map(|caracter| (caracter, span.style)));
    }
    celulas.resize(colunas.saturating_sub(1), (' ', tema.fg));
    celulas.push(('│', tema.rule));
    celulas.truncate(colunas);
    celulas_para_linha(celulas)
}

fn escreve(celulas: &mut [(char, Style)], coluna: usize, texto: &str, estilo: Style) {
    for (i, caracter) in texto.chars().enumerate() {
        if let Some(celula) = celulas.get_mut(coluna + i) {
            *celula = (caracter, estilo);
        }
    }
}

// ------------------------------------------------------------ temas (§T.5)

/// A barra de teclas da caixa de temas, escrita por cima da moldura de baixo.
const TECLAS_TEMAS: &str = " j / k  escolher · Enter  gravar · Esc  voltar ";

/// As três linhas de amostra da caixa de temas (§T.5).
///
/// A caixa tapa as linhas 5-19 — quase toda a lista — e por trás dela não se vê
/// a barra invertida, nem o `H`, nem o `[x]`. A amostra é o único sítio onde os
/// três papéis aparecem juntos, e é o que permite decidir entre dois temas com
/// `fg`/`high`/`done` parecidos sem sair da caixa.
///
/// Os títulos são os das tarefas do fixture — reais e curtos — e a coluna da
/// direita é **texto fixo**: uma idade relativa a hoje mudaria o desenho de um
/// dia para o outro, e o que a amostra mostra é o desenho dos papéis, não a
/// lista de quem abre a caixa (a lista está por trás, a mudar com o tema).
struct Amostra {
    prioridade: Priority,
    titulo: &'static str,
    /// A coluna da direita, já composta (`há 9 d`, `✓ há 20 d`).
    coluna: &'static str,
    feita: bool,
    /// Leva a barra invertida (o papel `selec`), como a linha selecionada.
    marcada: bool,
}

const AMOSTRA: [Amostra; 3] = [
    Amostra {
        prioridade: Priority::Medium,
        titulo: "Backup do vault BrainStorm",
        coluna: "há 9 d",
        feita: false,
        marcada: true,
    },
    Amostra {
        prioridade: Priority::High,
        titulo: "Rever o PR do dashboard axum",
        coluna: "há 2 d",
        feita: false,
        marcada: false,
    },
    Amostra {
        prioridade: Priority::Low,
        titulo: "Instalar a FiraCode Nerd Font",
        coluna: "✓ há 20 d",
        feita: true,
        marcada: false,
    },
];

/// Caixa de temas sobreposta (§T.5).
///
/// O mesmo rectângulo e o mesmo `Clear` da ajuda ([`CAIXA_TEMAS`]) — e por isso
/// a mesma moldura, desenhada pelos mesmos dois helpers — mas com desenho de
/// **lista** e não de duas colunas de teclas: a lista dos quatro temas do
/// catálogo, a régua da amostra com três linhas de tarefa e a linha do modo de
/// cor. O tema do desenho é o que está a ser pré-visualizado ([`App::theme`]), o
/// que faz a caixa inteira — molduras, nomes e amostra — mudar de cor a cada
/// `j`/`k`.
fn desenha_temas(frame: &mut Frame, app: &App, area: Rect) {
    let largura = CAIXA_TEMAS.0.min(area.width);
    let altura = CAIXA_TEMAS.1.min(area.height);
    let caixa = Rect::new(
        area.x + (area.width - largura) / 2,
        area.y + (area.height - altura) / 2,
        largura,
        altura,
    );
    frame.render_widget(Clear, caixa);
    // Como na ajuda: o `Clear` repõe as células a `Reset` e leva o fundo do tema
    // com ele — repinta-se a caixa logo a seguir.
    pinta_o_fundo(frame, app, caixa);
    frame.render_widget(
        Paragraph::new(linhas_dos_temas(app, largura, altura)),
        caixa,
    );
}

fn linhas_dos_temas(app: &App, largura: u16, altura: u16) -> Vec<Line<'static>> {
    let colunas = usize::from(largura);
    let mut linhas = Vec::with_capacity(usize::from(altura));

    linhas.push(moldura_do_topo(app.theme, " tema ", colunas));
    for conteudo in interiores_dos_temas(app, largura, altura) {
        linhas.push(com_paredes(app.theme, &conteudo, colunas));
    }
    linhas.push(moldura_da_base(app.theme, Some(TECLAS_TEMAS), colunas));

    linhas
}

/// As treze linhas interiores da caixa (a moldura ocupa as outras duas), na
/// ordem do frame `80x24-14-temas`: o título da secção, os quatro temas do
/// [`CATALOGO`], a régua da amostra com as três linhas de tarefa, o modo de cor —
/// e as duas linhas em branco que separam os blocos.
fn interiores_dos_temas(app: &App, largura: u16, altura: u16) -> Vec<Line<'static>> {
    // A largura útil de uma linha de tarefa dentro da caixa: a largura menos a
    // parede, a coluna em branco e o mesmo do lado direito (60 − 4 = 56).
    let util = largura.saturating_sub(4);
    let mut linhas = Vec::new();

    linhas.push(Line::default());
    linhas.push(Line::from(Span::styled("Temas", app.theme.accent)));
    for (indice, tema) in CATALOGO.iter().enumerate() {
        linhas.push(linha_do_catalogo(app, tema, indice, util));
    }
    linhas.push(Line::default());
    linhas.push(regua_da_amostra(app, util));
    for amostra in &AMOSTRA {
        linhas.push(linha_da_amostra(app, amostra, util));
    }
    linhas.push(Line::default());
    linhas.push(linha_do_modo_cor(app));

    // A caixa é recortada por um terminal mais baixo (o `App` não sabe a área do
    // ecrã): o que não couber sai e o que faltar fica em branco — nunca meia
    // caixa com as linhas trocadas.
    linhas.truncate(usize::from(altura).saturating_sub(2));
    linhas.resize(usize::from(altura).saturating_sub(2), Line::default());
    linhas
}

/// Uma linha do catálogo: o nome do tema, `▶ ` se for a do cursor (o que está a
/// ser pré-visualizado) e `em uso` à direita se for a do tema gravado.
///
/// As duas marcas são **glifos, não cor** (§T.5): `▶` é onde se está, `em uso` é
/// o que o `Esc` repõe. Com a caixa acabada de abrir estão na mesma linha —
/// depois de navegar separam-se, e é isso que se lê.
fn linha_do_catalogo(app: &App, tema: &Theme, indice: usize, largura: u16) -> Line<'static> {
    let do_cursor = indice == app.theme_cursor;
    let nome = format!("{}{}", if do_cursor { "▶ " } else { "  " }, tema.nome);
    let estilo = if do_cursor {
        app.theme.accent
    } else {
        app.theme.fg
    };
    if tema.slug != app.tema_gravado().slug {
        return Line::from(Span::styled(nome, estilo));
    }
    barra(
        Span::styled(nome, estilo),
        Span::styled("em uso", app.theme.fg),
        largura,
    )
}

/// `─ amostra · <slug> ─…─` dentro da caixa (§T.5): é a única linha do ecrã onde
/// o *slug* aparece — o nome é para escolher, o slug é para escrever no
/// `config.json` ou no `--theme`.
fn regua_da_amostra(app: &App, largura: u16) -> Line<'static> {
    regua_com_etiqueta(
        app.theme,
        &format!("─ amostra · {} ", app.theme.slug),
        largura,
    )
}

/// Uma das três linhas de amostra: mesma matemática de colunas de §2, à largura
/// da caixa (§T.5) — a barra invertida, o `H` do `high` e o `[x]` riscado do
/// `done`.
fn linha_da_amostra(app: &App, amostra: &Amostra, largura: u16) -> Line<'static> {
    linha_de_tarefa(
        app.theme,
        &Tarefa {
            titulo: amostra.titulo,
            prioridade: amostra.prioridade,
            feita: amostra.feita,
            coluna: amostra.coluna,
            estilo_coluna: if amostra.feita {
                app.theme.done
            } else {
                app.theme.fg
            },
        },
        amostra.marcada,
        largura,
    )
}

/// A linha do modo de cor (§T.5): em `ansi` diz que o ecrã desenha o `classico`.
///
/// Existe para o `ansi` não parecer avariado: sem ela, navegar por quatro temas
/// num terminal de 16 cores mostrava um ecrã que não mudava (ADR §Decisão 3).
fn linha_do_modo_cor(app: &App) -> Line<'static> {
    let texto = if app.modo_cor == ModoCor::Ansi {
        "modo de cor: ansi — o ecrã usa o Clássico"
    } else {
        "modo de cor: rgb"
    };
    Line::from(Span::styled(texto, app.theme.fg))
}

/// Converte células (carácter + estilo) numa linha, juntando vizinhas iguais —
/// é a mesma técnica do gerador dos goldens, e evita depender da matemática dos
/// `Span` para acertar em colunas exactas.
fn celulas_para_linha(celulas: Vec<(char, Style)>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (caracter, estilo) in celulas {
        match spans.last_mut() {
            Some(span) if span.style == estilo => span.content.to_mut().push(caracter),
            _ => spans.push(Span::styled(caracter.to_string(), estilo)),
        }
    }
    Line::from(spans)
}

// ------------------------------------------------------------------ medidas

fn hoje() -> NaiveDate {
    Local::now().date_naive()
}

/// Idade como a coluna da data a mostra (§2): `hoje`, `ontem`, `há 3 d`,
/// `há 2 m` e, a partir de um ano, a data.
///
/// **Sem `há N sem`**: a spec nomeia-o, mas o golden `80×24-1-normal` mostra
/// `há 20 d` para vinte dias e `há 12 d` para doze — as semanas não cabem entre
/// o dia e o mês sem contradizer os frames, e os frames é que são comparados.
fn idade(data: NaiveDate, referencia: NaiveDate) -> String {
    let dias = referencia.signed_duration_since(data).num_days();
    match dias {
        d if d <= 0 => "hoje".to_owned(),
        1 => "ontem".to_owned(),
        d if d < 30 => format!("há {d} d"),
        d if d < 365 => format!("há {} m", d / 30),
        _ => data.format("%Y-%m-%d").to_string(),
    }
}

/// Trunca a `largura` colunas, com `…` final quando corta — por caracteres, não
/// por bytes (§2), e medindo em colunas.
///
/// `pub(crate)` porque o `App` também corta: as mensagens de acção que levam um
/// nome de categoria (§C.4) cortam **o nome** e nunca a contagem, e é esta a
/// função que mede em colunas (um nome com acentos não se corta por bytes).
pub(crate) fn cortar(texto: &str, largura: usize) -> String {
    if texto.width() <= largura {
        return texto.to_owned();
    }
    let mut saida = String::new();
    let mut usado = 0;
    for caracter in texto.chars() {
        let colunas = caracter.width().unwrap_or(0);
        if usado + colunas > largura.saturating_sub(1) {
            break;
        }
        saida.push(caracter);
        usado += colunas;
    }
    saida.push('…');
    saida
}

/// Texto da linha 22 pronto a escrever (§8, armadilha 3): `/home/tiago` passa a
/// `~` e, se ainda não couber, o caminho corta-se **pela esquerda** com `…`,
/// porque num caminho a parte útil é o fim.
fn abreviar(texto: &str, largura: usize) -> String {
    abreviar_com_casa(texto, largura, dirs::home_dir().as_deref())
}

fn abreviar_com_casa(texto: &str, largura: usize, casa: Option<&Path>) -> String {
    let abreviado = match casa {
        Some(casa) => texto.replace(casa.to_string_lossy().as_ref(), "~"),
        None => texto.to_owned(),
    };
    if abreviado.width() <= largura {
        return abreviado;
    }
    // O corte começa no caminho: em `~/…` o `~` sai com os componentes que
    // caírem, senão ficava `~…/db.json`, que ninguém lê.
    let Some(inicio) = abreviado.find("~/").or_else(|| abreviado.find('/')) else {
        return abreviado;
    };
    let (cabeca, cauda) = abreviado.split_at(inicio);
    let mut partes: Vec<&str> = cauda.split('/').skip(1).collect();
    while partes.len() > 1 {
        partes.remove(0);
        let candidato = format!("{cabeca}…/{}", partes.join("/"));
        if candidato.width() <= largura {
            return candidato;
        }
    }
    // Último recurso: fica o nome do ficheiro, ainda identificável.
    format!("{cabeca}…/{}", partes.join("/"))
}

const fn marca_de_prioridade(prioridade: Priority) -> char {
    match prioridade {
        Priority::High => 'H',
        Priority::Medium => 'M',
        Priority::Low => 'L',
    }
}

fn com_maiuscula(texto: &str) -> String {
    let mut caracteres = texto.chars();
    match caracteres.next() {
        Some(primeiro) => primeiro.to_uppercase().chain(caracteres).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use chrono::NaiveDate;

    fn data(ano: i32, mes: u32, dia: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(ano, mes, dia).expect("data válida")
    }

    #[test]
    fn idade_usa_os_rotulos_do_golden() {
        let referencia = data(2026, 9, 18);
        assert_eq!(idade(data(2026, 9, 18), referencia), "hoje");
        assert_eq!(idade(data(2026, 9, 17), referencia), "ontem");
        assert_eq!(idade(data(2026, 9, 16), referencia), "há 2 d");
        assert_eq!(idade(data(2026, 9, 6), referencia), "há 12 d");
        assert_eq!(idade(data(2026, 8, 29), referencia), "há 20 d");
        assert_eq!(idade(data(2026, 8, 20), referencia), "há 29 d");
        assert_eq!(idade(data(2026, 8, 19), referencia), "há 1 m");
        assert_eq!(idade(data(2025, 10, 22), referencia), "há 11 m");
        assert_eq!(idade(data(2025, 1, 5), referencia), "2025-01-05");
        // Uma data no futuro (relógio adiantado) não inventa «há -1 d».
        assert_eq!(idade(data(2026, 9, 25), referencia), "hoje");
    }

    #[test]
    fn cortar_conta_colunas_e_marca_o_corte() {
        assert_eq!(cortar("café", 20), "café", "o que cabe fica inteiro");
        assert_eq!(cortar("café", 4), "café");
        assert_eq!(cortar("café", 3), "ca…");
        let titulo = "Refactor do módulo de persistência para escrita atómica (tmp+rename)";
        let cortado = cortar(titulo, 59);
        assert_eq!(
            cortado,
            "Refactor do módulo de persistência para escrita atómica (t…"
        );
        assert_eq!(cortado.width(), 59, "o corte ocupa as 59 colunas exactas");
        assert_eq!(cortar("", 0), "");
    }

    #[test]
    fn abreviar_troca_a_casa_por_til_e_corta_pela_esquerda() {
        let casa = Path::new("/home/tiago");
        assert_eq!(
            abreviar_com_casa(
                "Erro: não gravou /home/tiago/.local/share/todo-ratatui/db.json — lista intacta",
                79,
                Some(casa)
            ),
            "Erro: não gravou ~/.local/share/todo-ratatui/db.json — lista intacta"
        );
        assert_eq!(
            abreviar_com_casa("sem caminho nenhum", 79, Some(casa)),
            "sem caminho nenhum",
            "sem caminho não há nada para encurtar"
        );
        assert_eq!(
            abreviar_com_casa(
                "Erro: não gravou /home/tiago/.local/share/todo-ratatui/db.json — lista intacta",
                55,
                Some(casa)
            ),
            "Erro: não gravou …/todo-ratatui/db.json — lista intacta",
            "corta pela esquerda, componente a componente, e leva o `~` com eles"
        );
    }

    #[test]
    fn marca_e_nome_da_prioridade() {
        assert_eq!(marca_de_prioridade(Priority::High), 'H');
        assert_eq!(marca_de_prioridade(Priority::Medium), 'M');
        assert_eq!(marca_de_prioridade(Priority::Low), 'L');
        assert_eq!(com_maiuscula("alta"), "Alta");
        assert_eq!(com_maiuscula("média"), "Média");
        assert_eq!(com_maiuscula(""), "");
    }

    #[test]
    fn row_line_preenche_a_largura_toda() {
        let tema = Theme::default();
        let todo = Todo::try_new("Comprar café").expect("título");
        assert_eq!(row_line(tema, &todo, false, 80).width(), 80);
        assert_eq!(
            row_line(tema, &todo, true, 80).width(),
            80,
            "a barra é sólida"
        );
        assert_eq!(row_line(tema, &todo, false, 120).width(), 120);
    }
}
