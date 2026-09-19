//! Provas do catálogo de temas (`tui::theme`).
//!
//! A cor é a única parte do ecrã que não se vê num `golden file` de texto: os
//! 14 frames de `tests/render.rs` provam o desenho e não provam nada sobre a
//! paleta. Este ficheiro prova-a, e prova três coisas diferentes:
//!
//! 1. **Que os valores são os medidos.** O contraste WCAG 2.2 é recalculado
//!    aqui a partir dos valores do tema, com a fórmula escrita neste ficheiro —
//!    não com uma tabela copiada da §T.3 do desenho. Se alguém trocar um hex
//!    por outro que reprove o limiar, o teste falha; [`paleta_e_a_da_secao_t2`]
//!    fixa os hex um a um, para o contraste não ser a única barreira.
//! 2. **Que o `classico` continua a ser a v1.0.1.** A extracção da paleta para
//!    um módulo não pode mudar um byte do que sai no terminal: as sequências
//!    são comparadas com as que a v1.0.1 emite (`38;5;6`, `38;5;1`, `38;5;2`,
//!    `38;5;8`), via `Display` do `Colored` — o mesmo caminho que o crossterm
//!    usa para escrever. Os literais são uma **captura do binário da v1.0.1**
//!    em `tmux` 80×24, e não uma segunda leitura de `src/tui/theme.rs`: é essa
//!    diferença que prende a equivalência ([`classico_emite_as_sequencias_da_v1`]).
//! 3. **Que o rebaixamento não existe.** Em modo `ansi` um tema com fundo
//!    resolve para o `classico`; não há RGB reduzido a aproximações.

use crossterm::style::{Color as CorDoTerminal, Colored};
use ratatui::style::{Color, Modifier, Style};

use todo_ratatui::tui::theme::{CATALOGO, ModoCor, SLUG_CLASSICO, SLUG_POR_OMISSAO, Theme};

// ------------------------------------------------------------- a régua do teste

/// Limiar da WCAG 2.2 para texto normal (§T.3).
const AA_TEXTO: f64 = 4.5;
/// Limiar para componente/gráfico: uma régua não é texto (§T.3).
const AA_COMPONENTE: f64 = 3.0;

/// Fundo de referência do tema que não pinta fundo.
///
/// O `classico` não tem `bg` — é isso que o deixa respeitar o terminal — mas o
/// contraste tem de ser medido contra alguma coisa: a §T.3 mede-o contra o
/// `background` da alacritty onde ele foi desenhado (`#1A1B26`).
const FUNDO_DE_REFERENCIA: u32 = 0x1a1b26;

/// O que se vê no papel `fg` do `classico`, que não tem cor própria: o
/// `foreground` do terminal (§T.2, `#a9b1d6`).
const FG_DO_TERMINAL: u32 = 0xa9b1d6;

/// A paleta ANSI da alacritty do Tiago **nos índices que o `classico` usa**
/// (§T.2 e §T.4), a mesma que `audits/temas-contraste.py` mediu: 1 = o
/// `high`/`err`, 2 = o `done`, 6 = o `accent`, 8 = a régua.
///
/// Um índice não é uma cor: é um nome que o terminal resolve. Esta tabela é a
/// resolução **daquela** alacritty — noutro terminal os mesmos índices podem
/// medir outro rácio, e é esse o preço (declarado em §T.3) de o `classico` não
/// pintar nada. Os três Tokyo Night não passam por aqui: declaram RGB.
const PALETA_ANSI: [(u8, u32); 4] = [
    (1, 0xff5555),
    (2, 0x50fa7b),
    (6, 0x8be9fd),
    (8, 0x6272a4),
];

fn canais(hex: u32) -> (u8, u8, u8) {
    (
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

/// O `Color` do ratatui correspondente a um hex `0xRRGGBB`.
fn rgb(hex: u32) -> Color {
    let (r, g, b) = canais(hex);
    Color::Rgb(r, g, b)
}

/// Os canais sRGB de uma cor **com valor**. Um índice ANSI vale o que a
/// [`PALETA_ANSI`] diz — e um índice fora dela é um erro, não uma cor
/// aproximada.
fn canais_de(cor: Color) -> (u8, u8, u8) {
    match cor {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(indice) => {
            let (_, hex) = PALETA_ANSI
                .iter()
                .find(|(i, _)| *i == indice)
                .unwrap_or_else(|| panic!("índice {indice} não está na paleta medida de §T.3"));
            canais(*hex)
        }
        outra => panic!("cor sem valor sRGB conhecido: {outra:?}"),
    }
}

/// Os canais sRGB de um papel. `None` é o papel **sem cor** — o caso do
/// `classico` — e vale o que o terminal desenha ([`FG_DO_TERMINAL`]).
fn srgb(papel: Option<Color>) -> (u8, u8, u8) {
    papel.map_or(canais(FG_DO_TERMINAL), canais_de)
}

/// O fundo contra o qual um tema é medido: o `bg` do próprio tema, ou o fundo
/// de referência quando o tema não pinta nenhum (`classico`, §T.3).
fn fundo_de(tema: &Theme) -> (u8, u8, u8) {
    tema.bg.map_or(canais(FUNDO_DE_REFERENCIA), canais_de)
}

/// Luminância relativa sRGB (WCAG 2.2), escrita aqui para os números não
/// virem copiados da spec: canal ≤ 0,03928 → `c/12,92`; senão
/// `((c+0,055)/1,055)^2,4`; `L = 0,2126·R + 0,7152·G + 0,0722·B`.
fn luminancia((r, g, b): (u8, u8, u8)) -> f64 {
    let canal = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * canal(r) + 0.7152 * canal(g) + 0.0722 * canal(b)
}

/// Rácio de contraste: `(L_claro + 0,05) / (L_escuro + 0,05)`, em qualquer
/// ordem — o claro é o que tem maior luminância.
fn razao(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (la, lb) = (luminancia(a), luminancia(b));
    let (claro, escuro) = if la >= lb { (la, lb) } else { (lb, la) };
    (claro + 0.05) / (escuro + 0.05)
}

/// A cor do ratatui na cor do crossterm — a ponte que permite ver a sequência
/// que sai mesmo, através do `Display` do [`Colored`].
fn cor_do_terminal(cor: Color) -> CorDoTerminal {
    match cor {
        Color::Rgb(r, g, b) => CorDoTerminal::Rgb { r, g, b },
        Color::Indexed(indice) => CorDoTerminal::AnsiValue(indice),
        outra => panic!("sem sequência ANSI equivalente para {outra:?}"),
    }
}

// ------------------------------------------------------------------ 1. o catálogo

/// Um teste a percorrer o catálogo: nenhum tema é inalcançável pelo nome e
/// nenhum slug se repete (dois temas com o mesmo slug seriam um deles
/// inalcançável, e o disco guardaria a mesma preferência para os dois).
#[test]
fn catalogo_tem_slugs_unicos_e_alcancaveis_pelo_nome() {
    assert_eq!(CATALOGO.len(), 4, "o catálogo de §T.1 tem quatro temas");
    assert_eq!(
        Theme::default().slug,
        SLUG_POR_OMISSAO,
        "o primeiro do catálogo é o tema por omissão"
    );
    assert_eq!(
        Theme::default(),
        &CATALOGO[0],
        "`Theme::default()` é o primeiro do catálogo"
    );

    let mut vistos: Vec<&str> = Vec::new();
    for tema in &CATALOGO {
        assert!(!tema.slug.is_empty(), "tema sem slug: {tema:?}");
        assert!(!tema.nome.is_empty(), "tema sem nome para a caixa: {tema:?}");
        assert!(
            !vistos.contains(&tema.slug),
            "slug repetido no catálogo: {}",
            tema.slug
        );
        vistos.push(tema.slug);

        let achado = Theme::por_slug(tema.slug).unwrap_or_else(|| {
            panic!("`{}` está no catálogo mas não se acha pelo nome", tema.slug)
        });
        assert_eq!(achado, tema, "`por_slug` devolveu outro tema");
    }
    assert_eq!(vistos.len(), CATALOGO.len());

    assert_eq!(Theme::por_slug("nao-existe"), None);
    assert_eq!(Theme::por_slug(""), None);
    assert_eq!(
        Theme::por_slug("Tokyo Night"),
        None,
        "o nome da caixa não é o slug do disco"
    );
}

// -------------------------------------------------------- 2. os valores medidos

/// O contraste de §T.3, **recalculado**: cada papel de texto contra o fundo do
/// próprio tema ≥ 4,5:1, a régua ≥ 3:1, e o papel `fg` sobre o fundo ≥ 4,5:1
/// (é o par que a barra `REVERSED` inverte, §3 do desenho).
///
/// `cargo test --test temas -- --nocapture` imprime a tabela calculada.
#[test]
fn contraste_de_cada_papel_contra_o_fundo_do_proprio_tema() {
    for tema in &CATALOGO {
        let fundo = fundo_de(tema);
        let papeis_de_texto = [
            ("fg", tema.fg),
            ("accent", tema.accent),
            ("high", tema.high),
            ("done", tema.done),
            ("err", tema.err),
        ];
        for (nome, estilo) in papeis_de_texto {
            let medido = razao(srgb(estilo.fg), fundo);
            println!("{:<18} {nome:<7} {medido:>6.2}:1", tema.slug);
            assert!(
                medido >= AA_TEXTO,
                "`{nome}` de `{}` mede {medido:.2}:1 (texto exige {AA_TEXTO}:1)",
                tema.slug
            );
        }

        let regua = razao(srgb(tema.rule.fg), fundo);
        println!("{:<18} {:<7} {regua:>6.2}:1", tema.slug, "rule");
        assert!(
            regua >= AA_COMPONENTE,
            "a régua de `{}` mede {regua:.2}:1 (componente exige {AA_COMPONENTE}:1)",
            tema.slug
        );

        // A linha selecionada leva **só** o papel `fg` (§3): com a barra
        // invertida, é este par que se lê.
        let barra = razao(srgb(tema.fg.fg), fundo);
        assert!(
            barra >= AA_TEXTO,
            "a barra `REVERSED` de `{}` mede {barra:.2}:1 (texto exige {AA_TEXTO}:1)",
            tema.slug
        );
    }
}

/// Os hex de §T.2, um a um.
///
/// Os rácios do teste acima deixam passar qualquer hex claro; o que se quer
/// travar aqui é que a paleta do catálogo é a que o `@designer` mediu e citou,
/// e não uma cor parecida escolhida no sítio. A régua é a chave `dark5`
/// (`#737aa2`) nos **três** Tokyo Night — uma só chave a explicar (§T.3).
#[test]
fn paleta_e_a_da_secao_t2() {
    let tokyo_night = [
        (
            SLUG_POR_OMISSAO,
            0x1a1b26,
            0xc0caf5,
            0x7dcfff,
            0xf7768e,
            0x9ece6a,
        ),
        (
            "tokyo-night-storm",
            0x24283b,
            0xc0caf5,
            0x7dcfff,
            0xf7768e,
            0x9ece6a,
        ),
        (
            "tokyo-night-moon",
            0x222436,
            0xc8d3f5,
            0x86e1fc,
            0xff757f,
            0xc3e88d,
        ),
    ];

    for (slug, bg, fg, accent, high, done) in tokyo_night {
        let tema = Theme::por_slug(slug).expect("tema do catálogo");
        assert_eq!(tema.bg, Some(rgb(bg)), "{slug}: fundo");
        assert_eq!(tema.fg.fg, Some(rgb(fg)), "{slug}: fg");
        assert_eq!(tema.accent.fg, Some(rgb(accent)), "{slug}: accent");
        assert_eq!(tema.high.fg, Some(rgb(high)), "{slug}: high");
        assert_eq!(tema.done.fg, Some(rgb(done)), "{slug}: done");
        assert_eq!(tema.rule.fg, Some(rgb(0x737aa2)), "{slug}: rule = dark5");
        assert_eq!(tema.accent.add_modifier, Modifier::BOLD, "{slug}: accent");
        assert_eq!(tema.err.fg, tema.high.fg, "{slug}: err é o high");
        assert_eq!(
            tema.err.add_modifier,
            Modifier::BOLD,
            "{slug}: err é o high a negrito (§T.2)"
        );
        assert_eq!(
            tema.placeholder.fg, tema.fg.fg,
            "{slug}: o marcador de posição não baixa o contraste (§1)"
        );
        assert_eq!(tema.placeholder.add_modifier, Modifier::ITALIC);
        assert_eq!(tema.riscado.fg, tema.fg.fg);
        assert_eq!(tema.riscado.add_modifier, Modifier::CROSSED_OUT);
        assert_eq!(tema.selec, Style::new().add_modifier(Modifier::REVERSED));
    }

    // O `classico` declara índices, não RGB — é o que faz a paleta ficar
    // reproduzível (§T.4) —, não pinta fundo e não força `fg`. Os índices são os
    // medidos no binário da v1.0.1 (`38;5;6`/`38;5;1`/`38;5;2`/`38;5;8`), não os
    // que o nome das cores da v1.0.1 sugere: quem reescrever isto a partir do
    // `theme.rs` (ou de `Color::Red` → `38;5;9`) falha o
    // `classico_emite_as_sequencias_da_v1` abaixo.
    let classico = Theme::por_slug(SLUG_CLASSICO).expect("tema do catálogo");
    assert_eq!(classico.bg, None);
    assert_eq!(classico.fg, Style::new());
    assert_eq!(classico.accent.fg, Some(Color::Indexed(6)));
    assert_eq!(classico.high.fg, Some(Color::Indexed(1)));
    assert_eq!(classico.done.fg, Some(Color::Indexed(2)));
    assert_eq!(classico.rule.fg, Some(Color::Indexed(8)));
    assert_eq!(classico.err.fg, Some(Color::Indexed(1)));
    assert_eq!(classico.err.add_modifier, Modifier::BOLD);
    assert_eq!(classico.placeholder, Style::new().add_modifier(Modifier::ITALIC));
    assert_eq!(classico.riscado, Style::new().add_modifier(Modifier::CROSSED_OUT));
    assert_eq!(classico.selec, Style::new().add_modifier(Modifier::REVERSED));

    // Um só tema sem fundo: é ele que o modo `ansi` usa (`ModoCor::resolver`),
    // e a invariante que faz essa escolha ser determinística.
    let sem_fundo: Vec<&str> = CATALOGO
        .iter()
        .filter(|tema| tema.bg.is_none())
        .map(|tema| tema.slug)
        .collect();
    assert_eq!(sem_fundo, [SLUG_CLASSICO]);
}

// --------------------------------------- 3. equivalência com a v1.0.1 (§T.4)

/// O `classico` emite as **mesmas sequências** que a v1.0.1 emitia.
///
/// A v1.0.1 escrevia `Color::Red`/`Color::Green`/`Color::Cyan`/`AnsiValue(8)`;
/// o `classico` escreve `Indexed(1)`/`Indexed(2)`/`Indexed(6)`/`Indexed(8)`.
/// São o mesmo byte no terminal — e é o que este teste compara, passando pelo
/// mesmo `Display` que o backend usa para escrever.
///
/// **Os literais não se derivam do tema.** São a captura real do binário da tag
/// `v1.0.1` (worktree em `~/.cache/review/v1.0.1` sobre `5a4667a`, o mesmo
/// `db.json`, `tmux` 80×24), papel a papel:
///
/// | papel  | nome na v1.0.1  | sequência medida | o que a v1.1 declara |
/// | ------ | --------------- | ---------------- | -------------------- |
/// | accent | `Color::Cyan`   | `38;5;6`         | `Indexed(6)`         |
/// | high   | `Color::Red`    | `38;5;1`         | `Indexed(1)`         |
/// | done   | `Color::Green`  | `38;5;2`         | `Indexed(2)`         |
/// | rule   | `AnsiValue(8)`  | `38;5;8`         | `Indexed(8)`         |
/// | err    | `Color::Red`    | `38;5;1` + bold  | `Indexed(1)` + bold  |
///
/// A conversão do `ratatui` para o `crossterm`
/// (`ratatui-crossterm-0.1.2/src/lib.rs:410-424`: `Red`→`DarkRed`,
/// `Green`→`DarkGreen`, `Cyan`→`DarkCyan`) é o que faz `Color::Red` sair como
/// `38;5;1` e não `38;5;9`. A v1.1 começou com os índices *bright* por ler o
/// nome em vez da conversão, e este teste não os apanhou precisamente por
/// re-derivar o esperado do próprio tema: comparar `Indexed(14)` com o literal
/// `"38;5;14"` é uma tautologia — passa com o valor certo e com o errado. Daqui
/// para a frente o valor certo é o da tabela acima: repor `Indexed(14)` (ou
/// `(9)`, ou `(10)`) no tema tem de fazer este teste falhar.
#[test]
fn classico_emite_as_sequencias_da_v1() {
    // A cor tem de ser forçada: com `NO_COLOR` definido no ambiente, o
    // `Display` do `Colored` escreve **nada** e o teste comparava `""` com
    // `38;5;6` — falhava por causa do ambiente e não do tema (verificado:
    // trocar este `true` por `false` e correr com `NO_COLOR=1` dá
    // `left: ""`). O crossterm lê a variável uma só vez (memoiza no primeiro
    // acesso), logo forçar aqui vale para o resto deste processo de teste.
    crossterm::style::force_color_output(true);

    let classico = Theme::por_slug(SLUG_CLASSICO).expect("tema do catálogo");
    let papeis = [
        ("accent", classico.accent, "38;5;6"),
        ("high", classico.high, "38;5;1"),
        ("done", classico.done, "38;5;2"),
        ("rule", classico.rule, "38;5;8"),
        ("err", classico.err, "38;5;1"),
    ];
    for (nome, estilo, esperado) in papeis {
        let cor = estilo
            .fg
            .unwrap_or_else(|| panic!("o papel `{nome}` do `classico` tem cor"));
        assert_eq!(
            Colored::ForegroundColor(cor_do_terminal(cor)).to_string(),
            esperado,
            "o papel `{nome}` do `classico` tem de sair como na v1.0.1"
        );
    }

    // `fg`, `placeholder` e `riscado` não pintam **nada** (é o que deixa o
    // terminal decidir); o que os distingue são os modificadores.
    assert_eq!(classico.fg.fg, None);
    assert_eq!(classico.placeholder.fg, None);
    assert_eq!(classico.riscado.fg, None);
    assert!(
        classico
            .placeholder
            .add_modifier
            .contains(Modifier::ITALIC)
    );
    assert!(
        classico
            .riscado
            .add_modifier
            .contains(Modifier::CROSSED_OUT)
    );

    // O outro caminho: os temas com fundo emitem 24 bits, o que o terminal sem
    // truecolor não sabe desenhar — e é por isso que `ModoCor` existe.
    let tema_por_omissao = Theme::default();
    let fg = tema_por_omissao.fg.fg.expect("o tema por omissão tem `fg`");
    assert_eq!(
        Colored::ForegroundColor(cor_do_terminal(fg)).to_string(),
        "38;2;192;202;245",
        "o `fg` do `tokyo-night` é `#c0caf5` em 24 bits"
    );
}

// ------------------------------------------------------- 4. o modo de cor (§3)

/// `ModoCor`: detecção sem TTY, e rebaixamento que **não** é rebaixamento —
/// em `ansi` um tema que pinta fundo dá o `classico` (e o pedido fica
/// guardado, ADR §Decisão 3).
#[test]
fn modo_de_cor_nao_rebaixa_tema_com_fundo() {
    let tokyo = Theme::por_slug(SLUG_POR_OMISSAO).expect("tema do catálogo");
    let classico = Theme::por_slug(SLUG_CLASSICO).expect("tema do catálogo");

    // `Auto` decide-se por variáveis de ambiente, não pelo TTY: aqui não há
    // terminal nenhum e isto não entra em pânico.
    assert!(matches!(
        ModoCor::Auto.detectar(),
        ModoCor::Rgb | ModoCor::Ansi
    ));
    assert_eq!(ModoCor::Rgb.detectar(), ModoCor::Rgb, "explícito não se mexe");
    assert_eq!(ModoCor::Ansi.detectar(), ModoCor::Ansi);

    assert_eq!(ModoCor::Ansi.resolver(tokyo).slug, SLUG_CLASSICO);
    assert_eq!(ModoCor::Rgb.resolver(tokyo).slug, SLUG_POR_OMISSAO);
    assert_eq!(ModoCor::Ansi.resolver(classico).slug, SLUG_CLASSICO);
    assert_eq!(ModoCor::Rgb.resolver(classico).slug, SLUG_CLASSICO);

    for tema in &CATALOGO {
        let esperado = if tema.bg.is_some() {
            SLUG_CLASSICO
        } else {
            tema.slug
        };
        assert_eq!(
            ModoCor::Ansi.resolver(tema).slug,
            esperado,
            "em `ansi`, `{}` tem de dar o tema que não pinta fundo",
            tema.slug
        );
        assert_eq!(
            ModoCor::Rgb.resolver(tema).slug,
            tema.slug,
            "em `rgb` o tema pedido é o tema desenhado"
        );
    }
}
