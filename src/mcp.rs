//! Servidor MCP (Model Context Protocol) local.
//!
//! `rustdraw mcp [arquivo.excalidraw]` deixa um assistente de IA (Claude
//! Desktop, Claude Code, ...) ler, editar e exportar desenhos: o cliente inicia
//! o processo e conversa pelo stdin/stdout, uma mensagem JSON-RPC por linha.
//! Com `file`, cada ferramenta lê o arquivo, aplica a mudança e grava de novo;
//! sem ele, a chamada vai para o editor aberto (veja `live`). O editor também
//! atende MCP por HTTP com este mesmo `Server` (`Server::for_editor`).

use crate::edit;
use crate::export::{self, ExportOpts};
use crate::fonts;
use crate::frame;
use crate::live;
use crate::model::SceneFile;
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// Lado máximo (pixels) da imagem devolvida por `preview`.
const PREVIEW_MAX_PX: f32 = 1600.0;

const INSTRUCTIONS: &str = "Edita desenhos .excalidraw do RustDraw (editor no estilo Excalidraw). \
Use get_scene para ver os elementos e seus ids, add_elements para criar formas, textos e setas \
(setas ligam em formas com start/end {\"element\": id}), update_elements e delete_elements para \
mudar, e preview para conferir o resultado como imagem. Imagens entram com add_elements \
{\"type\": \"image\", \"path\": arquivo PNG/JPG}. Coordenadas em pixels, y cresce para baixo. \
Sem `file`, as ferramentas agem no desenho aberto no editor RustDraw (se ele permitir), e o usuário \
vê cada mudança na hora.";

#[derive(Clone)]
pub struct Server {
    default_file: Option<PathBuf>,
    /// Como este processo se apresenta ao editor aberto.
    client: live::Client,
    /// Dentro do editor (MCP por HTTP): chamadas sem `file` vão direto para ele.
    editor: Option<live::Editor>,
}

impl Server {
    pub fn new(default_file: Option<PathBuf>) -> Self {
        Self {
            default_file,
            client: live::Client {
                name: "assistente sem nome".into(),
                session: crate::model::new_id(),
            },
            editor: None,
        }
    }

    /// Servidor de uma sessão MCP por HTTP, dentro do editor.
    pub fn for_editor(editor: live::Editor, client: live::Client) -> Self {
        Self {
            default_file: None,
            client,
            editor: Some(editor),
        }
    }
}

/// Lê mensagens do stdin até ele fechar.
pub fn run(default_file: Option<PathBuf>) -> std::io::Result<()> {
    let mut server = Server::new(default_file);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(resp) = server.handle_line(&line) {
            writeln!(out, "{resp}")?;
            out.flush()?;
        }
    }
    Ok(())
}

fn rpc_error(id: Value, code: i64, msg: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg.into()}})
}

fn text_result(v: Value, is_error: bool) -> Value {
    let text = match v {
        Value::String(s) => s,
        v => v.to_string(),
    };
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

impl Server {
    /// Responde a uma linha (mensagem ou lote). `None` para notificações.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        match serde_json::from_str::<Value>(line) {
            Err(e) => Some(rpc_error(
                Value::Null,
                -32700,
                format!("JSON inválido: {e}"),
            )),
            Ok(Value::Array(batch)) => {
                let out: Vec<Value> = batch.iter().filter_map(|m| self.handle(m)).collect();
                (!out.is_empty()).then_some(Value::Array(out))
            }
            Ok(msg) => self.handle(&msg),
        }
    }

    fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            // respostas a pedidos nossos (não fazemos nenhum) são ignoradas
            if msg.get("result").is_some() || msg.get("error").is_some() {
                return None;
            }
            return Some(rpc_error(
                id.unwrap_or(Value::Null),
                -32600,
                "pedido inválido",
            ));
        };
        let id = id?; // notificação: nada a responder
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": self.tools()})),
            "tools/call" => self.call(&params),
            "resources/list" => Ok(json!({"resources": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            _ => Err((-32601, format!("método desconhecido: {method}"))),
        };
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, m)) => rpc_error(id, code, m),
        })
    }

    /// As ferramentas. No editor elas só agem no desenho aberto, então o
    /// parâmetro `file` sai da lista.
    fn tools(&self) -> Value {
        let mut list = tools();
        if self.editor.is_some()
            && let Some(tools) = list.as_array_mut()
        {
            for t in tools {
                if let Some(props) = t["inputSchema"]["properties"].as_object_mut() {
                    props.remove("file");
                    props.remove("overwrite");
                }
            }
        }
        list
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let info = &params["clientInfo"];
        // só o nome, sem a versão: "Sempre permitir" vale também depois de atualizar
        if let Some(name) = info["title"].as_str().or(info["name"].as_str()) {
            self.client.name = name.to_string();
        }
        let asked = params.get("protocolVersion").and_then(|v| v.as_str());
        let version = asked
            .filter(|v| PROTOCOL_VERSIONS.contains(v))
            .unwrap_or(PROTOCOL_VERSIONS[0]);
        let mut instructions = INSTRUCTIONS.to_string();
        if let Some(f) = &self.default_file {
            instructions.push_str(&format!(
                " Arquivo padrão: {} (o parâmetro `file` pode ser omitido).",
                f.display()
            ));
        }
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "rustdraw", "version": env!("CARGO_PKG_VERSION")},
            "instructions": instructions,
        })
    }

    fn call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or((-32602, "falta `name`".to_string()))?;
        if !TOOLS.contains(&name) {
            return Err((-32602, format!("ferramenta desconhecida: {name}")));
        }
        let empty = Map::new();
        let args = params
            .get("arguments")
            .and_then(|a| a.as_object())
            .unwrap_or(&empty);
        // dentro do editor, mexer em arquivos passaria por fora da aprovação
        // do usuário: só o desenho aberto (que pede aprovação) é atendido
        if self.editor.is_some() && self.target(args).is_some() {
            return Ok(text_result(json!(FILE_IN_EDITOR), true));
        }
        Ok(match self.target(args) {
            Some(path) => self.call_file(name, args, &path),
            None => match match &self.editor {
                Some(e) => e.call(&self.client, name, args),
                None => live::call(&self.client, name, args),
            } {
                Ok(r) => r,
                Err(live::CallError::Failed(e)) => text_result(json!(e), true),
                Err(live::CallError::NotRunning) => text_result(json!(NO_TARGET), true),
            },
        })
    }

    /// Arquivo a editar: o do parâmetro `file` ou o padrão. `None` = o editor aberto.
    fn target(&self, args: &Map<String, Value>) -> Option<PathBuf> {
        match args.get("file").and_then(|f| f.as_str()) {
            Some(f) if !f.is_empty() => Some(PathBuf::from(f)),
            _ => self.default_file.clone(),
        }
    }

    /// Carrega o arquivo, executa a ferramenta (tudo ou nada) e grava se mudou.
    fn call_file(&self, name: &str, args: &Map<String, Value>, path: &Path) -> Value {
        let out = if name == "new_scene" {
            new_scene_file(args, path).map(|v| text_result(v, false))
        } else {
            edit::load(path).and_then(|mut scene| {
                let source = path.display().to_string();
                let (r, changed) = run_tool(name, args, &mut scene, &source)?;
                if changed {
                    edit::save(path, &scene)?;
                }
                Ok(r)
            })
        };
        out.unwrap_or_else(|e| text_result(json!(e), true))
    }
}

/// `file` mandado ao endereço MCP do editor.
const FILE_IN_EDITOR: &str = "O endereço MCP do RustDraw edita só o desenho aberto no editor \
(com a aprovação do usuário): tire o parâmetro `file`. Para editar um arquivo .excalidraw \
direto, use o servidor `rustdraw mcp`.";

/// Sem arquivo e sem editor aberto com a opção ligada.
const NO_TARGET: &str = "Nenhum desenho para editar: informe `file` (caminho do .excalidraw) \
ou abra o RustDraw e ligue \"Permitir edição por IA\" no menu.";

const TOOLS: [&str; 7] = [
    "new_scene",
    "get_scene",
    "add_elements",
    "update_elements",
    "delete_elements",
    "export_image",
    "preview",
];

fn new_scene_file(args: &Map<String, Value>, path: &Path) -> Result<Value, String> {
    let overwrite = args.get("overwrite").and_then(|v| v.as_bool()) == Some(true);
    if path.exists() && !overwrite {
        return Err(format!(
            "{} já existe (use overwrite: true para substituir)",
            path.display()
        ));
    }
    let bg = args
        .get("background")
        .and_then(|v| v.as_str())
        .unwrap_or("#ffffff");
    edit::save(path, &SceneFile::new(vec![], bg, false))?;
    Ok(json!({"created": path.display().to_string()}))
}

/// Executa uma ferramenta sobre uma cena em memória (de um arquivo ou do editor
/// aberto). Devolve o resultado do `tools/call` e se a cena mudou; num erro a
/// cena fica como estava. `source` diz de onde a cena veio.
pub fn run_tool(
    name: &str,
    args: &Map<String, Value>,
    scene: &mut SceneFile,
    source: &str,
) -> Result<(Value, bool), String> {
    let changed = matches!(
        name,
        "new_scene" | "add_elements" | "update_elements" | "delete_elements"
    );
    let v = match name {
        "new_scene" => {
            let bg = args
                .get("background")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| scene.view_background())
                .unwrap_or_else(|| "#ffffff".into());
            *scene = SceneFile::new(vec![], &bg, scene.grid_enabled());
            json!({"cleared": source})
        }
        "get_scene" => get_scene(args, scene, source),
        "add_elements" => modify(scene, args, "elements", |els, files, v| {
            let ids = edit::add(els, files, list(v)?)?;
            Ok(json!({"added": ids}))
        })?,
        "update_elements" => modify(scene, args, "elements", |els, _, v| {
            let ids = edit::update(els, list(v)?)?;
            Ok(json!({"updated": ids}))
        })?,
        "delete_elements" => modify(scene, args, "ids", |els, _, v| {
            let ids: Vec<String> = serde_json::from_value(v.clone())
                .map_err(|_| "`ids` precisa ser uma lista de ids".to_string())?;
            let n = edit::delete(els, &ids)?;
            Ok(json!({"deleted": n}))
        })?,
        "export_image" => export_image(args, scene)?,
        "preview" => return Ok((preview(args, scene)?, false)),
        _ => return Err(format!("ferramenta desconhecida: {name}")),
    };
    Ok((text_result(v, false), changed))
}

fn get_scene(args: &Map<String, Value>, scene: &SceneFile, source: &str) -> Value {
    if args.get("raw").and_then(|v| v.as_bool()) == Some(true) {
        return json!(scene.to_json());
    }
    let els = &scene.elements;
    let bounds = export::bounds_with_text(els).map(|b| {
        json!({"x": b.min.x.round(), "y": b.min.y.round(),
               "width": b.width().round(), "height": b.height().round()})
    });
    json!({
        "file": source,
        "viewBackgroundColor": scene.view_background(),
        "bounds": bounds,
        "elements": els.iter().map(|e| edit::summary(e, els)).collect::<Vec<_>>(),
    })
}

/// Aplica `f` numa cópia dos elementos e só a mantém se der certo.
fn modify(
    scene: &mut SceneFile,
    args: &Map<String, Value>,
    key: &str,
    f: impl FnOnce(
        &mut Vec<crate::model::Element>,
        &mut Map<String, Value>,
        &Value,
    ) -> Result<Value, String>,
) -> Result<Value, String> {
    let v = args.get(key).ok_or_else(|| format!("falta `{key}`"))?;
    let mut els = scene.elements.clone();
    let mut files = scene.files_map();
    let out = f(&mut els, &mut files, v)?;
    scene.elements = els;
    scene.set_files(&files);
    Ok(out)
}

fn opts(scene: &SceneFile, args: &Map<String, Value>, scale: f32) -> ExportOpts {
    let mut o = ExportOpts {
        scale: args
            .get("scale")
            .and_then(|v| v.as_f64())
            .map_or(scale, |s| s as f32),
        dark: args.get("dark").and_then(|v| v.as_bool()) == Some(true),
        background: args.get("background").and_then(|v| v.as_bool()) != Some(false),
        ..Default::default()
    };
    if let Some(bg) = scene.view_background() {
        o.view_background = bg;
    }
    o.frame = args
        .get("frame")
        .and_then(|v| v.as_str())
        .map(|f| frame::find(&scene.elements, f).unwrap_or_else(|| f.to_string()));
    o.files = scene.files_map();
    o
}

fn export_image(args: &Map<String, Value>, scene: &SceneFile) -> Result<Value, String> {
    let output = args
        .get("output")
        .and_then(|v| v.as_str())
        .ok_or("falta `output` (caminho .png ou .svg)")?;
    let opts = opts(scene, args, 2.0);
    let lower = output.to_lowercase();
    let data = if lower.ends_with(".svg") {
        export::render_svg(&scene.elements, &opts)?.into_bytes()
    } else if lower.ends_with(".png") {
        export::render_png(&scene.elements, &opts)?
    } else {
        return Err("`output` precisa terminar em .png ou .svg".into());
    };
    let out = Path::new(output);
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(out, &data).map_err(|e| format!("{output}: {e}"))?;
    Ok(json!({"written": output, "bytes": data.len()}))
}

fn preview(args: &Map<String, Value>, scene: &SceneFile) -> Result<Value, String> {
    let mut opts = opts(scene, args, 1.0);
    let b = export::export_bounds(&scene.elements, &opts).ok_or("nada para mostrar")?;
    let side = (b.width().max(b.height()) + opts.padding * 2.0) * opts.scale;
    if side > PREVIEW_MAX_PX {
        opts.scale *= PREVIEW_MAX_PX / side;
    }
    let png = export::render_png(&scene.elements, &opts)?;
    Ok(
        json!({"content": [{"type": "image", "data": fonts::base64(&png), "mimeType": "image/png"}]}),
    )
}

fn list(v: &Value) -> Result<&[Value], String> {
    v.as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| "precisa ser uma lista de objetos".into())
}

fn file_prop() -> Value {
    json!({"type": "string", "description": "Caminho do arquivo .excalidraw (de preferência absoluto). Sem ele, edita o desenho aberto no RustDraw (com \"Permitir edição por IA\" ligado no menu)."})
}

/// Campos aceitos para criar/alterar um elemento.
fn element_schema(creating: bool) -> Value {
    let mut props = json!({
        "id": {"type": "string", "description": if creating {
            "Id opcional (único). Útil para ligar setas a formas criadas no mesmo pedido."
        } else {
            "Id do elemento a alterar."
        }},
        "type": {"type": "string", "enum": ["rectangle", "diamond", "ellipse", "text", "line", "arrow", "freedraw", "image"]},
        "path": {"type": "string", "description": "Imagem: caminho de um PNG, JPG, GIF ou WebP no disco; é embutido no arquivo. Sem width/height usa o tamanho da imagem (até 1440 px); com um só deles mantém a proporção."},
        "fileId": {"type": "string", "description": "Imagem: reaproveita uma imagem que já está na cena (em vez de `path`)."},
        "x": {"type": "number", "description": "Canto superior esquerdo (texto: ponto de ancoragem conforme textAlign; linhas: origem de `points`)."},
        "y": {"type": "number"},
        "width": {"type": "number", "description": "Formas. Padrão 160."},
        "height": {"type": "number", "description": "Formas. Padrão 80."},
        "angle": {"type": "number", "description": "Rotação em graus."},
        "label": {"type": ["string", "null"], "description": "Texto centralizado dentro de retângulo, losango ou elipse. null ou \"\" remove."},
        "text": {"type": "string", "description": "Conteúdo de um elemento `text` (\\n quebra linha)."},
        "fontSize": {"type": "number", "description": "Padrão 20."},
        "fontFamily": {"description": "hand (1, padrão), normal (2) ou code (3).", "oneOf": [{"type": "string", "enum": ["hand", "normal", "code"]}, {"type": "integer"}]},
        "textAlign": {"type": "string", "enum": ["left", "center", "right"]},
        "points": {"type": "array", "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                   "description": "Linhas, setas e desenho livre: pontos relativos a (x, y), ex. [[0,0],[200,0]]."},
        "start": {"type": ["object", "null"], "description": "Seta/linha: início em {\"x\",\"y\"} ou ligado a uma forma com {\"element\": id, \"anchor\": center|top|right|bottom|left}. null solta.",
                  "properties": {"element": {"type": "string"}, "anchor": {"type": "string", "enum": ["center", "top", "right", "bottom", "left"]}, "x": {"type": "number"}, "y": {"type": "number"}}},
        "end": {"type": ["object", "null"], "description": "Como `start`, para a ponta final."},
        "startArrowhead": {"type": ["string", "null"], "enum": ["arrow", "triangle", "bar", "dot", null]},
        "endArrowhead": {"type": ["string", "null"], "enum": ["arrow", "triangle", "bar", "dot", null], "description": "Setas têm \"arrow\" no fim por padrão."},
        "strokeColor": {"type": "string", "description": "Cor CSS, ex. #1e1e1e (padrão), #e03131, #2f9e44, #1971c2, #f08c00."},
        "backgroundColor": {"type": "string", "description": "transparent (padrão) ou cor, ex. #ffc9c9, #b2f2bb, #a5d8ff, #ffec99."},
        "fillStyle": {"type": "string", "enum": ["solid", "hachure", "cross-hatch"]},
        "strokeWidth": {"type": "number", "description": "1, 2 (padrão) ou 4."},
        "strokeStyle": {"type": "string", "enum": ["solid", "dashed", "dotted"]},
        "roughness": {"type": "number", "description": "0 arquiteto, 1 artista (padrão), 2 cartunista."},
        "opacity": {"type": "number", "description": "0 a 100."},
        "rounded": {"type": "boolean", "description": "Cantos arredondados (padrão true)."},
        "arrowType": {"type": "string", "enum": ["sharp", "round", "elbow"], "description": "Setas: sharp (reta), round (curva, padrão) ou elbow (cotovelo: só segmentos horizontais e verticais, contornando as formas ligadas)."},
        "locked": {"type": "boolean"},
        "link": {"type": ["string", "null"]},
        "groupIds": {"type": "array", "items": {"type": "string"}}
    });
    let required = if creating {
        json!(["type"])
    } else {
        json!(["id"])
    };
    if !creating {
        let props = props.as_object_mut().unwrap();
        for k in ["type", "path", "fileId"] {
            props.remove(k);
        }
    }
    json!({"type": "object", "properties": props, "required": required})
}

fn tools() -> Value {
    json!([
        {
            "name": "new_scene",
            "description": "Cria um desenho .excalidraw vazio. Sem `file`, limpa o desenho aberto no editor (o usuário pode desfazer).",
            "inputSchema": {"type": "object", "properties": {
                "file": file_prop(),
                "background": {"type": "string", "description": "Cor de fundo, padrão #ffffff."},
                "overwrite": {"type": "boolean", "description": "Substitui o arquivo se já existir."}
            }}
        },
        {
            "name": "get_scene",
            "description": "Lista os elementos do desenho (id, tipo, posição, tamanho, texto, estilo, ligações das setas, rótulos) e a área ocupada.",
            "inputSchema": {"type": "object", "properties": {
                "file": file_prop(),
                "raw": {"type": "boolean", "description": "Devolve o JSON completo do arquivo."}
            }}
        },
        {
            "name": "add_elements",
            "description": "Adiciona elementos, na ordem dada e por cima dos existentes. Devolve os ids criados. Exemplo: [{\"type\":\"rectangle\",\"id\":\"api\",\"x\":0,\"y\":0,\"label\":\"API\"},{\"type\":\"ellipse\",\"id\":\"db\",\"x\":300,\"y\":0,\"label\":\"Banco\"},{\"type\":\"arrow\",\"start\":{\"element\":\"api\"},\"end\":{\"element\":\"db\"}}].",
            "inputSchema": {"type": "object", "properties": {
                "file": file_prop(),
                "elements": {"type": "array", "items": element_schema(true)}
            }, "required": ["elements"]}
        },
        {
            "name": "update_elements",
            "description": "Altera elementos pelo id; só os campos informados mudam. Setas ligadas e rótulos acompanham as formas.",
            "inputSchema": {"type": "object", "properties": {
                "file": file_prop(),
                "elements": {"type": "array", "items": element_schema(false)}
            }, "required": ["elements"]}
        },
        {
            "name": "delete_elements",
            "description": "Apaga elementos pelo id (o rótulo de uma forma sai junto; setas ligadas ficam soltas).",
            "inputSchema": {"type": "object", "properties": {
                "file": file_prop(),
                "ids": {"type": "array", "items": {"type": "string"}}
            }, "required": ["ids"]}
        },
        {
            "name": "export_image",
            "description": "Exporta o desenho para PNG ou SVG (pela extensão de `output`).",
            "inputSchema": {"type": "object", "properties": {
                "file": file_prop(),
                "output": {"type": "string", "description": "Caminho de saída .png ou .svg."},
                "scale": {"type": "number", "description": "Escala do PNG, padrão 2."},
                "dark": {"type": "boolean", "description": "Modo escuro."},
                "background": {"type": "boolean", "description": "Com fundo (padrão true)."},
                "frame": {"type": "string", "description": "Exporta só o conteúdo deste frame (nome ou id)."}
            }, "required": ["output"]}
        },
        {
            "name": "preview",
            "description": "Devolve uma imagem PNG do desenho, para conferir o resultado.",
            "inputSchema": {"type": "object", "properties": {
                "file": file_prop(),
                "scale": {"type": "number", "description": "Padrão 1 (a imagem é limitada a 1600 px)."},
                "dark": {"type": "boolean"},
                "frame": {"type": "string", "description": "Mostra só este frame (nome ou id)."}
            }}
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(s: &mut Server, id: i64, name: &str, args: Value) -> Value {
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                         "params": {"name": name, "arguments": args}});
        s.handle_line(&msg.to_string()).unwrap()["result"].clone()
    }

    #[test]
    fn handshake_and_listing() {
        let mut s = Server::new(None);
        let r = s
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#)
            .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "rustdraw");
        assert!(
            s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
        let r = s
            .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .unwrap();
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 7);
        let r = s
            .handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"nada"}"#)
            .unwrap();
        assert_eq!(r["error"]["code"], -32601);
        let r = s.handle_line("{oops").unwrap();
        assert_eq!(r["error"]["code"], -32700);
    }

    #[test]
    fn edit_and_export_a_file() {
        let dir = std::env::temp_dir().join(format!("rustdraw-mcp-{}", crate::model::new_id()));
        let file = dir.join("d.excalidraw");
        let mut s = Server::new(Some(file.clone()));
        let r = call(&mut s, 1, "new_scene", json!({}));
        assert_eq!(r["isError"], false, "{r}");
        let r = call(
            &mut s,
            2,
            "add_elements",
            json!({"elements": [
                {"type": "rectangle", "id": "a", "label": "A"},
                {"type": "diamond", "id": "b", "x": 300},
                {"type": "arrow", "start": {"element": "a"}, "end": {"element": "b"}}
            ]}),
        );
        assert_eq!(r["isError"], false, "{r}");
        // um erro no meio do lote não grava nada
        let r = call(
            &mut s,
            3,
            "add_elements",
            json!({"elements": [{"type": "text", "text": "x"}, {"type": "nope"}]}),
        );
        assert_eq!(r["isError"], true);
        let r = call(&mut s, 4, "get_scene", json!({}));
        let scene: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(scene["elements"].as_array().unwrap().len(), 4);
        assert_eq!(scene["elements"][0]["label"], "A");
        let svg = dir.join("out.svg");
        let r = call(
            &mut s,
            5,
            "export_image",
            json!({"output": svg.to_str().unwrap()}),
        );
        assert_eq!(r["isError"], false, "{r}");
        assert!(std::fs::read_to_string(&svg).unwrap().starts_with("<svg"));
        let r = call(&mut s, 6, "preview", json!({}));
        assert_eq!(r["content"][0]["mimeType"], "image/png");
        let r = call(&mut s, 7, "new_scene", json!({}));
        assert_eq!(r["isError"], true, "não sobrescreve sem overwrite");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_tool_in_memory() {
        let mut scene = SceneFile::new(vec![], "#ffffff", false);
        let args = json!({"elements": [{"type": "ellipse", "id": "e"}]});
        let (r, changed) =
            run_tool("add_elements", args.as_object().unwrap(), &mut scene, "app").unwrap();
        assert!(changed, "{r}");
        assert_eq!(scene.elements.len(), 1);
        let (r, changed) = run_tool("get_scene", &Map::new(), &mut scene, "app").unwrap();
        assert!(!changed);
        assert!(
            r["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"file\":\"app\"")
        );
        let bad = json!({"ids": ["x"]});
        assert!(
            run_tool(
                "delete_elements",
                bad.as_object().unwrap(),
                &mut scene,
                "app"
            )
            .is_err()
        );
        assert_eq!(scene.elements.len(), 1, "erro não muda a cena");
        let (_, changed) = run_tool("new_scene", &Map::new(), &mut scene, "app").unwrap();
        assert!(changed && scene.elements.is_empty());
    }
}
