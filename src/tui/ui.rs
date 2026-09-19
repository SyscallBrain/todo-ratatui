//! Desenho do ecrã: do [`App`] para o `Buffer`, sem estado próprio.
//!
//! A grelha é a do `@designer` (`design-system/todo-ratatui.md` §2–§4) e os
//! testes de `tests/render.rs` comparam-na com os 14 *golden files* de
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
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::core::{Priority, TRASH_LIMIT, Todo, Trashed};

use super::app::{App, Status, UNDO_HINT};
use super::event::InputMode;

// ---------------------------------------------------------------- tokens (§1)

/// Papel `fg` — texto normal. É o estilo por omissão: um `Span::raw` já o tem.
const FG: Style = Style::new();
/// Papel `accent` — nome da app, etiquetas de modo, nomes de secção da ajuda.
const ACCENT: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
/// Papel `high` — prioridade alta e avisos. O `H` faz parte da linha: a cor
/// nunca é o único sinal (§1).
const HIGH: Style = Style::new().fg(Color::Red);
/// Papel `done` — a caixa da concluída.
const DONE: Style = Style::new().fg(Color::Green);
/// Papel `rule` — réguas. `Indexed(8)` dá 3,63:1: chega para gráficos, **não**
/// para texto (§0), por isso só aparece aqui.
const RULE: Style = Style::new().fg(Color::Indexed(8));
/// Papel `err` — vermelho **e** o prefixo «Erro:» no próprio texto.
const ERR: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
/// Linha selecionada: `REVERSED` e **só** o papel `fg` (§3) — um só foreground,
/// para a barra ficar uniforme e nenhuma cor de prioridade se perder.
const SELEC: Style = Style::new().add_modifier(Modifier::REVERSED);
/// Marcador de posição: itálico, não cinzento (cinzento reprovaria AA, §4).
const PLACEHOLDER: Style = Style::new().add_modifier(Modifier::ITALIC);
/// Título da concluída: riscado (§1) — o contraste do texto não se baixa.
const RISCADO: Style = Style::new().add_modifier(Modifier::CROSSED_OUT);

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

/// Teclas da ajuda: a lista principal (`80x24-7-ajuda`).
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
    ("", "i  importar · x  exportar"),
    ("", ""),
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

/// Desenha o ecrã completo a partir de `&App`.
///
/// Função pura: não guarda nada entre chamadas. O `ListState` do `App` é
/// consumido por cópia (é `Copy`), pelo que o deslocamento da lista é
/// recalculado a cada desenho a partir da seleção — que é a única coisa que o
/// `App` promete manter.
pub fn ui(frame: &mut Frame, app: &App) {
    let area = frame.area();
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
    frame.render_widget(Paragraph::new(regua(area.width)), linha(1));
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
            Paragraph::new(regua_da_selecionada(area.width)),
            linha(topo),
        );
        frame.render_widget(
            Paragraph::new(painel(app, area.width)),
            Rect::new(area.x, area.y + topo + 1, area.width, ALTURA_PAINEL - 1),
        );
    }

    let base = area.height - ALTURA_RODAPE;
    frame.render_widget(Paragraph::new(regua(area.width)), linha(base));
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
        Span::styled("todo-ratatui", ACCENT),
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
            (format!(" · lixo: {lixo} (cheio)"), HIGH)
        } else {
            (format!(" · lixo: {lixo} (L)"), FG)
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

fn regua(largura: u16) -> Line<'static> {
    Line::from(Span::styled("─".repeat(usize::from(largura)), RULE))
}

/// Régua com etiqueta à esquerda — a do painel de detalhe (`─ selecionada ─…`).
fn regua_da_selecionada(largura: u16) -> Line<'static> {
    let cabeca = "─ selecionada ";
    let resto = usize::from(largura).saturating_sub(cabeca.width());
    Line::from(Span::styled(format!("{cabeca}{}", "─".repeat(resto)), RULE))
}

// ---------------------------------------------------------------------- lista

/// A linha de uma tarefa da lista.
///
/// É a função que torna os testes de render possíveis (§6): mesma matemática de
/// colunas de §2 — `▶ ` em 0, `[x] ` em 2, `H ` em 6, título em 8 cortado a
/// `largura - 21` e a data alinhada à direita — e preenchimento até `largura`,
/// porque é isso que faz a barra da linha selecionada ser sólida até ao fim.
#[must_use]
pub fn row_line(todo: &Todo, selecionado: bool, largura: u16) -> Line<'static> {
    let (coluna, estilo) = if todo.is_done() {
        // Nas concluídas a coluna é a idade de **conclusão**, com o `✓` a
        // desfazer a ambiguidade (§2).
        let quando = todo
            .completed_at
            .map_or_else(hoje, |quando| quando.date_naive());
        (format!("✓ {}", idade(quando, hoje())), DONE)
    } else {
        (idade(todo.created_at.date_naive(), hoje()), FG)
    };
    linha_de_tarefa(todo, &coluna, estilo, selecionado, largura)
}

/// Linha de uma entrada do lixo: mesma grelha, e a coluna da direita é sempre a
/// data de **remoção** — nunca o `✓` de conclusão.
fn linha_do_lixo(entrada: &Trashed, selecionado: bool, largura: u16) -> Line<'static> {
    let coluna = idade(entrada.deleted_at.date_naive(), hoje());
    linha_de_tarefa(&entrada.todo, &coluna, FG, selecionado, largura)
}

fn linha_de_tarefa(
    todo: &Todo,
    coluna: &str,
    estilo_coluna: Style,
    selecionado: bool,
    largura: u16,
) -> Line<'static> {
    let titulo = cortar(
        &todo.title,
        usize::from(largura.saturating_sub(RESERVA_DIREITA)),
    );
    let marca = marca_de_prioridade(todo.priority);
    let prefixo = if selecionado { "▶ " } else { "  " };
    let caixa = if todo.is_done() { "[x] " } else { "[ ] " };
    let usado = COLUNA_TITULO + titulo.width() + coluna.width();
    let espaco = usize::from(largura).saturating_sub(usado);

    if selecionado {
        // A barra é uniforme: os spans usam só o papel `fg` (§3).
        return Line::from(Span::styled(
            format!(
                "{prefixo}{caixa}{marca} {titulo}{}{coluna}",
                " ".repeat(espaco)
            ),
            SELEC,
        ));
    }

    Line::from(vec![
        Span::raw(prefixo.to_owned()),
        Span::styled(caixa.to_owned(), if todo.is_done() { DONE } else { FG }),
        Span::styled(
            format!("{marca} "),
            if todo.priority == Priority::High {
                HIGH
            } else {
                FG
            },
        ),
        Span::styled(titulo, if todo.is_done() { RISCADO } else { FG }),
        Span::raw(" ".repeat(espaco)),
        Span::styled(coluna.to_owned(), estilo_coluna),
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
                ListItem::new(linha_do_lixo(entrada, selecionado == Some(i), largura))
            })
            .collect()
    } else {
        app.visible()
            .into_iter()
            .enumerate()
            .map(|(i, todo)| ListItem::new(row_line(todo, selecionado == Some(i), largura)))
            .collect()
    };
    // O realce cobre a linha toda (é o `row_area` do `List`), e é por isso que
    // não há aqui um segundo preenchimento: a barra sólida vem daqui.
    List::new(itens).highlight_style(SELEC)
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
        .map(|(chave, valor)| linha_do_painel(chave, &cortar(&valor, disponivel)))
        .collect()
}

fn linha_do_painel(chave: &str, valor: &str) -> Line<'static> {
    let usado = 2 + chave.width();
    Line::from(vec![
        Span::raw("  "),
        Span::styled(chave.to_owned(), ACCENT),
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
        Status::Error(texto) => Line::from(Span::styled(abreviar(texto, util), ERR)),
        // Uma mensagem que não expira desenha-se como qualquer outra: o
        // `sticky` decide o prazo, não o estilo (§4).
        Status::Message { texto, .. } | Status::Sticky(texto) => {
            Line::from(Span::styled(abreviar(texto, util), FG))
        }
        Status::Idle if matches!(app.mode, InputMode::Trash) => pre_visualizacao_do_lixo(app, util),
        Status::Idle if com_painel => Line::default(),
        Status::Idle => match app.selected() {
            Some(todo) if !todo.description.is_empty() => Line::from(Span::styled(
                abreviar(&format!("Descrição: {}", todo.description), util),
                FG,
            )),
            _ => Line::default(),
        },
    }
}

/// `Nova  buffer` com o cursor do terminal no fim — o `▏` dos goldens é só do
/// mockup: no `Buffer` essa célula não tem nada (§8, armadilha 1).
fn entrada(app: &App) -> Line<'static> {
    let mut spans = vec![
        Span::styled(app.mode.label().to_owned(), ACCENT),
        Span::raw("  "),
    ];
    let buffer = app.input();
    if buffer.is_empty() {
        spans.push(Span::styled(app.mode.placeholder().to_owned(), PLACEHOLDER));
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
            FG,
        )),
        _ => Line::default(),
    }
}

/// Barra do rodapé: só as teclas vivas no contexto actual (§5) — na vista do
/// lixo, por exemplo, `d` e `a` não estão cá.
fn barra_de_ajuda(app: &App) -> Line<'static> {
    let texto = if matches!(app.mode, InputMode::Help) {
        BARRA_AJUDA
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
    Line::from(Span::styled(texto, FG))
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
    let mut topo = vec![('╭', RULE), ('─', RULE), ('─', RULE)];
    topo.extend(" ajuda ".chars().map(|caracter| (caracter, ACCENT)));
    while topo.len() < colunas.saturating_sub(1) {
        topo.push(('─', RULE));
    }
    topo.push(('╮', RULE));
    topo.truncate(colunas);
    linhas.push(celulas_para_linha(topo));

    for indice in 0..usize::from(altura).saturating_sub(2) {
        let mut celulas = vec![('│', RULE)];
        celulas.extend(std::iter::repeat_n((' ', FG), colunas.saturating_sub(2)));
        celulas.push(('│', RULE));
        celulas.truncate(colunas);
        if let Some((esquerda, direita)) = teclas.get(indice) {
            // Os nomes de secção (linha 1) são o único texto em `accent`.
            let estilo = if indice == 1 { ACCENT } else { FG };
            escreve(&mut celulas, COLUNA_AJUDA_ESQ, esquerda, estilo);
            escreve(&mut celulas, COLUNA_AJUDA_DIR, direita, estilo);
        }
        linhas.push(celulas_para_linha(celulas));
    }

    let mut base = vec![('╰', RULE)];
    while base.len() < colunas.saturating_sub(1) {
        base.push(('─', RULE));
    }
    base.push(('╯', RULE));
    base.truncate(colunas);
    linhas.push(celulas_para_linha(base));

    linhas
}

fn escreve(celulas: &mut [(char, Style)], coluna: usize, texto: &str, estilo: Style) {
    for (i, caracter) in texto.chars().enumerate() {
        if let Some(celula) = celulas.get_mut(coluna + i) {
            *celula = (caracter, estilo);
        }
    }
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
fn cortar(texto: &str, largura: usize) -> String {
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
        let todo = Todo::try_new("Comprar café").expect("título");
        assert_eq!(row_line(&todo, false, 80).width(), 80);
        assert_eq!(row_line(&todo, true, 80).width(), 80, "a barra é sólida");
        assert_eq!(row_line(&todo, false, 120).width(), 120);
    }
}
