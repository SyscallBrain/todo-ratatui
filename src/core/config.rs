//! Preferências do utilizador: ficheiro pequeno, separado dos dados.
//!
//! Aqui vive **preferência**, não dado: um tema é uma escolha de quem usa, e
//! perdê-la não pode custar uma tarefa nem impedir abrir a lista (ADR §Decisão
//! 2, 3 e 7). Daí o desenho:
//!
//! - o ficheiro é `dirs::config_dir()/todo-ratatui/config.json` — separado do
//!   `db.json`, que fica em `dirs::data_dir()` e **não** muda de formato;
//! - ficheiro ausente, ilegível ou corrompido nunca é fatal e nunca é
//!   reescrito: o arranque segue com os valores por omissão e devolve um aviso
//!   para o `stderr`. Substituí-lo é decisão do utilizador, ao gravar;
//! - a escrita é atómica e partilhada com o `store` ([`super::atomic`]), e roda
//!   a geração anterior para `config.json.bak`.
//!
//! Nada aqui conhece `ratatui`/`crossterm`: o `core` guarda o *slug* do tema
//! como texto, e a paleta é que resolve o slug (`tests/boundary.rs` trava isto).

use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::atomic;

/// Nome do directório dentro do `config_dir()` do sistema.
///
/// Coincide de propósito com o `APP_DIR` do [`store`](super::store): é a mesma
/// aplicação, com os dados em `dirs::data_dir()` e a preferência aqui.
const APP_DIR: &str = "todo-ratatui";

/// Nome do ficheiro de preferências dentro do directório da aplicação.
pub const CONFIG_FILE_NAME: &str = "config.json";

/// Variável de ambiente que substitui o caminho do ficheiro de preferências.
pub const ENV_CONFIG: &str = "TODO_RATATUI_CONFIG";

/// Preferências em disco.
///
/// Um só campo, com o *slug* do tema (ex.: `"tokyo-night-storm"`). Guarda-se o
/// slug e não a paleta: a paleta é código revisto, o ficheiro é do utilizador
/// (ADR §Decisão 1). Um ficheiro sem `theme` é um ficheiro válido — diz apenas
/// que ainda não há preferência.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub theme: Option<String>,
}

/// Erros do ficheiro de preferências. Como nos [`StoreError`](super::StoreError),
/// cada variante identifica o caminho: um erro sem caminho é inútil a quem o lê.
#[derive(Debug)]
pub enum ConfigError {
    /// Não há `dirs::config_dir()`: nem `XDG_CONFIG_HOME` nem `HOME` resolvem.
    NoConfigDir,
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl ConfigError {
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Io { path, .. } | Self::Json { path, .. } => Some(path),
            Self::NoConfigDir => None,
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "falha de I/O em «{}»: {source}", path.display())
            }
            Self::Json { path, source } => {
                write!(f, "JSON inválido em «{}»: {source}", path.display())
            }
            Self::NoConfigDir => f.write_str(
                "não consigo determinar o directório de configuração: define \
                 XDG_CONFIG_HOME ou HOME, ou usa --config <caminho>",
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::NoConfigDir => None,
        }
    }
}

/// Resolve o caminho do ficheiro de preferências, por ordem de precedência:
/// `--config <caminho>` → `TODO_RATATUI_CONFIG` →
/// `dirs::config_dir()/todo-ratatui/config.json` (= `$XDG_CONFIG_HOME` →
/// `$HOME/.config`).
///
/// `config_dir()` e não `data_dir()`: isto é uma preferência desta máquina, não
/// um dado a preservar (a simetria com o [`resolve_path`](super::store::resolve_path)
/// do `store`, que vive em `data_dir()`).
pub fn resolve_path(explicit: Option<&Path>) -> Result<PathBuf, ConfigError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(from_env) = env::var_os(ENV_CONFIG).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(from_env));
    }
    let base = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
    Ok(base.join(APP_DIR).join(CONFIG_FILE_NAME))
}

/// Texto do aviso que acompanha a queda para os valores por omissão.
///
/// Diz as três coisas que quem lê o `stderr` precisa de saber: o que se passou,
/// com que valores o programa segue, e que o ficheiro não foi tocado.
fn aviso(err: &ConfigError) -> String {
    format!("{err} — sigo com os valores por omissão; o ficheiro fica como está")
}

impl Config {
    /// Lê o ficheiro. Nunca falha e nunca escreve.
    ///
    /// - ficheiro ausente → `(default, None)`: não há nada a avisar;
    /// - ilegível, corrompido ou com o tipo errado (`{"theme": 3}`) →
    ///   `(default, Some(aviso))`, com o original **intacto**: nem reescrito nem
    ///   truncado.
    ///
    /// Devolve um aviso em vez de um erro, e não um `Result`, de propósito: uma
    /// preferência estragada não pode impedir abrir a lista (ADR §Decisão 7). O
    /// aviso já vai formatado para o `stderr`.
    #[must_use]
    pub fn load_or_default(path: &Path) -> (Self, Option<String>) {
        if !path.exists() {
            return (Self::default(), None);
        }

        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(source) => {
                return (
                    Self::default(),
                    Some(aviso(&ConfigError::Io {
                        path: path.to_path_buf(),
                        source,
                    })),
                );
            }
        };

        match serde_json::from_slice(&bytes) {
            Ok(config) => (config, None),
            Err(source) => (
                Self::default(),
                Some(aviso(&ConfigError::Json {
                    path: path.to_path_buf(),
                    source,
                })),
            ),
        }
    }

    /// Grava as preferências, de forma atómica e com rotação da geração anterior
    /// para `config.json.bak` (o mesmo padrão do `db.json`, ADR §Decisão 7).
    ///
    /// Cria o directório-pai: na primeira gravação, `~/.config/todo-ratatui/`
    /// ainda não existe. Falhar a gravar devolve o erro a quem chama — a sessão
    /// continua com o tema escolhido, sem `panic!` (ADR §Decisão 8).
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let mut json = serde_json::to_vec_pretty(self).map_err(|source| ConfigError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        json.push(b'\n');

        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(|source| ConfigError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
        }

        atomic::write_atomic(path, &json).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "todo-ratatui-config-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("criar diretório de teste");
        dir
    }

    fn com_tema(slug: &str) -> Config {
        Config {
            theme: Some(slug.to_owned()),
        }
    }

    fn temporarios_em(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .expect("listar")
            .filter_map(Result::ok)
            .map(|entrada| entrada.file_name().to_string_lossy().into_owned())
            .filter(|nome| nome.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn round_trip_do_slug() {
        let dir = temp_dir("round-trip");
        let path = dir.join(CONFIG_FILE_NAME);

        com_tema("tokyo-night-storm")
            .save(&path)
            .expect("gravar a preferência");

        let cru = fs::read_to_string(&path).expect("ler");
        let value: serde_json::Value = serde_json::from_str(&cru).expect("o ficheiro é JSON");
        assert_eq!(value["theme"], serde_json::json!("tokyo-night-storm"));

        let (config, aviso) = Config::load_or_default(&path);
        assert_eq!(aviso, None, "um ficheiro nosso não gera aviso: {cru}");
        assert_eq!(config, com_tema("tokyo-night-storm"));
    }

    #[test]
    fn ficheiro_ausente_da_o_default_sem_aviso() {
        let dir = temp_dir("ausente");
        let path = dir.join(CONFIG_FILE_NAME);

        let (config, aviso) = Config::load_or_default(&path);
        assert_eq!(config, Config::default());
        assert_eq!(config.theme, None);
        assert_eq!(aviso, None, "sem ficheiro não há nada a avisar");
        assert!(!path.exists(), "ler não cria o ficheiro");
    }

    /// Um ficheiro que não se consegue ler como [`Config`] fica intacto: a
    /// leitura devolve o default **e** um aviso, sem tocar num byte (a
    /// substituição só acontece quando o utilizador grava).
    fn mau_ficheiro_avisa_e_fica_intacto(tag: &str, conteudo: &[u8]) {
        let dir = temp_dir(tag);
        let path = dir.join(CONFIG_FILE_NAME);
        fs::write(&path, conteudo).expect("escrever");
        let antes = fs::read(&path).expect("ler antes");

        let (config, aviso) = Config::load_or_default(&path);

        assert_eq!(config, Config::default(), "cai nos valores por omissão");
        let aviso = aviso.expect("um ficheiro estragado tem de avisar");
        assert!(
            aviso.contains(&path.display().to_string()),
            "o aviso nomeia o ficheiro: {aviso}"
        );
        assert!(
            aviso.contains("JSON inválido"),
            "o aviso diz o que se passou: {aviso}"
        );
        assert_eq!(
            fs::read(&path).expect("ler depois"),
            antes,
            "o ficheiro ficou byte-a-byte igual"
        );
        assert!(temporarios_em(&dir).is_empty(), "ler não deixa temporários");
    }

    #[test]
    fn tipo_errado_avisa_e_fica_intacto() {
        mau_ficheiro_avisa_e_fica_intacto("tipo-errado", br#"{"theme": 3}"#);
    }

    #[test]
    fn json_invalido_avisa_e_fica_intacto() {
        mau_ficheiro_avisa_e_fica_intacto("json-invalido", b"{ isto nao e json");
    }

    #[test]
    fn ficheiro_vazio_avisa_e_fica_intacto() {
        mau_ficheiro_avisa_e_fica_intacto("vazio", b"");
    }

    #[test]
    fn ficheiro_ilegivel_avisa_e_fica_intacto() {
        let dir = temp_dir("ilegivel");
        let path = dir.join(CONFIG_FILE_NAME);
        fs::write(&path, br#"{"theme": "tokyo-night"}"#).expect("escrever");
        let antes = fs::read(&path).expect("ler antes");

        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("tirar a leitura");
        if fs::read(&path).is_ok() {
            // A correr como root as permissões não travam nada, logo este caso
            // não é reproduzível aqui. Dizê-lo em vez de passar em silêncio.
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("devolver");
            eprintln!("a saltar: ficheiro ilegível não é reproduzível a correr como root");
            return;
        }

        let (config, aviso) = Config::load_or_default(&path);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("devolver");

        assert_eq!(config, Config::default(), "sem leitura, cai no default");
        let aviso = aviso.expect("um ficheiro ilegível avisa");
        assert!(
            aviso.contains(&path.display().to_string()),
            "o aviso nomeia o ficheiro: {aviso}"
        );
        assert_eq!(fs::read(&path).expect("ler depois"), antes);
    }

    #[test]
    fn save_roda_o_bak_e_nao_deixa_temporarios() {
        let dir = temp_dir("rotacao");
        let path = dir.join(CONFIG_FILE_NAME);
        let bak = atomic::backup_path_for(&path);

        com_tema("tokyo-night")
            .save(&path)
            .expect("primeira gravação");
        assert!(
            !bak.exists(),
            "a primeira gravação não tem geração anterior para rodar"
        );

        com_tema("tokyo-night-moon")
            .save(&path)
            .expect("segunda gravação");

        let (actual, _) = Config::load_or_default(&path);
        assert_eq!(actual, com_tema("tokyo-night-moon"), "o ficheiro é o novo");
        let (anterior, _) = Config::load_or_default(&bak);
        assert_eq!(
            anterior,
            com_tema("tokyo-night"),
            "o config.json.bak tem a geração anterior"
        );

        let restos = temporarios_em(&dir);
        assert!(restos.is_empty(), "ficaram temporários: {restos:?}");
    }

    #[test]
    fn resolve_path_precede_e_cai_no_config_dir() {
        let explicito = PathBuf::from("/tmp/explicito/config.json");
        assert_eq!(
            resolve_path(Some(&explicito)).expect("--config"),
            explicito,
            "--config precede tudo"
        );

        // Seguro em edição 2024: este binário de testes mexe na variável de
        // ambiente do processo num único teste, e nenhum outro chama
        // `resolve_path(None)` com ela posta (mesmo cuidado do `db_path.rs`).
        unsafe {
            env::set_var(ENV_CONFIG, "/tmp/do-ambiente/config.json");
        }
        assert_eq!(
            resolve_path(Some(&explicito)).expect("--config"),
            explicito,
            "--config precede também a env"
        );
        assert_eq!(
            resolve_path(None).expect("env"),
            PathBuf::from("/tmp/do-ambiente/config.json"),
            "TODO_RATATUI_CONFIG precede o dirs::config_dir()"
        );

        unsafe {
            env::remove_var(ENV_CONFIG);
        }
        let do_dirs = resolve_path(None).expect("dirs");
        assert!(
            do_dirs.ends_with(Path::new(APP_DIR).join(CONFIG_FILE_NAME)),
            "sem env cai no config_dir(), veio «{}»",
            do_dirs.display()
        );
        assert!(
            do_dirs.is_absolute(),
            "o caminho do config_dir() é absoluto, veio «{}»",
            do_dirs.display()
        );

        // A mensagem do erro que a máquina não reproduz (sem HOME o `dirs` cai
        // no passwd) fica presa directamente, como o `db_path.rs` faz com o
        // `NoDataDir`: tem de nomear as variáveis e a saída.
        let mensagem = ConfigError::NoConfigDir.to_string();
        assert!(
            mensagem.contains("XDG_CONFIG_HOME")
                && mensagem.contains("HOME")
                && mensagem.contains("--config"),
            "a mensagem tem de nomear as duas variáveis e a saída, veio «{mensagem}»"
        );
    }
}
