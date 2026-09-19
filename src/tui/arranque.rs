//! Resolução do arranque: que tema, que modo de cor e que ficheiro de
//! preferências esta sessão vai usar.
//!
//! O [`resolver`] é a cadeia de precedência do ADR (§Decisão 2 e 3) numa
//! função só: `--theme` → `TODO_RATATUI_THEME` → `config.json` → default, e
//! `--color` → `TODO_RATATUI_COLOR` → `auto`. Um valor inválido **não** é
//! fatal: fica um aviso e a cadeia segue para o valor seguinte.
//!
//! Porque é que isto não vive no `main.rs`, que é onde o cartão T4 o pede: um
//! teste de integração (`tests/arranque.rs`) não vê dentro do binário — só vê a
//! biblioteca (`todo_ratatui::…`) e o binário **construído**
//! (`CARGO_BIN_EXE_*`). A precedência só é demonstrável sem TTY se viver do
//! lado da biblioteca, e o `tui` é o lado certo: é ele que conhece o
//! [`Theme`]. O `main.rs` fica com o que é dele — analisar a linha de comando,
//! imprimir o que aqui se decidiu e devolver o código de saída.
//!
//! Três coisas que este módulo **não** faz:
//!
//! * não escreve nada — `--theme` e a env são *overrides de sessão* e só o
//!   `Enter` da caixa de temas grava (T5, ADR §Decisão 2);
//! * não decide o que sai no ecrã: o modo `ansi` troca o tema pelo
//!   [`ModoCor::resolver`] e o `config.json` fica como está (ADR §Decisão 3);
//! * não entra em pânico e não recusa o arranque: nada de preferência
//!   estragada (nem de ambiente torto) pode impedir abrir a lista.

use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::core::config::{self, Config};

use super::theme::{CATALOGO, ModoCor, Theme};

/// Variável de ambiente que substitui `--theme` (ADR §Decisão 2).
pub const ENV_THEME: &str = "TODO_RATATUI_THEME";

/// Variável de ambiente que substitui `--color` (ADR §Decisão 3).
pub const ENV_COLOR: &str = "TODO_RATATUI_COLOR";

/// As três flags deste cartão, já lidas da linha de comandos.
///
/// É um subconjunto do `Args` do `main.rs` de propósito: o que não é
/// preferência (`--db`, `--help`) não chega aqui.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Opcoes {
    pub theme: Option<String>,
    pub color: Option<String>,
    pub config: Option<PathBuf>,
}

/// As variáveis de ambiente que a resolução consulta, **já lidas**.
///
/// Existem como estrutura (em vez de `env::var` dentro do [`resolver`]) para a
/// resolução ser determinística nos testes: um teste passa o que quer e não
/// depende do ambiente do processo que corre `cargo test`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ambiente {
    pub theme: Option<String>,
    pub color: Option<String>,
    pub config: Option<OsString>,
}

impl Ambiente {
    /// Lê as três variáveis do processo. Vazio é como não estivesse definida,
    /// a mesma regra do [`config::resolve_path`].
    #[must_use]
    pub fn do_processo() -> Self {
        Self {
            theme: definida(ENV_THEME),
            color: definida(ENV_COLOR),
            config: env::var_os(config::ENV_CONFIG).filter(|valor| !valor.is_empty()),
        }
    }
}

/// Valor não vazio de uma variável de ambiente, se estiver definida.
fn definida(nome: &str) -> Option<String> {
    env::var(nome).ok().filter(|valor| !valor.is_empty())
}

/// O arranque resolvido: o que o [`super::run`] precisa e o que o `main.rs`
/// tem de dizer ao utilizador.
///
/// Os três campos são o que o cartão pede que a resolução devolva — tema, modo
/// de cor e caminho do `config.json` —, mais os avisos que a acompanham. O
/// `config_path` é [`Option`] porque a única falha possível aqui é não haver
/// onde guardar preferências (sem `XDG_CONFIG_HOME` nem `HOME`): nesse caso o
/// tema ainda se escolhe nesta sessão e o `App` fica sem caminho onde gravar,
/// em vez de inventar um (é o caso «sem config» que o T5 trata).
#[derive(Debug)]
pub struct Resolucao {
    /// O tema que a cadeia de precedência escolheu — antes do modo de cor.
    ///
    /// É o que fica guardado (T5) e o que o aviso do `ansi` nomeia: sem ele,
    /// `--theme tokyo-night-moon --color ansi` não tinha como dizer qual foi o
    /// tema pedido, já que [`Resolucao::tema`] já veio como `classico`.
    pub pedido: &'static Theme,
    /// O tema com que a sessão vai desenhar — [`Resolucao::pedido`] passado
    /// pelo modo de cor (em `ansi`, um tema com fundo dá o `classico`).
    pub tema: &'static Theme,
    /// O modo de cor pedido (não o detectado): quem quiser saber o que o
    /// terminal respondeu chama [`ModoCor::detectar`].
    pub modo: ModoCor,
    /// O ficheiro de preferências desta sessão (lido, nunca escrito aqui).
    pub config_path: Option<PathBuf>,
    /// Avisos para o `stderr`, por ordem de descoberta. Cada um é uma frase
    /// inteira, pronta a imprimir.
    pub avisos: Vec<String>,
}

/// A forma imprimível de `texto`: cada carácter de controlo sai como a
/// sequência visível `\u{1b}` (a grafia do `Debug` do Rust), e tudo o resto fica
/// tal e qual.
///
/// É a **fronteira de impressão** do programa (ADR §Decisão 2). O `stderr` é o
/// único sítio que escreve no terminal fora do ratatui, e até ele chegam dados
/// não controlados: o *slug* de `--theme`, o valor de `--color`, o *token* cru
/// de um argumento desconhecido, e os `Display` de
/// [`ConfigError`](crate::core::ConfigError)/[`StoreError`](crate::core::StoreError),
/// que embutem `path.display()`. Sem isto, uma sequência OSC 52 posta num slug
/// copiava texto para a área de transferência de quem lê o aviso, e um `\n`
/// forjava uma linha inteira a imitar uma mensagem do programa.
///
/// Só caracteres de controlo (`char::is_control`, a categoria `Cc`, com `\n` e
/// `\t` incluídos): `char::escape_default` **não** serve, porque escapa também o
/// que não é ASCII — `café` sairia `caf\u{e9}` e o aviso passaria a mentir
/// sobre o valor que o utilizador escreveu.
///
/// A sanitização é **só na impressão**: o `config.json` e o `db.json` continuam
/// a guardar o valor original.
#[must_use]
pub fn imprimivel(texto: &str) -> String {
    let mut saida = String::with_capacity(texto.len());
    for caracter in texto.chars() {
        if caracter.is_control() {
            let _ = write!(saida, "\\u{{{:x}}}", u32::from(caracter));
        } else {
            saida.push(caracter);
        }
    }
    saida
}

impl Resolucao {
    /// Escreve os avisos no `stderr`, um por linha e com o prefixo `aviso:`.
    ///
    /// O `main.rs` chama isto **antes** de [`super::run`]: o ecrã alternativo
    /// ainda não está ligado e uma mensagem escrita depois desaparecia por
    /// baixo do primeiro desenho.
    ///
    /// Cada aviso passa por [`imprimivel`]: é aqui, num só sítio, que se garante
    /// que nada do que os avisos carregam (slug, valor de `--color`, caminhos)
    /// chega ao terminal com caracteres de controlo.
    pub fn avisar(&self) {
        for aviso in &self.avisos {
            eprintln!("aviso: {}", imprimivel(aviso));
        }
    }
}

/// Resolve o tema, o modo de cor e o caminho do `config.json`.
///
/// Lê o ficheiro de preferências (nunca o escreve) e valida cada *slug* com
/// [`Theme::por_slug`]. Um valor inválido de precedência **mais alta** ganha o
/// aviso mas não a decisão: `--theme nao-existe` avisa e cai no valor seguinte
/// da cadeia, e um valor válido numa fonte de precedência mais alta cala as
/// fontes de baixo (não se avisa do que nunca chegou a ser usado).
///
/// Não devolve `Result`: a única falha possível — não haver `config_dir()` — é
/// absorvida num aviso e a resolução segue sem preferência guardada.
#[must_use]
pub fn resolver(opcoes: &Opcoes, ambiente: &Ambiente) -> Resolucao {
    let mut avisos = Vec::new();

    // `--config` > `TODO_RATATUI_CONFIG` > `dirs::config_dir()`. Passa-se o
    // valor já escolhido ao `resolve_path` de T1: é lá que vive a última etapa
    // da cadeia, e assim não há duas regras de caminhos a divergir.
    let config_path = match config::resolve_path(
        opcoes
            .config
            .as_deref()
            .or(ambiente.config.as_deref().map(Path::new)),
    ) {
        Ok(caminho) => Some(caminho),
        Err(err) => {
            avisos.push(format!(
                "{err} — sigo sem preferência guardada (o tema escolhe-se nesta sessão)"
            ));
            None
        }
    };

    let (preferencias, aviso) = match config_path.as_deref() {
        Some(caminho) => Config::load_or_default(caminho),
        None => (Config::default(), None),
    };
    avisos.extend(aviso);

    let tema = o_tema(
        &[
            ("--theme", opcoes.theme.as_deref()),
            (ENV_THEME, ambiente.theme.as_deref()),
            (config::CONFIG_FILE_NAME, preferencias.theme.as_deref()),
        ],
        &mut avisos,
    );

    let (modo, fonte) = o_modo(
        &[
            ("--color", opcoes.color.as_deref()),
            (ENV_COLOR, ambiente.color.as_deref()),
        ],
        &mut avisos,
    );

    let desenhado = modo.resolver(tema);
    if desenhado.slug != tema.slug {
        // Aconteceu o que o ADR §Decisão 3 prevê: um tema com fundo num
        // terminal de 16 cores. O aviso diz **qual** o modo que levou a isso —
        // pedido ou detectado — e garante que o ficheiro não é tocado: a
        // escolha fica lá para a próxima vez que houver truecolor.
        let origem = match fonte {
            Some(fonte) => format!("{fonte}={}", nome(modo)),
            None => format!("sem truecolor (detectado: {})", nome(modo.detectar())),
        };
        avisos.push(format!(
            "{origem}: o tema «{}» pinta o fundo e não cabe em 16 cores — uso «{}» \
             nesta sessão; o {} fica como está",
            tema.slug,
            desenhado.slug,
            config::CONFIG_FILE_NAME
        ));
    }

    Resolucao {
        pedido: tema,
        tema: desenhado,
        modo,
        config_path,
        avisos,
    }
}

/// O primeiro *slug* válido da cadeia; o último recurso é o [`Theme::default`].
///
/// Cada valor inválido deixa um aviso com a lista dos que existem, porque é
/// essa a informação que falta a quem escreveu o *slug* errado.
fn o_tema(fontes: &[(&str, Option<&str>)], avisos: &mut Vec<String>) -> &'static Theme {
    let mut houve_candidato = false;
    for (fonte, valor) in fontes {
        let Some(slug) = valor else { continue };
        houve_candidato = true;
        if let Some(tema) = Theme::por_slug(slug) {
            return tema;
        }
        avisos.push(format!(
            "{fonte}: tema «{slug}» desconhecido — os temas são: {}",
            slugs()
        ));
    }
    let padrao = Theme::default();
    if houve_candidato {
        avisos.push(format!(
            "nenhum tema válido na cadeia — uso «{}», o tema por omissão",
            padrao.slug
        ));
    }
    padrao
}

/// O primeiro modo de cor válido da cadeia e a fonte que o decidiu.
///
/// A fonte (`None` = caiu no `auto`) é o que o aviso do `ansi` precisa de
/// dizer: um `--color ansi` pedido não é a mesma coisa que um terminal que não
/// anuncia truecolor, ainda que o resultado seja o mesmo [`ModoCor::Ansi`].
fn o_modo(
    fontes: &[(&'static str, Option<&str>)],
    avisos: &mut Vec<String>,
) -> (ModoCor, Option<&'static str>) {
    for (fonte, valor) in fontes {
        let Some(modo) = *valor else { continue };
        match modo {
            "auto" => return (ModoCor::Auto, Some(fonte)),
            "rgb" => return (ModoCor::Rgb, Some(fonte)),
            "ansi" => return (ModoCor::Ansi, Some(fonte)),
            _ => avisos.push(format!(
                "{fonte}: modo de cor «{modo}» desconhecido — os valores são: auto, rgb, ansi"
            )),
        }
    }
    (ModoCor::Auto, None)
}

/// Os *slugs* do catálogo, na ordem em que a caixa os mostra.
fn slugs() -> String {
    CATALOGO
        .iter()
        .map(|tema| tema.slug)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Nome do modo como se escreve na linha de comandos — a mesma grafia de
/// `--color`, para o aviso poder ser copiado para lá.
const fn nome(modo: ModoCor) -> &'static str {
    match modo {
        ModoCor::Auto => "auto",
        ModoCor::Rgb => "rgb",
        ModoCor::Ansi => "ansi",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imprimivel_deixa_o_texto_normal_intacto() {
        // Acentos, «», emoji e o ☕ continuam a ser o que o utilizador escreveu:
        // é o que distingue isto do `char::escape_default`.
        let texto = "café «á» ☕ — tema desconhecido";
        assert_eq!(imprimivel(texto), texto);
        assert_eq!(imprimivel(""), "");
        assert_eq!(imprimivel("tokyo-night-moon"), "tokyo-night-moon");
    }

    #[test]
    fn imprimivel_escapa_so_os_caracteres_de_controlo() {
        // A sequência do achado SA-01 (OSC 52, que copia para a área de
        // transferência) e um `\n`, que forjava uma linha no `stderr`.
        assert_eq!(imprimivel("a\x1bb\nc"), "a\\u{1b}b\\u{a}c");
        assert_eq!(
            imprimivel("x\x1b]52;c;SGVsbG8=\x07"),
            "x\\u{1b}]52;c;SGVsbG8=\\u{7}"
        );
        assert_eq!(imprimivel("\t\r\u{7f}"), "\\u{9}\\u{d}\\u{7f}");

        // A propriedade, e não só os exemplos: nenhuma saída tem um carácter
        // de controlo.
        let saida = imprimivel("a\x1bb\nc\td\x00e");
        assert!(
            !saida.chars().any(char::is_control),
            "ficou um carácter de controlo: {saida:?}"
        );
    }
}
