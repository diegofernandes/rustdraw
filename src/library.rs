//! Biblioteca de formas, compatível com o formato `.excalidrawlib` do Excalidraw.
//!
//! Cada item é um conjunto de elementos (uma forma ou um grupo) guardado para
//! ser reutilizado. A biblioteca fica num arquivo local, no diretório de dados
//! do usuário, e não depende do site de bibliotecas do Excalidraw.

use crate::model::{Element, new_id, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub const EXT: &str = "excalidrawlib";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LibraryItem {
    #[serde(default = "new_id")]
    pub id: String,
    #[serde(default = "d_status")]
    pub status: String,
    #[serde(default)]
    pub elements: Vec<Element>,
    #[serde(default = "now_ms")]
    pub created: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Campos desconhecidos, preservados ao salvar de novo.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn d_status() -> String {
    "unpublished".into()
}

impl LibraryItem {
    /// Novo item a partir de elementos do desenho (como "Adicionar à biblioteca").
    pub fn new(elements: Vec<Element>) -> Self {
        Self {
            id: new_id(),
            status: d_status(),
            elements,
            created: now_ms(),
            name: None,
            extra: Map::new(),
        }
    }

    /// Mesmo critério do Excalidraw (`isUniqueItem`): mesmos elementos, na
    /// mesma ordem, com o mesmo `id` e `versionNonce`.
    fn same_as(&self, o: &LibraryItem) -> bool {
        self.elements.len() == o.elements.len()
            && self
                .elements
                .iter()
                .zip(&o.elements)
                .all(|(a, b)| a.id == b.id && a.version_nonce == b.version_nonce)
    }
}

fn clean(mut els: Vec<Element>) -> Vec<Element> {
    els.retain(|e| !e.is_deleted);
    els
}

/// Lê um `.excalidrawlib` (versão 1, com `library`, ou 2, com `libraryItems`).
pub fn parse(text: &str) -> Result<Vec<LibraryItem>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("JSON inválido: {e}"))?;
    if v.get("type").and_then(|t| t.as_str()) != Some("excalidrawlib") {
        return Err("não é um arquivo de biblioteca (.excalidrawlib)".into());
    }
    let list = v
        .get("libraryItems")
        .or_else(|| v.get("library"))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    let mut items = vec![];
    for raw in list {
        let item = if raw.is_array() {
            // formato antigo: cada item é só a lista de elementos
            let els: Vec<Element> = serde_json::from_value(raw)
                .map_err(|e| format!("item da biblioteca inválido: {e}"))?;
            LibraryItem::new(els)
        } else {
            serde_json::from_value::<LibraryItem>(raw)
                .map_err(|e| format!("item da biblioteca inválido: {e}"))?
        };
        let item = LibraryItem {
            elements: clean(item.elements),
            ..item
        };
        if !item.elements.is_empty() {
            items.push(item);
        }
    }
    Ok(items)
}

/// Serializa no formato da versão 2, igual ao "Exportar biblioteca" do Excalidraw.
pub fn to_json(items: &[LibraryItem]) -> String {
    serde_json::to_string_pretty(&json!({
        "type": "excalidrawlib",
        "version": 2,
        "source": "https://github.com/diegofernandes/rustdraw",
        "libraryItems": items,
    }))
    .unwrap_or_default()
}

/// Junta `incoming` à biblioteca: os itens novos vão para o começo e os que já
/// existem são ignorados. Devolve quantos foram adicionados.
pub fn merge(items: &mut Vec<LibraryItem>, incoming: Vec<LibraryItem>) -> usize {
    let mut new: Vec<LibraryItem> = vec![];
    for it in incoming {
        if !items.iter().chain(&new).any(|o| o.same_as(&it)) {
            new.push(it);
        }
    }
    let n = new.len();
    new.append(items);
    *items = new;
    n
}

/// Move o item `id` para a posição `to` (índice na lista final).
pub fn move_item(items: &mut Vec<LibraryItem>, id: &str, to: usize) -> bool {
    let Some(from) = items.iter().position(|i| i.id == id) else {
        return false;
    };
    let it = items.remove(from);
    items.insert(to.min(items.len()), it);
    from != to.min(items.len() - 1)
}

/// Move o item `id` para logo antes de `before` (que fica depois dele).
pub fn move_before(items: &mut Vec<LibraryItem>, id: &str, before: &str) -> bool {
    if id == before {
        return false;
    }
    let Some(from) = items.iter().position(|i| i.id == id) else {
        return false;
    };
    let it = items.remove(from);
    let to = items
        .iter()
        .position(|i| i.id == before)
        .unwrap_or(items.len());
    items.insert(to, it);
    from != to
}

/// Renomeia; um nome vazio remove o nome.
pub fn rename(items: &mut [LibraryItem], id: &str, name: &str) -> bool {
    let Some(it) = items.iter_mut().find(|i| i.id == id) else {
        return false;
    };
    let name = name.trim();
    let new = (!name.is_empty()).then(|| name.to_string());
    let changed = it.name != new;
    it.name = new;
    changed
}

/// Troca os elementos de um item (edição no canvas), mantendo id, nome e data.
pub fn replace_elements(items: &mut [LibraryItem], id: &str, els: Vec<Element>) -> bool {
    let els = clean(els);
    match items.iter_mut().find(|i| i.id == id) {
        Some(it) if !els.is_empty() => {
            it.elements = els;
            true
        }
        _ => false,
    }
}

/// Onde a biblioteca pessoal fica salva.
pub fn path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("rustdraw").join(format!("biblioteca.{EXT}")))
}

pub fn load(p: &Path) -> Result<Vec<LibraryItem>, String> {
    let text = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    parse(&text)
}

/// Grava de forma atômica (arquivo temporário + renomear).
pub fn save(p: &Path, items: &[LibraryItem]) -> Result<(), String> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, to_json(items)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

/// O texto parece um `.excalidrawlib` (para abrir/arrastar sem depender da extensão).
pub fn is_library(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("type")?.as_str().map(|t| t == "excalidrawlib"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Style;

    #[test]
    fn reads_v1_and_v2() {
        let v1 = r#"{"type":"excalidrawlib","version":1,"library":[
            [{"id":"a","type":"rectangle","x":0,"y":0,"width":10,"height":10}],
            [{"id":"b","type":"ellipse","x":0,"y":0,"width":5,"height":5,"isDeleted":true}]
        ]}"#;
        let items = parse(v1).unwrap();
        assert_eq!(
            items.len(),
            1,
            "item só com elementos apagados é descartado"
        );
        assert_eq!(items[0].elements[0].kind, "rectangle");
        assert_eq!(items[0].status, "unpublished");

        let v2 = r#"{"type":"excalidrawlib","version":2,"source":"https://excalidraw.com",
          "libraryItems":[{"id":"x","status":"published","created":5,"name":"Caixa",
            "elements":[{"id":"a","type":"rectangle","x":0,"y":0,"width":10,"height":10}]}]}"#;
        let items = parse(v2).unwrap();
        assert_eq!(items[0].id, "x");
        assert_eq!(items[0].name.as_deref(), Some("Caixa"));
        assert_eq!(items[0].created, 5);

        assert!(parse(r#"{"type":"excalidraw","elements":[]}"#).is_err());
        assert!(is_library(v2) && !is_library("{}"));
    }

    #[test]
    fn roundtrip_and_merge() {
        let r = Element::new("rectangle", 0.0, 0.0, &Style::default());
        let mut items = vec![LibraryItem::new(vec![r.clone()])];
        let back = parse(&to_json(&items)).unwrap();
        assert_eq!(back, items);
        assert!(to_json(&items).contains("\"version\": 2"));

        // o mesmo item não entra duas vezes; um novo vai para o começo
        let e = Element::new("ellipse", 0.0, 0.0, &Style::default());
        let added = merge(
            &mut items,
            vec![LibraryItem::new(vec![r]), LibraryItem::new(vec![e])],
        );
        assert_eq!(added, 1);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].elements[0].kind, "ellipse");
    }

    #[test]
    fn manage_items() {
        let el = |k: &str| Element::new(k, 0.0, 0.0, &Style::default());
        let mut items: Vec<LibraryItem> = ["rectangle", "ellipse", "diamond"]
            .iter()
            .map(|k| LibraryItem::new(vec![el(k)]))
            .collect();
        let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let order = |items: &[LibraryItem]| -> Vec<String> {
            items.iter().map(|i| i.elements[0].kind.clone()).collect()
        };

        assert!(move_item(&mut items, &ids[0], 99));
        assert_eq!(order(&items), ["ellipse", "diamond", "rectangle"]);
        assert!(move_item(&mut items, &ids[0], 0));
        assert!(!move_item(&mut items, &ids[0], 0));
        assert!(move_before(&mut items, &ids[2], &ids[1]));
        assert_eq!(order(&items), ["rectangle", "diamond", "ellipse"]);
        assert!(!move_before(&mut items, &ids[2], &ids[2]));

        assert!(rename(&mut items, &ids[1], "  Nuvem "));
        assert_eq!(items[2].name.as_deref(), Some("Nuvem"));
        assert!(!rename(&mut items, &ids[1], "Nuvem"));
        assert!(rename(&mut items, &ids[1], " "));
        assert_eq!(items[2].name, None);

        let created = items[0].created;
        assert!(replace_elements(
            &mut items,
            &ids[0],
            vec![el("text"), el("arrow")]
        ));
        assert_eq!(items[0].elements.len(), 2);
        assert_eq!(
            (items[0].id.as_str(), items[0].created),
            (ids[0].as_str(), created)
        );
        assert!(!replace_elements(&mut items, &ids[0], vec![]));
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("rustdraw-lib-{}", new_id()));
        let p = dir.join("lib.excalidrawlib");
        let items = vec![LibraryItem::new(vec![Element::new(
            "diamond",
            1.0,
            2.0,
            &Style::default(),
        )])];
        save(&p, &items).unwrap();
        assert_eq!(load(&p).unwrap(), items);
        let _ = std::fs::remove_dir_all(dir);
    }
}
