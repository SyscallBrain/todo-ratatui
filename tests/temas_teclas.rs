//! T5: a caixa de temas — abrir, navegar com pré-visualização, gravar (ou não) e
//! falhar a gravar sem perder nada.
//!
//! Cada critério de aceitação do cartão é um teste com o nome à frente. Tudo
//! corre sem TTY: as teclas são `KeyEvent::new(…)` e o ficheiro de preferências
//! vive num directório temporário próprio de cada teste — `std::env::temp_dir()`
//! mais um `uuid`, como nos testes do `db.json`. Nenhum teste escreve no config
//! real do utilizador, e o teste «sem caminho de config» prova-o byte a byte.
//!
//! O que este ficheiro **não** prova: o desenho da caixa (é o cartão T6, contra
//! o frame `80x24-14-temas.txt`). Aqui fica o estado e as teclas.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use todo_ratatui::core::{Config, Filter, Store};
use todo_ratatui::tui::theme::{CATALOGO, SLUG_CLASSICO};
use todo_ratatui::tui::{Action, App, InputMode, ModoCor, Theme, map_key};

// ------------------------------------------------------------------- fixture

/// Base de dados e ficheiro de preferências num directório temporário deste
/// teste, isolado dos outros (o nome leva o pid e um `uuid`).
struct Base {
    dir: PathBuf,
    db: PathBuf,
    config: PathBuf,
}

fn base(tag: &str) -> Base {
    let dir = std::env::temp_dir().join(format!(
        "todo-ratatui-temas-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).expect("criar o directório do teste");
    Base {
        db: dir.join("db.json"),
        config: dir.join("config.json"),
        dir,
    }
}

/// O `App` do arranque: tema resolvido e ficheiro de preferências conhecido.
fn app(base: &Base, slug: &str) -> App {
    let store = Store::open(&base.db).expect("abrir a base de dados");
    let tema = Theme::por_slug(slug).expect("slug do catálogo");
    App::com_tema(store, tema, Some(base.config.clone()), ModoCor::Rgb)
}

fn escreve_preferencia(base: &Base, slug: &str) {
    Config {
        theme: Some(slug.to_owned()),
    }
    .save(&base.config)
    .expect("gravar a preferência de partida");
}

/// O que o ficheiro de preferências diz, lido pelo leitor a sério.
fn preferencia(base: &Base) -> Option<String> {
    Config::load_or_default(&base.config).0.theme
}

// --------------------------------------------------------------------- teclas

fn carrega(app: &mut App, code: KeyCode) -> Action {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn tecla(app: &mut App, c: char) -> Action {
    carrega(app, KeyCode::Char(c))
}

fn entra(app: &mut App) -> Action {
    carrega(app, KeyCode::Enter)
}

fn escapa(app: &mut App) -> Action {
    carrega(app, KeyCode::Esc)
}

// ------------------------------------------------------------------ critério 1

/// Critério 1: `T` abre a caixa e `t` continua a ser `ToggleAll` — a tecla nova
/// não tirou o significado a nenhuma da tabela (a varredura exaustiva da tabela
/// do modo normal vive em `src/tui/event.rs`, ao lado dela).
#[test]
fn t_abre_a_caixa_e_o_t_minusculo_continua_a_alternar_todas() {
    let apertar = |c: char| {
        map_key(
            &InputMode::Normal,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
    };
    assert_eq!(apertar('T'), Action::ThemeView);
    assert_eq!(apertar('t'), Action::ToggleAll);
    assert_ne!(
        apertar('T'),
        apertar('t'),
        "maiúsculas são distintas, como `g`/`G`"
    );
}

/// O `T` é só da lista: na vista do lixo não abre nada (a caixa não se abre por
/// cima da vista do lixo) e com a caixa já aberta não a reabre.
#[test]
fn o_t_so_existe_na_lista() {
    let base = base("t-no-lixo");
    let mut app = app(&base, SLUG_CLASSICO);

    tecla(&mut app, 'L');
    assert_eq!(app.mode, InputMode::Trash);
    assert_eq!(tecla(&mut app, 'T'), Action::Ignore);
    assert_eq!(app.mode, InputMode::Trash, "o lixo fica onde estava");

    escapa(&mut app);
    tecla(&mut app, 'T');
    assert_eq!(app.mode, InputMode::Theme);
    assert_eq!(
        tecla(&mut app, 'T'),
        Action::Ignore,
        "não se reabre a caixa"
    );
    assert_eq!(app.mode, InputMode::Theme);
}

// ------------------------------------------------------------------ critério 2

/// Critério 2: abrir a caixa e fechar com `Esc` deixa o tema **e** o
/// `config.json` inalterados, mesmo depois de se navegar (um `ThemeMove` a meio
/// não grava nada).
#[test]
fn esc_depois_de_navegar_nao_toca_no_ficheiro_nem_no_tema() {
    let base = base("esc-sem-gravar");
    escreve_preferencia(&base, "tokyo-night-moon");
    let mut app = app(&base, "tokyo-night-moon");
    let antes = fs::read(&base.config).expect("ler o ficheiro");

    tecla(&mut app, 'T');
    assert_eq!(app.mode, InputMode::Theme);
    assert_eq!(app.theme.slug, "tokyo-night-moon", "abre no tema em uso");
    assert_eq!(
        app.theme_cursor, 2,
        "com o cursor na posição do tema em uso"
    );

    // Navegar: a pré-visualização aplica-se logo…
    tecla(&mut app, 'j');
    assert_eq!(app.theme.slug, SLUG_CLASSICO, "pré-visualização ao vivo");
    assert_eq!(app.theme_cursor, 3);
    tecla(&mut app, 'k');
    assert_eq!(app.theme_cursor, 2, "volta ao tema de entrada");
    tecla(&mut app, 'k');
    assert_eq!(app.theme.slug, "tokyo-night-storm");

    // …e o `Esc` fecha sem gravar: o disco não foi tocado uma única vez.
    escapa(&mut app);
    assert_eq!(app.mode, InputMode::Normal);
    assert_eq!(
        app.theme.slug, "tokyo-night-moon",
        "o `Esc` repõe o tema de entrada"
    );
    assert_eq!(
        fs::read(&base.config).expect("reler o ficheiro"),
        antes,
        "o config.json ficou byte-a-byte igual"
    );
    assert_eq!(preferencia(&base).as_deref(), Some("tokyo-night-moon"));
}

// ------------------------------------------------------------------ critério 3

/// Critério 3: navegar duas posições e `Enter` grava o *slug* esperado no
/// ficheiro e deixa o tema aplicado no `App`.
#[test]
fn dois_j_e_enter_gravam_o_slug_e_aplicam_o_tema() {
    let base = base("enter-grava");
    escreve_preferencia(&base, "tokyo-night");
    let mut app = app(&base, "tokyo-night");

    tecla(&mut app, 'T');
    tecla(&mut app, 'j');
    tecla(&mut app, 'j');
    assert_eq!(app.theme_cursor, 2);
    assert_eq!(
        app.theme.slug, "tokyo-night-moon",
        "pré-visualização ao vivo"
    );

    entra(&mut app);

    assert_eq!(app.mode, InputMode::Normal, "o `Enter` fecha a caixa");
    assert_eq!(app.theme.slug, "tokyo-night-moon", "o tema fica aplicado");
    assert_eq!(
        preferencia(&base).as_deref(),
        Some("tokyo-night-moon"),
        "gravou o slug do tema que estava debaixo do cursor"
    );
    let cru = fs::read_to_string(&base.config).expect("ler o ficheiro");
    assert!(
        cru.contains("\"tokyo-night-moon\""),
        "o ficheiro tem o slug escolhido: {cru}"
    );
    assert!(
        !app.status.is_error(),
        "gravar não é erro: {:?}",
        app.status
    );
}

/// As setas fazem o mesmo que `j`/`k`, e o `Enter` sem navegar grava o tema que
/// já estava em uso (nada muda).
#[test]
fn as_setas_navegam_e_o_enter_sem_navegar_grava_o_mesmo_tema() {
    let base = base("setas");
    escreve_preferencia(&base, "tokyo-night-storm");
    let mut app = app(&base, "tokyo-night-storm");

    tecla(&mut app, 'T');
    carrega(&mut app, KeyCode::Down);
    assert_eq!(app.theme.slug, "tokyo-night-moon", "↓ anda para a frente");
    carrega(&mut app, KeyCode::Up);
    assert_eq!(app.theme.slug, "tokyo-night-storm", "↑ anda para trás");
    entra(&mut app);

    assert_eq!(app.theme.slug, "tokyo-night-storm");
    assert_eq!(preferencia(&base).as_deref(), Some("tokyo-night-storm"));
}

// ------------------------------------------------------------------ critério 4

/// Critério 4: cancelar depois de navegar repõe **exactamente** o tema de
/// entrada, com `Esc` e com `q` (as duas formas de fechar sem gravar), e o
/// cursor volta ao tema aplicado.
#[test]
fn cancelar_repoe_exactamente_o_tema_de_entrada() {
    for (saida, chave) in [("esc", KeyCode::Esc), ("q", KeyCode::Char('q'))] {
        let tag = format!("cancelar-{saida}");
        let base = base(&tag);
        escreve_preferencia(&base, "tokyo-night-storm");
        let mut app = app(&base, "tokyo-night-storm");
        let antes = fs::read(&base.config).expect("ler o ficheiro");

        tecla(&mut app, 'T');
        // Três para baixo (até ao fim do catálogo) e um para cima: o que interessa
        // é que o número de passos não conte para nada.
        tecla(&mut app, 'j');
        tecla(&mut app, 'j');
        tecla(&mut app, 'j');
        tecla(&mut app, 'k');
        assert_ne!(app.theme.slug, "tokyo-night-storm", "navegou");

        carrega(&mut app, chave);

        assert_eq!(app.mode, InputMode::Normal, "{saida}: fecha a caixa");
        assert_eq!(
            app.theme.slug, "tokyo-night-storm",
            "{saida}: repõe o tema de entrada"
        );
        assert_eq!(
            app.theme_cursor, 1,
            "{saida}: o cursor volta à posição do tema aplicado"
        );
        assert_eq!(
            fs::read(&base.config).expect("reler o ficheiro"),
            antes,
            "{saida}: não gravou nada"
        );
        assert!(
            !app.should_quit(),
            "{saida}: fechar a caixa não sai da aplicação"
        );
    }
}

/// O cursor não dá a volta: para na primeira e na última posição do catálogo
/// (como a seleção da lista).
#[test]
fn o_cursor_da_caixa_nao_da_a_volta() {
    let base = base("cursor-limites");
    let mut app = app(&base, SLUG_CLASSICO);
    assert_eq!(
        app.theme_cursor,
        CATALOGO.len() - 1,
        "o clássico é o último"
    );

    tecla(&mut app, 'T');
    for _ in 0..5 {
        tecla(&mut app, 'j');
    }
    assert_eq!(app.theme_cursor, CATALOGO.len() - 1, "para no último");
    assert_eq!(app.theme.slug, SLUG_CLASSICO);

    for _ in 0..9 {
        tecla(&mut app, 'k');
    }
    assert_eq!(app.theme_cursor, 0, "e para no primeiro");
    assert_eq!(app.theme.slug, "tokyo-night");
}

// ------------------------------------------------------------------ critério 5

/// Critério 5: com o directório do config só-de-leitura, o `Enter` produz
/// `Status::Error` com o prefixo `Erro:`, o `App` continua utilizável e o tema
/// continua aplicado nesta sessão.
///
/// É a política §4 do desenho (a mesma da falha a gravar o `db.json`): a
/// mensagem diz **onde** não se gravou e nada é perdido — nem a lista, nem a
/// escolha desta sessão, nem o ficheiro anterior.
#[test]
fn gravar_num_directorio_so_de_leitura_da_erro_e_o_app_continua_vivo() {
    let base = base("so-de-leitura");
    escreve_preferencia(&base, "tokyo-night");
    let mut app = app(&base, "tokyo-night");
    let antes = fs::read(&base.config).expect("ler o ficheiro");

    // O `save` cria o `config.json.tmp` ao lado do destino: sem escrita no
    // directório, a gravação falha antes de tocar no ficheiro bom. A sonda
    // abaixo distingue «sem permissão» de «a correr como root», onde o
    // `0o500` não trava nada e o caso não é reproduzível.
    let mut perm = fs::metadata(&base.dir).expect("metadata").permissions();
    perm.set_mode(0o500);
    fs::set_permissions(&base.dir, perm.clone()).expect("tirar a escrita ao directório");
    let reproduzivel = fs::write(base.dir.join("sonda"), b"x").is_err();
    let _ = fs::remove_file(base.dir.join("sonda"));

    tecla(&mut app, 'T');
    tecla(&mut app, 'j');
    entra(&mut app);

    perm.set_mode(0o700);
    fs::set_permissions(&base.dir, perm).expect("devolver a escrita ao directório");
    if !reproduzivel {
        eprintln!("a saltar as asserções: a correr como root o 0o500 não impede a escrita");
        return;
    }

    let texto = app.status.text().expect("há mensagem").to_owned();
    assert!(app.status.is_error(), "devia ser erro: {texto}");
    assert!(texto.starts_with("Erro:"), "o prefixo do desenho: {texto}");
    assert!(
        texto.contains("config.json"),
        "o erro nomeia o ficheiro que não gravou: {texto}"
    );
    assert!(
        texto.contains("só nesta sessão"),
        "e diz que o tema vale nesta sessão: {texto}"
    );
    assert!(
        !app.should_quit(),
        "um erro de gravação não fecha a aplicação"
    );
    assert_eq!(app.mode, InputMode::Normal, "a caixa fecha na mesma");
    assert_eq!(
        app.theme.slug, "tokyo-night-storm",
        "o tema fica aplicado só nesta sessão"
    );
    assert_eq!(
        fs::read(&base.config).expect("reler o ficheiro"),
        antes,
        "o config.json anterior não foi tocado"
    );

    // «Continua utilizável»: a lista responde na mesma à tecla seguinte.
    let filtro = app.filter;
    assert_eq!(tecla(&mut app, 'f'), Action::CycleFilter);
    assert_ne!(app.filter, filtro, "a TUI continua a responder");
    assert_eq!(app.filter, Filter::Active);
}

// ------------------------------------------------------- sem caminho de config

/// O `App` dos testes (ou de um sistema sem `XDG_CONFIG_HOME` nem `HOME`) não
/// tem caminho de config: o `Enter` aplica o tema e fecha **sem** tentar gravar
/// — e, sobretudo, sem inventar um caminho no `~/.config` real.
#[test]
fn sem_caminho_de_config_o_enter_aplica_o_tema_e_nao_grava_nada() {
    let base = base("sem-config");
    let real = dirs::config_dir().map(|casa| casa.join("todo-ratatui").join("config.json"));
    let real_antes = real.as_deref().and_then(|caminho| fs::read(caminho).ok());

    let store = Store::open(&base.db).expect("abrir a base de dados");
    let mut app = App::new(store);
    assert!(
        app.config_path.is_none(),
        "o construtor dos testes não tem config"
    );

    tecla(&mut app, 'T');
    tecla(&mut app, 'j');
    entra(&mut app);

    assert_eq!(app.mode, InputMode::Normal, "fecha na mesma");
    assert_eq!(app.theme.slug, "tokyo-night-storm", "o tema aplica-se");
    assert!(
        !app.status.is_error(),
        "sem caminho não há erro: {:?}",
        app.status
    );
    assert!(
        !base.config.exists(),
        "não inventou um ficheiro no directório do teste"
    );
    assert_eq!(
        real.as_deref().and_then(|caminho| fs::read(caminho).ok()),
        real_antes,
        "e não escreveu no config do utilizador"
    );
}

// --------------------------------------------------------------- a caixa fecha

/// Com a caixa aberta, nada do modo normal vale: nem `a`, nem `e`, nem `d` (a
/// lista está à vista por baixo da sobreposição). O `Ctrl+C` — que vale em
/// todos os modos — sai sem gravar.
#[test]
fn com_a_caixa_aberta_nenhuma_tecla_da_lista_faz_nada() {
    let base = base("caixa-isola");
    let mut store = Store::open(&base.db).expect("abrir a base de dados");
    for titulo in ["Rever o PR", "Comprar café"] {
        store.add(titulo).expect("adicionar");
    }
    let tema = Theme::por_slug("tokyo-night").expect("slug do catálogo");
    let mut app = App::com_tema(store, tema, Some(base.config.clone()), ModoCor::Rgb);
    app.list_state.select(Some(1));
    let titulos: Vec<String> = app
        .store()
        .todos()
        .iter()
        .map(|todo| todo.title.clone())
        .collect();
    let selecao = app.list_state.selected();
    let filtro = app.filter;
    let ordem = app.sort;
    let lixo = app.store().trash_len();

    tecla(&mut app, 'T');
    for c in [
        'a', 'e', 'd', 'u', 't', 'c', 's', 'f', 'i', 'x', 'L', '?', 'g', 'G', '1', ' ', '/', 'z',
    ] {
        assert_eq!(
            tecla(&mut app, c),
            Action::Ignore,
            "«{c}» não vale com a caixa aberta"
        );
    }

    assert_eq!(app.mode, InputMode::Theme, "a caixa fica aberta");
    assert_eq!(
        app.store()
            .todos()
            .iter()
            .map(|todo| todo.title.clone())
            .collect::<Vec<String>>(),
        titulos,
        "nenhuma tarefa mudou"
    );
    assert_eq!(app.store().trash_len(), lixo, "nada foi para o lixo");
    assert_eq!(app.list_state.selected(), selecao, "a seleção não mexeu");
    assert_eq!(app.filter, filtro);
    assert_eq!(app.sort, ordem);
    assert_eq!(
        app.status,
        todo_ratatui::tui::Status::Idle,
        "nenhuma mensagem"
    );

    // O `Ctrl+C` continua a sair — e sair não grava a preferência.
    app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit());
    assert!(
        !base.config.exists(),
        "sair com a caixa aberta não grava nada"
    );
}
