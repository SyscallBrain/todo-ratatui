# Histórico de alterações

O formato segue de perto o [Keep a Changelog](https://keepachangelog.com/pt-PT/1.1.0/);
as versões são as do `Cargo.toml` e as etiquetas do repositório (`vX.Y.Z`).

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
