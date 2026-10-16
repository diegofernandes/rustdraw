//! Setas ligadas ("grudadas") a formas.
//!
//! Cada ponta de uma seta pode guardar uma ligação (`startBinding` /
//! `endBinding`, no mesmo formato do Excalidraw) para uma forma. Há dois modos:
//!
//! - **ponto fixo**: a ponta fica num ponto de encaixe da forma (meio de cada
//!   lado), guardado em `fixedPoint` como fração da caixa da forma;
//! - **livre** (órbita): a ponta mira o centro da forma e para no contorno.
//!
//! A forma lista as setas ligadas a ela em `boundElements`. Quando uma forma
//! muda, [`sync`] recalcula as pontas das setas ligadas a ela.

use crate::geom::{self, P, rotate};
use crate::model::Element;
use serde_json::{Value, json};
use std::collections::HashMap;

/// Distância entre a ponta da seta e o contorno da forma.
pub const GAP: f64 = 5.0;
/// Distância (pixels de tela) em que a ponta passa a ligar na forma.
pub const BIND_PX: f32 = 16.0;
/// Distância (pixels de tela) em que a ponta encaixa num ponto fixo.
pub const SNAP_PX: f32 = 14.0;
/// Pontos de encaixe: meio de cada lado (cima, direita, baixo, esquerda).
pub const ANCHORS: [[f64; 2]; 4] = [[0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]];

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub element_id: String,
    /// Ponto fixo como fração da caixa da forma; `None` = modo livre.
    pub fixed: Option<[f64; 2]>,
    pub gap: f64,
}

impl Binding {
    pub fn new(element_id: &str, fixed: Option<[f64; 2]>) -> Self {
        Self {
            element_id: element_id.into(),
            fixed,
            gap: GAP,
        }
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let element_id = v.get("elementId")?.as_str()?.to_string();
        let fixed = v.get("fixedPoint").and_then(|f| {
            let a = f.as_array()?;
            Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
        });
        // Excalidraw recente: `mode` "orbit" mira o centro mesmo com `fixedPoint`;
        // "inside" prende a ponta exatamente no ponto, sem folga.
        let mode = v.get("mode").and_then(|m| m.as_str());
        let fixed = if mode == Some("orbit") { None } else { fixed };
        let gap = if mode == Some("inside") {
            0.0
        } else {
            v.get("gap").and_then(|g| g.as_f64()).unwrap_or(GAP)
        };
        Some(Self {
            element_id,
            fixed,
            gap,
        })
    }

    pub fn to_value(&self) -> Value {
        json!({
            "elementId": self.element_id,
            "focus": 0.0,
            "gap": self.gap,
            "fixedPoint": self.fixed.map(|f| json!([f[0], f[1]])),
        })
    }
}

fn key(end: bool) -> &'static str {
    if end { "endBinding" } else { "startBinding" }
}

pub fn get(el: &Element, end: bool) -> Option<Binding> {
    el.extra.get(key(end)).and_then(Binding::from_value)
}

/// Define (ou remove) a ligação de uma ponta. Não chama `touch`.
pub fn set(el: &mut Element, end: bool, b: Option<&Binding>) {
    let v = b.map(Binding::to_value).unwrap_or(Value::Null);
    el.extra.insert(key(end).into(), v);
}

/// Formas em que uma seta pode ser ligada.
pub fn is_bindable(el: &Element) -> bool {
    match el.kind.as_str() {
        "rectangle" | "diamond" | "ellipse" => true,
        "text" => el.extra.get("containerId").is_none_or(|v| v.is_null()),
        _ => false,
    }
}

/// Posição (mundo) de um ponto dado como fração da caixa da forma.
pub fn anchor_world(el: &Element, f: [f64; 2]) -> P {
    let b = geom::local_box(el);
    let p = P::new(
        b.min.x + b.width() * f[0] as f32,
        b.min.y + b.height() * f[1] as f32,
    );
    rotate(p, b.center(), el.angle as f32)
}

/// Forma (e ponto de encaixe, se houver) sob uma ponta de seta.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub fixed: Option<[f64; 2]>,
}

/// Procura, de cima para baixo, uma forma perto de `p` para ligar a ponta.
/// Formas têm preferência sobre textos, para que o rótulo escrito dentro de
/// uma forma não "roube" a seta.
pub fn candidate(elements: &[Element], p: P, zoom: f32, exclude: &str) -> Option<Candidate> {
    let bind_tol = BIND_PX / zoom;
    let snap_tol = SNAP_PX / zoom;
    let try_bind = |el: &Element| {
        let ol = geom::outline(el)?;
        let c = geom::center(el);
        let q = rotate(p, c, -(el.angle as f32));
        let near =
            geom::point_in_polygon(q, &ol) || geom::dist_to_polyline(q, &ol, true) <= bind_tol;
        if !near {
            return None;
        }
        let fixed = ANCHORS
            .iter()
            .map(|a| (*a, (anchor_world(el, *a) - p).len()))
            .filter(|(_, d)| *d <= snap_tol)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(a, _)| a);
        Some(Candidate {
            id: el.id.clone(),
            fixed,
        })
    };
    let pass = |text: bool| {
        elements
            .iter()
            .rev()
            .filter(|e| e.id != exclude && !e.is_deleted && is_bindable(e))
            .filter(|e| (e.kind == "text") == text)
            .find_map(try_bind)
    };
    pass(false).or_else(|| pass(true))
}

/// Primeiro ponto em que o segmento `a`→`b` cruza o polígono fechado (o mais perto de `a`).
fn segment_hit(a: P, b: P, poly: &[P]) -> Option<P> {
    let d = b - a;
    let mut best: Option<f32> = None;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let e = q - p;
        let den = d.x * e.y - d.y * e.x;
        if den.abs() < 1e-9 {
            continue;
        }
        let w = p - a;
        let t = (w.x * e.y - w.y * e.x) / den;
        let u = (w.x * d.y - w.y * d.x) / den;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    }
    best.map(|t| a + d * t)
}

/// Onde fica a ponta ligada a `target`, sabendo para onde a seta segue (`toward`).
pub fn endpoint(target: &Element, b: &Binding, toward: P) -> P {
    let c = geom::center(target);
    if let Some(f) = b.fixed {
        let a = anchor_world(target, f);
        return a + (a - c).normalized() * b.gap as f32;
    }
    let Some(ol) = geom::outline(target) else {
        return c;
    };
    let angle = target.angle as f32;
    let q = rotate(toward, c, -angle);
    let mut dir = (q - c).normalized();
    if dir.len_sq() == 0.0 {
        dir = P::new(1.0, 0.0);
    }
    // de fora da forma em direção ao centro; se `toward` está dentro, parte de longe
    let b0 = geom::local_box(target);
    let far = c + dir * (b0.width() + b0.height() + 1.0);
    let from = if geom::point_in_polygon(q, &ol) {
        far
    } else {
        q
    };
    let hit = segment_hit(from, c, &ol).unwrap_or(c);
    rotate(hit + dir * b.gap as f32, c, angle)
}

/// Pontos absolutos da seta com a rotação já aplicada.
fn world_points(el: &Element) -> Vec<P> {
    let c = geom::center(el);
    let a = el.angle as f32;
    geom::abs_points(el)
        .into_iter()
        .map(|p| rotate(p, c, a))
        .collect()
}

/// Ponto para onde a ponta (`end`) mira: o ponto vizinho da seta ou, numa seta
/// de dois pontos, o alvo da outra ponta.
fn aim(pts: &[P], end: bool, other: Option<(&Element, &Binding)>) -> P {
    let n = pts.len();
    if n > 2 {
        return if end { pts[n - 2] } else { pts[1] };
    }
    match other {
        Some((t, b)) => match b.fixed {
            Some(f) => anchor_world(t, f),
            None => geom::center(t),
        },
        None => {
            if end {
                pts[0]
            } else {
                pts[n - 1]
            }
        }
    }
}

/// Recalcula as pontas ligadas de `arrow`. `None` se nada mudou.
pub fn updated_arrow<'a>(
    arrow: &Element,
    find: impl Fn(&str) -> Option<&'a Element>,
) -> Option<Element> {
    let pts = world_points(arrow);
    if pts.len() < 2 {
        return None;
    }
    let resolve = |end: bool| {
        let b = get(arrow, end)?;
        let t = find(&b.element_id).filter(|t| is_bindable(t))?;
        Some((t, b))
    };
    let (sb, eb) = (resolve(false), resolve(true));
    if sb.is_none() && eb.is_none() {
        return None;
    }
    let mut new = pts.clone();
    let n = pts.len();
    if let Some((t, b)) = &sb {
        new[0] = endpoint(t, b, aim(&pts, false, eb.as_ref().map(|(t, b)| (*t, b))));
    }
    if let Some((t, b)) = &eb {
        new[n - 1] = endpoint(t, b, aim(&pts, true, sb.as_ref().map(|(t, b)| (*t, b))));
    }
    let same = arrow.angle == 0.0
        && new
            .iter()
            .zip(geom::abs_points(arrow))
            .all(|(a, b)| (*a - b).len() < 0.01);
    if same {
        return None;
    }
    let mut el = arrow.clone();
    el.angle = 0.0;
    el.x = new[0].x as f64;
    el.y = new[0].y as f64;
    el.set_points(
        new.iter()
            .map(|p| [(p.x - new[0].x) as f64, (p.y - new[0].y) as f64])
            .collect(),
    );
    Some(el)
}

/// Recalcula uma seta específica agora (usado enquanto ela é desenhada).
pub fn refresh_arrow(elements: &mut [Element], id: &str) {
    let Some(i) = elements.iter().position(|e| e.id == id) else {
        return;
    };
    let els: &[Element] = elements;
    let upd = updated_arrow(&els[i], |id| els.iter().find(|e| e.id == id));
    if let Some(u) = upd {
        elements[i] = u;
        elements[i].touch();
    }
}

fn signature(arrow: &Element, find: &impl Fn(&str) -> Option<usize>, els: &[Element]) -> Vec<i64> {
    let mut sig = vec![arrow.version, arrow.version_nonce];
    for end in [false, true] {
        if let Some(t) = get(arrow, end).and_then(|b| find(&b.element_id)) {
            sig.extend([els[t].version, els[t].version_nonce]);
        } else {
            sig.push(-1);
        }
    }
    sig
}

/// Atualiza as setas cujas formas ligadas (ou elas mesmas) mudaram desde a
/// última chamada. Setas ainda não vistas só são registradas, para que um
/// arquivo aberto apareça exatamente como foi salvo.
pub fn sync(elements: &mut [Element], seen: &mut HashMap<String, Vec<i64>>) {
    let index: HashMap<String, usize> = elements
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.clone(), i))
        .collect();
    let find = |id: &str| index.get(id).copied();
    for i in 0..elements.len() {
        let a = &elements[i];
        if a.kind != "arrow" || (get(a, false).is_none() && get(a, true).is_none()) {
            continue;
        }
        let sig = signature(a, &find, elements);
        let known = seen.get(&a.id);
        if known == Some(&sig) {
            continue;
        }
        let first = known.is_none();
        let id = a.id.clone();
        let els: &[Element] = elements;
        if !first && let Some(u) = updated_arrow(&els[i], |id| find(id).map(|j| &els[j])) {
            elements[i] = u;
            elements[i].touch();
        }
        let sig = signature(&elements[i], &find, elements);
        seen.insert(id, sig);
    }
    if seen.len() > elements.len() + 256 {
        seen.retain(|k, _| index.contains_key(k));
    }
}

/// Remove ligações para formas que não existem mais e reescreve o
/// `boundElements` das formas para listar exatamente as setas ligadas a elas.
pub fn tidy(elements: &mut [Element]) {
    let ids: HashMap<String, usize> = elements
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.clone(), i))
        .collect();
    let mut bound: HashMap<String, Vec<String>> = HashMap::new();
    for el in elements.iter_mut() {
        let mut changed = false;
        for end in [false, true] {
            let Some(b) = get(el, end) else { continue };
            // só setas ligam; uma linha (ex.: convertida com Tab) perde a ligação
            if el.kind != "arrow" || b.element_id == el.id || !ids.contains_key(&b.element_id) {
                set(el, end, None);
                changed = true;
            } else if !bound.get(&b.element_id).is_some_and(|v| v.contains(&el.id)) {
                bound.entry(b.element_id).or_default().push(el.id.clone());
            }
        }
        if changed {
            el.touch();
        }
    }
    for el in elements.iter_mut().filter(|e| is_bindable(e)) {
        let old: Vec<Value> = el
            .bound_elements
            .as_ref()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut new: Vec<Value> = old
            .iter()
            .filter(|v| v.get("type").and_then(|t| t.as_str()) != Some("arrow"))
            .cloned()
            .collect();
        for a in bound.get(&el.id).into_iter().flatten() {
            new.push(json!({"id": a, "type": "arrow"}));
        }
        if new != old {
            el.bound_elements = Some(Value::Array(new));
            el.touch();
        }
    }
}

/// Espelha os pontos fixos de uma seta (ao espelhar a seta junto com as formas).
pub fn mirror_fixed(el: &mut Element, horizontal: bool) {
    for end in [false, true] {
        if let Some(mut b) = get(el, end)
            && let Some(f) = b.fixed.as_mut()
        {
            if horizontal {
                f[0] = 1.0 - f[0];
            } else {
                f[1] = 1.0 - f[1];
            }
            set(el, end, Some(&b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Style;

    fn rect(x: f64, y: f64) -> Element {
        let mut r = Element::new("rectangle", x, y, &Style::default());
        r.roundness = None;
        r.width = 100.0;
        r.height = 50.0;
        r
    }

    fn arrow(a: [f64; 2], b: [f64; 2]) -> Element {
        let mut e = Element::new("arrow", a[0], a[1], &Style::default());
        e.set_points(vec![[0.0, 0.0], [b[0] - a[0], b[1] - a[1]]]);
        e
    }

    fn close(a: P, b: P) -> bool {
        (a - b).len() < 0.05
    }

    #[test]
    fn binding_json_roundtrip_and_excalidraw_modes() {
        let b = Binding::new("r1", Some([1.0, 0.5]));
        assert_eq!(Binding::from_value(&b.to_value()), Some(b));
        let free = Binding::new("r1", None);
        assert_eq!(Binding::from_value(&free.to_value()), Some(free));
        // formato antigo do Excalidraw: só focus/gap
        let old = json!({"elementId": "x", "focus": 0.2, "gap": 8});
        let o = Binding::from_value(&old).unwrap();
        assert_eq!((o.fixed, o.gap), (None, 8.0));
        let orbit = json!({"elementId": "x", "fixedPoint": [0.3, 0.1], "mode": "orbit"});
        assert_eq!(Binding::from_value(&orbit).unwrap().fixed, None);
        let inside = json!({"elementId": "x", "fixedPoint": [0.3, 0.1], "mode": "inside"});
        let i = Binding::from_value(&inside).unwrap();
        assert_eq!((i.fixed, i.gap), (Some([0.3, 0.1]), 0.0));
        assert_eq!(Binding::from_value(&Value::Null), None);
    }

    #[test]
    fn candidate_snaps_to_anchor_or_is_free() {
        let r = rect(0.0, 0.0);
        let els = vec![r.clone()];
        let c = candidate(&els, P::new(103.0, 26.0), 1.0, "").unwrap();
        assert_eq!(c.fixed, Some([1.0, 0.5]));
        let c = candidate(&els, P::new(30.0, 20.0), 1.0, "").unwrap();
        assert_eq!((c.id.as_str(), c.fixed), (r.id.as_str(), None));
        assert!(candidate(&els, P::new(300.0, 20.0), 1.0, "").is_none());
        assert!(candidate(&els, P::new(30.0, 20.0), 1.0, &r.id).is_none());
        // rótulo de texto por cima da forma: a seta liga na forma
        let mut label = Element::new("text", 30.0, 15.0, &Style::default());
        label.width = 40.0;
        label.height = 20.0;
        let els = vec![r.clone(), label.clone()];
        assert_eq!(
            candidate(&els, P::new(40.0, 20.0), 1.0, "").unwrap().id,
            r.id
        );
        // texto solto também aceita setas
        let els = vec![label.clone()];
        assert_eq!(
            candidate(&els, P::new(40.0, 20.0), 1.0, "").unwrap().id,
            label.id
        );
    }

    #[test]
    fn free_endpoint_stops_at_outline() {
        let r = rect(0.0, 0.0); // centro (50, 25)
        let b = Binding::new(&r.id, None);
        let p = endpoint(&r, &b, P::new(300.0, 25.0));
        assert!(close(p, P::new(100.0 + GAP as f32, 25.0)), "{p:?}");
        let p = endpoint(&r, &b, P::new(50.0, -200.0));
        assert!(close(p, P::new(50.0, -(GAP as f32))), "{p:?}");
        // mesmo vindo de dentro, para no contorno
        let p = endpoint(&r, &b, P::new(60.0, 25.0));
        assert!(close(p, P::new(100.0 + GAP as f32, 25.0)), "{p:?}");
    }

    #[test]
    fn fixed_endpoint_follows_rotation() {
        let mut r = rect(0.0, 0.0);
        r.angle = std::f64::consts::FRAC_PI_2;
        let b = Binding::new(&r.id, Some([1.0, 0.5]));
        // lado direito girado 90° vai para baixo do centro (50, 25)
        let p = endpoint(&r, &b, P::new(0.0, 0.0));
        assert!(close(p, P::new(50.0, 25.0 + 50.0 + GAP as f32)), "{p:?}");
    }

    #[test]
    fn arrows_follow_moved_shapes() {
        let r1 = rect(0.0, 0.0);
        let r2 = rect(300.0, 0.0);
        let mut a = arrow([105.0, 25.0], [295.0, 25.0]);
        set(&mut a, false, Some(&Binding::new(&r1.id, Some([1.0, 0.5]))));
        set(&mut a, true, Some(&Binding::new(&r2.id, None)));
        let mut els = vec![r1, r2, a];
        let mut seen = HashMap::new();
        sync(&mut els, &mut seen); // primeira vez: só registra
        let before = els[2].clone();
        sync(&mut els, &mut seen);
        assert_eq!(els[2], before, "sem mudanças, a seta fica igual");

        // move a forma de destino para baixo
        els[1].y += 200.0;
        els[1].touch();
        sync(&mut els, &mut seen);
        let pts = geom::abs_points(&els[2]);
        assert!(close(pts[0], P::new(105.0, 25.0)), "{pts:?}");
        let end = pts[1];
        // mira o centro (350, 225) e para no contorno de r2
        let r2 = &els[1];
        let ol = geom::outline(r2).unwrap();
        let dir = (geom::center(r2) - end).normalized();
        // recuando a folga na direção do centro, cai no contorno
        let d = geom::dist_to_polyline(end + dir * GAP as f32, &ol, true);
        assert!(d < 0.05, "distância {d}");
        let want = (geom::center(r2) - pts[0]).normalized();
        assert!(dir.dot(want) > 0.99);
    }

    #[test]
    fn tidy_cleans_dangling_and_lists_bound_arrows() {
        let r = rect(0.0, 0.0);
        let mut a = arrow([105.0, 25.0], [200.0, 25.0]);
        set(&mut a, false, Some(&Binding::new(&r.id, None)));
        set(&mut a, true, Some(&Binding::new("sumiu", None)));
        let mut r = r;
        r.bound_elements =
            Some(json!([{"id": "t1", "type": "text"}, {"id": "velha", "type": "arrow"}]));
        let mut els = vec![r, a];
        tidy(&mut els);
        assert_eq!(get(&els[1], true), None);
        assert!(get(&els[1], false).is_some());
        assert_eq!(
            els[0].bound_elements,
            Some(json!([{"id": "t1", "type": "text"}, {"id": els[1].id, "type": "arrow"}]))
        );
        // apagar a forma solta a seta
        els.remove(0);
        tidy(&mut els);
        assert_eq!(get(&els[0], false), None);
    }
}
