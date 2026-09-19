//! Catálogo de temas: a cor do ecrã como dados com nome.
//!
//! Na v1.0.1 a cor vivia em 9 constantes de [`super::ui`], todas fixas. Aqui
//! passa a ser um [`Theme`] — os mesmos papéis, mais o `bg` — escolhido de um
//! [`CATALOGO`] de quatro temas cujo *slug* é o que fica gravado no
//! `config.json` (a preferência é do [`crate::core::config`], que só conhece
//! texto: `ratatui::style::Color` não entra no `core`, e `tests/boundary.rs`
//! trava isso).
//!
//! Três coisas que este ficheiro **não** decide sozinho:
//!
//! * **Os valores.** Os hex são os da tabela §T.2 de
//!   `design-system/todo-ratatui.md`, citados da fonte canónica
//!   (`folke/tokyonight.nvim`); os `Indexed(n)` do `classico` são os índices
//!   que a v1.0.1 emite. Nenhuma cor é escolhida aqui.
//! * **Se os valores servem.** Os rácios de contraste WCAG 2.2 (§T.3) são
//!   recalculados por `tests/temas.rs` a partir destes valores: trazer um hex
//!   que reprove os limiares falha o teste, em vez de sair no ecrã.
//! * **O que sai no terminal.** O `classico` tem de continuar a emitir as
//!   sequências da v1.0.1 (`38;5;6`, `38;5;1`, `38;5;2`, `38;5;8`); também é
//!   teste (`tests/temas.rs`), porque é uma equivalência que ninguém vê a olho.
//!   Os índices são os que o binário da v1.0.1 escrevia, medidos — não os que o
//!   nome da cor sugere (ver [`classico`]).
//!
//! O rebaixamento de cor é decisão nossa ([`ModoCor`]): o `ratatui`/`crossterm`
//! escrevem sempre `38;2;r;g;b`, independentemente do que o terminal suporta.

use crossterm::style::available_color_count;
use ratatui::style::{Color, Modifier, Style};

// ------------------------------------------------------------------ o contrato

/// Um tema: os nove papéis de cor do ecrã e o fundo (§1 e §T.2 do desenho).
///
/// Os papéis são os da v1.0.1, com o significado de `src/tui/ui.rs`; o que o
/// tema acrescenta é o [`Theme::bg`]. `slug` é identificador de disco (mudá-lo
/// é migração), `nome` é só o que a caixa de temas mostra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Identificador estável — é o que vai para o `config.json` e para
    /// `--theme` (ex.: `"tokyo-night"`).
    pub slug: &'static str,
    /// Nome que a caixa de temas mostra (ex.: `"Tokyo Night"`).
    pub nome: &'static str,
    /// Fundo do ecrã. `Some` = o tema pinta o ecrã todo; `None` = respeita o
    /// fundo do terminal (é o que o `classico` faz, como a v1.0.1).
    ///
    /// Não é decoração: os rácios de §T.3 só valem em qualquer terminal porque
    /// o fundo é nosso. Pintá-lo mata transparência dentro do ecrã do programa
    /// (ADR §Decisão 4) — e é essa a troca aceite.
    pub bg: Option<Color>,
    /// Papel `fg` — texto normal. No `classico` não tem cor: o terminal decide.
    pub fg: Style,
    /// Papel `accent` — nome da app, etiquetas de modo, secções da ajuda.
    pub accent: Style,
    /// Papel `high` — prioridade alta e avisos. A cor nunca é o único sinal:
    /// o `H` faz parte da linha (§1).
    pub high: Style,
    /// Papel `done` — a caixa da concluída.
    pub done: Style,
    /// Papel `rule` — réguas. Limiar 3:1 e não 4,5:1: não é texto (§T.3).
    pub rule: Style,
    /// Papel `err` — o mesmo vermelho do `high` a negrito, como na v1: um
    /// vermelho, dois usos, e o prefixo `Erro:` a distinguir.
    pub err: Style,
    /// Linha selecionada — `REVERSED` e **sem** cor (§3): um só foreground,
    /// para a barra ficar uniforme. Válido com fundo pintado porque o par
    /// invertido mais fraco (o `high` no `storm`) ainda mede 5,51:1 (§T.3).
    pub selec: Style,
    /// Marcador de posição — itálico, **não** cinzento (cinzento reprovaria AA).
    pub placeholder: Style,
    /// Título da concluída — riscado. O contraste do texto não se baixa.
    pub riscado: Style,
}

/// Cor a partir do hex `0xRRGGBB` de §T.2.
///
/// O hex fica literal no código de propósito: é conferível linha a linha com a
/// tabela do desenho, e uma cor a mais aqui vê-se.
const fn hex(rgb: u32) -> Color {
    Color::from_u32(rgb)
}

/// A régua dos três temas Tokyo Night: a chave `dark5`.
///
/// Escolhida pelo `@designer` entre as quatro cinzentas da paleta: as outras
/// três reprovam os 3:1 num ou mais temas (§T.3). Uma só chave a explicar.
const REGUA_TOKYO: u32 = 0x737aa2;

/// Tema Tokyo Night a partir dos valores que o distinguem.
///
/// `night` e `storm` partilham **tudo** menos o fundo (§T.1: `night.lua` é um
/// `deepcopy` de `storm.lua` com três chaves trocadas), por isso os dois saem
/// deste construtor em vez de serem duas paletas a manter em paralelo.
const fn tokyo_night(
    slug: &'static str,
    nome: &'static str,
    bg: u32,
    fg: u32,
    accent: u32,
    high: u32,
    done: u32,
) -> Theme {
    let texto = Style::new().fg(hex(fg));
    Theme {
        slug,
        nome,
        bg: Some(hex(bg)),
        fg: texto,
        accent: Style::new().fg(hex(accent)).add_modifier(Modifier::BOLD),
        high: Style::new().fg(hex(high)),
        done: Style::new().fg(hex(done)),
        rule: Style::new().fg(hex(REGUA_TOKYO)),
        // `err` = `high` + bold: um só vermelho no ecrã (§T.2).
        err: Style::new().fg(hex(high)).add_modifier(Modifier::BOLD),
        selec: Style::new().add_modifier(Modifier::REVERSED),
        placeholder: texto.add_modifier(Modifier::ITALIC),
        riscado: texto.add_modifier(Modifier::CROSSED_OUT),
    }
}

/// O que a v1.0.1 desenha hoje, reproduzido byte a byte (§T.4).
///
/// Os índices são **explícitos** e não as cores nomeadas (`Color::Cyan`) porque
/// as nomeadas são um nome para uma intenção, e um índice é o que sai: é o que
/// torna a paleta reproduzível e mensurável.
///
/// E a correspondência **não** é a que o nome sugere. `IntoCrossterm<CrosstermColor>
/// for Color` (`ratatui-crossterm-0.1.2/src/lib.rs:410-424`) manda `Red` →
/// `DarkRed`, `Green` → `DarkGreen` e `Cyan` → `DarkCyan`; é o `Display` do
/// `Colored` do crossterm (`crossterm-0.29.0/src/style/types/colored.rs:131-146`)
/// que escreve esses nomes como `38;5;1`, `38;5;2` e `38;5;6` — e não
/// `38;5;9`/`38;5;10`/`38;5;14`, que são os índices *bright* que a v1.1 usou
/// primeiro por ler o nome em vez da conversão. Os literais de `tests/temas.rs`
/// são a captura real do binário da v1.0.1 em `tmux` 80×24, não uma segunda
/// leitura destas constantes.
///
/// Sem `bg` e sem `fg` — o `classico` não pinta nada, é o único que respeita o
/// terminal.
const fn classico() -> Theme {
    Theme {
        slug: SLUG_CLASSICO,
        nome: "Clássico (ANSI)",
        bg: None,
        fg: Style::new(),
        accent: Style::new()
            .fg(Color::Indexed(6))
            .add_modifier(Modifier::BOLD),
        high: Style::new().fg(Color::Indexed(1)),
        done: Style::new().fg(Color::Indexed(2)),
        rule: Style::new().fg(Color::Indexed(8)),
        err: Style::new()
            .fg(Color::Indexed(1))
            .add_modifier(Modifier::BOLD),
        selec: Style::new().add_modifier(Modifier::REVERSED),
        placeholder: Style::new().add_modifier(Modifier::ITALIC),
        riscado: Style::new().add_modifier(Modifier::CROSSED_OUT),
    }
}

/// O *slug* do tema que não pinta fundo — a rede de segurança dos terminais
/// sem truecolor (ADR §Decisão 3).
pub const SLUG_CLASSICO: &str = "classico";

/// O *slug* do tema por omissão (ADR §Decisão 1).
pub const SLUG_POR_OMISSAO: &str = "tokyo-night";

/// Os quatro temas, na ordem em que a caixa os apresenta.
///
/// `tokyo-night` **primeiro**: é o [`Theme::default`] e a ordem da lista é a do
/// catálogo (§T.1). Os três Tokyo Night são os únicos que pintam fundo.
pub const CATALOGO: [Theme; 4] = [
    tokyo_night(
        SLUG_POR_OMISSAO,
        "Tokyo Night",
        0x1a1b26,
        0xc0caf5,
        0x7dcfff,
        0xf7768e,
        0x9ece6a,
    ),
    tokyo_night(
        "tokyo-night-storm",
        "Tokyo Night Storm",
        0x24283b,
        0xc0caf5,
        0x7dcfff,
        0xf7768e,
        0x9ece6a,
    ),
    tokyo_night(
        "tokyo-night-moon",
        "Tokyo Night Moon",
        0x222436,
        0xc8d3f5,
        0x86e1fc,
        0xff757f,
        0xc3e88d,
    ),
    classico(),
];

impl Theme {
    /// O tema com este *slug*, ou `None` se o catálogo não o tiver.
    ///
    /// Um slug desconhecido **não** é fatal: quem chama segue para o
    /// [`Theme::default`] e avisa no `stderr` (ADR §Decisão 2).
    #[must_use]
    pub fn por_slug(slug: &str) -> Option<&'static Theme> {
        CATALOGO.iter().find(|tema| tema.slug == slug)
    }

    /// O tema por omissão: o primeiro do catálogo.
    ///
    /// Devolve `&'static Theme` e não um `Theme`: o tema é um dado que vive no
    /// binário e quem o usa guarda a referência em vez de copiar os nove
    /// papéis. Não se implementa `Default` no lugar disto porque
    /// `<Theme as Default>::default()` devolveria uma cópia por valor — as
    /// duas coisas a dizer o mesmo, com uma delas a enganar quem lê.
    #[expect(
        clippy::should_implement_trait,
        reason = "o contrato fixa `Theme::default() -> &'static Theme`; `Default::default()` devolveria `Theme` por valor"
    )]
    #[must_use]
    pub fn default() -> &'static Theme {
        &CATALOGO[0]
    }
}

// ------------------------------------------------------------- modo de cor (§3 do ADR)

/// Como a cor é entregue ao terminal.
///
/// É preciso porque o `ratatui`/`crossterm` **não** rebaixam RGB: um
/// `Color::Rgb` sai sempre como `38;2;r;g;b` e, num terminal de 16 cores, o que
/// aparece é imprevisível. O rebaixamento, se houver, é decisão nossa — e é
/// esta: em [`ModoCor::Ansi`] não se rebaixa nada, troca-se o tema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModoCor {
    /// Ver o que o terminal diz (`COLORTERM`/`TERM`).
    Auto,
    /// 24 bits (`38;2;r;g;b`): os três temas Tokyo Night.
    Rgb,
    /// Só os 16 índices ANSI: o `classico`.
    Ansi,
}

impl ModoCor {
    /// Resolve [`ModoCor::Auto`] pelo que o terminal declara; os modos
    /// explícitos ficam como estão.
    ///
    /// `crossterm::style::available_color_count()` (lê `COLORTERM`/`TERM`, e
    /// não o TTY) devolve `u16::MAX` quando o terminal diz `truecolor`/`24bit`,
    /// `256` quando diz `256`, e `8` por omissão (ADR §Decisão 3). Um terminal
    /// que minta (`tmux` sem `Tc`) cai no `classico` e a saída é `--color rgb`.
    ///
    /// Toma `self` para o arranque poder ser uma cadeia só,
    /// `modo.detectar().resolver(tema)`, em vez de um `if modo == Auto` ao
    /// lado. Não olha para o TTY nem entra em pânico.
    #[must_use]
    pub fn detectar(self) -> Self {
        match self {
            Self::Auto if available_color_count() == u16::MAX => Self::Rgb,
            Self::Auto => Self::Ansi,
            explicito => explicito,
        }
    }

    /// O tema que este modo consegue desenhar.
    ///
    /// Em `ansi`, um tema que pinta fundo **não** é rebaixado às cegas:
    /// devolve-se o `classico` (o único sem `bg`, §T.1) e o pedido fica
    /// guardado para a próxima vez que houver truecolor — o `config.json` não
    /// é reescrito (ADR §Decisão 3). Os temas sem fundo resolvem para si
    /// próprios nos dois modos.
    ///
    /// `Auto` passa por [`ModoCor::detectar`] em vez de valer `rgb` em
    /// silêncio: um `Auto` esquecido tem de dar o mesmo que o arranque daria.
    #[must_use]
    pub fn resolver(self, tema: &'static Theme) -> &'static Theme {
        if self.detectar() == Self::Ansi && tema.bg.is_some() {
            return CATALOGO
                .iter()
                .find(|candidato| candidato.bg.is_none())
                .unwrap_or(tema);
        }
        tema
    }
}
