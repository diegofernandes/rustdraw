//! Texto dentro de uma forma (o "rótulo"), como no Excalidraw.
//!
//! O rótulo é um elemento `text` com `containerId` apontando para a forma, e
//! a forma lista o texto no seu `boundElements` (`{"id": .., "type": "text"}`).
//! O texto não tem posição própria: fica sempre no centro da forma (ou
//! encostado em cima/embaixo, pelo `verticalAlign`), girado junto com ela.
//!
//! Linhas e setas também têm rótulo: o texto fica no meio do caminho, sem
//! girar com a linha, e a linha não é desenhada atrás dele (ver [`holes`]).
//! No Excalidraw só as setas têm rótulo; uma linha com rótulo abre lá com o
//! texto no lugar, mas a linha passa por trás dele.

use crate::geom::{self, P};
use crate::model::Element;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Folga entre o texto e a borda da forma (a mesma do Excalidraw).
const PADDING: f64 = 5.0;

/// Folga (dos lados, em cima e embaixo) do texto dentro de `c`: as notas
/// têm mais respiro.
fn padding(c: &Element) -> (f64, f64) {
    if crate::note::is_note(c) {
        crate::note::PADDING
    } else {
        (PADDING, PADDING)
    }
}

/// Elementos que podem ter um rótulo.
pub fn can_contain(e: &Element) -> bool {
    matches!(
        e.kind.as_str(),
        "rectangle" | "ellipse" | "diamond" | "arrow" | "line"
    )
}

/// Formas fechadas que podem conter um rótulo (o texto mora dentro delas).
fn is_shape(e: &Element) -> bool {
    can_contain(e) && !e.is_linear()
}

/// Largura mínima do rótulo de uma seta, em tamanhos de fonte
/// (`ARROW_LABEL_FONT_SIZE_TO_MIN_WIDTH_RATIO` do Excalidraw).
const LINE_MIN_WIDTH: f64 = 11.0;
/// Fração da largura da seta que o rótulo pode ocupar
/// (`ARROW_LABEL_WIDTH_FRACTION` do Excalidraw).
const LINE_WIDTH_FRACTION: f64 = 0.7;

/// Meio do caminho de uma linha/seta (mundo, já girado): o ponto do meio,
/// se houver um número ímpar de pontos, ou o meio do trecho central
/// (`getContainerCenter` do Excalidraw).
pub fn path_mid(c: &Element) -> P {
    let pts = geom::abs_points(c);
    let n = pts.len();
    let m = match n {
        0 => return geom::center(c),
        _ if n % 2 == 1 => pts[n / 2],
        _ => {
            let i = n / 2 - 1;
            if c.is_round() && !c.is_elbow() && n > 2 {
                crate::rough::curve_mid(&pts, i)
            } else {
                (pts[i] + pts[i + 1]) * 0.5
            }
        }
    };
    geom::rotate(m, geom::center(c), c.angle as f32)
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
        if !is_shape(e) || e.locked {
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

/// Largura que um texto de tamanho `font_size` pode ocupar dentro de `c`
/// (`getBoundTextMaxWidth` do Excalidraw).
pub fn max_width(c: &Element, font_size: f64) -> f64 {
    if c.is_linear() {
        return (c.width.abs() * LINE_WIDTH_FRACTION).max(font_size * LINE_MIN_WIDTH);
    }
    let w = geom::local_box(c).width() as f64;
    let inner = match c.kind.as_str() {
        "ellipse" => (w / 2.0 * std::f64::consts::SQRT_2).round(),
        "diamond" => (w / 2.0).round(),
        _ => w,
    };
    inner - padding(c).0 * 2.0
}

/// Altura que a forma `kind` precisa para caber um texto de altura `h`
/// (`computeContainerDimensionForBoundText` do Excalidraw).
fn needed_height(c: &Element, h: f64) -> f64 {
    let d = h.ceil() + padding(c).1 * 2.0;
    match c.kind.as_str() {
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
        if let Some(&c) = container_id(&els[i]).and_then(|c| index.get(c)) {
            changed |= fit_one(els, i, c, &wrap, &width);
        }
    }
    changed
}

/// [`fit`] só do rótulo `i`, que está na forma `c`.
fn fit_one(
    els: &mut [Element],
    i: usize,
    c: usize,
    wrap: &impl Fn(&Element, &str, f64) -> String,
    width: &impl Fn(&Element, &str) -> f64,
) -> bool {
    let mut changed = false;
    let max = max_width(&els[c], els[i].font_size());
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
    // a linha não cresce: o texto fica por cima dela
    let c = &mut els[c];
    let need = needed_height(c, h);
    if !c.is_linear() && c.height >= 0.0 && c.height < need - 0.01 {
        c.height = need;
        c.touch();
        changed = true;
    }
    changed
}

/// Quebra e põe no lugar o rótulo da forma `container` (sem o editor: quem
/// mede é `width`). Usado ao editar o arquivo por comandos.
pub fn refit(els: &mut [Element], container: &str, width: impl Fn(&Element, &str) -> f64) {
    let Some(c) = els.iter().position(|e| e.id == container && can_contain(e)) else {
        return;
    };
    let Some(i) = els.iter().position(|e| container_id(e) == Some(container)) else {
        return;
    };
    let wrap = |t: &Element, s: &str, max: f64| wrap(s, max, |l| width(t, l));
    fit_one(els, i, c, &wrap, &width);
    let (x, y, angle) = placement(&els[i], &els[c]);
    let t = &mut els[i];
    (t.x, t.y, t.angle) = (x, y, angle);
    t.touch();
}

/// Quebra `text` em linhas de no máximo `max` de largura, palavra a palavra;
/// uma palavra maior que a linha é cortada entre letras.
pub fn wrap(text: &str, max: f64, width: impl Fn(&str) -> f64) -> String {
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

/// Onde o rótulo `t` deve ficar dentro de `c` (x, y do canto e ângulo).
fn placement(t: &Element, c: &Element) -> (f64, f64, f64) {
    if c.is_linear() {
        // no meio do caminho, sem girar com a linha (como no Excalidraw)
        let m = path_mid(c);
        return (
            m.x as f64 - t.width / 2.0,
            m.y as f64 - t.height / 2.0,
            t.angle,
        );
    }
    let b = geom::local_box(c);
    let (w, h) = (b.width() as f64, b.height() as f64);
    let (px, py) = padding(c);
    let room_x = (w / 2.0 - px - t.width / 2.0).max(0.0);
    let room_y = (h / 2.0 - py - t.height / 2.0).max(0.0);
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

/// Folga em volta do texto onde a linha não passa (`BOUND_TEXT_PADDING`).
const HOLE_PADDING: f32 = 5.0;

/// O buraco que o rótulo `t` abre na linha que o contém.
pub fn hole(t: &Element) -> geom::Hole {
    let b = geom::local_box(t);
    geom::Hole {
        center: b.center(),
        half: P::new(
            b.width() / 2.0 + HOLE_PADDING,
            b.height() / 2.0 + HOLE_PADDING,
        ),
        angle: t.angle as f32,
    }
}

/// Os buracos que os rótulos abrem nas linhas e setas, pelo id da linha.
/// (Rótulos de formas entram também, mas só linhas usam o buraco.)
pub fn holes(els: &[Element]) -> HashMap<&str, geom::Hole> {
    els.iter()
        .filter_map(|t| container_id(t).map(|c| (c, hole(t))))
        .collect()
}

/// Primitivas de `el`, sem o trecho de linha atrás do rótulo.
pub fn prims(el: &Element, holes: &HashMap<&str, geom::Hole>) -> Vec<geom::Prim> {
    let prims = geom::element_prims(el);
    match holes.get(el.id.as_str()).filter(|_| el.is_linear()) {
        Some(h) => geom::cut_out(prims, h),
        None => prims,
    }
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

    fn with_line(kind: &str, pts: Vec<[f64; 2]>) -> Vec<Element> {
        let mut l = Element::new(kind, 100.0, 100.0, &Style::default());
        l.id = "l".into();
        l.roundness = None;
        l.set_points(pts);
        let mut t = Element::new("text", 0.0, 0.0, &Style::default());
        t.id = "t".into();
        t.width = 40.0;
        t.height = 20.0;
        attach(&mut t, &mut l);
        vec![l, t]
    }

    fn label_center(els: &[Element]) -> P {
        geom::local_box(&els[1]).center()
    }

    #[test]
    fn line_label_sits_in_the_middle_of_the_path_without_turning() {
        // dois pontos: no meio do segmento
        let mut els = with_line("arrow", vec![[0.0, 0.0], [200.0, 100.0]]);
        assert!(can_contain(&els[0]));
        sync(&mut els);
        assert_eq!(label_center(&els), P::new(200.0, 150.0));
        assert_eq!(
            els[0].bound_elements,
            Some(json!([{"id": "t", "type": "text"}]))
        );

        // número ímpar de pontos: no ponto do meio
        let mut els = with_line("line", vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]]);
        sync(&mut els);
        assert_eq!(label_center(&els), P::new(200.0, 100.0));

        // número par: no meio do trecho central
        let mut els = with_line(
            "arrow",
            vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [200.0, 100.0]],
        );
        sync(&mut els);
        assert_eq!(label_center(&els), P::new(200.0, 150.0));

        // girar a seta leva o texto junto, mas ele continua de pé
        els[0].angle = std::f64::consts::FRAC_PI_2;
        els[0].touch();
        sync(&mut els);
        assert_eq!(els[1].angle, 0.0);
        let c = label_center(&els);
        assert!((c.x - 200.0).abs() < 1e-3 && (c.y - 150.0).abs() < 1e-3);
    }

    #[test]
    fn line_label_wraps_like_excalidraw_and_the_line_does_not_grow() {
        let mut els = with_line("arrow", vec![[0.0, 0.0], [400.0, 0.0]]);
        // 70% da largura da seta, mas pelo menos 11 vezes o tamanho da fonte
        assert_eq!(max_width(&els[0], 20.0), 280.0);
        assert_eq!(max_width(&els[0], 40.0), 440.0);
        els[1].set_text("um rótulo bem comprido que não cabe numa linha só");
        fit10(&mut els);
        assert!(els[1].text().contains('\n'));
        assert!(els[1].width <= 280.0);
        assert_eq!(els[0].height, 0.0, "a seta não cresce");
    }

    #[test]
    fn line_label_cuts_a_hole_only_in_its_line() {
        let mut els = with_line("arrow", vec![[0.0, 0.0], [200.0, 0.0]]);
        sync(&mut els);
        let h = holes(&els);
        assert_eq!(h["l"].center, P::new(200.0, 100.0));
        assert_eq!(h["l"].half, P::new(25.0, 15.0));
        let strokes = |prims: &[geom::Prim]| {
            prims
                .iter()
                .filter(|p| matches!(p, geom::Prim::Stroke { .. }))
                .count()
        };
        let whole = geom::element_prims(&els[0]);
        let cut = prims(&els[0], &h);
        assert!(strokes(&cut) > strokes(&whole), "o traço foi partido");
        for p in &cut {
            if let geom::Prim::Stroke { pts, .. } = p {
                assert!(
                    pts.iter()
                        .all(|q| (q.x - 200.0).abs() >= 24.0 || q.y.abs() > 110.0)
                );
            }
        }
        // numa forma o rótulo fica por cima, sem buraco
        let shape = scene();
        let h = holes(&shape);
        assert_eq!(
            prims(&shape[0], &h).len(),
            geom::element_prims(&shape[0]).len()
        );
    }

    #[test]
    fn text_is_not_written_inside_a_line() {
        let els = with_line(
            "line",
            vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 0.0]],
        );
        assert_eq!(container_at(&els, P::new(170.0, 130.0)), None);
    }

    /// Cada letra mede 10.
    fn w10(s: &str) -> f64 {
        s.chars().count() as f64 * 10.0
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
