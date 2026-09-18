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
//! O caso «não há TTY nenhum» não está aqui: para o reproduzir sem arriscar
//! herdar o terminal de quem corre os testes, usa-se a linha de comandos
//! (`echo q | ./target/debug/todo_ratatui`), como o critério 7 do cartão pede.
//!
//! Um prazo em cada corrida não é gosto de robustez: sem ele, um defeito nesta
//! política deixaria o `cargo test` pendurado numa TUI à espera de uma tecla.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use todo_ratatui::core::backup_path_for;

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
