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

/// Largura que o texto pode ocupar dentro de `c` (`getBoundTextMaxWidth`
/// do Excalidraw).
pub fn max_width(c: &Element) -> f64 {
    let w = geom::local_box(c).width() as f64;
    let inner = match c.kind.as_str() {
        "ellipse" => (w / 2.0 * std::f64::consts::SQRT_2).round(),
        "diamond" => (w / 2.0).round(),
        _ => w,
    };
    inner - PADDING * 2.0
}

/// Altura que a forma `kind` precisa para caber um texto de altura `h`
/// (`computeContainerDimensionForBoundText` do Excalidraw).
fn needed_height(kind: &str, h: f64) -> f64 {
    let d = h.ceil() + PADDING * 2.0;
    match kind {
        "ellipse" => (d / std::f64::consts::SQRT_2 * 2.0).round(),
        "diamond" => 2.0 * d,
        _ => d,
    }
}

/// Quebra o texto de cada rótulo na largura da sua forma e aumenta a altura
/// da forma quando o texto não cabe, como o Excalidraw faz. `wrap` quebra o
/// texto do rótulo `t` numa largura e `width` mede uma linha dele. Devolve
/// `true` se algo mudou.
pub fn fit(
    els: &mut [Element],
    wrap: impl Fn(&Element, &str, f64) -> String,
    width: impl Fn(&Element, &str) -> f64,
) -> bool {
    let index: HashMap<String, usize> = els
        .iter()
        .enumerate()
        .filter(|(_, e)| can_contain(e))
        .map(|(i, e)| (e.id.clone(), i))
        .collect();
    let mut changed = false;
    for i in 0..els.len() {
        let Some(&c) = container_id(&els[i]).and_then(|c| index.get(c)) else {
            continue;
        };
        let max = max_width(&els[c]);
        let t = &els[i];
        let source = t.str_field("originalText").unwrap_or(t.text());
        let text = wrap(t, source, max);
        let lines = text.split('\n').count() as f64;
        let w = text.split('\n').map(|l| width(t, l)).fold(1.0, f64::max);
        let h = lines * t.font_size() * t.line_height();
        let t = &mut els[i];
        let far = |a: f64, b: f64| (a - b).abs() > 0.01;
        if t.text() != text || far(t.width, w) || far(t.height, h) {
            t.set_field("text", json!(text));
            t.width = w;
            t.height = h;
            t.touch();
            changed = true;
        }
        let c = &mut els[c];
        let need = needed_height(&c.kind, h);
        if c.height >= 0.0 && c.height < need - 0.01 {
            c.height = need;
            c.touch();
            changed = true;
        }
    }
    changed
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
    // (quase sempre já está: só então vale percorrer movendo)
    let below = els.iter().enumerate().any(|(i, e)| {
        container_id(e)
            .and_then(|c| index.get(c))
            .is_some_and(|&c| c > i && can_contain(&els[c]))
    });
    let mut i = if below { 0 } else { els.len() };
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

    let index = if below {
        els.iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect()
    } else {
        index
    };
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
        // o caso comum (já em dia) sem copiar a lista
        let listed = e.bound_elements.as_ref().and_then(|v| v.as_array());
        let texts = listed
            .into_iter()
            .flatten()
            .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("text"));
        let in_mine = |v: &Value| {
            v.get("id")
                .and_then(|i| i.as_str())
                .is_some_and(|id| mine.is_some_and(|m| m.iter().any(|x| x == id)))
        };
        let mut n = 0;
        let mut ok = true;
        for v in texts {
            n += 1;
            ok &= in_mine(v);
        }
        let has_all = mine
            .into_iter()
            .flatten()
            .all(|id| listed.is_some_and(|l| has_text(l, id)));
        if ok && has_all && n == mine.map_or(0, Vec::len) {
            continue;
        }
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

    /// Cada letra mede 10.
    fn w10(s: &str) -> f64 {
        s.chars().count() as f64 * 10.0
    }

    /// Quebra `text` em linhas de no máximo `max` de largura, palavra a palavra;
    /// uma palavra maior que a linha é cortada entre letras.
    fn wrap(text: &str, max: f64, width: impl Fn(&str) -> f64) -> String {
        let mut out: Vec<String> = Vec::new();
        for para in text.split('\n') {
            if width(para) <= max {
                out.push(para.to_string());
                continue;
            }
            let mut line = String::new();
            for word in para.split(' ') {
                let joined = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if width(&joined) <= max {
                    line = joined;
                    continue;
                }
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                // a palavra sozinha não cabe: corta onde der (pelo menos uma letra)
                for ch in word.chars() {
                    let mut next = line.clone();
                    next.push(ch);
                    if !line.is_empty() && width(&next) > max {
                        out.push(std::mem::replace(&mut line, ch.to_string()));
                    } else {
                        line = next;
                    }
                }
            }
            out.push(line);
        }
        out.join("\n")
    }

    fn fit10(els: &mut [Element]) -> bool {
        fit(els, |_, s, max| wrap(s, max, w10), |_, s| w10(s))
    }

    #[test]
    fn wrap_breaks_between_words_and_inside_long_ones() {
        assert_eq!(wrap("abc def", 100.0, w10), "abc def");
        assert_eq!(wrap("abc def ghi", 70.0, w10), "abc def\nghi");
        assert_eq!(wrap("um\n\ndois", 40.0, w10), "um\n\ndois");
        assert_eq!(wrap("abcdefg x", 30.0, w10), "abc\ndef\ng x");
        // nem uma letra cabe: uma por linha, sem travar
        assert_eq!(wrap("ab", -5.0, w10), "a\nb");
    }

    #[test]
    fn fit_wraps_the_label_and_grows_the_shape() {
        let mut els = scene();
        els[1].set_text("um texto que não cabe na largura");
        assert!(fit10(&mut els));
        // 200 de largura menos a folga dos dois lados: 19 letras por linha
        let t = &els[1];
        assert_eq!(t.text(), "um texto que não\ncabe na largura");
        assert_eq!(
            t.str_field("originalText"),
            Some("um texto que não cabe na largura")
        );
        assert_eq!(t.width, 160.0);
        assert_eq!(t.height, 2.0 * 20.0 * 1.25);
        assert_eq!(els[0].height, 100.0, "coube: a forma não muda");
        assert!(!fit10(&mut els), "já está quebrado");

        // texto mais alto que a forma: a forma cresce
        els[1].set_text("a\nb\nc\nd\ne");
        fit10(&mut els);
        assert_eq!(els[0].height, 5.0 * 25.0 + 10.0);
    }
}
