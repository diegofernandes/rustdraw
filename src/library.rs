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
