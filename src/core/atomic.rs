//! Escrita atómica de ficheiro, partilhada pelo `store` e pelo `config`.
//!
//! Uma só implementação: [`write_atomic`]. O padrão é o que o `store` já usava
//! para o `db.json` (`tmp` ao lado do destino + `rename`), extraído para aqui
//! sem mudar a ordem de nada (ADR §Decisão 6). O `store` continua a ser o dono
//! do caminho dos **dados**; o `config` traz a **preferência**.

use std::fs::{self, File};
use std::io::{self, Write};
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
/// Não cria o directório-pai nem sincroniza o directório: quem chama sabe que
/// caminho tem de existir e o que quer fazer com o resultado.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path_for(path);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    if path.is_file() {
        fs::rename(path, backup_path_for(path))?;
    }

    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "todo-ratatui-atomic-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("criar diretório de teste");
        dir
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
}
