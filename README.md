# RustDraw

Editor de diagramas no estilo **Excalidraw**, nativo em Rust e **100% offline** — sem navegador, sem servidor, sem conta. Um único executável (~10 MB) com tudo embutido, inclusive a fonte manuscrita Virgil.

![RustDraw](docs/screenshot.png)

## Recursos

- **Formas**: retângulo, losango, elipse, seta, linha, desenho livre e texto
- **Visual "desenhado à mão"**: traços irregulares com os mesmos algoritmos do rough.js (o motor do Excalidraw), determinísticos por `seed`
- **Estilos**: cor de traço e fundo, preenchimento (hachura, hachura cruzada, sólido), espessura, tracejado/pontilhado, "desleixo" (arquiteto/artista/cartunista), bordas arredondadas, opacidade, pontas de seta (seta, triângulo, barra, ponto), fonte (à mão/normal/código), tamanho e alinhamento
- **Edição**: seleção por clique ou caixa, mover, redimensionar (inclusive girado), girar (Shift = passos de 15°), editar pontos de linhas, linhas ponto a ponto, duplicar (Ctrl+D ou Alt+arrastar), agrupar, camadas, borracha, desfazer/refazer
- **Compatível com `.excalidraw`**: abre e salva o mesmo JSON do excalidraw.com, preservando campos que o RustDraw não conhece. Ctrl+C gera o formato da área de transferência do Excalidraw — dá para colar direto lá
- **Exportação**: PNG (1×/2×/3×, com/sem fundo, modo escuro) e SVG autocontido (com a fonte embutida), com pré-visualização
- **Conforto**: tema escuro, grade com encaixe, zoom no cursor, salvamento automático (o desenho volta ao reabrir), arrastar arquivo para a janela

## Instalar e rodar

Pré-requisito: [Rust](https://rustup.rs) 1.85 ou mais recente.

```bash
git clone https://github.com/diegofernandes/rustdraw
cd rustdraw
cargo run --release                         # abre o editor
cargo run --release -- desenho.excalidraw   # abre um arquivo
```

O binário fica em `target/release/rustdraw` (`rustdraw.exe` no Windows) e pode ser copiado para qualquer lugar. Depois de compilado, não precisa de internet.

No Linux, para compilar: `sudo apt install libxkbcommon-dev libgl1-mesa-dev` (Debian/Ubuntu). Os diálogos de abrir/salvar usam o `xdg-desktop-portal` (ou `zenity`); sem eles, "Salvar" grava em `~/Documentos` e você pode abrir arquivos arrastando-os para a janela.

### Exportar pela linha de comando

```bash
rustdraw export entrada.excalidraw saida.png --scale 2
rustdraw export entrada.excalidraw saida.svg --dark --no-bg
```

## Atalhos

| Ferramenta | Tecla | | Ação | Atalho |
|---|---|---|---|---|
| Mão | `H` / Espaço+arrastar | | Desfazer / Refazer | `Ctrl+Z` / `Ctrl+Shift+Z` |
| Seleção | `V` / `1` | | Copiar / Colar | `Ctrl+C` / `Ctrl+V` |
| Retângulo | `R` / `2` | | Duplicar | `Ctrl+D` |
| Losango | `D` / `3` | | Agrupar | `Ctrl+G` / `Ctrl+Shift+G` |
| Elipse | `O` / `4` | | Camadas | `Ctrl+[` `Ctrl+]` |
| Seta | `A` / `5` | | Zoom | `Ctrl+roda`, `Ctrl+±`, `Ctrl+0`, `Shift+1` |
| Linha | `L` / `6` | | Salvar / Abrir | `Ctrl+S` / `Ctrl+O` |
| Desenho livre | `P` / `7` | | Exportar imagem | `Ctrl+Shift+E` |
| Texto | `T` / `8` / duplo clique | | Grade / Tema escuro | `Ctrl+'` / `Alt+Shift+D` |
| Borracha | `E` / `0` | | Todos os atalhos | `?` ou `F1` |

## Arquitetura

| Arquivo | Responsabilidade |
|---|---|
| `src/model.rs` | Elementos e cena no formato `.excalidraw` (serde), cores |
| `src/rough.rs` | Geração de traços irregulares (porta do rough.js) e hachuras |
| `src/geom.rs` | Elementos → primitivas de desenho, caixas, hit-test |
| `src/render.rs` | Desenho das primitivas no canvas (egui) |
| `src/export.rs` | PNG (tiny-skia + fontdue) e SVG |
| `src/app.rs` | Estado do editor, ferramentas, mouse/teclado, histórico, arquivos |
| `src/ui.rs` | Barra de ferramentas, painel de propriedades, menu e janelas |
| `src/fonts.rs` | Fontes embutidas |

As três saídas (tela, PNG e SVG) usam as mesmas primitivas geradas em `geom.rs`, então o que se vê é o que se exporta.

```bash
cargo test   # modelo/round-trip, geometria, rough, exportação, redimensionamento
```

## Limitações atuais

Ainda não há: imagens, frames, setas "grudadas" às formas (bindings) e texto dentro de contêiner. Elementos desses tipos vindos do Excalidraw são preservados ao salvar e aparecem como uma moldura tracejada.

## Créditos

- Inspirado no [Excalidraw](https://github.com/excalidraw/excalidraw) (MIT) e no [rough.js](https://github.com/rough-stuff/rough) (MIT)
- Fonte **Virgil** © Ellinor Rapp, sob a SIL Open Font License 1.1 — ver [`assets/fonts/Virgil-LICENSE.md`](assets/fonts/Virgil-LICENSE.md). Distribuída sem modificações (WOFF2 original)
- Interface com [egui/eframe](https://github.com/emilk/egui)
