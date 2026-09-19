# Histórico de alterações

O formato segue de perto o [Keep a Changelog](https://keepachangelog.com/pt-PT/1.1.0/);
as versões são as do `Cargo.toml` e as etiquetas do repositório (`vX.Y.Z`).

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
