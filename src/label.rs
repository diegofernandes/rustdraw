//! Texto dentro de uma forma (o "rótulo"), como no Excalidraw.
//!
//! O rótulo é um elemento `text` com `containerId` apontando para a forma, e
//! a forma lista o texto no seu `boundElements` (`{"id": .., "type": "text"}`).
//! O texto não tem posição própria: fica sempre no centro da forma (ou
//! encostado em cima/embaixo, pelo `verticalAlign`), girado junto com ela.

use crate::geom::{self, P};
use crate::model::Element;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Folga entre o texto e a borda da forma (a mesma do Excalidraw).
const PADDING: f64 = 5.0;

/// Formas que podem conter um rótulo.
pub fn can_contain(e: &Element) -> bool {
    matches!(e.kind.as_str(), "rectangle" | "ellipse" | "diamond")
}

/// Id da forma que contém o texto `e`, se houver.
pub fn container_id(e: &Element) -> Option<&str> {
    (e.kind == "text")
        .then(|| e.str_field("containerId"))
        .flatten()
}

/// Índice da forma que contém o elemento `i` (se ele for um rótulo).
pub fn container_of(els: &[Element], i: usize) -> Option<usize> {
    let c = container_id(&els[i])?;
    els.iter().position(|e| e.id == c && can_contain(e))
}

/// Índice do rótulo da forma `id`.
pub fn label_of(els: &[Element], id: &str) -> Option<usize> {
    els.iter().position(|e| container_id(e) == Some(id))
}

/// Forma (de cima para baixo) que tem `p` dentro dela, mesmo sem fundo:
/// é onde um texto escrito em `p` vai morar.
pub fn container_at(els: &[Element], p: P) -> Option<usize> {
    (0..els.len()).rev().find(|&i| {
        let e = &els[i];
        if !can_contain(e) || e.locked {
            return false;
        }
        // por dentro conta como se a forma fosse preenchida
        let mut solid = e.clone();
        solid.background_color = "#000000".into();
        geom::hit_test(&solid, p, 0.0)
    })
}

/// `ids` mais os rótulos das formas de `ids`.
pub fn with_labels(els: &[Element], ids: &HashSet<String>) -> HashSet<String> {
    let mut out = ids.clone();
    for e in els {
        if container_id(e).is_some_and(|c| ids.contains(c)) {
            out.insert(e.id.clone());
        }
    }
    out
}

/// Liga o texto `text` à forma `container`. Não chama `touch` na forma.
pub fn attach(text: &mut Element, container: &mut Element) {
    text.set_field("containerId", json!(container.id));
    text.set_field("verticalAlign", json!("middle"));
    text.set_field("textAlign", json!("center"));
    let mut bound = bound_list(container);
    if !has_text(&bound, &text.id) {
        bound.push(json!({"id": text.id, "type": "text"}));
        container.bound_elements = Some(Value::Array(bound));
    }
}

fn bound_list(e: &Element) -> Vec<Value> {
    e.bound_elements
        .as_ref()
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn has_text(bound: &[Value], id: &str) -> bool {
    bound.iter().any(|v| {
        v.get("type").and_then(|t| t.as_str()) == Some("text")
            && v.get("id").and_then(|i| i.as_str()) == Some(id)
    })
}

/// Onde o rótulo `t` deve ficar dentro de `c` (x, y do canto e ângulo).
fn placement(t: &Element, c: &Element) -> (f64, f64, f64) {
    let b = geom::local_box(c);
    let (w, h) = (b.width() as f64, b.height() as f64);
    let room_x = (w / 2.0 - PADDING - t.width / 2.0).max(0.0);
    let room_y = (h / 2.0 - PADDING - t.height / 2.0).max(0.0);
    let dx = match t.text_align() {
        "left" => -room_x,
        "right" => room_x,
        _ => 0.0,
    };
    let dy = match t.str_field("verticalAlign") {
        Some("top") => -room_y,
        Some("bottom") => room_y,
        _ => 0.0,
    };
    let (s, co) = c.angle.sin_cos();
    let center: P = b.center();
    let cx = center.x as f64 + dx * co - dy * s;
    let cy = center.y as f64 + dx * s + dy * co;
    (cx - t.width / 2.0, cy - t.height / 2.0, c.angle)
}

/// Põe cada rótulo no lugar dentro da sua forma, logo acima dela na ordem
/// de desenho, e mantém o `boundElements` das formas em dia. Um texto cuja
/// forma não existe vira texto solto. Devolve `true` se algo mudou.
pub fn sync(els: &mut Vec<Element>) -> bool {
    let mut changed = false;
    let index: HashMap<String, usize> = els
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.clone(), i))
        .collect();
    for e in els.iter_mut() {
        if container_id(e).is_some_and(|c| !index.contains_key(c)) {
            e.set_field("containerId", Value::Null);
            e.touch();
            changed = true;
        }
    }

    // rótulo logo acima da forma, para não ficar escondido atrás dela
    let mut i = 0;
    while i < els.len() {
        if let Some(c) = container_of(els, i)
            && c > i
        {
            let t = els.remove(i);
            els.insert(c, t);
            changed = true;
            continue;
        }
        i += 1;
    }

    let index: HashMap<String, usize> = els
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.clone(), i))
        .collect();
    let mut labels: HashMap<String, Vec<String>> = HashMap::new();
    for i in 0..els.len() {
        let Some(&c) = container_id(&els[i]).and_then(|c| index.get(c)) else {
            continue;
        };
        if !can_contain(&els[c]) {
            continue;
        }
        labels
            .entry(els[c].id.clone())
            .or_default()
            .push(els[i].id.clone());
        let (x, y, angle) = placement(&els[i], &els[c]);
        let t = &mut els[i];
        let far = |a: f64, b: f64| (a - b).abs() > 1e-3;
        if far(t.x, x) || far(t.y, y) || far(t.angle, angle) {
            t.x = x;
            t.y = y;
            t.angle = angle;
            t.touch();
            changed = true;
        }
    }

    // `boundElements` da forma lista exatamente os seus rótulos
    for e in els.iter_mut() {
        let mine = labels.get(&e.id);
        let old = bound_list(e);
        let mut new: Vec<Value> = old
            .iter()
            .filter(|v| {
                v.get("type").and_then(|t| t.as_str()) != Some("text")
                    || v.get("id")
                        .and_then(|i| i.as_str())
                        .is_some_and(|id| mine.is_some_and(|m| m.iter().any(|x| x == id)))
            })
            .cloned()
            .collect();
        for id in mine.into_iter().flatten() {
            if !has_text(&new, id) {
                new.push(json!({"id": id, "type": "text"}));
            }
        }
        if new != old {
            e.bound_elements = Some(Value::Array(new));
            e.touch();
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Style;

    fn scene() -> Vec<Element> {
        let mut r = Element::new("rectangle", 0.0, 0.0, &Style::default());
        r.id = "r".into();
        r.width = 200.0;
        r.height = 100.0;
        let mut t = Element::new("text", 500.0, 500.0, &Style::default());
        t.id = "t".into();
        t.width = 40.0;
        t.height = 20.0;
        attach(&mut t, &mut r);
        vec![r, t]
    }

    #[test]
    fn label_goes_to_the_center_and_follows_the_shape() {
        let mut els = scene();
        assert!(sync(&mut els));
        assert_eq!((els[1].x, els[1].y), (80.0, 40.0));
        assert!(!sync(&mut els), "já está no lugar");

        // mover e girar a forma leva o texto junto
        els[0].x += 30.0;
        els[0].y += 10.0;
        els[0].width = 300.0;
        els[0].angle = 0.5;
        assert!(sync(&mut els));
        assert_eq!((els[1].x, els[1].y), (160.0, 50.0));
        assert_eq!(els[1].angle, 0.5);
    }

    #[test]
    fn label_stays_above_its_shape_and_is_freed_without_it() {
        let mut els = scene();
        els.swap(0, 1);
        sync(&mut els);
        assert_eq!(els[0].id, "r");
        assert_eq!(els[1].id, "t");

        els.remove(0);
        sync(&mut els);
        assert_eq!(container_id(&els[0]), None);
    }

    #[test]
    fn text_goes_into_the_shape_under_the_pointer_even_without_fill() {
        let els = scene();
        assert_eq!(container_at(&els, P::new(100.0, 50.0)), Some(0));
        assert_eq!(container_at(&els, P::new(300.0, 50.0)), None);
    }

    #[test]
    fn bound_elements_track_the_labels() {
        let mut els = scene();
        els[0].bound_elements = Some(json!([
            {"id": "a", "type": "arrow"},
            {"id": "gone", "type": "text"}
        ]));
        sync(&mut els);
        assert_eq!(
            els[0].bound_elements,
            Some(json!([{"id": "a", "type": "arrow"}, {"id": "t", "type": "text"}]))
        );
    }
}
