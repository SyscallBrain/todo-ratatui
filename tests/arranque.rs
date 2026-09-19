//! Política de arranque, presa pelo binário a sério (não pela biblioteca).
//!
//! A adenda 2 do ADR divide os erros em dois: falhar a **abrir** a base de
//! dados recusa o arranque — mensagem no `stderr`, código de saída ≠ 0, sem
//! TUI — e falhar a **gravar** durante a sessão aparece na faixa da linha 22.
//! Este ficheiro prende a primeira metade: corre o binário com `--db` a apontar
//! para cada ficheiro mau e exige que (a) nada entre na TUI, (b) a mensagem
//! nomeie o `db.json` **e** o `.bak` — que é a saída do utilizador — e (c) o
//! ficheiro fique byte-a-byte igual, prova de que nada o reescreveu.
//!
//! O caso «não há TTY nenhum» aparece agora também aqui, nos casos de
//! preferência: o binário corre com `stdin` a `/dev/null` e o `raw mode`
//! falha, o que prova que a resolução das flags acontece **antes** de o ecrã
//! alternativo ligar (o aviso sai e, logo a seguir, o erro do terminal).
//!
//! Um prazo em cada corrida não é gosto de robustez: sem ele, um defeito nesta
//! política deixaria o `cargo test` pendurado numa TUI à espera de uma tecla.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use todo_ratatui::core::backup_path_for;
use todo_ratatui::tui::arranque::{self, Ambiente, Opcoes};
use todo_ratatui::tui::theme::{SLUG_CLASSICO, SLUG_POR_OMISSAO};

/// Caminho do binário construído para estes testes (`Cargo` garante que existe
/// antes de os correr).
const BIN: &str = env!("CARGO_BIN_EXE_todo_ratatui");

/// Quanto tempo o binário pode demorar a sair antes de ser considerado
/// pendurado (uma TUI à espera de uma tecla não sai sozinha).
const PRAZO: Duration = Duration::from_secs(10);

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "todo-ratatui-arranque-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("criar diretório de teste");
    dir
}

/// Corre o binário com `--db <db>` e devolve o que ele disse, matando-o se não
/// sair dentro do [`PRAZO`].
fn corre(db: &Path) -> Output {
    let mut child = Command::new(BIN)
        .arg("--db")
        .arg(db)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("correr o binário");

    let inicio = Instant::now();
    while child.try_wait().expect("esperar pelo binário").is_none() {
        if inicio.elapsed() > PRAZO {
            let _ = child.kill();
            let _ = child.wait_with_output();
            panic!(
                "o binário ainda estava a correr depois de {PRAZO:?} — entrou na TUI em vez de \
                 recusar o arranque de «{}»",
                db.display()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("recolher a saída")
}

/// As três coisas que todos os casos de falha ao abrir têm de cumprir.
fn recusa_arrancar(saida: &Output, db: &Path) {
    let stderr = String::from_utf8_lossy(&saida.stderr);
    assert!(
        !saida.status.success(),
        "devia recusar arrancar e saiu com sucesso: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "recusou com panic em vez de mensagem: {stderr}"
    );
    assert!(
        stderr.contains(&db.display().to_string()),
        "a mensagem não nomeia o db.json «{}»: {stderr}",
        db.display()
    );
    let bak = backup_path_for(db);
    assert!(
        stderr.contains(&bak.display().to_string()),
        "a mensagem não nomeia o .bak «{}»: {stderr}",
        bak.display()
    );
    assert!(
        saida.stdout.is_empty(),
        "a TUI escreveu no stdout: {}",
        String::from_utf8_lossy(&saida.stdout)
    );
}

/// O ficheiro continua exactamente como estava: é o que prova que a recusa de
/// arranque não sobrescreveu nada.
fn ficheiro_intacto(db: &Path, antes: &[u8]) {
    assert_eq!(
        fs::read(db).expect("reler o ficheiro"),
        antes,
        "o ficheiro de «{}» foi alterado",
        db.display()
    );
}

#[test]
fn directorio_em_vez_de_ficheiro_recusa_arrancar() {
    let dir = dir("directorio");
    let db = dir.join("db.json");
    fs::create_dir(&db).expect("criar o «ficheiro» como directório");

    let saida = corre(&db);
    recusa_arrancar(&saida, &db);
    assert!(
        String::from_utf8_lossy(&saida.stderr).contains(&db.display().to_string()),
        "e o directório fica onde estava: {}",
        db.display()
    );
    assert!(db.is_dir());
}

#[test]
fn json_invalido_recusa_arrancar() {
    let dir = dir("json-invalido");
    let db = dir.join("db.json");
    fs::write(&db, b"{ isto nao e json ").expect("escrever lixo");
    let antes = fs::read(&db).expect("ler");

    let saida = corre(&db);
    recusa_arrancar(&saida, &db);
    assert!(
        String::from_utf8_lossy(&saida.stderr).contains("JSON inválido"),
        "a mensagem diz o que se passou: {}",
        String::from_utf8_lossy(&saida.stderr)
    );
    ficheiro_intacto(&db, &antes);
}

#[test]
fn schema_futuro_recusa_arrancar() {
    let dir = dir("schema-futuro");
    let db = dir.join("db.json");
    fs::write(&db, br#"{"schema": 99, "todos": [], "trash": []}"#).expect("escrever");
    let antes = fs::read(&db).expect("ler");

    let saida = corre(&db);
    recusa_arrancar(&saida, &db);
    assert!(
        String::from_utf8_lossy(&saida.stderr).contains("schema 99"),
        "a mensagem diz que schema encontrou: {}",
        String::from_utf8_lossy(&saida.stderr)
    );
    ficheiro_intacto(&db, &antes);
}

#[test]
fn array_legado_sem_envelope_recusa_arrancar() {
    let dir = dir("legado");
    let db = dir.join("db.json");
    // Forma do `rtodo` antigo: `Vec<Todo>` cru, sem envelope (é o que o import
    // do T4 sabe converter; abri-lo como base de dados é que não).
    fs::write(
        &db,
        br#"[{"title": "comprar cafe", "description": null, "done": false, "time": "HIGH", "date": "2023-01-05"}]"#,
    )
    .expect("escrever legado");
    let antes = fs::read(&db).expect("ler");

    let saida = corre(&db);
    recusa_arrancar(&saida, &db);
    assert!(
        String::from_utf8_lossy(&saida.stderr).contains("formato legado"),
        "a mensagem diz que é o formato antigo: {}",
        String::from_utf8_lossy(&saida.stderr)
    );
    ficheiro_intacto(&db, &antes);
}

#[test]
fn ficheiro_ilegivel_recusa_arrancar() {
    let dir = dir("ilegivel");
    let db = dir.join("db.json");
    fs::write(&db, br#"{"schema": 2, "todos": [], "trash": []}"#).expect("escrever");
    let antes = fs::read(&db).expect("ler");

    fs::set_permissions(&db, fs::Permissions::from_mode(0o000)).expect("tirar a leitura");
    if fs::read(&db).is_ok() {
        // Estamos a correr como root: as permissões não travam nada, logo este
        // caso não é reproduzível aqui. Dizê-lo em vez de passar em silêncio.
        fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).expect("devolver a leitura");
        eprintln!("a saltar: ficheiro ilegível não é reproduzível a correr como root");
        return;
    }

    let saida = corre(&db);
    recusa_arrancar(&saida, &db);
    assert!(
        String::from_utf8_lossy(&saida.stderr).contains("falha de I/O"),
        "a mensagem diz que não conseguiu ler: {}",
        String::from_utf8_lossy(&saida.stderr)
    );

    fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).expect("devolver a leitura");
    ficheiro_intacto(&db, &antes);
}

// --------------------------------------------------------------------------
// Preferências (T4): a cadeia `--theme` > env > `config.json` > default, a
// detecção de cor e a regra que atravessa tudo — nenhuma preferência, por má
// que seja, impede abrir a lista (ADR §Decisão 2, 3 e 7).
// --------------------------------------------------------------------------

/// Escreve um `config.json` com o tema pedido e devolve o seu caminho.
fn config_com(dir: &Path, slug: &str) -> PathBuf {
    let path = dir.join("config.json");
    fs::write(&path, format!("{{\"theme\": \"{slug}\"}}\n")).expect("escrever o config.json");
    path
}

/// Um caminho que não existe: o `config.json` ausente, que é o que garante que
/// nenhum caso destes toque no `~/.config/todo-ratatui/` a sério.
fn config_ausente(dir: &Path) -> PathBuf {
    dir.join("nao-existe.json")
}

/// Corre o binário com argumentos livres e as três variáveis de preferência
/// **limpas** (o ambiente de quem corre `cargo test` não pode decidir o
/// resultado), matando-o se não sair dentro do [`PRAZO`].
fn corre_com(args: &[&str]) -> Output {
    corre_com_env(args, &[])
}

/// O mesmo, com variáveis de ambiente à escolha: as três de preferência ficam
/// sempre limpas primeiro, e só as que vierem em `ambiente` são postas.
fn corre_com_env(args: &[&str], ambiente: &[(&str, &str)]) -> Output {
    let mut comando = Command::new(BIN);
    comando
        .args(args)
        .env_remove("TODO_RATATUI_THEME")
        .env_remove("TODO_RATATUI_COLOR")
        .env_remove("TODO_RATATUI_CONFIG");
    for (nome, valor) in ambiente {
        comando.env(nome, valor);
    }

    let mut child = comando
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("correr o binário");

    let inicio = Instant::now();
    while child.try_wait().expect("esperar pelo binário").is_none() {
        if inicio.elapsed() > PRAZO {
            let _ = child.kill();
            let _ = child.wait_with_output();
            panic!("o binário não saiu em {PRAZO:?} com {args:?} — entrou na TUI?");
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("recolher a saída")
}

/// O `stderr` de uma corrida, como texto.
fn stderr_de(saida: &Output) -> String {
    String::from_utf8_lossy(&saida.stderr).into_owned()
}

/// Argumentos do binário com um `db.json` descartável — o que impede qualquer
/// caso de mexer na base de dados real do utilizador.
fn args_com_db(dir: &Path) -> Vec<String> {
    vec![
        "--db".to_owned(),
        dir.join("db.json").display().to_string(),
    ]
}

/// Os nomes dos ficheiros de um directório, ordenados. Serve para provar que
/// uma corrida não deixou nada atrás (`.bak`, `.tmp`, um config novo).
fn ficheiros_em(dir: &Path) -> Vec<String> {
    let mut nomes: Vec<String> = fs::read_dir(dir)
        .expect("listar")
        .filter_map(Result::ok)
        .map(|entrada| entrada.file_name().to_string_lossy().into_owned())
        .collect();
    nomes.sort();
    nomes
}

#[test]
fn a_precedencia_do_tema_e_flag_env_config_e_default() {
    let dir = dir("precedencia");
    let config = config_com(&dir, "tokyo-night-storm");
    let db = dir.join("db.json");
    let db = db.display().to_string();

    // 1. `--theme` ganha a tudo.
    //
    // `--color rgb` em todos os casos desta função: sem ele o modo `auto`
    // depende do terminal de quem corre os testes (aqui não há truecolor e um
    // tema com fundo passaria a `classico`), e o que se está a provar aqui é a
    // cadeia do tema, não a da cor.
    let da_flag = arranque::resolver(
        &Opcoes {
            theme: Some("tokyo-night-moon".to_owned()),
            color: Some("rgb".to_owned()),
            config: Some(config.clone()),
        },
        &Ambiente {
            theme: Some("classico".to_owned()),
            color: None,
            config: None,
        },
    );
    assert_eq!(
        da_flag.tema.slug, "tokyo-night-moon",
        "--theme precede a env e o ficheiro"
    );
    assert_eq!(
        da_flag.config_path.as_deref(),
        Some(config.as_path()),
        "o caminho resolvido é o do --config"
    );
    assert!(
        da_flag.avisos.is_empty(),
        "um valor válido não avisa nada: {:?}",
        da_flag.avisos
    );

    // 2. Sem flag, ganha a env.
    let da_env = arranque::resolver(
        &Opcoes {
            color: Some("rgb".to_owned()),
            config: Some(config.clone()),
            ..Opcoes::default()
        },
        &Ambiente {
            theme: Some("tokyo-night-moon".to_owned()),
            ..Ambiente::default()
        },
    );
    assert_eq!(da_env.tema.slug, "tokyo-night-moon", "a env precede o ficheiro");
    assert!(da_env.avisos.is_empty(), "{:?}", da_env.avisos);

    // 3. Sem flag nem env, ganha o `config.json`.
    let do_ficheiro = arranque::resolver(
        &Opcoes {
            color: Some("rgb".to_owned()),
            config: Some(config.clone()),
            ..Opcoes::default()
        },
        &Ambiente::default(),
    );
    assert_eq!(
        do_ficheiro.tema.slug, "tokyo-night-storm",
        "o config.json precede o default"
    );
    assert!(do_ficheiro.avisos.is_empty(), "{:?}", do_ficheiro.avisos);

    // 4. Sem nada, fica o default — e um ficheiro ausente não é avaria.
    let so_default = arranque::resolver(
        &Opcoes {
            color: Some("rgb".to_owned()),
            config: Some(config_ausente(&dir)),
            ..Opcoes::default()
        },
        &Ambiente::default(),
    );
    assert_eq!(so_default.pedido.slug, SLUG_POR_OMISSAO);
    assert!(so_default.avisos.is_empty(), "{:?}", so_default.avisos);

    // 5. O caminho do config: `--config` > `TODO_RATATUI_CONFIG`.
    let do_ambiente = dir.join("do-ambiente.json");
    let da_env_do_caminho = arranque::resolver(
        &Opcoes {
            config: Some(config.clone()),
            ..Opcoes::default()
        },
        &Ambiente {
            config: Some(OsString::from(&do_ambiente)),
            ..Ambiente::default()
        },
    );
    assert_eq!(
        da_env_do_caminho.config_path.as_deref(),
        Some(config.as_path()),
        "--config precede TODO_RATATUI_CONFIG"
    );

    let sem_flag = arranque::resolver(
        &Opcoes::default(),
        &Ambiente {
            config: Some(OsString::from(&do_ambiente)),
            ..Ambiente::default()
        },
    );
    assert_eq!(
        sem_flag.config_path.as_deref(),
        Some(do_ambiente.as_path()),
        "TODO_RATATUI_CONFIG decide quando não há --config"
    );

    // 6. A mesma cadeia, presa pelo binário a sério: com `--theme` a flag ganha
    // ao ficheiro (o aviso do ficheiro estragado não aparece) e o `--config`
    // apontado é o lido.
    let saida = corre_com(&[
        "--db",
        &db,
        "--config",
        &config.display().to_string(),
        "--theme",
        "tokyo-night-moon",
    ]);
    let stderr = stderr_de(&saida);
    assert!(
        !stderr.contains("tokyo-night-storm"),
        "com --theme válido, o ficheiro nem é consultado: {stderr}"
    );
}

#[test]
fn um_slug_desconhecido_avisa_e_segue_o_valor_seguinte() {
    let dir = dir("slug-desconhecido");
    let config = config_com(&dir, "classico");

    // `--theme` estragado: avisa e cai no valor seguinte (o ficheiro).
    let da_cadeia = arranque::resolver(
        &Opcoes {
            theme: Some("nao-existe".to_owned()),
            color: Some("rgb".to_owned()),
            config: Some(config.clone()),
        },
        &Ambiente::default(),
    );
    assert_eq!(da_cadeia.pedido.slug, SLUG_CLASSICO, "cai no config.json");
    let avisos = da_cadeia.avisos.join("\n");
    assert!(
        avisos.contains("--theme") && avisos.contains("nao-existe"),
        "o aviso nomeia a fonte e o valor: {avisos}"
    );
    for slug in ["tokyo-night", "tokyo-night-storm", "tokyo-night-moon", SLUG_CLASSICO] {
        assert!(
            avisos.contains(slug),
            "o aviso lista os temas válidos («{slug}» falta): {avisos}"
        );
    }

    // Sem ficheiro por onde continuar: fica o default e diz-se qual é.
    let so_default = arranque::resolver(
        &Opcoes {
            theme: Some("nao-existe".to_owned()),
            config: Some(config_ausente(&dir)),
            ..Opcoes::default()
        },
        &Ambiente::default(),
    );
    assert_eq!(so_default.pedido.slug, SLUG_POR_OMISSAO);
    assert!(
        so_default
            .avisos
            .iter()
            .any(|aviso| aviso.contains(SLUG_POR_OMISSAO) && aviso.contains("omissão")),
        "o aviso diz com que tema se segue: {:?}",
        so_default.avisos
    );
}

#[test]
fn o_binario_com_tema_invalido_avisa_e_sai_sem_entrar_na_tui() {
    let dir = dir("bin-tema-invalido");
    let mut argv = args_com_db(&dir);
    argv.push("--config".to_owned());
    argv.push(config_ausente(&dir).display().to_string());
    argv.push("--theme".to_owned());
    argv.push("nao-existe".to_owned());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let saida = corre_com(&argv);
    let stderr = stderr_de(&saida);

    assert!(
        !saida.status.success(),
        "devia sair com erro e saiu com sucesso: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "avisou com panic: {stderr}");
    assert!(
        stderr.contains("nao-existe") && stderr.contains("--theme"),
        "o aviso nomeia o valor e a flag: {stderr}"
    );
    assert!(
        stderr.contains(SLUG_POR_OMISSAO),
        "o aviso diz qual foi o tema usado: {stderr}"
    );
    assert!(
        saida.stdout.is_empty(),
        "a TUI escreveu no stdout: {}",
        String::from_utf8_lossy(&saida.stdout)
    );
    // A prova de que a preferência se resolve **antes** do ecrã alternativo:
    // o aviso sai, e só depois é que a falta de TTY se anuncia.
    let aviso = stderr.find("aviso:").expect("tinha de avisar");
    let erro = stderr
        .find("não há terminal interactivo")
        .expect("sem TTY, o arranque falha com mensagem");
    assert!(
        aviso < erro,
        "o aviso tem de sair antes da falha do terminal: {stderr}"
    );

    // Uma preferência má não cria nem reescreve nada: no directório só existe
    // o que o teste lá pôs (a base de dados só nasce à primeira gravação).
    assert!(
        ficheiros_em(&dir).is_empty(),
        "o arranque deixou ficheiros atrás: {:?}",
        ficheiros_em(&dir)
    );
}

#[test]
fn o_modo_ansi_usa_o_classico_e_nao_reescreve_o_config() {
    let dir = dir("ansi");
    let config = config_com(&dir, "tokyo-night-moon");
    let antes = fs::read(&config).expect("ler o config antes");

    // 1. A função pura: `ansi` troca o tema com fundo pelo `classico` e di-lo.
    let resolucao = arranque::resolver(
        &Opcoes {
            color: Some("ansi".to_owned()),
            config: Some(config.clone()),
            ..Opcoes::default()
        },
        &Ambiente::default(),
    );
    assert_eq!(
        resolucao.modo,
        todo_ratatui::tui::ModoCor::Ansi,
        "o modo pedido é o que fica na resolução"
    );
    assert_eq!(
        resolucao.pedido.slug, "tokyo-night-moon",
        "o tema pedido é o do ficheiro"
    );
    assert_eq!(
        resolucao.tema.slug, SLUG_CLASSICO,
        "em ansi, o tema com fundo resolve para o classico"
    );
    assert!(
        resolucao.avisos.iter().any(|aviso| aviso.contains("--color=ansi")
            && aviso.contains("tokyo-night-moon")
            && aviso.contains(SLUG_CLASSICO)),
        "o aviso diz o modo pedido, o tema que não cabe e o que foi usado: {:?}",
        resolucao.avisos
    );

    // 2. `--color rgb` é a saída de emergência: mantém o tema e não avisa.
    let a_forcar_rgb = arranque::resolver(
        &Opcoes {
            theme: Some("tokyo-night-moon".to_owned()),
            color: Some("rgb".to_owned()),
            config: Some(config.clone()),
        },
        &Ambiente::default(),
    );
    assert_eq!(a_forcar_rgb.tema.slug, "tokyo-night-moon");
    assert!(a_forcar_rgb.avisos.is_empty(), "{:?}", a_forcar_rgb.avisos);

    // 3. O binário com as mesmas flags: sai sem TTY e não toca no ficheiro.
    let mut argv = args_com_db(&dir);
    argv.push("--config".to_owned());
    argv.push(config.display().to_string());
    argv.push("--theme".to_owned());
    argv.push("tokyo-night-moon".to_owned());
    argv.push("--color".to_owned());
    argv.push("ansi".to_owned());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let saida = corre_com(&argv);
    assert!(!saida.status.success(), "sem TTY o arranque falha");
    assert!(
        stderr_de(&saida).contains(SLUG_CLASSICO),
        "o aviso do classico sai no stderr: {}",
        stderr_de(&saida)
    );
    assert_eq!(
        fs::read(&config).expect("ler o config depois"),
        antes,
        "o config.json ficou byte-a-byte igual"
    );
    assert_eq!(
        ficheiros_em(&dir),
        vec!["config.json".to_owned()],
        "não ficou um .bak/.tmp para trás (nem nasceu um db.json)"
    );
}

#[test]
fn o_modo_de_cor_desconhecido_avisa_e_cai_no_valor_seguinte() {
    let dir = dir("modo-de-cor");
    let config = config_ausente(&dir);

    let da_env = arranque::resolver(
        &Opcoes {
            theme: Some("tokyo-night".to_owned()),
            color: Some("vinte-e-quatro-bits".to_owned()),
            config: Some(config.clone()),
        },
        &Ambiente {
            color: Some("rgb".to_owned()),
            ..Ambiente::default()
        },
    );
    assert_eq!(
        da_env.modo,
        todo_ratatui::tui::ModoCor::Rgb,
        "a env segura a cadeia"
    );
    assert_eq!(da_env.tema.slug, "tokyo-night", "rgb mantém o tema com fundo");
    assert!(
        da_env.avisos[0].contains("--color") && da_env.avisos[0].contains("auto, rgb, ansi"),
        "o aviso nomeia a fonte e os valores: {:?}",
        da_env.avisos
    );

    let sem_mais_nada = arranque::resolver(
        &Opcoes {
            theme: Some("tokyo-night".to_owned()),
            color: Some("vinte-e-quatro-bits".to_owned()),
            config: Some(config),
        },
        &Ambiente::default(),
    );
    assert_eq!(
        sem_mais_nada.modo,
        todo_ratatui::tui::ModoCor::Auto,
        "sem mais nada fica o auto"
    );
}

#[test]
fn um_config_corrompido_avisa_e_nao_impede_o_arranque() {
    let dir = dir("config-corrompido");
    let config = dir.join("config.json");
    fs::write(&config, b"{ isto nao e json ").expect("escrever lixo");
    let antes = fs::read(&config).expect("ler o config antes");

    let mut argv = args_com_db(&dir);
    argv.push("--config".to_owned());
    argv.push(config.display().to_string());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let saida = corre_com(&argv);
    let stderr = stderr_de(&saida);

    assert!(
        stderr.contains("aviso:") && stderr.contains("JSON inválido"),
        "o aviso vem do `load_or_default` e diz o que se passou: {stderr}"
    );
    assert!(
        stderr.contains(&config.display().to_string()),
        "o aviso nomeia o ficheiro: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "{stderr}");
    // A lista só não abre por falta de TTY: a preferência estragada não é a
    // causa da falha (a recusa por `db.json` inválido continua a ser a única
    // que impede arrancar).
    assert!(
        stderr.contains("não há terminal interactivo"),
        "a falha é do terminal, não do config: {stderr}"
    );
    assert_eq!(
        fs::read(&config).expect("ler o config depois"),
        antes,
        "ler uma preferência estragada não a reescreve"
    );
}

#[test]
fn a_ajuda_lista_as_tres_flags_novas() {
    let saida = corre_com(&["--theme", "tokyo-night-moon", "--help"]);
    let stdout = String::from_utf8_lossy(&saida.stdout);

    assert!(saida.status.success(), "--help sai com sucesso");
    for flag in ["--db", "--theme", "--color", "--config"] {
        assert!(
            stdout.contains(flag),
            "a ajuda não lista {flag}: {stdout}"
        );
    }
    for slug in ["tokyo-night", "tokyo-night-storm", "tokyo-night-moon", SLUG_CLASSICO] {
        assert!(stdout.contains(slug), "a ajuda não lista o tema {slug}");
    }
    assert!(
        saida.stderr.is_empty(),
        "a ajuda não avisa nada: {}",
        stderr_de(&saida)
    );
}

// --------------------------------------------------------------------------
// Fronteira de impressão (T1, SA-01): nenhum texto que o binário escreve no
// `stderr` pode conter caracteres de controlo, venha ele do `argv`, de uma
// variável de ambiente, do `config.json` ou de um caminho. O caso concreto do
// achado é uma sequência OSC 52 — copia texto para a área de transferência de
// quem lê o aviso no terminal.
// --------------------------------------------------------------------------

/// O valor injectado nos testes: `\x1b]52;…\x07` é a OSC 52 do achado, com um
/// prefixo para o valor não ser confundível com um slug válido.
const ESCAPE_OSC: &str = "x\x1b]52;c;SGVsbG8=\x07";

/// Nenhum carácter de controlo no `stderr`, além do `\n` que separa linhas: é a
/// propriedade que o saneamento promete, mais forte do que procurar o ESC.
fn sem_controlo(stderr: &str) -> bool {
    stderr.chars().all(|c| c == '\n' || !c.is_control())
}

/// As três asserções de todos os casos de injecção: nada de controlo, nada do
/// escape cru, e a forma visível `\u{1b}` presente (prova de que o valor foi
/// saneado e não simplesmente engolido).
fn saneado(stderr: &str, o_que: &str) {
    assert!(
        sem_controlo(stderr),
        "{o_que}: o stderr tem caracteres de controlo: {stderr:?}"
    );
    for proibido in ['\x1b', '\x07', '\x00'] {
        assert!(
            !stderr.contains(proibido),
            "{o_que}: o stderr tem {proibido:?}: {stderr:?}"
        );
    }
    assert!(
        stderr.contains("\\u{1b}"),
        "{o_que}: falta a forma visível «\\u{{1b}}» no stderr: {stderr:?}"
    );
}

/// Argumentos com um `db.json` descartável **e** um `--config` que não existe:
/// nenhum caso destes lê o `config.json` real do utilizador.
fn args_descartaveis(dir: &Path) -> Vec<String> {
    let mut argv = args_com_db(dir);
    argv.push("--config".to_owned());
    argv.push(config_ausente(dir).display().to_string());
    argv
}

#[test]
fn o_tema_com_escapes_na_env_sai_visivel_no_stderr() {
    let dir = dir("injecao-env");
    let argv = args_descartaveis(&dir);
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let saida = corre_com_env(&argv, &[("TODO_RATATUI_THEME", ESCAPE_OSC)]);
    let stderr = stderr_de(&saida);

    saneado(&stderr, "tema pela env");
    assert!(
        stderr.contains("TODO_RATATUI_THEME") && stderr.contains("]52;c;SGVsbG8="),
        "o aviso continua a nomear a fonte e o valor: {stderr:?}"
    );
}

#[test]
fn o_tema_com_escapes_na_flag_sai_visivel_no_stderr() {
    let dir = dir("injecao-flag");
    let mut argv = args_descartaveis(&dir);
    argv.push("--theme".to_owned());
    argv.push(ESCAPE_OSC.to_owned());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let stderr = stderr_de(&corre_com(&argv));

    saneado(&stderr, "tema pela flag");
    assert!(
        stderr.contains("--theme"),
        "o aviso nomeia a flag: {stderr:?}"
    );
}

#[test]
fn o_tema_com_escapes_no_config_sai_visivel_no_stderr() {
    let dir = dir("injecao-config");
    // No JSON os dois caracteres vão escapados (`\u001b`, `\u0007`): um
    // carácter de controlo cru dentro de uma string não é JSON válido, e o
    // `serde_json` recusá-lo-ia antes de o valor chegar ao aviso.
    let config = dir.join("config.json");
    fs::write(&config, r#"{"theme": "x\u001b]52;c;SGVsbG8=\u0007"}"#)
        .expect("escrever o config.json envenenado");

    let mut argv = args_com_db(&dir);
    argv.push("--config".to_owned());
    argv.push(config.display().to_string());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let stderr = stderr_de(&corre_com(&argv));

    saneado(&stderr, "tema pelo config.json");
    assert!(
        stderr.contains("config.json") && stderr.contains("desconhecido"),
        "o aviso vem da cadeia do tema: {stderr:?}"
    );
}

#[test]
fn o_argumento_desconhecido_com_escapes_sai_visivel_no_stderr() {
    let dir = dir("injecao-argv");
    let mut argv = args_descartaveis(&dir);
    // Sem `=` de propósito: com um `=` o `parse_args` partia o argumento ao
    // meio e o escape já não chegava inteiro à mensagem.
    argv.push("--nao-existe\x1b]52;c\x07".to_owned());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let saida = corre_com(&argv);
    let stderr = stderr_de(&saida);

    assert!(
        !saida.status.success(),
        "um argumento desconhecido recusa o arranque"
    );
    saneado(&stderr, "argumento desconhecido");
    assert!(
        stderr.contains("argumento desconhecido") && stderr.contains("--nao-existe"),
        "a mensagem continua a dizer o que se passou: {stderr:?}"
    );
}

#[test]
fn o_caminho_da_base_de_dados_com_escapes_sai_visivel_no_stderr() {
    let dir = dir("injecao-caminho");
    // O erro de leitura nomeia o caminho: é por aqui que um `--db` com escapes
    // chegava ao terminal.
    let db = dir.join(format!("db{ESCAPE_OSC}.json"));
    fs::write(&db, b"{ isto nao e json ").expect("escrever lixo");

    let mut argv = args_descartaveis(&dir);
    argv[1] = db.display().to_string();
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let stderr = stderr_de(&corre_com(&argv));

    saneado(&stderr, "caminho do --db");
    assert!(
        stderr.contains("]52;c;SGVsbG8=") && stderr.contains("JSON inválido"),
        "a mensagem nomeia o ficheiro e o que se passou: {stderr:?}"
    );
}

#[test]
fn o_saneamento_nao_mexe_no_que_nao_e_ascii() {
    let dir = dir("injecao-acentos");
    let mut argv = args_descartaveis(&dir);
    argv.push("--theme".to_owned());
    // Um *slug* inválido com acentos: tem de sair inteiro e legível. É o que
    // distingue o helper do `char::escape_default`, que daria `caf\u{e9}`.
    argv.push("café ☕".to_owned());
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();

    let stderr = stderr_de(&corre_com(&argv));

    assert!(
        stderr.contains("«café ☕»"),
        "o valor acentuado saiu adulterado: {stderr:?}"
    );
    assert!(
        !stderr.contains("\\u{e9}") && !stderr.contains("\\u{20}"),
        "o saneamento escapou não-ASCII: {stderr:?}"
    );
}
