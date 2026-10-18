//! Frames: áreas nomeadas que agrupam elementos, como os do Excalidraw.
//!
//! Um frame é um elemento `"type": "frame"` com um `name` (ou `null`, que
//! aparece como "Frame"). Os elementos dentro dele guardam o id do frame em
//! `frameId`; eles são recortados pela área do frame, andam junto com ele e
//! podem ser exportados sozinhos.

use crate::geom::{P, Rect, aabb, local_box};
use crate::model::Element;
use std::collections::{HashMap, HashSet};

/// Tamanho do nome (pixels de tela no canvas, unidades de mundo na exportação).
pub const NAME_SIZE: f32 = 14.0;
/// Distância entre o nome e a borda de cima do frame.
pub const NAME_GAP: f32 = 3.0;
/// Raio dos cantos da borda.
pub const RADIUS: f32 = 8.0;
/// Cor do nome no tema claro (o escuro aplica o filtro do Excalidraw).
pub const NAME_COLOR: [u8; 4] = [0x99, 0x99, 0x99, 255];

pub fn is_frame(e: &Element) -> bool {
    matches!(e.kind.as_str(), "frame" | "magicframe")
}

/// Nome exibido (o Excalidraw mostra "Frame" quando `name` é `null`).
pub fn name(e: &Element) -> String {
    match e.str_field("name").map(str::trim) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ if e.kind == "magicframe" => "AI Frame".into(),
        _ => "Frame".into(),
    }
}

/// Id do frame com este nome (ou id), para exportar pela linha de comando.
pub fn find(els: &[Element], name_or_id: &str) -> Option<String> {
    let frames = || els.iter().filter(|e| is_frame(e));
    frames()
        .find(|e| e.id == name_or_id)
        .or_else(|| frames().find(|e| name(e) == name_or_id))
        .map(|e| e.id.clone())
}

/// Área do frame (frames não giram).
pub fn rect(e: &Element) -> Rect {
    local_box(e)
}

/// Áreas dos frames da cena, por id.
pub fn rects(els: &[Element]) -> HashMap<&str, Rect> {
    els.iter()
        .filter(|e| is_frame(e))
        .map(|e| (e.id.as_str(), rect(e)))
        .collect()
}

/// Frame (de cima para baixo) cuja área contém `p`, ignorando os de `skip`.
pub fn frame_at(els: &[Element], p: P, skip: &HashSet<String>) -> Option<usize> {
    (0..els.len())
        .rev()
        .find(|&i| is_frame(&els[i]) && !skip.contains(&els[i].id) && rect(&els[i]).contains(p))
}

/// `ids` mais os filhos dos frames que estão em `ids`.
pub fn with_children(els: &[Element], ids: &HashSet<String>) -> HashSet<String> {
    let mut out = ids.clone();
    for e in els {
        if e.frame_id.as_ref().is_some_and(|f| ids.contains(f)) {
            out.insert(e.id.clone());
        }
    }
    out
}

/// O elemento aparece em `p`? Fora da área do frame dele, está recortado.
pub fn visible_at(els: &[Element], e: &Element, p: P) -> bool {
    let Some(fid) = &e.frame_id else { return true };
    els.iter()
        .find(|f| &f.id == fid && is_frame(f))
        .is_none_or(|f| rect(f).contains(p))
}

/// Põe os elementos de `ids` no frame `target` (ou tira do frame, com `None`).
/// Frames nunca ficam dentro de outros; o texto dentro de uma forma acompanha a forma.
pub fn assign(els: &mut [Element], ids: &HashSet<String>, target: Option<&str>) {
    let labels: Vec<String> = els
        .iter()
        .filter(|e| e.str_field("containerId").is_some_and(|c| ids.contains(c)))
        .map(|e| e.id.clone())
        .collect();
    for e in els.iter_mut() {
        if is_frame(e) || !(ids.contains(&e.id) || labels.contains(&e.id)) {
            continue;
        }
        if e.frame_id.as_deref() != target {
            e.frame_id = target.map(str::to_string);
            e.touch();
        }
    }
}

/// Depois de criar ou redimensionar o frame `id`: entram os elementos soltos
/// que ficaram inteiramente dentro dele (grupos só se couberem inteiros) e
/// saem os filhos que ficaram totalmente de fora.
pub fn refresh_members(els: &mut [Element], id: &str) {
    let Some(r) = els.iter().find(|e| e.id == id).map(rect) else {
        return;
    };
    let inside = |e: &Element| r.contains_rect(&aabb(e));
    // grupo (o mais externo) → todos os membros cabem?
    let mut groups: HashMap<&str, bool> = HashMap::new();
    for e in els.iter().filter(|e| !is_frame(e)) {
        if let Some(g) = e.group_ids.last() {
            let ok = groups.entry(g).or_insert(true);
            *ok = *ok && inside(e) && e.frame_id.as_deref().is_none_or(|f| f == id);
        }
    }
    let mut join = HashSet::new();
    let mut leave = HashSet::new();
    for e in els.iter().filter(|e| !is_frame(e)) {
        let fits = match e.group_ids.last() {
            Some(g) => groups[g.as_str()],
            None => inside(e),
        };
        match e.frame_id.as_deref() {
            None if fits => {
                join.insert(e.id.clone());
            }
            Some(f) if f == id && !overlaps(&r, &aabb(e)) => {
                leave.insert(e.id.clone());
            }
            _ => {}
        }
    }
    assign(els, &join, Some(id));
    assign(els, &leave, None);
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
}

/// Limpa `frameId` que aponta para frames que não existem mais (ou para não-frames).
pub fn tidy(els: &mut [Element]) {
    let frames: HashSet<String> = els
        .iter()
        .filter(|e| is_frame(e))
        .map(|e| e.id.clone())
        .collect();
    for e in els.iter_mut() {
        let bad = e
            .frame_id
            .as_ref()
            .is_some_and(|f| is_frame(e) || !frames.contains(f));
        if bad {
            e.frame_id = None;
            e.touch();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Style;

    fn el(kind: &str, x: f64, y: f64, w: f64, h: f64) -> Element {
        let mut e = Element::new(kind, x, y, &Style::default());
        e.width = w;
        e.height = h;
        e
    }

    #[test]
    fn new_frame_takes_what_fits_inside() {
        let f = el("frame", 0.0, 0.0, 200.0, 200.0);
        let inside = el("rectangle", 20.0, 20.0, 50.0, 50.0);
        let across = el("rectangle", 180.0, 20.0, 50.0, 50.0);
        let mut g1 = el("ellipse", 10.0, 100.0, 20.0, 20.0);
        let mut g2 = el("ellipse", 300.0, 100.0, 20.0, 20.0);
        g1.group_ids = vec!["g".into()];
        g2.group_ids = vec!["g".into()];
        let mut els = vec![inside, across, g1, g2, f];
        let fid = els[4].id.clone();
        refresh_members(&mut els, &fid);
        assert_eq!(els[0].frame_id.as_deref(), Some(fid.as_str()));
        assert_eq!(els[1].frame_id, None, "só parte dentro");
        assert_eq!(
            els[2].frame_id, None,
            "grupo que não cabe inteiro fica de fora"
        );
        assert_eq!(els[4].frame_id, None);

        // encolher o frame solta quem ficou totalmente de fora
        els[4].width = 10.0;
        els[4].height = 10.0;
        refresh_members(&mut els, &fid);
        assert_eq!(els[0].frame_id, None);
    }

    #[test]
    fn children_hit_only_inside_and_dangling_ids_are_cleared() {
        let f = el("frame", 0.0, 0.0, 100.0, 100.0);
        let mut c = el("rectangle", 50.0, 50.0, 100.0, 100.0);
        c.frame_id = Some(f.id.clone());
        let mut els = vec![c, f];
        assert!(visible_at(&els, &els[0], P::new(90.0, 90.0)));
        assert!(!visible_at(&els, &els[0], P::new(120.0, 120.0)));
        let ids: HashSet<String> = [els[1].id.clone()].into();
        assert_eq!(with_children(&els, &ids).len(), 2);
        assert_eq!(frame_at(&els, P::new(10.0, 10.0), &HashSet::new()), Some(1));
        assert_eq!(frame_at(&els, P::new(10.0, 10.0), &ids), None);
        els.pop();
        tidy(&mut els);
        assert_eq!(els[0].frame_id, None);
    }

    #[test]
    fn names() {
        let mut f = el("frame", 0.0, 0.0, 10.0, 10.0);
        assert_eq!(name(&f), "Frame");
        f.set_field("name", serde_json::json!("Tela 1"));
        assert_eq!(name(&f), "Tela 1");
    }
}
