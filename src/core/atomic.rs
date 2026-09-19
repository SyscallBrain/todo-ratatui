//! Escrita atómica de ficheiro, partilhada pelo `store` e pelo `config`.
//!
//! Uma só implementação: [`write_atomic`]. O padrão é o que o `store` já usava
//! para o `db.json` (`tmp` ao lado do destino + `rename`), extraído para aqui
//! sem mudar a ordem de nada (ADR §Decisão 6). O `store` continua a ser o dono
//! do caminho dos **dados**; o `config` traz a **preferência**.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Nome do ficheiro temporário de `path` (`db.json` → `db.json.tmp`).
///
/// O temporário tem de nascer **no mesmo directório do destino**: um `rename`
/// entre sistemas de ficheiros diferentes (por exemplo de `/tmp`, que costuma
/// ser `tmpfs`) não é atómico, e falha.
#[must_use]
pub fn tmp_path_for(path: &Path) -> PathBuf {
    with_suffix(path, "tmp")
}

/// Nome do backup de `path` (`db.json` → `db.json.bak`), onde fica a geração
/// anterior.
#[must_use]
pub fn backup_path_for(path: &Path) -> PathBuf {
    with_suffix(path, "bak")
}

/// Acrescenta um sufixo ao **nome** do ficheiro, preservando o directório
/// (`/a/b.json` + `tmp` → `/a/b.json.tmp`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    match path.file_name() {
        Some(nome) => {
            let mut com_sufixo = nome.to_os_string();
            com_sufixo.push(".");
            com_sufixo.push(suffix);
            path.with_file_name(com_sufixo)
        }
        // Sem nome de ficheiro (`/`, `..`) não há convenção a inventar.
        None => path.with_extension(suffix),
    }
}

/// Cria `path` como ficheiro **novo** e privado (`0600`).
///
/// É o construtor de todo o ficheiro de dados do programa; um `File::create`/
/// `fs::copy` num caminho de dados passa a ser um defeito (ADR §Consequências).
///
/// * `create_new(true)` — o `O_EXCL` do `open(2)` — recusa um caminho que já
///   exista em vez de o abrir: é o que fecha o achado SA-02. Com o
///   `File::create` de antes, um **symlink** plantado em `db.json.tmp` fazia a
///   escrita aterrar no ficheiro apontado, e um **hard link** fazia-a aterrar
///   no inode partilhado. Um `unlink` desliga o nome; nunca escreve através
///   dele.
/// * `mode(0o600)` fecha o achado SA-03: deixado ao `umask` (0002 no sistema em
///   uso) os dados nasciam a `664`, legíveis por qualquer utilizador local.
///
/// Um caminho que já exista é **removido** e a criação repete-se **uma** vez:
/// sem isso, um `<destino>.tmp` deixado por uma corrida interrompida bloqueava
/// a gravação para sempre. O nome removido é sempre um derivado do destino, e
/// quem lá planta um link já podia apagar o `db.json` — mas é um
/// comportamento novo, assumido no ADR. Se a remoção falhar (o caminho é um
/// directório, por exemplo), o erro sobe e a gravação falha como falharia
/// hoje: nunca se escreve por cima do que lá está.
pub fn criar_privado(path: &Path) -> io::Result<File> {
    match abrir_novo_privado(path) {
        Ok(ficheiro) => Ok(ficheiro),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(path)?;
            abrir_novo_privado(path)
        }
        Err(err) => Err(err),
    }
}

/// A abertura de [`criar_privado`], isolada porque é feita duas vezes (`O_EXCL`
/// + `0600`).
fn abrir_novo_privado(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// Cria `dir` (e os pais que faltarem) a `0700`, **quando é o programa a
/// criá-lo**.
///
/// Um directório que já exista **não é tocado**: o modo de um directório
/// instalado é uma escolha de quem lá tem os dados (partilha com o grupo, por
/// exemplo) e impor `0700` em cada gravação sobrepôr-se-ia a ela — e num
/// sistema de ficheiros onde o `chmod` falha (FAT, alguns montes de rede)
/// transformaria uma gravação num erro (ADR §Decisão 4). O `set_permissions`
/// é, por isso, *best effort*: é higiene, e higiene nunca derruba a gravação.
///
/// O `0700` fica no directório pedido; os pais que o `create_dir_all` tiver de
/// criar pelo caminho (por exemplo um `~/.local/share` inexistente) não são
/// nossos e ficam como o `umask` os deixar.
pub fn criar_directorio_privado(dir: &Path) -> io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    Ok(())
}

/// Substitui o conteúdo de `path` de forma atómica.
///
/// A sequência é, exactamente, a que o [`Store`](super::store::Store) seguia
/// antes de esta função existir:
///
/// 1. o temporário [`tmp_path_for`] é criado no **mesmo directório do destino**,
///    escrito por inteiro e sincronizado com `fsync`;
/// 2. se o destino já for um ficheiro, é rodado para [`backup_path_for`] por
///    `rename`;
/// 3. o temporário ocupa o lugar do destino, também por `rename`.
///
/// Os passos 2 e 3 são `rename` e não cópias: ou o ficheiro fica inteiro e novo,
/// ou fica o antigo — nunca meio-escrito. A rotação vive **dentro** desta
/// função, e não em quem chama, porque tem de acontecer entre o passo 1 e o
/// passo 3: o ficheiro novo é escrito por inteiro *antes* de se mexer no antigo,
/// logo uma falha a meio (disco cheio, por exemplo) não perde a geração
/// anterior. O `.bak` não existe na primeira escrita, quando ainda não há
/// geração anterior para guardar.
///
/// O temporário nasce por [`criar_privado`]: `0600` e sem seguir um nome que já
/// exista (os achados SA-02 e SA-03).
///
/// Não cria o directório-pai nem sincroniza o directório: quem chama sabe que
/// caminho tem de existir e o que quer fazer com o resultado.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path_for(path);
    {
        let mut file = criar_privado(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    if path.is_file() {
        let bak = backup_path_for(path);
        fs::rename(path, &bak)?;
        // O `.bak` é o **mesmo inode** do ficheiro rodado, logo herda o modo
        // dele: sem isto, a geração anterior de um `db.json` instalado a 664
        // continuava legível por outros utilizadores (o 0600 do temporário não
        // chega — ADR, correcção 5). Best effort de propósito: num sistema de
        // ficheiros sem `chmod` isto falha, e uma falha de higiene não pode
        // custar a gravação dos dados do utilizador. Não se avisa porque o
        // `core` não escreve para o terminal — quem o faz é o `tui`, e a
        // fronteira das camadas é dele (ADR §Decisão 2).
        //
        // Só quando o `.bak` é um ficheiro **regular**. Se o destino era um
        // symlink (um `--db` a apontar para um link é um uso legítimo — ADR
        // §SA-02), o `rename` move o *link* para o `.bak` e o
        // `set_permissions` seguiria esse link: o `chmod` aterraria no
        // ficheiro apontado, que o programa não criou, sobrepondo-se ao modo
        // de uma entrada pré-existente (ADR §Decisão 4).
        if fs::symlink_metadata(&bak).is_ok_and(|meta| meta.file_type().is_file()) {
            let _ = fs::set_permissions(&bak, fs::Permissions::from_mode(0o600));
        }
    }

    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::os::unix::fs::symlink;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "todo-ratatui-atomic-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("criar diretório de teste");
        dir
    }

    /// Os bits de permissão do ficheiro, sem o tipo (`0o600`, `0o700`, …).
    fn modo(path: &Path) -> u32 {
        fs::metadata(path)
            .expect("metadados do ficheiro")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn escreve_roda_o_bak_e_nao_deixa_temporarios() {
        let dir = temp_dir("escreve");
        let path = dir.join("db.json");

        write_atomic(&path, b"primeira").expect("primeira gravação");
        assert_eq!(fs::read(&path).expect("ler"), b"primeira");
        assert!(
            !backup_path_for(&path).exists(),
            "a primeira gravação não tem geração anterior para rodar"
        );

        write_atomic(&path, b"segunda").expect("segunda gravação");
        assert_eq!(fs::read(&path).expect("ler"), b"segunda");
        assert_eq!(
            fs::read(backup_path_for(&path)).expect("ler o bak"),
            b"primeira",
            "o .bak tem a geração anterior"
        );

        let restos: Vec<String> = fs::read_dir(&dir)
            .expect("listar")
            .filter_map(Result::ok)
            .map(|entrada| entrada.file_name().to_string_lossy().into_owned())
            .filter(|nome| nome.ends_with(".tmp"))
            .collect();
        assert!(restos.is_empty(), "ficaram temporários: {restos:?}");
    }

    #[test]
    fn temporario_fica_ao_lado_do_destino() {
        let dir = temp_dir("ao-lado");
        let path = dir.join("sub").join("config.json");

        // Nunca em `/tmp`: o nome do temporário deriva do destino, e é esse o
        // que o `rename` final consome.
        assert_eq!(tmp_path_for(&path), dir.join("sub").join("config.json.tmp"));
        assert_eq!(
            backup_path_for(&path),
            dir.join("sub").join("config.json.bak")
        );

        fs::create_dir_all(path.parent().expect("directório-pai")).expect("criar directório");
        write_atomic(&path, b"{\"theme\":\"classico\"}").expect("gravar");
        assert_eq!(fs::read(&path).expect("ler"), b"{\"theme\":\"classico\"}");
    }

    // -------------------------------------------------------------- SA-02/03

    /// Um `db.json.tmp` plantado como **symlink** para outro ficheiro. Com o
    /// `File::create` de antes, a escrita atravessava o link e aterraria na
    /// vítima; com `O_EXCL` + `unlink`, o link é desligado e a vítima fica como
    /// estava.
    #[test]
    fn symlink_plantado_no_temporario_nao_redirecciona_a_escrita() {
        let dir = temp_dir("symlink");
        let path = dir.join("db.json");
        let vitima = dir.join("vitima.txt");
        fs::write(&vitima, b"conteudo da vitima").expect("escrever a vítima");
        symlink(&vitima, tmp_path_for(&path)).expect("plantar o symlink");

        write_atomic(&path, b"novo").expect("a gravação tem de suceder");

        assert_eq!(
            fs::read(&vitima).expect("ler a vítima"),
            b"conteudo da vitima",
            "a escrita atravessou o symlink"
        );
        assert_eq!(fs::read(&path).expect("ler o destino"), b"novo");
        assert!(
            !fs::symlink_metadata(&path)
                .expect("metadados do destino")
                .file_type()
                .is_symlink(),
            "o destino tem de ser um ficheiro regular"
        );
        assert!(!tmp_path_for(&path).exists(), "o link ficou por lá");
    }

    /// O mesmo com um **hard link**: o `File::create` escrevia no inode
    /// partilhado (a vítima mudava de conteúdo); o `unlink` do nome deixa-a
    /// intacta.
    #[test]
    fn hard_link_plantado_no_temporario_nao_redirecciona_a_escrita() {
        let dir = temp_dir("hard-link");
        let path = dir.join("db.json");
        let vitima = dir.join("vitima.txt");
        fs::write(&vitima, b"conteudo da vitima").expect("escrever a vítima");
        fs::hard_link(&vitima, tmp_path_for(&path)).expect("plantar o hard link");

        write_atomic(&path, b"novo").expect("a gravação tem de suceder");

        assert_eq!(
            fs::read(&vitima).expect("ler a vítima"),
            b"conteudo da vitima",
            "a escrita aterrou no inode partilhado"
        );
        assert_eq!(fs::read(&path).expect("ler o destino"), b"novo");
    }

    /// O `create_new` não pode trocar um *clobber* por uma gravação impossível:
    /// um temporário deixado por uma corrida interrompida é substituído.
    #[test]
    fn temporario_orfao_nao_bloqueia_a_gravacao() {
        let dir = temp_dir("orfao");
        let path = dir.join("db.json");
        fs::write(tmp_path_for(&path), b"lixo da corrida interrompida").expect("deixar o lixo");

        write_atomic(&path, b"novo").expect("a gravação tem de suceder");

        assert_eq!(fs::read(&path).expect("ler o destino"), b"novo");
        assert!(!tmp_path_for(&path).exists(), "o lixo ficou no caminho");
    }

    #[test]
    fn os_ficheiros_criados_nascem_privados_inclusive_o_bak_rodado() {
        let dir = temp_dir("modos");
        let path = dir.join("db.json");

        write_atomic(&path, b"primeira").expect("primeira gravação");
        assert_eq!(modo(&path), 0o600, "o destino novo nasce 0600");

        write_atomic(&path, b"segunda").expect("segunda gravação");
        assert_eq!(
            modo(&backup_path_for(&path)),
            0o600,
            "o .bak rodado de um ficheiro já privado"
        );

        // O caso real do SA-03: um `db.json` instalado pela v1.0.1 com o `umask`
        // 0002, a 664. O `.bak` herda o modo do ficheiro rodado, logo o 0600 do
        // temporário não chegava aqui.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).expect("modo antigo");
        write_atomic(&path, b"terceira").expect("terceira gravação");
        assert_eq!(
            modo(&backup_path_for(&path)),
            0o600,
            "o .bak rodado herdou os 664 do ficheiro antigo"
        );
        assert_eq!(modo(&path), 0o600, "o destino volta a nascer privado");
    }

    /// Um destino que é um **symlink** (uso legítimo: um `--db` a apontar para
    /// um link) não pode levar o `chmod` do `.bak` até ao ficheiro apontado: o
    /// `rename` roda o *link*, e o `set_permissions` seguiria-o — o modo de um
    /// ficheiro que o programa não criou não se sobrepõe (ADR §Decisão 4).
    /// O `.bak` fica o link rodado (e o seu conteúdo, a geração antiga); o
    /// destino novo é que nasce regular e `0600`.
    #[test]
    fn o_bak_de_um_destino_symlink_nao_muda_o_modo_do_ficheiro_apontado() {
        let dir = temp_dir("destino-symlink");
        let alvo = dir.join("alvo.json");
        let path = dir.join("db.json");
        fs::write(&alvo, b"geracao antiga").expect("escrever o alvo");
        fs::set_permissions(&alvo, fs::Permissions::from_mode(0o664)).expect("modo antigo");
        symlink(&alvo, &path).expect("plantar o symlink");

        write_atomic(&path, b"novo").expect("a gravação tem de suceder");

        assert_eq!(
            modo(&alvo),
            0o664,
            "o chmod do `.bak` atravessou o link e aterrou no ficheiro apontado"
        );
        assert_eq!(
            fs::read(&alvo).expect("ler o alvo"),
            b"geracao antiga",
            "a escrita atravessou o link do destino"
        );
        assert_eq!(modo(&path), 0o600, "o destino novo nasce 0600");
        assert!(
            !fs::symlink_metadata(&path)
                .expect("metadados do destino")
                .file_type()
                .is_symlink(),
            "o destino tem de ser um ficheiro regular"
        );
        assert!(
            fs::symlink_metadata(backup_path_for(&path))
                .expect("metadados do bak")
                .file_type()
                .is_symlink(),
            "o `.bak` é o link rodado — é por isso que o chmod tem de ser guardado"
        );
    }

    #[test]
    fn o_directorio_criado_pelo_programa_e_0700_e_o_existente_nao_e_tocado() {
        let base = temp_dir("directorios");
        let novo = base.join("todo-ratatui");
        criar_directorio_privado(&novo).expect("criar o directório novo");
        assert_eq!(modo(&novo), 0o700, "o directório do programa nasce 0700");

        let existente = temp_dir("directorio-existente");
        fs::set_permissions(&existente, fs::Permissions::from_mode(0o755)).expect("0755");
        criar_directorio_privado(&existente).expect("um directório existente não é erro");
        assert_eq!(
            modo(&existente),
            0o755,
            "um directório já instalado é uma escolha do utilizador e não se toca"
        );
    }
}
