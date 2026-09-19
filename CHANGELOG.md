# Histórico de alterações

O formato segue de perto o [Keep a Changelog](https://keepachangelog.com/pt-PT/1.1.0/);
as versões são as do `Cargo.toml` e as etiquetas do repositório (`vX.Y.Z`).

## 1.2.0 — 2026-09-19

Categorias: uma por tarefa, com criar, renomear e eliminar dentro da aplicação, atribuição pela
caixa do `C`, filtro próprio e ordem na lista. **O formato dos dados muda** — o `db.json` passa
a `schema: 3` — e é o único sítio onde esta versão paga: um ficheiro `3` **não abre** na v1.1.0,
que recusa o arranque e diz qual o `schema` que encontrou, **sem lhe tocar**. Nada se perde (o
`.bak` é a geração anterior) e, no sentido contrário, um `db.json` da v1.1.0 **abre nesta versão
sem conversão nem aviso** — é reescrito como `3` na gravação seguinte, sem mudar um campo das
tarefas.

### Adicionado

- **Categorias** — `Category { id, name }` (o `id` é a chave, o nome é editável) com o nome
  **único** e comparado sem maiúsculas, e uma categoria **por tarefa**
  (`Todo.category_id: Option<CategoryId>`). O `core` ganha o CRUD completo, sem UI:
  `add_category`, `rename_category`, `delete_category` (limpa a atribuição das tarefas **na
  lista e no lixo**, e devolve a contagem) e `assign_category`.
- **Caixa de categorias (`C`)** — sobreposta à lista, no molde da caixa de temas, com mapa de
  teclas próprio: a primeira linha é sempre `sem categoria` (o `None`, não uma categoria),
  cada entrada diz quantas tarefas **da lista** tem e a linha de estado diz o que a tarefa
  selecionada tem agora. `Enter` atribui — e **tira** a atribuição, em `sem categoria` —, `a`
  cria, `e` renomeia, `d` elimina e `Esc` (ou `q`) fecha sem atribuir. Abre **mesmo sem
  tarefas**, que é a única forma de criar categorias numa base vazia. A caixa tem uma janela
  de dez entradas (`1–10 de 13`) e não cresce.
- **`F`, o filtro por categoria** — um eixo próprio, ao lado do `f`, que continua a governar só
  o estado: `todas → sem categoria → as categorias → todas`. A linha 2 escreve
  `· categoria: <nome>` **apenas quando o filtro restringe**, e o `Esc` em repouso — que já
  limpava a busca e o filtro — limpa-o também. Eliminar a categoria que está a filtrar repõe o
  filtro em `todas`, em vez de deixar uma lista vazia sem explicação.
- **Ordem por categoria** no ciclo do `s` (`SortKey::Category`), com as tarefas sem categoria
  no fim e a ordem de inserção dentro de cada grupo.
- **A categoria lê-se sem abrir a caixa**: uma coluna própria de 14 colunas na linha da tarefa
  (o nome cortado com `…`, `—` quando a tarefa não tem nenhuma, e nada na vista do lixo) e uma
  linha `Categoria` no painel de detalhe, com o nome inteiro. O título da linha cede as 15
  colunas (59 → 44 a 80×24; 84 a 120×32).
- **A guarda de duas pressões passa a ter alvo** (`Guarda::{EsvaziarLixo, EliminarCategoria}`):
  o `c` da vista do lixo e o `d` da caixa de categorias deixam de partilhar um sinalizador, que
  é como se esvaziaria o lixo a eliminar uma categoria.

### Alterado

- **O formato em disco passa a `schema: 3`**: entram `categories` (a lista, por ordem de
  inserção), `category_id` por tarefa e `dangling_recovered` — as referências a categorias que
  não resolvem (ficheiro editado à mão, ficheiros fundidos) são normalizadas para `null` na
  leitura e **contadas no ficheiro**, para o número não depender de a aplicação ainda estar de
  pé quando ele foi lido. A leitura aceita `2` e `3`; a gravação emite sempre `3`. **Um
  ficheiro `3` não abre na v1.1.0**: essa versão recusa o arranque com
  `schema 3 … não é suportado (esperado 2)` e não lhe toca — a saída é o `.bak`, que fica com a
  geração anterior.
- **O CSV passa a 9 colunas**, com `category` no fim a levar o **nome** da categoria (uma
  tarefa sem categoria sai com o campo vazio). O import **continua a aceitar o cabeçalho de 8
  colunas da v1.1.0** (tudo entra sem categoria) e recusa o resto, agora com uma mensagem que
  mostra os dois cabeçalhos aceites e o encontrado. A categoria é identificada pelo nome: um
  nome desconhecido é **criado**, um existente — mesmo com outras maiúsculas — é
  **reaproveitado**, e uma categoria nova só nasce para as tarefas que entram de facto (um CSV
  reimportado não inventa categorias). O relatório do import ganha as contagens de categorias
  criadas e reaproveitadas e de tarefas que ficaram sem categoria.
- **A ajuda `?`** ganha `F  filtrar categoria` e `C` (na linha do `T`) **sem crescer**: para
  abrir espaço, `j / ↓` e `k / ↑` fundem-se numa linha e `f  filtrar` passa a `f  filtrar
  estado`. O `s` inclui a categoria no ciclo e o `Esc` limpa os três eixos.
- **Os *golden files***: 11 novos (a caixa de categorias e os seus estados, o filtro, a ordem
  por categoria), 10 alterados (a coluna da categoria em todas as linhas da lista e a ajuda do
  `?`) e 5 iguais — os do lixo, que não mostra categorias.
- **`README.md`** documenta a caixa, o filtro, o CSV de 9 colunas e o formato `3`, e o âmbito
  passa a ser o da v1.2: a linha que punha «tags» fora sai (a resposta a esse pedido é **uma**
  categoria por tarefa; etiquetas múltiplas continuam fora) e a lista do que fica fora fica
  escrita.

### Notas

- Alcance medido da mudança de formato: um `db.json` da v1.1.0 abre sem conversão nem aviso, e
  a primeira gravação reescreve-o como `3` com as tarefas intactas. A volta atrás não é
  automática: o binário antigo recusa o ficheiro novo **sem lhe tocar**, e o caminho é o `.bak`
  (ou o `db.json.pre-restore` de um restauro).
- No CSV, a categoria viaja pelo **nome**: um ficheiro exportado **antes** de um rename traz o
  nome antigo, e é esse que as tarefas que entrarem levam — a categoria antiga é recriada ao
  lado da renomeada. Está registado no `README`.
- O `q` com a caixa de categorias aberta **fecha a caixa, não sai do programa** (a mesma
  decisão da caixa de temas), e o `Esc` em repouso continua a não sair.
- Divergências conhecidas, pré-existentes e não corrigidas aqui: as da 1.0.1 (o aviso de
  transbordo do lixo não é desenhado a vermelho e a guarda do `c` dura 5 s com uma mensagem de
  3 s).

## 1.1.0 — 2026-09-19

Temas: o ecrã passa a ser desenhado a partir de uma paleta com nome, escolhida dentro da
aplicação e guardada como preferência. **O formato dos dados não muda**: o `db.json`
continua a ser o envelope com `schema: 2`, a v1.0.1 continua a ler o mesmo ficheiro e esta
versão lê os ficheiros da v1.0.1 sem conversão nenhuma. A preferência vive num ficheiro à
parte, exactamente para que perdê-la não possa custar uma tarefa.

### Adicionado

- **Quatro temas**, com *slug* canónico (o que se escreve no `--theme` e o que fica no
  ficheiro): `tokyo-night` (**novo tema por omissão**), `tokyo-night-storm`,
  `tokyo-night-moon` e `classico` (o que a v1.0.1 desenhava). Os três Tokyo Night pintam o
  fundo do ecrã (`#1a1b26` no `night`), o que faz os contrastes medidos valerem em qualquer
  terminal; o `classico` não pinta nada e continua a respeitar o fundo do terminal.
- **Caixa de temas** (`T`): sobreposta à lista, com pré-visualização ao vivo (`j`/`k` e
  `↓`/`↑`), `Enter` para gravar e `Esc` (ou `q`) para fechar sem gravar, repondo o tema com
  que a caixa abriu. Mostra o catálogo (com `▶` na linha do cursor e `em uso` no tema
  gravado), a régua `─ amostra · <slug> ─`, três linhas de tarefa de amostra no tema
  pré-visualizado e o modo de cor. A ajuda `?` passa a listar `T  tema`.
- **A preferência de tema** em `~/.config/todo-ratatui/config.json`
  (`{"theme": "<slug>"}`), escrita atomicamente (`.tmp` ao lado + `rename`) com rotação da
  geração anterior para `config.json.bak`. **Apagar o ficheiro volta ao tema por omissão.**
- **`--theme <slug>` / `TODO_RATATUI_THEME`** — escolhem o tema da sessão. São *overrides*:
  ganham ao ficheiro de preferências e **não gravam nada**.
- **`--color <auto|rgb|ansi>` / `TODO_RATATUI_COLOR`** e **`--config <caminho>` /
  `TODO_RATATUI_CONFIG`** — mais o caminho da preferência. As duas formas de escrever o valor
  (`--theme x` e `--theme=x`) valem o mesmo.
- **Detecção de cor**: em `auto` (por omissão) pergunta-se ao terminal. Sem truecolor, os
  temas que pintam fundo não são rebaixados às cegas — o ecrã usa o `classico` e a
  preferência guardada **não** é reescrita. `--color rgb` força a paleta RGB, que é a saída
  quando a detecção mente (um `tmux` sem `Tc`, por exemplo).

### Alterado

- **O ecrã passa a ser desenhado por tema.** As nove cores fixas de `src/tui/ui.rs` passam a
  ser papéis nomeados de um tema (`src/tui/theme.rs`), e o fundo passa a ser pintado nos
  temas Tokyo Night. Os *golden files* de **texto** existentes não mudam por causa da cor
  (que eles não codificam): dos 14 de então, 13 ficam byte a byte iguais e o da ajuda muda
  (ganha a linha do `T`), mais um frame novo para a caixa de temas.
- **O tema por omissão deixa de ser «as cores ANSI do terminal»**: passa a ser o
  `tokyo-night`, uma paleta RGB. Um arranque sem configuração nenhuma muda de aspecto em
  relação à v1.0.1.
- `--help` e `README.md` descrevem as opções novas e a caixa de temas; a linha «temas
  configuráveis» sai do fora de âmbito e a secção **Temas** entra.
- **O `.bak` passa a acompanhar o nome do ficheiro.** A escrita atómica, extraída para
  `src/core/atomic.rs`, deriva o backup do destino (como o `config.json` já fazia) em vez do
  nome fixo `db.json.bak` da v1.0.1: com `--db /x/foo.json` o backup passa a ser
  `/x/foo.json.bak`. É a mesma geração anterior, num nome que não mente sobre o que lá está.
  O caminho por omissão não muda: `db.json` continua a dar `db.json.bak`.

### Segurança

Endurecimento a partir de uma auditoria ao código deste ramo (achados **SA-01**, **SA-02**
e **SA-03**; os SA-04 e SA-05 ficam recusados com motivo, ver as notas). Nada disto mexe no
formato dos dados: `schema` continua `2` e o `db.json` da v1.0.1 abre sem conversão.

- **Nada do que o programa escreve no `stderr` pode conter caracteres de controlo.** Os
  avisos de tema e de cor (que nomeiam o valor de `--theme`, `--color` e do `config.json`)
  e os erros de arranque passam a escrever esses caracteres na forma visível `\u{1b}`.
  Antes, um valor com uma sequência de escape OSC 52 copiava texto para a área de
  transferência de quem lesse o aviso no terminal, e um `\n` forjava uma linha a imitar uma
  mensagem do programa. O texto normal — acentos, `«»`, emoji — sai tal e qual, e o que é
  *guardado* não é tocado: o saneamento é só na impressão.
- **Os ficheiros criados nascem privados (`0600`) e os directórios que o programa cria
  (`~/.local/share/todo-ratatui`, `~/.config/todo-ratatui`) a `0700`** — `db.json`,
  `config.json`, o `.tmp`, o `.bak`, o `db.json.pre-restore` e o CSV do export (e o `.tmp`
  dele). Antes ficavam à mercê do `umask` (0644 com o `umask` habitual),
  legíveis por qualquer utilizador local. O `.bak` precisou de um passo explícito: o
  `rename` dá-lhe o modo do ficheiro rodado, e o `0600` do temporário não chegava lá. Esse
  passo é omitido quando o `.bak` é o symlink rodado de um destino que era um link (`--db` a
  apontar para um symlink): o `chmod` seguiria o link e mudaria o modo de um ficheiro que o
  programa não criou.
- **A escrita atómica deixou de seguir um link plantado no temporário.** O `<destino>.tmp`
  passa a ser criado com `O_EXCL` e `0600`: um symlink ou um hard link posto nesse caminho
  (derivado do destino, logo previsível) já não redirecciona a escrita — o nome é
  desligado, nunca atravessado. Um `.tmp` deixado por uma corrida interrompida é removido
  e a gravação repete-se **uma** vez; sem isso, um temporário órfão bloquearia a gravação
  para sempre.

### Notas

- Para voltar ao look da v1.0.1: o tema `classico` (`T` e escolher, ou `--theme classico`) ou
  `--color ansi`. Nenhum dos dois grava a preferência.
- **Permissões já instaladas: passo manual, opcional.** O programa fecha o que **cria** e
  não impõe `0700` a um `~/.local/share/todo-ratatui` ou `~/.config/todo-ratatui` que já
  exista (a partilha com o grupo pode ser uma escolha de quem lá tem os dados). Os
  ficheiros corrigem-se sozinhos na gravação seguinte; os directórios não:
  `chmod 700 ~/.local/share/todo-ratatui ~/.config/todo-ratatui`, se os quiseres fechados.
- **Fórmulas no CSV (SA-04): risco assumido, por escrito.** O CSV exportado é fiel ao que
  está guardado — um título que comece por `=`/`+`/`-`/`@` **não** é neutralizado, porque
  o prefixo `'` degradaria o ciclo exportar→importar (`- comprar pão` voltaria
  `'- comprar pão`). Abrir um CSV de proveniência desconhecida no Excel ou no Calc pede o
  assistente de importação de texto. Fica registado no `README`.
- **Caracteres de controlo dentro de um título (SA-05): sem validação nova.** Validar em
  `Todo::try_new` daria cobertura falsa — o import desserializa a tarefa por serde e não
  passa por lá. A defesa real é a do `ratatui`, que filtra o que desenha, e fica presa por
  um teste de invariante (`tests/injecao.rs`) que falha se ela desaparecer.
- Um `config.json` ilegível ou corrompido **não** impede abrir a lista: avisa no `stderr`,
  segue com o tema por omissão e **não** toca no ficheiro.
- Alcance medido da mudança de formato: **nenhuma** base de dados é tocada — `schema`
  continua `2` e nada foi acrescentado ao `db.json`; um ficheiro da v1.0.1 abre nesta versão
  sem conversão nem aviso.
- Divergências conhecidas, pré-existentes e não corrigidas aqui: as da 1.0.1 (o aviso de
  transbordo do lixo não é desenhado a vermelho e a guarda do `c` dura 5 s com uma mensagem
  de 3 s).

## 1.0.1 — 2026-09-19

Correcção do import de arrays JSON e dos estados que a §4/§5 do desenho prometia
e a 1.0.0 não cumpria. O formato em disco não muda: nem o envelope
(`{schema, todos, trash}`), nem o `db.json` de topo.

### Corrigido

- **Import de um array JSON**: um array passa a ser lido **registo a registo**,
  pelo formato de cada um. Um registo do formato novo (com algum de `id`,
  `priority`, `created_at`, `completed_at`, `due_at`) era lido como registo do
  `rtodo`: `priority` e `created_at` perdiam-se em silêncio, o `id` era
  substituído por um novo e importar o mesmo ficheiro duas vezes duplicava
  tudo. Agora é lido como tarefa, com a mesma validade do envelope, o `id` que
  lá estiver é preservado e o import continua a não duplicar. Os registos do
  `rtodo` (sem nenhum desses campos) continuam a ler-se como antes e os dois
  formatos podem conviver no mesmo ficheiro. Alcance medido: **nenhuma base em
  uso estava afectada** — esta versão nunca escreveu arrays sem envelope e
  recusa arrancar com um; o defeito exigia um ficheiro feito à mão ou um
  `jq '.todos' db.json`. O `README.md` passa a descrever a regra real.
- Vista do **lixo vazio**: tinha o corpo e o rodapé dos «sem resultados» (nomeava
  um filtro inactivo e anunciava uma tecla que não existe nessa vista) e a barra
  de ajuda oferecia `c esvaziar o lixo` com o lixo vazio. Ganha estado próprio,
  com precedência sobre os vazios da lista, e o rótulo da coluna `removida`
  deixa de aparecer quando não há coluna.
- **Mensagens de acção expiram**: o loop lia eventos em bloqueio
  (`event::read()`), logo não havia relógio nenhum e a linha 22 ficava com a
  última mensagem para sempre. Passam a expirar aos 3 s; o undo e o aviso de
  transbordo ficam, como a §4 exige.
- `a` (criar tarefa) passa a **seleccionar a tarefa nova**, em vez de deixar a
  selecção onde estava.

### Notas

- Divergências conhecidas, pré-existentes e não corrigidas aqui: o aviso de
  transbordo não é desenhado a vermelho (a §4 pede-o) e a mensagem da guarda do
  `c` expira aos 3 s enquanto a guarda dura 5 s.

## 1.0.0 — 2026-09-18

Primeira versão. **Reescrita de raiz do `rtodo`** (a CLI antiga, em
`github.com/TiagoRCorreia/rtodo`, que fica **arquivada**): não há código
partilhado entre os dois projectos — o modelo de dados, a persistência e o
import foram refeitos de raiz.

### Adicionado

- TUI em `ratatui`/`crossterm`: lista com prioridade (`H`/`M`/`L`), datas de
  criação e conclusão, painel de detalhe a partir de 96×28, ajuda sobreposta e
  barra de teclas sensível ao modo.
- Fronteira `core`/`tui`: `src/core/` (modelo, persistência, operações,
  import/export) não depende de `ratatui`/`crossterm` — invariante travado por
  teste (`tests/boundary.rs`).
- Persistência atómica: `db.json.tmp` ao lado do `db.json` + `fsync` +
  `rename`, e rotação da geração anterior para `db.json.bak`.
- Lixo com reposição: `d` e `c` movem para `trash` em vez de apagar; `u` repõe
  o último lote **na posição original** e sobrevive a `kill -9`; limite de 100
  entradas com aviso de transbordo; `L` abre a vista do lixo, onde `c` esvazia
  com uma guarda de duas pressões.
- Descrições por tarefa: `Ctrl+E` edita a descrição (que aparece no painel de
  detalhe e na linha 22 quando não há altura para o painel).
- Busca e filtro/ordenação vivos na linha 2: `/` procura (título e descrição),
  `f` cicla o filtro, `s` cicla a ordem; `Esc` limpa ambos.
- Import/export: `x` exporta CSV com cabeçalho, `i` importa CSV **ou** JSON
  (envelope novo, com o lixo, ou array no formato do `rtodo` antigo), com
  dedupe por `id` e relatório do que entrou.
- Import do `db.json` **legado** do `rtodo`: `time` → prioridade, `date` →
  data de criação, `id` novo e `completed_at` a `None` (não se inventa a data
  real de conclusão).
- Caminho da base de dados sobreponível por `--db <caminho>` e por
  `TODO_RATATUI_DB`; por omissão `~/.local/share/todo-ratatui/db.json`.
- Arranque seguro: ficheiro ilegível, JSON inválido, `schema` desconhecido ou
  array legado fazem o programa **recusar arrancar** com uma mensagem que
  nomeia o `db.json` e o `db.json.bak`, sem tocar no ficheiro. Uma falha de
  gravação durante a sessão sai na barra de estado («não gravou … — lista
  intacta»), sem perder o que está em memória.

### Notas

- `due_at` existe no formato em disco e aparece no painel quando vem de um
  import, mas **não é editável na v1**.
- Publicação no crates.io: fora de âmbito (`publish = false`).
