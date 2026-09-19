# todo-ratatui

Lista de tarefas (TODO) em TUI, escrita em Rust com [ratatui](https://ratatui.rs):
categorias, prioridades, descrições, busca, filtro e ordenação, lixo com reposição (o
«desfazer» que sobrevive a um `kill -9`) e import/export CSV e JSON.

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
[Dados](#dados)) e `--theme <slug>` escolhe o tema do ecrã (ver [Temas](#temas)).

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
| `s` | próxima ordem: prioridade → estado → prazo → categoria → mais antigas → mais recentes → prioridade |
| `f` | próximo filtro: todas → pendentes → concluídas |
| `F` | próximo filtro de categoria: todas → sem categoria → uma categoria → todas |
| `/` | buscar (título e descrição; `Enter` confirma, `Esc` desiste) |
| `t` | concluir todas — ou reabrir todas, se já estiverem concluídas |
| `c` | limpar concluídas (vão para o lixo, num só lote) |
| `i` | importar de um ficheiro |
| `x` | exportar para um ficheiro |
| `L` | entrar ou sair da vista do lixo |
| `T` | escolher o tema do ecrã (ver [Temas](#temas)) |
| `C` | caixa de categorias: atribuir, criar, renomear e eliminar (ver [Caixa de categorias](#caixa-de-categorias-c)) |
| `?` | ajuda |
| `Esc` | tirar a mensagem; sem mensagem, limpar busca, filtro e categoria |

Uma nota sobre os dois eixos que a linha 2 mostra: `f` governa o **estado** (todas, pendentes,
concluídas) e `F` governa a **categoria** — são eixos separados de propósito, senão nunca se
via «pendentes da categoria X». `F` cicla `todas → sem categoria → as categorias, pela ordem
de inserção → todas`, e a linha 2 só escreve `· categoria: <nome>` quando o filtro
**restringe** (`sem categoria` escreve-se por palavras); com `todas` não escreve nada. Se a
categoria filtrada for eliminada, o filtro volta a `todas` — uma lista vazia sem explicação
seria pior. A categoria de cada tarefa está na coluna própria da linha (ver
[Caixa de categorias](#caixa-de-categorias-c)) e o `Esc` em repouso limpa os três eixos
(busca, estado e categoria) sem sair da aplicação.

### Vista do lixo (`L`)

| Tecla | Acção |
| --- | --- |
| `j` · `↓` · `k` · `↑` · `g` · `G` | navegar |
| `Enter` · `Espaço` | restaurar a selecionada **na posição original** |
| `c` | esvaziar o lixo (pede duas pressões) |
| `Esc` | voltar à lista |
| `?` | ajuda |
| `Ctrl+C` | sair |

Nesta vista `a`, `e`, `d`, `u`, `1` `2` `3`, `t`, `s`, `f`, `i`, `x`, `C`, `F` e `q` não fazem
nada (é um mapa próprio): `u`, que na lista repõe o último lote, aqui teria dois sentidos, um
`c` com dois sentidos esvaziaria o lixo sem guarda, e `C`/`F` — as categorias — não entram
nesta vista: o rodapé não as anuncia e a linha do lixo não mostra categoria nenhuma (nem um
`—`, que afirmaria «sem categoria» sobre tarefas que podem ter uma).

### Linha de texto (nova tarefa, editar, buscar, importar, exportar, criar/renomear categoria)

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
- As mensagens de acção da linha 22 **expiram aos 3 segundos**. O que não expira sozinho: o
  undo disponível e o aviso de transbordo do lixo (ficam até outra acção os substituir ou até
  `Esc`) e as mensagens de erro, que saem com outra acção ou com `Esc`.

### Caixa de temas (`T`)

| Tecla | Acção |
| --- | --- |
| `j` · `↓` · `k` · `↑` | tema seguinte · anterior, com pré-visualização ao vivo |
| `Enter` | gravar a escolha e fechar |
| `Esc` · `q` | fechar **sem** gravar (volta ao tema com que a caixa abriu) |
| `Ctrl+C` | sair |

É um mapa próprio, como o da ajuda: com a caixa aberta nada da lista vale lá dentro — nem
`a`, nem `e`, nem `d`, nem o próprio `T`. A caixa mostra o catálogo (o `▶` marca a linha do
cursor, `em uso` o tema gravado), a régua `─ amostra · <slug> ─` — a única linha do ecrã onde
o *slug* aparece —, três linhas de tarefa de amostra já no tema pré-visualizado e uma última
linha com o modo de cor em uso (`rgb` ou `ansi`). A barra da linha 23 passa a
`Esc volta ao tema de entrada`, e a ajuda `?` lista `T  tema`.

Ver [Temas](#temas) para o catálogo e para onde a escolha fica guardada.

### Caixa de categorias (`C`)

| Tecla | Acção |
| --- | --- |
| `j` · `↓` · `k` · `↑` | entrada seguinte · anterior (para nos extremos, não roda) |
| `g` · `G` | primeira · última entrada |
| `Enter` | atribui a categoria do cursor à tarefa selecionada e fecha |
| `a` | criar uma categoria (abre a linha do nome) |
| `e` | renomear a categoria do cursor (abre a linha do nome) |
| `d` | eliminar a categoria do cursor (**pede duas pressões**) |
| `Esc` · `q` | fechar **sem** atribuir |
| `Ctrl+C` | sair |

É um mapa próprio, como o da ajuda e o das temas: com a caixa aberta nada da lista vale lá
dentro — nem o `d` que manda a tarefa para o lixo. A **primeira linha é sempre
`sem categoria`** (é a tarefa que não tem nenhuma, não uma categoria): lá, `e` e `d` não fazem
nada e o `Enter` **tira** a atribuição e fecha. Cada entrada mostra entre parênteses quantas
tarefas **da lista** tem (`Trabalho (4)`, `sem categoria (12)` — a lista, não o lixo), a linha
de estado diz o que a tarefa selecionada tem agora (`Atribuída: Trabalho`) e o rodapé só
anuncia as teclas vivas nesse contexto. Com mais categorias do que as dez que cabem, a caixa
ganha uma janela (`1–10 de 13`) que anda com o cursor; a caixa **não cresce**.

A caixa abre **mesmo sem tarefas** — é a única forma de criar categorias numa base vazia —, e
sem nenhuma categoria a linha de estado di-lo (`Sem categorias — a cria a primeira`); sem
tarefas, diz que não há a quem atribuir (`Sem tarefas — não há a quem atribuir`), em vez de
deixar parecer que o `Enter` está avariado.

`a` e `e` abrem a linha de texto na linha 22, **por cima** da caixa, e `Enter` guarda. O nome é
**único** (ignora maiúsculas: `Trabalho` e `trabalho` são o mesmo) e não pode ser vazio; um
nome recusado diz porquê, a linha fica aberta e o texto escrito **não se perde**. Renomear muda
o nome, não o `id` — as tarefas da categoria continuam a ser as mesmas.

`d` é a segunda operação do programa **sem undo** por trás (a primeira é esvaziar o lixo): a
primeira pressão arma uma guarda de cinco segundos e a linha de estado diz o preço —
`Eliminar «Trabalho»? 2 tarefas ficam sem categoria` —, o rodapé passa a
`d outra vez confirma · Esc cancela` e qualquer outra tecla desarma. A segunda pressão
elimina: as tarefas da categoria, **na lista e no lixo**, ficam sem categoria, e a contagem é
anunciada (`Eliminada «Trabalho» · 2 tarefas ficaram sem categoria`). Recriar a categoria é uma
linha; a atribuição das tarefas não volta sozinha — é por isso que a guarda existe.

Onde é que a categoria se lê sem abrir a caixa: uma coluna própria de **14 colunas** na linha
da tarefa (o nome cortado com `…`, `—` quando a tarefa não tem nenhuma, e nada na vista do
lixo) e uma linha `Categoria` no painel de detalhe, com o nome inteiro. O título da linha cede
as colunas (44 a 80×24, 84 a 120×32). A barra da linha 23 passa a
`Esc fecha sem atribuir` e a ajuda `?` lista `C` e `F`.

## Temas

A cor do ecrã é um tema com nome. São quatro, e o *slug* é o nome que se escreve no `--theme`
e o que fica guardado no ficheiro de preferências:

| Tema | `slug` | Fundo |
| --- | --- | --- |
| Tokyo Night | `tokyo-night` | `#1a1b26` |
| Tokyo Night Storm | `tokyo-night-storm` | `#24283b` |
| Tokyo Night Moon | `tokyo-night-moon` | `#222436` |
| Clássico (ANSI) | `classico` | — (o fundo do terminal) |

`tokyo-night` é o tema por omissão. O `classico` é o que a v1.0.1 desenhava — os mesmos índices
ANSI do terminal, sem pintar fundo — e é também a rede de segurança dos terminais sem
truecolor (ver [Terminais sem truecolor](#terminais-sem-truecolor)).

### Como se escolhe

`T` abre a [caixa de temas](#caixa-de-temas-t) sobreposta à lista: `j`/`k` (ou `↓`/`↑`)
percorrem o catálogo com pré-visualização ao vivo — a caixa toda, molduras incluídas, muda de
cor —, `Enter` grava a escolha e `Esc` (ou `q`) fecha sem gravar, repondo o tema com que a
caixa abriu.

### O ficheiro da preferência

A escolha fica em `~/.config/todo-ratatui/config.json` (`$XDG_CONFIG_HOME/todo-ratatui/config.json`
quando essa variável está definida), e é só isto:

```json
{
  "theme": "tokyo-night"
}
```

É **configuração**, não dados: vive em `~/.config` e não ao lado do `db.json` (que está em
`~/.local/share`), porque perdê-la não pode custar uma tarefa. **Apagar o ficheiro volta ao
tema por omissão** — não há mais nada a limpar. A escrita é atómica (`config.json.tmp` no
mesmo directório, `fsync`, `rename`) e roda a geração anterior para `config.json.bak`, como no
`db.json`.

Um `config.json` ilegível, corrompido ou com o tipo errado (`{"theme": 3}`) **não impede abrir
a lista**: o programa avisa no `stderr`, segue com o tema por omissão e **não toca no
ficheiro** — substituí-lo é decisão de quem grava, com `Enter`. Um `theme` com um *slug*
desconhecido vale o mesmo: aviso no `stderr` e a cadeia segue para o valor seguinte.

### Na linha de comandos

As três opções novas, cada uma com a variável de ambiente equivalente (a opção ganha à
variável):

| Opção | Variável | O que faz |
| --- | --- | --- |
| `--theme <slug>` | `TODO_RATATUI_THEME` | o tema desta sessão |
| `--color <auto\|rgb\|ansi>` | `TODO_RATATUI_COLOR` | como a cor é entregue ao terminal (`auto` por omissão) |
| `--config <caminho>` | `TODO_RATATUI_CONFIG` | outro ficheiro de preferências, em vez do `~/.config/todo-ratatui/config.json` |

```sh
todo_ratatui --theme tokyo-night-moon
TODO_RATATUI_THEME=tokyo-night-storm todo_ratatui
todo_ratatui --theme classico                    # o look da v1.0.1, só nesta sessão
todo_ratatui --config /tmp/prefs.json            # experimentar sem tocar na preferência
```

As duas formas de escrever o valor valem o mesmo: `--theme tokyo-night-moon` e
`--theme=tokyo-night-moon`.

A cadeia do tema é **`--theme` → `TODO_RATATUI_THEME` → `config.json` → `tokyo-night`** e a
da cor é **`--color` → `TODO_RATATUI_COLOR` → `auto`**. Um *slug* ou um modo de cor
desconhecido não é fatal: fica um aviso no `stderr`, o valor seguinte da cadeia decide, e a
lista abre na mesma.

**Nem a opção nem a variável gravam nada**: são *overrides* da sessão. Quem grava é o `Enter`
da caixa de temas — por isso um `--theme classico` de passagem não troca a preferência
guardada.

### Terminais sem truecolor

O modo `auto` (por omissão) pergunta ao terminal se ele suporta truecolor. Se não suportar
— ou se `--color ansi` for pedido —, os três temas Tokyo Night **não são rebaixados às
cegas**: o ecrã passa a usar o `classico` e o `config.json` **fica como está**, a escolha
guardada à espera da próxima vez que houver truecolor. Nada é reescrito por causa disto, e o
modo pedido também não é gravado.

O que está guardado só é substituído por um `Enter` na caixa de temas. Em `ansi` a caixa
desenha sempre o `classico` — num terminal de 16 cores não sai daí uma única sequência RGB,
nem a navegar —, e um `Enter` sem navegar grava o `classico` que está debaixo do cursor;
escolher um Tokyo Night em `ansi` **grava o pedido** (é o que fica à espera de truecolor), e
o ecrã só muda quando ele houver.

`--color rgb` força a paleta RGB mesmo que a detecção diga o contrário: é a saída quando a
detecção mente (um `tmux` sem `Tc`, por exemplo).

### O fundo é do tema

Os três temas Tokyo Night pintam o fundo do ecrã todo (`#1a1b26` no `tokyo-night`): é isso que
faz os contrastes medidos valerem em qualquer terminal, e é a troca aceite — lá dentro não se
vê o fundo do terminal, nem transparência. O `classico` não pinta nada e continua a respeitar
o terminal, como a v1.0.1.

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

**Permissões.** Todo o ficheiro que o programa cria nasce privado (`0600`), e os dois
directórios que ele cria — `~/.local/share/todo-ratatui` e `~/.config/todo-ratatui` —
nascem `0700`. Os `0600` valem para o `db.json`, o `config.json`, o `.tmp`, o `.bak`
(que, por ser o ficheiro rodado pelo `rename`, precisou de um ajuste
explícito — herda o modo do anterior) e o `db.json.pre-restore`. O caminho de destino de
uma exportação é escolha tua: só o CSV nasce `0600`, e os directórios que o export tiver de
criar pelo caminho ficam com o modo que o `umask` lhes der. Um directório que
**já exista** não é tocado: uma instalação da v1.0.x pode ter
`~/.local/share/todo-ratatui` e `~/.config/todo-ratatui` a `775` (do `umask`), e o
programa não passa por cima dessa escolha — fecha-os à mão com
`chmod 700 ~/.local/share/todo-ratatui ~/.config/todo-ratatui` se quiseres que outros
utilizadores locais não lá entrem. Os *ficheiros* corrigem-se sozinhos: o novo
substitui o antigo na gravação seguinte.

A escrita do temporário é `O_EXCL` (o `db.json.tmp` nunca é aberto se já existir): uma
gravação não segue um symlink nem escreve através de um hard link plantado nesse
caminho, que é derivado do destino e por isso previsível. Um `.tmp` deixado por uma
corrida interrompida é removido e a gravação repete-se **uma** vez, em vez de ficar
bloqueada para sempre.

A preferência de tema tem os seus próprios ficheiros, e **não** fica aqui:
`~/.config/todo-ratatui/config.json`, com o `config.json.bak` da geração anterior (o mesmo par
de escrita atómica do `db.json`) — é configuração, não dado (ver
[Temas](#temas)).

O **formato dos dados mudou na v1.2**: o envelope passa a
`{schema, todos, trash, trash_dropped, categories, dangling_recovered}` com `schema: 3` (ver
[Formato em disco](#formato-em-disco)). Um ficheiro escrito pela v1.1.0 abre nesta versão sem
conversão nem aviso e é reescrito como `3` na gravação seguinte; o contrário **não** vale — a
v1.1.0 recusa um ficheiro `3`, com a mensagem do `schema`, e não lhe toca. O tema vive fora dos
dados precisamente por isso: apagar a preferência (ou o `~/.config` todo) não custa uma tarefa.

Se uma gravação falhar (disco cheio, directório sem permissão de escrita), a linha 22 diz
`Erro: não gravou <caminho> — lista intacta`: nada foi escrito, o que está em memória
mantém-se e a aplicação continua a responder.

Se o ficheiro **não se conseguir abrir** — ilegível, JSON inválido, `schema` desconhecido
ou um array legado — o programa **recusa arrancar**: escreve o erro e o caminho do `.bak`
no `stderr` e sai com código diferente de zero, antes de desenhar a TUI. É de propósito:
uma lista vazia em memória seria, na gravação seguinte, uma sobrescrita do ficheiro que
ainda podia ser recuperado.

Tudo o que o programa escreve no `stderr` — os avisos de tema/cor e os erros de
arranque — passa por um saneamento: caracteres de controlo saem na forma visível
`\u{1b}` e o resto (acentos, `«»`, emoji) fica tal e qual. Assim um valor de `--theme`,
`--color` ou `--config` com sequências de escape (por exemplo a OSC 52, que copia texto
para a área de transferência) deixa de poder mexer no terminal de quem lê o aviso: o
valor continua a ver-se, agora legível.

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
id,title,description,done,priority,created_at,completed_at,due_at,category
```

A coluna `category` (a 9.ª) leva o **nome** da categoria, e não o `id` — é o formato que se lê
e se escreve à mão, e um `uuid` não diz nada a quem abre a folha de cálculo. Uma tarefa sem
categoria sai com o campo vazio.

**Importar (`i`)** — o que é lido depende da extensão do ficheiro:

- `.csv` → CSV com o cabeçalho de 9 colunas acima, **ou com o de 8 colunas da v1.1.0** (sem
  `category`): um ficheiro exportado por essa versão continua a importar, e tudo entra sem
  categoria. O cabeçalho é comparado como **conjunto de colunas** — a ordem não conta — e um CSV
  com outro cabeçalho, ou sem cabeçalho nenhum, é **recusado**, com uma mensagem que mostra os
  dois cabeçalhos aceites e o encontrado: sem essa comparação a primeira linha seria comida
  como cabeçalho e a primeira tarefa desaparecia em silêncio (era um defeito do `rtodo` antigo).
- qualquer outra extensão → JSON: o envelope novo (`{schema, todos, trash, categories}`, que é
  o próprio `db.json` — traz também o lixo e as categorias) ou um array, lido registo a
  registo pelo formato de cada um (ver abaixo).

A categoria, no CSV, viaja **pelo nome**, e é o import que a resolve contra a base: um nome que
não exista é **criado** (e contado no relatório), um nome que já exista — mesmo escrito com
outras maiúsculas — é **reaproveitado** e as tarefas apontam à que cá está. A consequência a
saber: um CSV exportado **antes** de um rename traz o **nome antigo**, e é esse que as tarefas
que entrarem levam — a categoria antiga é recriada ao lado da nova, porque o `id` não viaja no
CSV. E as categorias novas só nascem para as tarefas que entram **de facto**: reimportar um
ficheiro cujas tarefas já cá estão (os `id` repetem-se) não inventa categorias nenhumas.

O import **nunca duplica**: registos cujo `id` já exista (na base ou no ficheiro) são
ignorados, e a linha 22 diz o que entrou (o que não aconteceu não se escreve):

```
Importado: 2 lidos, 2 inseridos, 0 duplicados ignorados, 1 sem data legível
Importado: 4 lidos, 4 inseridos, 0 duplicados ignorados, 3 categorias criadas
Importação sem alterações: 3 lidos, 0 inseridos, 3 duplicados ignorados
```

Um import que falhe a meio não altera nada, e o erro nomeia o ficheiro e a linha.

O CSV exportado é **fiel** ao que está guardado: não há neutralização de fórmulas (nada
de prefixar um apóstrofo a títulos que comecem por `=`, `+`, `-` ou `@`), porque isso
partiria o ciclo exportar→importar — um título `- comprar pão` voltaria do import como
`'- comprar pão`, sem aviso. Consequência a saber: abrir um CSV com títulos de
proveniência desconhecida numa **folha de cálculo** (Excel, Calc) pode fazer o programa
avaliar esses títulos como fórmulas — usa o assistente de importação de texto da folha
de cálculo e confirma que as colunas ficam como texto.

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

`db.json` real, de uma sessão com uma tarefa (numa categoria) e outra no lixo:

```json
{
  "schema": 3,
  "todos": [
    {
      "id": "556c7153-c608-4c45-a1d6-858deb9003a5",
      "title": "Comprar café em grão na Nota Roja",
      "description": "",
      "done": false,
      "priority": "medium",
      "created_at": "2026-09-18T20:11:21.527979419+01:00",
      "completed_at": null,
      "due_at": null,
      "category_id": "2f1c0f9c-4b4a-4e0f-9c1a-6f0a1b2c3d4e"
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
        "due_at": null,
        "category_id": "2f1c0f9c-4b4a-4e0f-9c1a-6f0a1b2c3d4e"
      },
      "index": 1,
      "deleted_at": "2026-09-18T20:11:22.151882033+01:00",
      "batch": 1
    }
  ],
  "trash_dropped": 0,
  "categories": [
    {
      "id": "2f1c0f9c-4b4a-4e0f-9c1a-6f0a1b2c3d4e",
      "name": "Café e compras"
    }
  ],
  "dangling_recovered": 0
}
```

A geração é a **`schema: 3`**, a das categorias: `categories` é a lista (por ordem de
inserção), `category_id` é a categoria de cada tarefa (`null` quando não tem nenhuma) e
`dangling_recovered` conta as referências a categorias que não resolvem — um ficheiro editado
à mão, dois ficheiros fundidos — que a leitura normalizou para `null`. O princípio é **uma
tarefa nunca se perde nem fica escondida por causa de uma categoria**, e a contagem fica
gravada no ficheiro para não depender de a aplicação ainda estar de pé quando ele foi lido.

Um ficheiro **`2`** (o da v1.1.0) abre nesta versão **sem conversão nem aviso**: as chaves que
ele não tem nascem vazias, nenhuma tarefa muda, e a gravação seguinte escreve `3` — e roda a
geração anterior para o `.bak`, como sempre.

O contrário **não vale**: um ficheiro `3` **não abre na v1.1.0**. Essa versão recusa o arranque,
diz qual o `schema` que encontrou e **não lhe toca**:

```
erro: schema 3 em «/caminho/db.json» não é suportado (esperado 2)
a base de dados não foi alterada; a geração anterior está em «/caminho/db.json.bak»
```

Não se estraga nada: o ficheiro `3` continua a abrir aqui, e a saída é a de sempre — o `.bak`
(a geração anterior) e o `db.json.pre-restore` de um restauro (ver
[Os ficheiros ao lado](#os-ficheiros-ao-lado)).

## Desenvolvimento

- `src/core/` — modelo, persistência, operações e import/export. **Não depende de
  `ratatui`/`crossterm`**, o que o torna testável sem terminal; a invariante é travada por
  `tests/boundary.rs`.
- `src/tui/` — estado (`app.rs`), teclas (`event.rs`) e desenho (`ui.rs`); as cores são dados
  com nome (`theme.rs`) e o arranque (`arranque.rs`) resolve tema, modo de cor e ficheiro de
  preferências — o `core` (e portanto o `db.json`) só conhece o *slug*, como texto.
- `src/main.rs` — terminal e arranque.

```sh
cargo test                                  # 264 testes, incluindo os goldens
cargo clippy --all-targets -- -D warnings
```

Os 26 ficheiros de `tests/frames/` são *golden files*: o ecrã desenhado é comparado linha a
linha com eles, a 80×24 e a 120×32.

## Licença

MIT, com os dois avisos de copyright — o do `rtodo` original e o desta obra. Ver
[LICENSE](LICENSE); o que mudou em cada versão está no [CHANGELOG.md](CHANGELOG.md).

## Âmbito da v1.2

- Categorias: **uma por tarefa**, com criar, renomear e eliminar na [caixa do `C`](#caixa-de-categorias-c),
  atribuição, filtro próprio (`F`) e ordem por categoria no `s` (ver [Teclas](#teclas)). Fora:
  hierarquia ou sub-categorias; cor, ícone ou ordem manual por categoria; mais do que uma
  categoria por tarefa; regras por categoria (prazo, recorrência); atribuir várias tarefas de
  uma vez; categorias na vista do lixo; procurar por prefixo dentro da caixa; categorias
  definidas à mão no `config.json`.
- `due_at` (prazo) existe no formato em disco e aparece no painel de detalhe quando vem de
  um import, mas **não é editável na interface**.
- Temas: os quatro do catálogo, com a caixa do `T` e a preferência guardada (ver
  [Temas](#temas)). Fora: temas definidos pelo utilizador (um ficheiro de paletas), tema
  claro, cor por campo e sincronizar a preferência entre máquinas.
- Fora: descrição multi-linha, várias listas, rato, notificações.
- Sem base de dados SQLite: o `db.json` é o único formato (ver
  [Formato em disco](#formato-em-disco)).
- Não há publicação no crates.io (`publish = false` no `Cargo.toml`).
