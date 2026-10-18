# RustDraw

Editor de diagramas no estilo **Excalidraw**, nativo em Rust e **100% offline** — sem navegador, sem servidor, sem conta. Um único executável (~10 MB) com tudo embutido, inclusive a fonte manuscrita Virgil.

![RustDraw](docs/screenshot.png)

## Recursos

- **Formas**: retângulo, losango, elipse, seta, linha, desenho livre e texto
- **Visual "desenhado à mão"**: traços irregulares com os mesmos algoritmos do rough.js (o motor do Excalidraw), determinísticos por `seed`
- **Estilos**: cor de traço e fundo, preenchimento (hachura, hachura cruzada, sólido), espessura, tracejado/pontilhado, "desleixo" (arquiteto/artista/cartunista), bordas arredondadas, opacidade, pontas de seta (seta, triângulo, barra, ponto), fonte (à mão/normal/código), tamanho e alinhamento
- **Edição**: seleção por clique ou caixa, mover, redimensionar (inclusive girado), girar (Shift = passos de 15°), editar pontos de linhas, linhas ponto a ponto, duplicar (Ctrl+D ou Alt+arrastar), agrupar, camadas, borracha, desfazer/refazer
- **Menu do botão direito**: como no Excalidraw, um menu para o canvas vazio (colar, copiar como PNG/SVG, selecionar tudo, destravar tudo, grade, modo zen, visualização, estatísticas) e outro para elementos (recortar, copiar, colar, estilos, agrupar, camadas, inverter, duplicar, travar, apagar), com os atalhos ao lado de cada item
- **Setas que grudam nas formas**: a ponta da seta liga num retângulo, losango, elipse ou texto e acompanha a forma quando ela é movida, redimensionada ou girada. Perto do meio de um lado, encaixa nesse **ponto fixo**; no resto da forma fica **livre**, mirando o centro e parando no contorno. Segure `Ctrl` (`Cmd` no macOS) ao arrastar para não grudar; mover só a seta a solta
- **Frames**: áreas nomeadas (ferramenta `F`) para montar várias telas ou slides num desenho só. O que fica dentro do frame é recortado pela borda dele e anda junto quando ele é movido; arrastar um elemento para dentro ou para fora de um frame o coloca ou tira dele. Duplo clique no nome (ou `Enter` com o frame selecionado) renomeia. Apagar ou duplicar o frame leva o conteúdo junto. Para exportar um frame sozinho, selecione-o e use "Exportar frame…" no botão direito (ou marque "Somente o frame selecionado" na exportação): sai só o conteúdo, do tamanho exato do frame
- **Compatível com `.excalidraw`**: abre e salva o mesmo JSON do excalidraw.com, preservando campos que o RustDraw não conhece. Ctrl+C gera o formato da área de transferência do Excalidraw — dá para colar direto lá
- **Exportação**: PNG (1×/2×/3×, com/sem fundo, modo escuro) e SVG autocontido (com a fonte embutida), com pré-visualização
- **Conforto**: tema escuro, grade com encaixe, zoom no cursor, salvamento automático (o desenho volta ao reabrir), arrastar arquivo para a janela

## Baixar

Binários prontos para **Linux** (x86_64), **Windows** (x86_64) e **macOS** (universal: Apple Silicon e Intel) ficam na página de [Releases](https://github.com/diegofernandes/rustdraw/releases). Basta extrair e rodar `rustdraw` (`rustdraw.exe` no Windows).

> No macOS o binário não é assinado: na primeira vez, clique com o botão direito → Abrir, ou rode `xattr -d com.apple.quarantine rustdraw`.

Para publicar uma nova versão, basta criar a tag (`git tag v0.2.0 && git push origin v0.2.0`, ou pela tela de Releases do GitHub). O workflow `Release` usa a versão da tag no build, compila as três plataformas e publica a release com os pacotes e o `SHA256SUMS.txt`. Não é preciso editar o `Cargo.toml` antes; ele pode ser atualizado depois, quando for conveniente.

## Compilar do código-fonte

Pré-requisito: [Rust](https://rustup.rs) 1.95 ou mais recente.

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
rustdraw export slides.excalidraw tela1.png --frame "Tela 1"   # só um frame
```

`examples/frames.excalidraw` tem um exemplo com dois frames.

## Servidor MCP (para assistentes de IA)

`rustdraw mcp` é um servidor [MCP](https://modelcontextprotocol.io) local: um assistente como o Claude Desktop ou o Claude Code consegue ler, desenhar, editar e exportar arquivos `.excalidraw` pelo RustDraw. Ele conversa só pelo stdin/stdout do processo que o cliente inicia, sem abrir porta nem usar a rede.

```bash
claude mcp add rustdraw -- /caminho/para/rustdraw mcp            # Claude Code
claude mcp add rustdraw -- /caminho/para/rustdraw mcp ~/d.excalidraw   # com arquivo padrão
```

No Claude Desktop, em `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "rustdraw": { "command": "/caminho/para/rustdraw", "args": ["mcp"] }
  }
}
```

| Ferramenta | O que faz |
|---|---|
| `new_scene` | Cria um desenho vazio |
| `get_scene` | Lista os elementos (id, tipo, posição, tamanho, texto, estilo, ligações) |
| `add_elements` | Adiciona formas, textos, linhas e setas em lote; setas ligam em formas pelo id (`start`/`end`) e formas aceitam um rótulo (`label`) |
| `update_elements` | Muda posição, tamanho, estilo ou texto; setas ligadas e rótulos acompanham |
| `delete_elements` | Apaga elementos e solta as setas ligadas a eles |
| `export_image` | Grava PNG ou SVG (do desenho todo ou de um frame, em `frame`) |
| `preview` | Devolve um PNG do desenho (ou de um frame) para o assistente conferir o resultado |

Cada ferramenta recebe o caminho do arquivo em `file` (opcional se um arquivo padrão foi passado na linha de comando), lê, aplica a mudança e grava. Um lote com erro não grava nada. Se o mesmo arquivo estiver aberto no editor, reabra-o para ver as mudanças.

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
| Manter ferramenta | `Q` | | Alinhar | `Ctrl+Shift+setas` |
| Trocar tipo da forma | `Tab` / `Shift+Tab` | | Inverter | `Shift+H` / `Shift+V` |
| Cor do traço / fundo | `S` / `G` | | Copiar / colar estilos | `Ctrl+Alt+C` / `Ctrl+Alt+V` |
| Modo zen | `Alt+Z` | | Copiar como PNG | `Shift+Alt+C` |
| Modo de visualização | `Alt+R` | | Travar / destravar | `Ctrl+Shift+L` |
| Estatísticas | `Alt+/` | | Tamanho da fonte | `Ctrl+Shift+<` / `>` |
| Ajustar à seleção | `Shift+2` / `Shift+3` | | Limpar o canvas | `Ctrl+Delete` |
| Frame | `F` | | Renomear frame | `Enter` / duplo clique no nome |

Os atalhos seguem os do [Excalidraw](https://excalidraw.com) (diálogo `?`). No macOS, `Ctrl` vira `Cmd`. O botão direito abre o menu de opções.

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
| `src/frame.rs` | Frames: quem está dentro de qual, recorte, nomes |
| `src/fonts.rs` | Fontes embutidas |
| `src/edit.rs` | Edição da cena por comandos JSON (criar, alterar, apagar, rótulos, setas ligadas) |
| `src/mcp.rs` | Servidor MCP por stdio (JSON-RPC) |

As três saídas (tela, PNG e SVG) usam as mesmas primitivas geradas em `geom.rs`, então o que se vê é o que se exporta.

```bash
cargo test   # modelo/round-trip, geometria, rough, exportação, redimensionamento, ligação de setas, edição por comandos, MCP
```

## Limitações atuais

Ainda não há: imagens e texto dentro de contêiner. Elementos desses tipos vindos do Excalidraw são preservados ao salvar e aparecem como uma moldura tracejada.

## Créditos

- Inspirado no [Excalidraw](https://github.com/excalidraw/excalidraw) (MIT) e no [rough.js](https://github.com/rough-stuff/rough) (MIT)
- Fonte **Virgil** © Ellinor Rapp, sob a SIL Open Font License 1.1 — ver [`assets/fonts/Virgil-LICENSE.md`](assets/fonts/Virgil-LICENSE.md). Distribuída sem modificações (WOFF2 original)
- Interface com [egui/eframe](https://github.com/emilk/egui)
