//! Testes de render: o ecrã contra os 14 *golden files* do @designer.
//!
//! Os goldens vivem em `tests/frames/*.txt`, **dentro do repositório**: nenhum
//! teste pode depender do workspace do `@designer`, senão não corre a partir de
//! um clone limpo. Cada teste constrói o estado do `App` que o frame mostra,
//! desenha-o num [`TestBackend`] do tamanho do frame e compara **linha a
//! linha**, com os espaços à direita removidos dos dois lados.
//!
//! Duas armadilhas medidas na spec (`§8`) estão aqui codificadas:
//!
//! 1. O `▏` dos goldens **não** entra no `Buffer` — é o cursor do terminal, que
//!    no mockup se desenhou para o frame ser legível. Nos frames 3 e 4 a célula
//!    do cursor sai do lado do golden e a coluna do cursor afirma-se à parte,
//!    medida em colunas (`unicode-width`), não em caracteres.
//! 2. A linha de erro mostra um caminho que varia com a máquina: compara-se com
//!    o segmento do caminho substituído por `<DB>` nos dois lados (o golden
//!    escreve `…/.local/share/todo-ratatui/db.json`).
//!
//! O que os frames **não** provam — três linhas 22 que são anotação do mockup e
//! não uma mensagem que o `App` produza (frames 2, 5 e 120×32) — é posto no
//! estado pelo próprio fixture, com a razão escrita no teste. O resto é sempre
//! conduzido pelas teclas: se o `App` deixar de produzir uma daquelas linhas, o
//! teste falha (há testes próprios para as mensagens que o `App` escreve).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local, TimeZone};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Position;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

use todo_ratatui::core::{Priority, SortKey, Store, TRASH_LIMIT, Todo, TodoId, Trashed};
use todo_ratatui::tui::app::UNDO_HINT;
use todo_ratatui::tui::ui::ui;
use todo_ratatui::tui::{App, InputMode, Status};

// ------------------------------------------------------------------ os dados

/// As 9 tarefas dos frames. A tabela é **a mesma do gerador dos goldens**
/// (`mockups/todo-ratatui/gerar-frames.py`): títulos, prioridades, idades e
/// descrições têm de coincidir, senão o ecrã não é o do frame.
struct Linha {
    id: &'static str,
    prioridade: Priority,
    titulo: &'static str,
    /// Idade de criação, em dias, relativamente a hoje.
    criada: i64,
    /// Idade de conclusão (`None` = pendente).
    concluida: Option<i64>,
    descricao: &'static str,
}

const TAREFAS: [Linha; 9] = [
    Linha {
        id: "1",
        prioridade: Priority::High,
        titulo: "Rever o PR do dashboard axum",
        criada: 2,
        concluida: None,
        descricao: "conflitos no Cargo.lock; falta o reviewer",
    },
    Linha {
        id: "2",
        prioridade: Priority::High,
        titulo: "Refactor do módulo de persistência para escrita atómica (tmp+rename)",
        criada: 0,
        concluida: None,
        descricao: "ver ADR — tmp ao lado do ficheiro, nunca em /tmp",
    },
    Linha {
        id: "3",
        prioridade: Priority::Medium,
        titulo: "Renovar o Cartão de Cidadão",
        criada: 12,
        concluida: None,
        descricao: "marcar online; leva 2 semanas a chegar",
    },
    Linha {
        id: "4",
        prioridade: Priority::Medium,
        titulo: "Backup do vault BrainStorm para o NAS",
        criada: 9,
        concluida: None,
        descricao: "rsync e verificar o checksum depois",
    },
    Linha {
        id: "5",
        prioridade: Priority::Medium,
        titulo: "Escrever post TugaTux sobre ratatui",
        criada: 5,
        concluida: None,
        descricao: "screenshot do WIP do todo-ratatui",
    },
    Linha {
        id: "6",
        prioridade: Priority::Low,
        titulo: "Comprar café em grão na Nota Roja",
        criada: 0,
        concluida: None,
        descricao: "250 g de Etiópia, moagem média",
    },
    Linha {
        id: "7",
        prioridade: Priority::Low,
        titulo: "Testar o ratatui 0.30 com TestBackend",
        criada: 3,
        concluida: None,
        descricao: "base dos testes de render da T6",
    },
    Linha {
        id: "8",
        prioridade: Priority::Medium,
        titulo: "Configurar tmux no portátil",
        criada: 20,
        concluida: Some(3),
        descricao: "prefixo Ctrl+a, sem rato",
    },
    Linha {
        id: "9",
        prioridade: Priority::Low,
        titulo: "Instalar a FiraCode Nerd Font",
        criada: 30,
        concluida: Some(20),
        descricao: "",
    },
];

/// Data/hora local a `dias` dias de hoje, à hora indicada.
fn quando(dias: i64, hora: u32, minuto: u32) -> DateTime<Local> {
    let data = Local::now().date_naive() - Duration::days(dias);
    let naive = data.and_hms_opt(hora, minuto, 0).expect("hora válida");
    Local
        .from_local_datetime(&naive)
        .earliest()
        .expect("hora local inequívoca")
}

fn construir(linha: &Linha) -> Todo {
    let mut todo = Todo::try_new(linha.titulo).expect("título do fixture");
    // Id explícito e curto: o painel de detalhe mostra-o, e o frame `120x32`
    // conta com `id 2`. (O `App` a sério usa `Uuid` — o golden é que foi feito
    // com ids numerados.)
    todo.id = TodoId::from(linha.id.to_owned());
    todo.priority = linha.prioridade;
    todo.description = linha.descricao.to_owned();
    todo.created_at = quando(linha.criada, 9, 0);
    if let Some(dias) = linha.concluida {
        todo.done = true;
        todo.completed_at = Some(quando(dias, 9, 0));
    }
    todo
}

fn tarefas_do_frame() -> Vec<Todo> {
    TAREFAS.iter().map(construir).collect()
}

fn entrada_do_lixo(
    id: &str,
    prioridade: Priority,
    titulo: &str,
    feita: bool,
    indice: usize,
    removida_em: i64,
) -> Trashed {
    let mut todo = Todo::try_new(titulo).expect("título do fixture");
    todo.id = TodoId::from(id.to_owned());
    todo.priority = prioridade;
    todo.created_at = quando(removida_em + 5, 9, 0);
    if feita {
        todo.done = true;
        todo.completed_at = Some(quando(removida_em + 1, 9, 0));
    }
    Trashed {
        todo,
        index: indice,
        deleted_at: quando(removida_em, 9, 0),
        batch: 1,
    }
}

/// As 3 entradas do lixo dos frames, na ordem em que a `Db` as guarda (a vista
/// mostra-as ao contrário: a remoção mais recente primeiro).
fn lixo_do_frame() -> Vec<Trashed> {
    vec![
        entrada_do_lixo(
            "l3",
            Priority::Medium,
            "Responder ao email do senhorio",
            true,
            6,
            6,
        ),
        entrada_do_lixo(
            "l2",
            Priority::Low,
            "Arquivar os recibos de 2025",
            false,
            5,
            2,
        ),
        entrada_do_lixo(
            "l1",
            Priority::Medium,
            "Pagar a subscrição do Spotify",
            false,
            // Índice 3 na lista: é a «posição 4» que a linha 22 do frame anuncia.
            3,
            0,
        ),
    ]
}

// ------------------------------------------------------------------ fixture

/// Base de dados num directório temporário próprio deste teste.
struct Base {
    caminho: PathBuf,
    store: Store,
}

fn base(tag: &str) -> Base {
    // Nome curto de propósito: a linha 22 do erro mostra este caminho, e um
    // caminho comprido seria abreviado pela esquerda (testado à parte, no
    // `ui`, com um caminho fixo).
    let curto = uuid::Uuid::new_v4().simple().to_string();
    let dir = std::env::temp_dir().join(format!("tr-{tag}-{}-{}", std::process::id(), &curto[..8]));
    fs::create_dir_all(&dir).expect("criar directório de teste");
    let caminho = dir.join("db.json");
    let store = Store::open(&caminho).expect("abrir a base de dados do fixture");
    Base { caminho, store }
}

/// Monta o estado completo e devolve o caminho da base de dados (para as
/// asserções que falam dele) e o `App`.
fn app_com(
    tag: &str,
    tarefas: Vec<Todo>,
    lixo: Vec<Trashed>,
    selecao: Option<usize>,
) -> (PathBuf, App) {
    let mut base = base(tag);
    {
        let db = base.store.db_mut();
        db.todos = tarefas;
        db.trash = lixo;
    }
    base.store.save().expect("gravar o fixture");
    let mut app = App::new(base.store);
    if selecao.is_some() {
        app.list_state.select(selecao);
    }
    (base.caminho, app)
}

/// O estado de `80x24-1-normal`: as 9 tarefas, o lixo a 3 e a seleção no 2.º.
fn app_da_lista(tag: &str) -> (PathBuf, App) {
    app_com(tag, tarefas_do_frame(), lixo_do_frame(), Some(1))
}

// ------------------------------------------------------------------ teclas

fn carrega(app: &mut App, tecla: char) {
    app.on_key(KeyEvent::new(KeyCode::Char(tecla), KeyModifiers::NONE));
}

fn escreve(app: &mut App, texto: &str) {
    for caracter in texto.chars() {
        carrega(app, caracter);
    }
}

fn escapa(app: &mut App) {
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
}

/// `/` + termo + `Enter`.
fn abre_busca(app: &mut App, termo: &str) {
    carrega(app, '/');
    escreve(app, termo);
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

// ------------------------------------------------------------------ ecrã

/// O ecrã desenhado, com o terminal de teste que o produziu.
struct Ecra {
    terminal: Terminal<TestBackend>,
    linhas: Vec<String>,
}

fn desenhar(app: &App, largura: u16, altura: u16) -> Ecra {
    let mut terminal = Terminal::new(TestBackend::new(largura, altura)).expect("terminal de teste");
    terminal
        .draw(|frame| ui(frame, app))
        .expect("desenhar o ecrã");
    let linhas = linhas_do_buffer(terminal.backend().buffer());
    Ecra { terminal, linhas }
}

impl Ecra {
    fn cursor(&self) -> Position {
        self.terminal.backend().cursor_position()
    }

    fn cursor_visivel(&self) -> bool {
        self.terminal.backend().cursor_visible()
    }

    fn estilo(&self, x: u16, y: u16) -> Style {
        self.terminal
            .backend()
            .buffer()
            .cell((x, y))
            .expect("célula dentro do ecrã")
            .style()
    }
}

/// Cada linha do `Buffer` sem os espaços à direita: os goldens estão
/// preenchidos até à última coluna (é o que faz a barra ser sólida) e as
/// células vazias à direita não são conteúdo (armadilha 2).
fn linhas_do_buffer(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).map_or(" ", Cell::symbol))
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

// ------------------------------------------------------------------ goldens

fn caminho_do_golden(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("frames")
        .join(format!("{nome}.txt"))
}

fn golden(nome: &str) -> Vec<String> {
    let caminho = caminho_do_golden(nome);
    fs::read_to_string(&caminho)
        .unwrap_or_else(|erro| panic!("golden «{}» ilegível: {erro}", caminho.display()))
        .lines()
        .map(|linha| linha.trim_end().to_owned())
        .collect()
}

fn aplica(linha: &str, mascaras: &[(String, String)]) -> String {
    mascaras.iter().fold(linha.to_owned(), |texto, (de, para)| {
        texto.replace(de, para)
    })
}

fn compara(nome: &str, obtido: &[String], esperado: &[String]) {
    assert_eq!(
        obtido.len(),
        esperado.len(),
        "{nome}: o ecrã tem {} linhas e o golden tem {}",
        obtido.len(),
        esperado.len()
    );
    let divergencias: Vec<String> = obtido
        .iter()
        .zip(esperado)
        .enumerate()
        .filter(|(_, (obtida, esperada))| obtida != esperada)
        .map(|(y, (obtida, esperada))| {
            format!("  linha {y}:\n    golden |{esperada}|\n    ecrã   |{obtida}|")
        })
        .collect();
    assert!(
        divergencias.is_empty(),
        "{nome}: {} de {} linhas diferentes\n{}",
        divergencias.len(),
        esperado.len(),
        divergencias.join("\n")
    );
}

fn assert_frame(nome: &str, app: &App, largura: u16, altura: u16) -> Ecra {
    assert_frame_mascarado(nome, app, largura, altura, &[])
}

fn assert_frame_mascarado(
    nome: &str,
    app: &App,
    largura: u16,
    altura: u16,
    mascaras: &[(String, String)],
) -> Ecra {
    let ecra = desenhar(app, largura, altura);
    let esperado: Vec<String> = golden(nome)
        .iter()
        .map(|linha| aplica(linha, mascaras))
        .collect();
    let obtido: Vec<String> = ecra
        .linhas
        .iter()
        .map(|linha| aplica(linha, mascaras))
        .collect();
    compara(nome, &obtido, &esperado);
    ecra
}

/// O `▏` do golden é o cursor do terminal: sai da comparação (armadilha 1).
fn sem_cursor() -> Vec<(String, String)> {
    vec![("▏".to_owned(), String::new())]
}

/// Máscaras para uma linha que mostra o caminho da base de dados (critério 2):
/// o golden escreve a forma curta e o teste corre num directório temporário.
fn mascaras_do_caminho(caminho: &Path) -> Vec<(String, String)> {
    let bruto = caminho.display().to_string();
    let mut mascaras = vec![
        (
            "…/.local/share/todo-ratatui/db.json".to_owned(),
            "<DB>".to_owned(),
        ),
        (bruto.clone(), "<DB>".to_owned()),
    ];
    if let Some(casa) = dirs::home_dir()
        && let Some(sem_casa) = bruto.strip_prefix(&casa.display().to_string())
    {
        mascaras.push((format!("~{sem_casa}"), "<DB>".to_owned()));
    }
    mascaras
}

// ------------------------------------------------------------------ os frames

/// (a) Três tarefas em estados de conclusão diferentes, acentos e truncagem
/// com `…` — e as concluídas no fim, cada grupo por prioridade.
#[test]
fn frame_1_normal() {
    let (_caminho, app) = app_da_lista("normal");
    let ecra = assert_frame("80x24-1-normal", &app, 80, 24);

    assert!(
        ecra.linhas[3].contains("[ ] H Rever o PR do dashboard axum"),
        "pendentes em cima, com a prioridade pela letra"
    );
    assert!(
        ecra.linhas[4]
            .starts_with("▶ [ ] H Refactor do módulo de persistência para escrita atómica (t…"),
        "a linha selecionada é a barra invertida e o título corta com `…`: {}",
        ecra.linhas[4]
    );
    assert!(
        ecra.linhas[10].contains("[x] M Configurar tmux no portátil")
            && ecra.linhas[10].contains("✓ há 3 d"),
        "concluída: `[x]` e a idade de conclusão com `✓`"
    );
    assert!(
        ecra.linhas[9].contains("[ ] L Testar o ratatui"),
        "as pendentes `L` ficam **antes** da `M` concluída: é a decisão de apresentação do `tui` (o `SortKey` do core é puro)"
    );
    assert!(
        ecra.linhas[8].contains("Comprar café em grão") && ecra.linhas[4].contains("módulo"),
        "acentos e cedilhas desenham-se nas mesmas colunas"
    );
    assert_eq!(
        ecra.linhas[23],
        "a nova  e editar  Espaço concluir  d remover  u desfazer  / buscar  ? ajuda",
        "a barra de ajuda cabe inteira em 80 colunas"
    );
}

/// Busca e filtro activos: o que está escondido fica visível no cabeçalho e na
/// linha 2.
#[test]
fn frame_2_busca_e_filtro() {
    let (_caminho, mut app) = app_da_lista("busca");
    carrega(&mut app, 'f');
    abre_busca(&mut app, "café");
    assert!(
        app.status.text().is_some_and(|t| t.contains("resultados")),
        "o `App` confirma a busca com a própria mensagem (que não é a do frame)"
    );
    // A linha 22 deste frame é a anotação do mockup para o estado de busca —
    // não uma mensagem que o `App` escreva —, e o gerador não marcou seleção
    // nenhuma (passou o índice 5 a uma lista de um item). O fixture reproduz o
    // que o frame mostra; o que este frame fixa é o cabeçalho e a linha 2.
    app.status = Status::message(
        "Busca activa: 1 de 7 pendentes    Esc limpa a busca e o filtro".to_owned(),
    );
    app.list_state.select(None);

    let ecra = assert_frame("80x24-2-busca-filtro", &app, 80, 24);
    assert!(
        ecra.linhas[0].contains("1 de 9 tarefas"),
        "{}",
        ecra.linhas[0]
    );
    assert!(
        ecra.linhas[2].contains("filtro: pendentes · ordem: prioridade · busca \"café\""),
        "{}",
        ecra.linhas[2]
    );
}

/// (b) Modo `Adding` com o cursor na coluna certa.
#[test]
fn frame_3_adicionar() {
    let (_caminho, mut app) = app_da_lista("adicionar");
    carrega(&mut app, 'a');
    escreve(&mut app, "Comprar café em grão na Nota Roja");
    assert_eq!(app.mode, InputMode::Adding);

    let ecra = assert_frame_mascarado("80x24-3-adicionar", &app, 80, 24, &sem_cursor());

    // Armadilha 1: a coluna do cursor afirma-se à parte, medida em colunas —
    // largura da etiqueta + largura do buffer, não `chars().count()`.
    assert!(ecra.cursor_visivel(), "nos modos de texto o cursor aparece");
    assert_eq!(ecra.cursor(), Position::new(39, 22));
    assert_eq!(
        usize::from(ecra.cursor().x),
        "Nova".width() + 2 + app.input().width(),
        "a etiqueta «Nova» (4) + 2 espaços + o buffer (33 colunas)"
    );
}

/// (b2) Buffer vazio: marcador de posição itálico, não cinzento.
#[test]
fn frame_4_adicionar_vazio() {
    let (_caminho, mut app) = app_da_lista("adicionar-vazio");
    carrega(&mut app, 'a');
    assert_eq!(app.input(), "");

    let ecra = assert_frame_mascarado("80x24-4-adicionar-vazio", &app, 80, 24, &sem_cursor());
    assert_eq!(
        ecra.cursor(),
        Position::new(6, 22),
        "o cursor fica a seguir à etiqueta"
    );
    assert!(
        ecra.estilo(6, 22).add_modifier.contains(Modifier::ITALIC),
        "o marcador é itálico (não cinzento: cinzento reprovaria AA)"
    );
}

/// Mensagem de remoção: enquanto houver undo a barra reduz-se à única acção que
/// resta.
#[test]
fn frame_5_undo() {
    let (_caminho, mut app) = app_da_lista("undo");
    // A mensagem é a do próprio `App` (mesmo `format!`, com a marca partilhada
    // `UNDO_HINT`): o que o frame mostra é o estado *com* a mensagem, não o
    // estado depois de remover — a lista ainda tem as 9.
    app.status = Status::message(format!(
        "Removida «Comprar café em grão na Nota Roja»    {UNDO_HINT}"
    ));
    let ecra = assert_frame("80x24-5-undo", &app, 80, 24);
    assert_eq!(ecra.linhas[23], "u desfazer  ? ajuda");
}

/// Erro de escrita: vermelho, com o prefixo «Erro:», o caminho da base de dados
/// e a garantia de que a lista em memória não mudou.
#[test]
fn frame_6_erro() {
    let (caminho, mut app) = app_da_lista("erro");
    let antes = app.visible().len();

    // Falha de gravação a sério: o directório fica sem permissão de escrita e a
    // acção seguinte não consegue gravar. A acção é `1` (prioridade alta) sobre
    // uma tarefa que já é alta: nada muda na lista, e é isso que a mensagem
    // promete.
    let dir = caminho
        .parent()
        .expect("directório da base de dados")
        .to_path_buf();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).expect("retirar a escrita");
    carrega(&mut app, '1');
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("devolver a escrita");

    assert!(
        app.status.is_error(),
        "gravar tinha de falhar: {:?}",
        app.status.text()
    );
    assert_eq!(app.visible().len(), antes, "a lista em memória não mudou");

    let ecra = assert_frame_mascarado("80x24-6-erro", &app, 80, 24, &mascaras_do_caminho(&caminho));

    let linha = &ecra.linhas[22];
    assert!(linha.starts_with("Erro: não gravou "), "{linha}");
    assert!(linha.ends_with("— lista intacta"), "{linha}");
    assert!(linha.contains(&caminho.display().to_string()), "{linha}");
    assert!(
        linha.width() <= 79,
        "a linha 22 tem 79 colunas úteis: {linha}"
    );
    let estilo = ecra.estilo(0, 22);
    assert_eq!(estilo.fg, Some(Color::Red), "o erro é vermelho");
    assert!(
        estilo.add_modifier.contains(Modifier::BOLD),
        "e o prefixo «Erro:» é o sinal que não depende da cor"
    );
    assert_eq!(ecra.linhas[23], "Esc  limpar a mensagem    q  sair");
}

/// Ajuda sobreposta: 60×15 ao centro, com a área limpa antes (`Clear`).
#[test]
fn frame_7_ajuda() {
    let (_caminho, mut app) = app_da_lista("ajuda");
    carrega(&mut app, '?');
    assert_eq!(app.mode, InputMode::Help);

    let ecra = assert_frame("80x24-7-ajuda", &app, 80, 24);
    assert!(
        ecra.linhas[12].starts_with("          │"),
        "a caixa limpa o interior: a lista não aparece por baixo"
    );
    assert!(ecra.linhas[4].contains("╭── ajuda ") && ecra.linhas[18].contains("╰──"));
}

/// Base vazia: não é uma lista em branco — diz o que se passa e a tecla que
/// resolve.
#[test]
fn frame_8_vazio() {
    let (_caminho, app) = app_com("vazio", Vec::new(), lixo_do_frame(), None);
    let ecra = assert_frame("80x24-8-vazio", &app, 80, 24);
    assert!(ecra.linhas[0].contains("0 tarefas · 0 pendentes · 0 concluídas"));
    assert_eq!(
        ecra.linhas[11],
        format!("{}Sem tarefas.", " ".repeat(34)),
        "o texto fica centrado na lista"
    );
    assert_eq!(
        ecra.linhas[13],
        format!("{}a  adicionar a primeira", " ".repeat(28))
    );
    assert_eq!(ecra.linhas[23], "a nova  ? ajuda");
}

/// Busca sem resultados: distingue «não há tarefas» de «há, mas o filtro
/// esconde».
#[test]
fn frame_9_sem_resultados() {
    let (_caminho, mut app) = app_da_lista("sem-resultados");
    carrega(&mut app, 'f');
    abre_busca(&mut app, "netflix");
    escapa(&mut app); // tira a mensagem: o frame está em repouso
    assert_eq!(app.status, Status::Idle);

    let ecra = assert_frame("80x24-9-sem-resultados", &app, 80, 24);
    assert_eq!(ecra.linhas[22], "", "sem mensagem na linha 22");
    assert!(ecra.linhas[11].contains("Nenhuma tarefa corresponde a: filtro pendentes"));
    assert_eq!(ecra.linhas[23], "Esc limpar  a nova  ? ajuda");
}

/// (d) A vista do lixo distingue-se da lista: outra linha 2, outro rótulo da
/// coluna da data e o que o `Enter` faria.
#[test]
fn frame_10_lixo() {
    let (_caminho, mut app) = app_da_lista("lixo");
    carrega(&mut app, 'L');
    assert_eq!(app.mode, InputMode::Trash);
    // O frame tem a primeira entrada do lixo selecionada (a remoção mais
    // recente); entrar na vista conserva o índice, e o fixture põe-no onde o
    // frame o mostra.
    app.list_state.select(Some(0));

    let ecra = assert_frame("80x24-10-lixo", &app, 80, 24);
    assert!(ecra.linhas[0].starts_with("todo-ratatui"));
    assert!(
        ecra.linhas[0].ends_with("lixo: 3 de 100"),
        "no lixo o cabeçalho conta o lixo: {}",
        ecra.linhas[0]
    );
    assert!(ecra.linhas[2].starts_with("vista: lixo · ordem: removidas primeiro"));
    assert!(ecra.linhas[2].ends_with("removida"));
    assert!(
        ecra.linhas[22]
            .contains("Enter restaura «Pagar a subscrição do Spotify» para a posição 4 da lista")
    );
    assert_eq!(
        ecra.linhas[23], "Enter restaura  c esvaziar o lixo  Esc volta  ? ajuda",
        "no lixo o rodapé só mostra as teclas vivas neste contexto"
    );
}

/// A guarda do `c`: a única operação sem undo tem de ser armada.
#[test]
fn frame_11_lixo_armado() {
    let (_caminho, mut app) = app_da_lista("lixo-armado");
    carrega(&mut app, 'L');
    carrega(&mut app, 'c');
    assert!(app.armed(), "a primeira pressão arma");
    assert_eq!(app.store().trash_len(), 3, "e não esvazia nada");
    app.list_state.select(Some(0));

    let ecra = assert_frame("80x24-11-lixo-armado", &app, 80, 24);
    assert!(ecra.linhas[22].contains("Esvaziar o lixo? 3 entradas, sem volta atrás"));
    assert_eq!(ecra.linhas[23], "c confirmar  Esc cancela");
}

/// Transbordo do lixo: as entradas antigas saem e isso é anunciado — nada sai
/// em silêncio.
#[test]
fn frame_12_lixo_transbordo() {
    let mut tarefas = tarefas_do_frame();
    let mut extra = Todo::try_new("tarefa temporária").expect("título do fixture");
    extra.id = TodoId::from("extra".to_owned());
    extra.priority = Priority::Low;
    extra.created_at = quando(1, 9, 0);
    tarefas.push(extra);

    // 101 entradas no lixo + a remoção da tarefa extra = 102: o limite deita
    // fora as 2 mais antigas, e a mensagem diz quantas.
    let lixo: Vec<Trashed> = (0..TRASH_LIMIT + 1)
        .map(|i| {
            entrada_do_lixo(
                &format!("d{i}"),
                Priority::Low,
                &format!("entrada antiga {i}"),
                false,
                i,
                40,
            )
        })
        .collect();

    let (_caminho, mut app) = app_com("transbordo", tarefas, lixo, None);
    let indice = app
        .visible()
        .iter()
        .position(|todo| todo.title == "tarefa temporária")
        .expect("a tarefa extra está na vista");
    app.list_state.select(Some(indice));
    carrega(&mut app, 'd');

    assert_eq!(app.store().trash_len(), TRASH_LIMIT, "o lixo fica cheio");
    assert_eq!(app.visible().len(), 9, "só a extra saiu da lista");
    assert!(
        app.status.text().is_some_and(
            |texto| texto.contains("Aviso: 2 entradas antigas saíram do lixo (limite 100)")
        ),
        "a mensagem diz quantas saíram: {:?}",
        app.status.text()
    );
    app.list_state.select(Some(1));

    let ecra = assert_frame("80x24-12-lixo-transbordo", &app, 80, 24);
    assert!(
        ecra.linhas[2].contains("lixo: 100 (cheio)"),
        "o indicador persiste enquanto for verdade: {}",
        ecra.linhas[2]
    );
}

/// Vista do lixo sem nada lá dentro: estado próprio (não o vazio da lista, que
/// nomearia um filtro inactivo e anunciaria o `a`, tecla morta nesta vista) e
/// rodapé reduzido às teclas que aqui fazem alguma coisa.
#[test]
fn frame_13_lixo_vazio() {
    let (_caminho, mut app) = app_com("lixo-vazio", tarefas_do_frame(), Vec::new(), None);
    carrega(&mut app, 'L');
    assert_eq!(app.mode, InputMode::Trash);
    assert!(app.visible().is_empty(), "o lixo está vazio");

    let ecra = assert_frame("80x24-13-lixo-vazio", &app, 80, 24);
    assert!(
        ecra.linhas[0].ends_with("lixo: 0 de 100"),
        "{}",
        ecra.linhas[0]
    );
    assert_eq!(
        ecra.linhas[11],
        format!("{}Lixo vazio.", " ".repeat(34)),
        "o corpo é o do lixo vazio, centrado como os outros vazios"
    );
    assert_eq!(
        ecra.linhas[13],
        format!(
            "{}O que removeres na lista fica aqui e repõe-se com Enter.",
            " ".repeat(12)
        )
    );
    assert_eq!(
        ecra.linhas[2], "vista: lixo · ordem: removidas primeiro",
        "sem lixo não há coluna de datas para nomear: o rótulo `removida` sai"
    );
    assert_eq!(ecra.linhas[22], "", "sem mensagem na linha 22");
    assert_eq!(
        ecra.linhas[23], "Esc volta à lista  ? ajuda",
        "o rodapé só mostra as teclas vivas neste contexto"
    );

    // §4: dentro do lixo o vazio do lixo tem **precedência** sobre os vazios da
    // lista. Com a base também vazia, o que decide é o do lixo.
    let (_outro, mut sem_nada) = app_com("lixo-vazio-base", Vec::new(), Vec::new(), None);
    carrega(&mut sem_nada, 'L');
    let ecra = desenhar(&sem_nada, 80, 24);
    assert_eq!(
        ecra.linhas[11],
        format!("{}Lixo vazio.", " ".repeat(34)),
        "com a base vazia também, o corpo continua a ser o do lixo"
    );
    assert!(
        !ecra.linhas[11].contains("Sem tarefas"),
        "e não o vazio da lista: {}",
        ecra.linhas[11]
    );
    assert_eq!(ecra.linhas[23], "Esc volta à lista  ? ajuda");
}

/// Painel de detalhe: só existe a partir de 96×28 (o corte é medido em colunas
/// × linhas, não em pixéis).
#[test]
fn frame_120x32_detalhe() {
    let mut tarefas = tarefas_do_frame();
    // O frame mostra a tarefa criada às 14:03 do dia em que o mockup foi feito;
    // a hora é a de hoje (a idade da lista tem de continuar a ser «hoje») e a
    // data/hora absoluta compara-se mascarada, senão o teste só passava num dia.
    tarefas[1].created_at = quando(0, 14, 3);
    let (_caminho, mut app) = app_com("detalhe", tarefas, lixo_do_frame(), Some(1));
    // A linha 22 deste frame é uma anotação do mockup (o painel é que está em
    // causa).
    app.status = Status::message(
        "A descrição do selecionado aparece no painel quando houver altura; a partir de 96×28"
            .to_owned(),
    );

    let marca = quando(0, 14, 3).format("%Y-%m-%d %H:%M").to_string();
    let mascaras = vec![
        ("2026-09-18 14:03".to_owned(), "<DATA>".to_owned()),
        (marca.clone(), "<DATA>".to_owned()),
    ];
    let ecra = assert_frame_mascarado("120x32-detalhe", &app, 120, 32, &mascaras);

    assert!(ecra.linhas[24].contains("─ selecionada "));
    assert!(ecra.linhas[25].contains("Título") && ecra.linhas[25].contains("Refactor do módulo"));
    assert!(ecra.linhas[26].contains("Descrição") && ecra.linhas[26].contains("ver ADR"));
    assert!(
        ecra.linhas[27].contains("Alta (H)") && ecra.linhas[27].contains(&marca),
        "a data de criação aparece formatada: {}",
        ecra.linhas[27]
    );
    assert!(ecra.linhas[27].contains("concluída —"));
    assert!(ecra.linhas[28].contains("id 2"), "{}", ecra.linhas[28]);
}

// ------------------------------------------------------- regras de desenho

#[test]
fn os_quatorze_goldens_estao_no_repo() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("frames");
    let mut ficheiros: Vec<String> = fs::read_dir(&dir)
        .expect("ler tests/frames")
        .map(|entrada| {
            entrada
                .expect("entrada")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|nome| nome.ends_with(".txt"))
        .collect();
    ficheiros.sort();
    assert_eq!(
        ficheiros,
        [
            "120x32-detalhe.txt",
            "80x24-1-normal.txt",
            "80x24-10-lixo.txt",
            "80x24-11-lixo-armado.txt",
            "80x24-12-lixo-transbordo.txt",
            "80x24-13-lixo-vazio.txt",
            "80x24-2-busca-filtro.txt",
            "80x24-3-adicionar.txt",
            "80x24-4-adicionar-vazio.txt",
            "80x24-5-undo.txt",
            "80x24-6-erro.txt",
            "80x24-7-ajuda.txt",
            "80x24-8-vazio.txt",
            "80x24-9-sem-resultados.txt",
        ],
        "os 14 goldens fazem parte do repositório e cada um tem o seu teste"
    );
}

#[test]
fn nenhum_teste_depende_do_workspace_do_designer() {
    // Os goldens foram *copiados* para dentro do repositório: um teste que
    // apontasse para o workspace do @designer só passava nesta máquina.
    let agulha = format!("profiles{}{}", '/', "designer");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut suspeitos = Vec::new();
    for entrada in fs::read_dir(&dir).expect("ler tests/") {
        let caminho = entrada.expect("entrada").path();
        if caminho.extension().is_some_and(|ext| ext == "rs") {
            let conteudo = fs::read_to_string(&caminho).expect("ler ficheiro de teste");
            if conteudo.contains(&agulha) {
                suspeitos.push(caminho.display().to_string());
            }
        }
    }
    assert!(
        suspeitos.is_empty(),
        "testes a apontar para o workspace de outro perfil: {suspeitos:?}"
    );
}

#[test]
fn o_painel_de_detalhe_so_existe_a_partir_de_96x28() {
    let (_caminho, app) = app_da_lista("limite-painel");
    for (largura, altura, esperado) in [
        (120, 32, true),
        (96, 28, true),
        (95, 32, false),
        (96, 27, false),
        (120, 27, false),
    ] {
        let ecra = desenhar(&app, largura, altura);
        let tem_painel = ecra
            .linhas
            .iter()
            .any(|linha| linha.contains("─ selecionada "));
        assert_eq!(
            tem_painel, esperado,
            "{largura}×{altura}: painel de detalhe"
        );
        let linha_22 = &ecra.linhas[usize::from(altura) - 2];
        assert_eq!(
            linha_22.starts_with("Descrição: "),
            !esperado,
            "{largura}×{altura}: sem painel a descrição do selecionado vive na linha 22: {linha_22}"
        );
    }
}

#[test]
fn terminal_pequeno_mostra_uma_so_mensagem() {
    let (_caminho, app) = app_da_lista("pequeno");

    // Numa largura suficiente lê-se a mensagem inteira, e não há mais nada.
    let ecra = desenhar(&app, 80, 9);
    assert_eq!(
        ecra.linhas[0],
        "Terminal demasiado pequeno (80×9); mínimo 40×10"
    );
    assert!(
        ecra.linhas[1..].iter().all(String::is_empty),
        "sem desenho parcial: {:?}",
        &ecra.linhas[1..]
    );

    // Mais estreito do que a mensagem, o terminal corta-a — é o terminal, não
    // um desenho parcial nosso.
    for (largura, altura) in [(39, 24), (39, 9)] {
        let ecra = desenhar(&app, largura, altura);
        assert!(
            ecra.linhas[0].starts_with("Terminal demasiado pequeno (39×"),
            "{}",
            ecra.linhas[0]
        );
        assert!(
            ecra.linhas[1..].iter().all(String::is_empty),
            "{largura}×{altura}: {:?}",
            &ecra.linhas[1..]
        );
    }

    // 40×10 é o mínimo suportado: já desenha o ecrã completo (o que não couber
    // na largura é cortado pelo terminal).
    let ecra = desenhar(&app, 40, 10);
    assert!(
        ecra.linhas[0].starts_with("todo-ratatui"),
        "{}",
        ecra.linhas[0]
    );
    assert!(ecra.linhas[9].starts_with("a nova"), "{}", ecra.linhas[9]);
}

#[test]
fn o_cursor_so_aparece_nos_modos_de_texto() {
    let (_caminho, app) = app_da_lista("cursor-repouso");
    let ecra = desenhar(&app, 80, 24);
    assert!(!ecra.cursor_visivel(), "em repouso o cursor fica escondido");

    for (tecla, etiqueta) in [
        ('a', "Nova"),
        ('e', "Editar"),
        ('/', "Busca"),
        ('i', "Importar"),
        ('x', "Exportar"),
    ] {
        let (_caminho, mut app) = app_da_lista("cursor-texto");
        carrega(&mut app, tecla);
        assert!(app.mode.is_text(), "«{tecla}» abre um modo de texto");
        escreve(&mut app, "café");
        let ecra = desenhar(&app, 80, 24);
        assert!(
            ecra.cursor_visivel(),
            "«{tecla}»: {} é um modo de texto",
            app.mode.label()
        );
        assert_eq!(ecra.cursor().y, 22, "«{tecla}»: o cursor vive na linha 22");
        assert_eq!(
            usize::from(ecra.cursor().x),
            etiqueta.width() + 2 + app.input().width(),
            "«{tecla}»: etiqueta + 2 espaços + buffer, medido em colunas"
        );
    }
}

#[test]
fn a_linha_selecionada_e_uma_barra_invertida_solida() {
    let (_caminho, app) = app_da_lista("barra-tokens");
    let ecra = desenhar(&app, 80, 24);

    // §3: a barra é `REVERSED` e cobre a linha toda — do `▶` à última coluna.
    for x in [0u16, 8, 40, 79] {
        assert!(
            ecra.estilo(x, 4).add_modifier.contains(Modifier::REVERSED),
            "coluna {x} da linha selecionada"
        );
    }
    for x in [0u16, 8, 40, 79] {
        assert!(
            !ecra.estilo(x, 3).add_modifier.contains(Modifier::REVERSED),
            "coluna {x} de uma linha não selecionada"
        );
    }

    // §1: só o `H` puxa a cor da prioridade; a concluída é `[x]` verde com o
    // título riscado; a régua é `Indexed(8)` (e não pinta texto nenhum).
    assert_eq!(
        ecra.estilo(6, 3).fg,
        Some(Color::Red),
        "o `H` da prioridade alta"
    );
    assert_eq!(
        ecra.estilo(2, 10).fg,
        Some(Color::Green),
        "`[x]` da concluída"
    );
    assert!(
        ecra.estilo(8, 10)
            .add_modifier
            .contains(Modifier::CROSSED_OUT),
        "o título da concluída é riscado"
    );
    assert_eq!(ecra.estilo(0, 1).fg, Some(Color::Indexed(8)), "a régua");
    assert_eq!(
        ecra.estilo(0, 0).fg,
        Some(Color::Cyan),
        "o nome da app é `accent`"
    );
}

#[test]
fn a_barra_do_desfazer_so_reduz_com_a_mensagem_de_remocao() {
    let (_caminho, mut app) = app_da_lista("barra-undo");
    carrega(&mut app, 'j');
    carrega(&mut app, 'd');
    assert!(
        app.status
            .text()
            .is_some_and(|texto| texto.contains(UNDO_HINT))
    );

    let ecra = desenhar(&app, 80, 24);
    assert_eq!(
        ecra.linhas[23], "u desfazer  ? ajuda",
        "é a mesma barra do frame 80x24-5-undo, agora com uma remoção a sério"
    );
    assert!(
        ecra.linhas[22].starts_with("Removida «"),
        "{}",
        ecra.linhas[22]
    );

    carrega(&mut app, 'u');
    let ecra = desenhar(&app, 80, 24);
    assert_eq!(
        ecra.linhas[23],
        "a nova  e editar  Espaço concluir  d remover  u desfazer  / buscar  ? ajuda",
        "com o undo feito, a barra volta ao normal"
    );
}

/// `a` + `Enter`: a seleção segue o `TodoId` devolvido pela `Db` (§5), não o
/// índice final — a mensagem da linha 22 e a barra realçada apontam para a
/// mesma linha. Com a lista maior que o corpo, a janela rola até ela.
#[test]
fn a_tarefa_nova_fica_selecionada_e_a_lista_rola_ate_ela() {
    // 30 tarefas, todas `M`: com a ordem por prioridade (estável) a vista fica
    // pela ordem de inserção e a tarefa nova entra em último — fora das 18
    // linhas do corpo.
    let tarefas: Vec<Todo> = (0..30)
        .map(|i| {
            let mut todo = Todo::try_new(format!("tarefa {i:02}")).expect("título do fixture");
            todo.id = TodoId::from(format!("t{i}"));
            todo.created_at = quando(i, 9, 0);
            todo
        })
        .collect();
    let (_caminho, mut app) = app_com("rolar-ate-a-nova", tarefas, Vec::new(), Some(0));

    carrega(&mut app, 'a');
    escreve(&mut app, "nova tarefa");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(
        app.selected().expect("há uma tarefa selecionada").title,
        "nova tarefa",
        "a seleção fica na tarefa criada"
    );
    assert_eq!(
        app.list_state.selected(),
        Some(30),
        "é o id devolvido pelo `add` que a põe lá, não um índice fixo"
    );

    let ecra = desenhar(&app, 80, 24);
    let linha = ecra.linhas[3..21]
        .iter()
        .position(|linha| linha.contains("nova tarefa"))
        .expect("a lista rolou até à tarefa nova, que não cabe nas 18 linhas do corpo");
    let y = 3 + u16::try_from(linha).expect("linha do corpo");
    assert!(
        ecra.estilo(0, y).add_modifier.contains(Modifier::REVERSED),
        "a barra realçada está na linha da tarefa nova"
    );
    assert!(
        ecra.linhas[22].contains("Adicionada «nova tarefa»"),
        "a mensagem da linha 22 e a barra concordam: {}",
        ecra.linhas[22]
    );
}

#[test]
fn a_ajuda_lista_as_teclas_do_contexto_actual() {
    let (_caminho, mut app) = app_da_lista("ajuda-lixo");
    carrega(&mut app, 'L');
    carrega(&mut app, '?');
    assert_eq!(app.mode, InputMode::Help);

    let ecra = desenhar(&app, 80, 24);
    let caixa = ecra.linhas[4..19].join("\n");
    assert!(
        caixa.contains("restaurar"),
        "a caixa do lixo lista o `Enter`"
    );
    assert!(caixa.contains("esvaziar o lixo"));
    assert!(caixa.contains("voltar à lista"));
    for ausente in [
        "nova tarefa",
        "d  remover",
        "limpar concluídas",
        "u  desfazer",
    ] {
        assert!(
            !caixa.contains(ausente),
            "«{ausente}» não vale no lixo:\n{caixa}"
        );
    }

    carrega(&mut app, '?');
    assert_eq!(
        app.mode,
        InputMode::Trash,
        "fechar a ajuda volta para o lixo"
    );
}

#[test]
fn a_tecla_s_liga_a_ordem_do_core_a_linha_2() {
    let (_caminho, mut app) = app_da_lista("ordem");
    let antes = desenhar(&app, 80, 24);
    assert!(
        antes.linhas[2].contains("ordem: prioridade"),
        "{}",
        antes.linhas[2]
    );
    assert_eq!(
        app.sort,
        SortKey::Priority,
        "a ordem por omissão é a do golden"
    );

    carrega(&mut app, 's');
    let depois = desenhar(&app, 80, 24);
    assert_eq!(app.sort, SortKey::Status);
    assert!(
        depois.linhas[2].contains("ordem: estado"),
        "{}",
        depois.linhas[2]
    );
}

#[test]
fn a_filtragem_e_a_busca_nao_mentem_no_cabecalho() {
    let (_caminho, mut app) = app_da_lista("cabecalho");
    carrega(&mut app, '/');
    escreve(&mut app, "café");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let ecra = desenhar(&app, 80, 24);
    assert!(
        ecra.linhas[0].contains("1 de 9 tarefas"),
        "{}",
        ecra.linhas[0]
    );
    assert!(
        ecra.linhas[0].ends_with("7 pendentes · 2 concluídas"),
        "as contagens são da base toda, não da vista: {}",
        ecra.linhas[0]
    );
    assert!(
        ecra.linhas[2].contains("busca \"café\""),
        "{}",
        ecra.linhas[2]
    );
}
