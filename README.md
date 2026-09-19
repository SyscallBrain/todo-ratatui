# todo-ratatui

Lista de tarefas (TODO) em TUI, escrita em Rust com [ratatui](https://ratatui.rs):
prioridades, descrições, busca, filtro e ordenação, lixo com reposição (o «desfazer» que
sobrevive a um `kill -9`) e import/export CSV e JSON.

É uma **reescrita de raiz do `rtodo`** — a CLI antiga de menus numerados
(`github.com/TiagoRCorreia/rtodo`), hoje arquivada. Não há código partilhado entre os dois
projectos, mas um `db.json` antigo entra por importação — ver
[Trazer dados do `rtodo`](#trazer-dados-do-rtodo).

```
todo-ratatui                            18 tarefas · 15 pendentes · 3 concluídas
────────────────────────────────────────────────────────────────────────────────
filtro: todas · ordem: prioridade                                         criada
▶ [ ] H Rever o PR do dashboard                                             hoje
  [ ] H Backup do vault para o NAS                                          hoje
  [ ] H Marcar consulta no dentista                                         hoje
  [ ] H Ler o capítulo sobre o backend                                      hoje
  [ ] H Configurar o tmux no portátil                                       hoje
  [ ] M Renovar o Cartão de Cidadão                                         hoje
  [ ] M Escrever o post sobre ratatui                                       hoje
  [ ] M Pagar a conta da luz                                                hoje
  [ ] M Testar o TestBackend a 120x32                                       hoje
  [ ] M Actualizar o firmware do router                                     hoje
  [ ] L Comprar café em grão na Nota Roja                                   hoje
  [ ] L Instalar a FiraCode Nerd Font                                       hoje
  [ ] L Rever o ADR do bridge WhatsApp                                      hoje
  [ ] L Arranjar o teclado do portátil                                      hoje
  [ ] L Levar o carro à revisão                                             hoje
  [x] H Exportar as tarefas para CSV                                      ✓ hoje
  [x] M Fechar a issue do parsing de datas                                ✓ hoje
  [x] L Planear a semana                                                  ✓ hoje
────────────────────────────────────────────────────────────────────────────────
Descrição: ver os comentários do reviewer antes de mexer
a nova  e editar  Espaço concluir  d remover  u desfazer  / buscar  ? ajuda
```

*Ecrã real de uma sessão a 80×24* (`tmux`, binário de release, 18 tarefas e três já
concluídas). A 120×32 aparece também o painel da tarefa selecionada, a partir de 96×28:

```
todo-ratatui                                                                    18 tarefas · 15 pendentes · 3 concluídas
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
filtro: todas · ordem: prioridade                                                                                 criada
▶ [ ] H Rever o PR do dashboard                                                                                     hoje
  [ ] H Backup do vault para o NAS                                                                                  hoje
  [ ] H Marcar consulta no dentista                                                                                 hoje
  [ ] H Ler o capítulo sobre o backend                                                                              hoje
  [ ] H Configurar o tmux no portátil                                                                               hoje
  [ ] M Renovar o Cartão de Cidadão                                                                                 hoje
  [ ] M Escrever o post sobre ratatui                                                                               hoje
  [ ] M Pagar a conta da luz                                                                                        hoje
  [ ] M Testar o TestBackend a 120x32                                                                               hoje
  [ ] M Actualizar o firmware do router                                                                             hoje
  [ ] L Comprar café em grão na Nota Roja                                                                           hoje
  [ ] L Instalar a FiraCode Nerd Font                                                                               hoje
  [ ] L Rever o ADR do bridge WhatsApp                                                                              hoje
  [ ] L Arranjar o teclado do portátil                                                                              hoje
  [ ] L Levar o carro à revisão                                                                                     hoje
  [x] H Exportar as tarefas para CSV                                                                              ✓ hoje
  [x] M Fechar a issue do parsing de datas                                                                        ✓ hoje
  [x] L Planear a semana                                                                                          ✓ hoje



─ selecionada ──────────────────────────────────────────────────────────────────────────────────────────────────────────
  Título      Rever o PR do dashboard
  Descrição   ver os comentários do reviewer antes de mexer
  Prioridade  Alta (H)    ·    criada 2026-09-18 20:10    ·    concluída —
  Prazo       —    ·    id ded86370-3cf3-4fa8-b2e0-26cb9482aa37
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

a nova  e editar  Espaço concluir  d remover  u desfazer  / buscar  ? ajuda
```

## Instalação

Precisa de **Rust 1.88** ou mais recente (`edition = 2024`; o MSRV é o máximo dos MSRV das
dependências, medido com `cargo tree`).

```sh
cargo install --path .
```

Fica o binário `todo_ratatui` em `~/.cargo/bin/todo_ratatui` — o nome do binário é o do
pacote (`name = "todo_ratatui"`); o repositório é que se chama `todo-ratatui`. Para
experimentar sem instalar:

```sh
cargo build --release
./target/release/todo_ratatui
```

`todo_ratatui --help` mostra as opções; `--db <caminho>` muda a base de dados (ver
[Dados](#dados)).

## Teclas

### Lista (modo normal)

| Tecla | Acção |
| --- | --- |
| `q` · `Ctrl+C` | sair |
| `j` · `↓` | linha seguinte |
| `k` · `↑` | linha anterior |
| `g` · `G` | primeira · última linha |
| `Espaço` · `Enter` | concluir ou reabrir a selecionada |
| `a` | nova tarefa |
| `e` | editar o título da selecionada |
| `Ctrl+E` | editar a descrição da selecionada |
| `d` | mandar a selecionada para o lixo |
| `u` | repor o último lote que ainda está no lixo |
| `1` `2` `3` | prioridade Alta · Média · Baixa |
| `s` | próxima ordem: prioridade → estado → prazo → mais antigas → mais recentes → prioridade |
| `f` | próximo filtro: todas → pendentes → concluídas |
| `/` | buscar (título e descrição; `Enter` confirma, `Esc` desiste) |
| `t` | concluir todas — ou reabrir todas, se já estiverem concluídas |
| `c` | limpar concluídas (vão para o lixo, num só lote) |
| `i` | importar de um ficheiro |
| `x` | exportar para um ficheiro |
| `L` | entrar ou sair da vista do lixo |
| `?` | ajuda |
| `Esc` | tirar a mensagem; sem mensagem, limpar busca e filtro |

### Vista do lixo (`L`)

| Tecla | Acção |
| --- | --- |
| `j` · `↓` · `k` · `↑` · `g` · `G` | navegar |
| `Enter` · `Espaço` | restaurar a selecionada **na posição original** |
| `c` | esvaziar o lixo (pede duas pressões) |
| `Esc` | voltar à lista |
| `?` | ajuda |
| `Ctrl+C` | sair |

Nesta vista `a`, `e`, `d`, `u`, `1` `2` `3`, `t`, `s`, `f`, `i`, `x` e `q` não fazem nada
(é um mapa próprio): `u`, que na lista repõe o último lote, aqui teria dois sentidos, e um
`c` com dois sentidos esvaziaria o lixo sem guarda.

### Linha de texto (nova tarefa, editar, buscar, importar, exportar)

| Tecla | Acção |
| --- | --- |
| `Enter` | confirmar |
| `Esc` | cancelar |
| `Backspace` · `Delete` · `←` · `→` · `Home` · `End` | editar |
| `Ctrl+U` | limpar a linha |
| `Ctrl+C` | sair |
| qualquer outra tecla | é texto (incluindo `q`, `j`, `d` e `?`) |

### Ajuda (`?`)

`?`, `Esc` ou `q` fecham; o resto é ignorado.

Três notas que evitam surpresas:

- `Esc` em repouso **não sai** da aplicação: tira a mensagem da linha 22 e, quando não há
  mensagem, limpa a busca e o filtro. Sair é `q` ou `Ctrl+C` — depois de um `d`, o reflexo
  de cancelar não pode fechar o programa.
- Na vista do lixo o `q` também não sai: sai-se com `Esc` e depois `q`, ou com `Ctrl+C`,
  que funciona em qualquer modo.
- As mensagens da linha 22 **não expiram sozinhas**: ficam até outra acção as substituir ou
  até `Esc`.

## Dados

### Onde vive o `db.json`

Por omissão em `~/.local/share/todo-ratatui/db.json` (`$XDG_DATA_HOME/todo-ratatui/db.json`
quando a variável está definida). São dados, não configuração: `~/.config` costuma estar em
dotfiles e em git, e as tarefas não devem ir no próximo commit. A precedência é:

1. `--db <caminho>`
2. a variável de ambiente `TODO_RATATUI_DB`
3. `~/.local/share/todo-ratatui/db.json`

```sh
todo_ratatui --db /tmp/experiencia.json
TODO_RATATUI_DB=/tmp/experiencia.json todo_ratatui
```

Nos caminhos escritos dentro da aplicação (importar, exportar) o `~` **não é expandido**:
escreve o caminho completo.

### Os ficheiros ao lado

- **`db.json`** — a base toda, num envelope JSON (ver [o formato](#formato-em-disco)).
  Cada gravação é atómica: `db.json.tmp` no mesmo directório, `fsync`, `rename`. Ou fica a
  geração nova, ou fica a antiga — nunca meia.
- **`db.json.bak`** — a **geração anterior**, reescrita a cada gravação bem-sucedida. É a
  rede de segurança contra corrupção ou contra um ficheiro que deixou de se ler: copia-o
  por cima do `db.json` e volta a abrir. (A biblioteca tem `Store::restore_backup()`, que
  copia o `db.json` actual para `db.json.pre-restore` antes de repor o `.bak` — **na v1 não
  há tecla para isso**; chama-se a partir de código.)
- **`db.json.pre-restore`** — só existe depois de um restauro desses.
- **`db.json.tmp`** — só existe durante uma gravação.

Se uma gravação falhar (disco cheio, directório sem permissão de escrita), a linha 22 diz
`Erro: não gravou <caminho> — lista intacta`: nada foi escrito, o que está em memória
mantém-se e a aplicação continua a responder.

Se o ficheiro **não se conseguir abrir** — ilegível, JSON inválido, `schema` desconhecido
ou um array legado — o programa **recusa arrancar**: escreve o erro e o caminho do `.bak`
no `stderr` e sai com código diferente de zero, antes de desenhar a TUI. É de propósito:
uma lista vazia em memória seria, na gravação seguinte, uma sobrescrita do ficheiro que
ainda podia ser recuperado.

### Lixo: o «desfazer» que sobrevive ao processo

`d` (remover) e `c` (limpar concluídas) **não apagam**: movem as tarefas para o `trash` do
próprio `db.json`, com o índice que ocupavam e a data. O undo é dado, não memória: sobrevive
a um `kill -9`. `u` repõe o **último lote que ainda está no lixo**, na posição original (um
`c` repõe as suas concluídas todas de uma vez; cada `u` seguinte anda um lote para trás).

O lixo guarda no máximo **100** entradas. Quando passa disso, as mais antigas saem e a linha
22 avisa quantas saíram — nada sai em silêncio:

```
Aviso: 2 entradas antigas saíram do lixo (limite 100)  ·  L ver o lixo
```

`L` abre a vista do lixo (`lixo: n de 100`, e `lixo: 100 (cheio)` quando está cheio), com a
mesma navegação, `Enter` para repor a selecionada e `c` para esvaziar. Esvaziar é a única
operação **sem undo**, e por isso a única que pede duas pressões:

```
Esvaziar o lixo? 3 entradas, sem volta atrás  ·  c outra vez confirma
```

A guarda desarma em 5 segundos ou com qualquer outra tecla.

### Import e export

`x` exporta e `i` importa; nos dois casos escreve-se o caminho na linha 22.

**Exportar (`x`)** — CSV com cabeçalho, uma linha por tarefa **da lista** (o lixo não sai no
CSV), criando os directórios do caminho se for preciso:

```
id,title,description,done,priority,created_at,completed_at,due_at
```

**Importar (`i`)** — o que é lido depende da extensão do ficheiro:

- `.csv` → CSV com o cabeçalho acima. Um CSV sem cabeçalho é **recusado**, com uma mensagem
  que diz o que faltava: sem essa comparação a primeira linha seria comida como cabeçalho e
  a primeira tarefa desaparecia em silêncio (era um defeito do `rtodo` antigo).
- qualquer outra extensão → JSON: o envelope novo (`{schema, todos, trash}`, que é o próprio
  `db.json` — traz também o lixo) ou um array, lido registo a registo pelo formato de cada um
  (ver abaixo).

O import **nunca duplica**: registos cujo `id` já exista (na base ou no ficheiro) são
ignorados, e a linha 22 diz o que entrou:

```
Importado: 2 lidos, 2 inseridos, 0 duplicados ignorados, 1 sem data legível
Importação sem alterações: 3 lidos, 0 inseridos, 3 duplicados ignorados
```

Um import que falhe a meio não altera nada, e o erro nomeia o ficheiro e a linha.

### Trazer dados do `rtodo`

A CLI antiga escrevia `~/.config/rtodo/db.json`, um array de
`{title, description, done, time, date}` sem envelope nenhum.

1. abre a aplicação (`todo_ratatui`);
2. carrega em `i`, escreve o **caminho completo** do ficheiro antigo (por exemplo
   `/home/tiago/.config/rtodo/db.json` — o `~` não é expandido) e confirma com `Enter`;
3. a linha 22 diz o que entrou, por exemplo
   `Importado: 2 lidos, 2 inseridos, 0 duplicados ignorados, 1 sem data legível`.

O que a conversão faz:

| No `rtodo` | Aqui |
| --- | --- |
| `title` | título (uma linha sem título é recusada, com o número da linha) |
| `description` | descrição |
| `done` | concluída |
| `time` | prioridade — `High`/`Medium`/`Low`, aceitando também `Alta`/`Média`/`Baixa` e sem distinguir maiúsculas; nulo ou vazio dá Média |
| `date` | data de criação (`2023-01-05` ou RFC3339); vazio dá agora, contado como «sem data legível» |
| — | `id` novo: o `rtodo` não tinha `id` |
| — | `completed_at` fica vazio: a data real da conclusão não existe no ficheiro antigo e inventá-la seria pior do que a ausência |

Dois avisos:

- O ficheiro antigo **não se abre como base de dados**: `--db` a apontar para ele faz o
  programa recusar arrancar («é um array JSON sem envelope: parece o formato legado do
  rtodo; converte-o com o import em vez de o abrires como base de dados»). É para importar,
  não para abrir — e recusa precisamente para não lhe escrever por cima. O ficheiro antigo
  fica como está.
- Um array JSON é lido **registo a registo**, pelo formato de cada um: um registo com
  algum dos campos que só o formato novo tem (`id`, `priority`, `created_at`,
  `completed_at`, `due_at`) é lido como tarefa, com a mesma validade do envelope — sem
  `id`, ou com uma `description` nula, dá erro com o número da linha, e o `id` que lá
  estiver é preservado (importar duas vezes o mesmo array não duplica nada); sem nenhum
  desses campos é lido como registo do `rtodo`, como na tabela acima. Os dois formatos
  podem conviver no mesmo ficheiro. Para levar a base de uma máquina para outra serve o
  envelope (`db.json`) ou um array de tarefas no formato novo, por exemplo
  `jq '.todos' db.json > arr.json` — que também é JSON normal e legível por `jq`.

O CSV do `rtodo` antigo fica de fora: esta versão importa o CSV que ela própria exporta.

### Formato em disco

`db.json` real, de uma sessão com uma tarefa e outra no lixo:

```json
{
  "schema": 2,
  "todos": [
    {
      "id": "556c7153-c608-4c45-a1d6-858deb9003a5",
      "title": "Comprar café em grão na Nota Roja",
      "description": "",
      "done": false,
      "priority": "medium",
      "created_at": "2026-09-18T20:11:21.527979419+01:00",
      "completed_at": null,
      "due_at": null
    }
  ],
  "trash": [
    {
      "todo": {
        "id": "81029ed2-4353-408b-910c-06e081a2c8ad",
        "title": "Tarefa que vai para o lixo",
        "description": "",
        "done": false,
        "priority": "medium",
        "created_at": "2026-09-18T20:11:21.841912294+01:00",
        "completed_at": null,
        "due_at": null
      },
      "index": 1,
      "deleted_at": "2026-09-18T20:11:22.151882033+01:00",
      "batch": 1
    }
  ],
  "trash_dropped": 0
}
```

## Desenvolvimento

- `src/core/` — modelo, persistência, operações e import/export. **Não depende de
  `ratatui`/`crossterm`**, o que o torna testável sem terminal; a invariante é travada por
  `tests/boundary.rs`.
- `src/tui/` — estado (`app.rs`), teclas (`event.rs`) e desenho (`ui.rs`).
- `src/main.rs` — terminal e arranque.

```sh
cargo test                                  # 120 testes, incluindo os goldens
cargo clippy --all-targets -- -D warnings
```

Os 13 ficheiros de `tests/frames/` são *golden files*: o ecrã desenhado é comparado linha a
linha com eles, a 80×24 e a 120×32.

## Licença

MIT, com os dois avisos de copyright — o do `rtodo` original e o desta obra. Ver
[LICENSE](LICENSE); o que mudou em cada versão está no [CHANGELOG.md](CHANGELOG.md).

## Âmbito da v1

- `due_at` (prazo) existe no formato em disco e aparece no painel de detalhe quando vem de
  um import, mas **não é editável na interface**.
- Fora: descrição multi-linha, tags, várias listas, rato, temas configuráveis, notificações.
- Não há publicação no crates.io (`publish = false` no `Cargo.toml`).
