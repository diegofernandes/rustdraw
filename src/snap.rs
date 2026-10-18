//! Encaixe em objetos ("Snap to objects" do Excalidraw): ao mover, criar ou
//! redimensionar, as bordas e o centro da caixa encaixam nas bordas e centros
//! das outras formas visíveis, e guias mostram o alinhamento.

use crate::geom::{self, P, Rect};
use crate::model::Element;
use std::collections::HashSet;

/// Distância de encaixe em pixels de tela (`SNAP_DISTANCE` do Excalidraw).
pub const SNAP_PX: f32 = 8.0;

/// Diferença máxima para considerar dois pontos alinhados ao desenhar as guias.
const EPS: f32 = 0.05;

/// Uma guia de alinhamento: segmento vertical (`x` fixo) ou horizontal (`y` fixo)
/// que liga os pontos alinhados.
#[derive(Clone, Debug, PartialEq)]
pub struct Guide {
    pub vertical: bool,
    pub from: P,
    pub to: P,
    pub points: Vec<P>,
}

/// Cantos e centro de uma caixa.
pub fn box_points(r: Rect) -> [P; 5] {
    [
        r.min,
        P::new(r.max.x, r.min.y),
        r.max,
        P::new(r.min.x, r.max.y),
        r.center(),
    ]
}

/// Pontos de encaixe das formas que não estão em `skip` e aparecem em `view`.
/// Ficam de fora textos dentro de formas e setas ligadas a algo em `skip`
/// (elas acompanham o que está sendo movido).
pub fn reference_points(els: &[Element], skip: &HashSet<String>, view: Rect) -> Vec<P> {
    let bound_to_skip = |e: &Element, key: &str| {
        e.extra
            .get(key)
            .and_then(|b| b.get("elementId"))
            .and_then(|v| v.as_str())
            .is_some_and(|id| skip.contains(id))
    };
    els.iter()
        .filter(|e| !skip.contains(&e.id) && !e.is_deleted)
        .filter(|e| e.str_field("containerId").is_none())
        .filter(|e| !bound_to_skip(e, "startBinding") && !bound_to_skip(e, "endBinding"))
        .map(geom::aabb)
        .filter(|b| {
            b.max.x >= view.min.x
                && b.min.x <= view.max.x
                && b.max.y >= view.min.y
                && b.min.y <= view.max.y
        })
        .flat_map(box_points)
        .collect()
}

/// Menor deslocamento que encaixa algum ponto de `moving` em algum de `refs`,
/// em cada eixo pedido (`axes` = x, y), dentro de `tol`. Zero onde não encaixa.
pub fn offset(moving: &[P], refs: &[P], tol: f32, axes: (bool, bool)) -> P {
    let best = |f: fn(P) -> f32| {
        let mut best: Option<f32> = None;
        for m in moving {
            for r in refs {
                let d = f(*r) - f(*m);
                if d.abs() <= tol && best.is_none_or(|b| d.abs() < b.abs()) {
                    best = Some(d);
                }
            }
        }
        best.unwrap_or(0.0)
    };
    P::new(
        if axes.0 { best(|p| p.x) } else { 0.0 },
        if axes.1 { best(|p| p.y) } else { 0.0 },
    )
}

/// Guias para os pontos de `moving` alinhados com algum de `refs`.
pub fn guides(moving: &[P], refs: &[P]) -> Vec<Guide> {
    let mut out = vec![];
    for vertical in [true, false] {
        let key = |p: &P| if vertical { p.x } else { p.y };
        let along = |p: &P| if vertical { p.y } else { p.x };
        let mut done: Vec<f32> = vec![];
        for m in moving {
            let k = key(m);
            if done.iter().any(|d| (d - k).abs() < EPS) {
                continue;
            }
            let mut pts: Vec<P> = refs
                .iter()
                .filter(|r| (key(r) - k).abs() < EPS)
                .copied()
                .collect();
            if pts.is_empty() {
                continue;
            }
            done.push(k);
            pts.extend(moving.iter().filter(|p| (key(p) - k).abs() < EPS));
            let lo = pts.iter().map(along).fold(f32::INFINITY, f32::min);
            let hi = pts.iter().map(along).fold(f32::NEG_INFINITY, f32::max);
            let (from, to) = if vertical {
                (P::new(k, lo), P::new(k, hi))
            } else {
                (P::new(lo, k), P::new(hi, k))
            };
            out.push(Guide {
                vertical,
                from,
                to,
                points: pts,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Style;

    fn rect(id: &str, x: f64, y: f64, w: f64, h: f64) -> Element {
        let mut e = Element::new("rectangle", x, y, &Style::default());
        e.id = id.into();
        e.width = w;
        e.height = h;
        e
    }

    fn big_view() -> Rect {
        Rect::from_two(P::new(-1e6, -1e6), P::new(1e6, 1e6))
    }

    #[test]
    fn snaps_to_nearest_edge_or_center() {
        let refs = box_points(Rect::from_two(P::new(0.0, 0.0), P::new(100.0, 100.0)));
        // caixa movida com a borda esquerda em 103 e o centro em 52 (y)
        let moving = box_points(Rect::from_two(P::new(103.0, 42.0), P::new(143.0, 62.0)));
        let d = offset(&moving, &refs, 8.0, (true, true));
        assert_eq!(d, P::new(-3.0, -2.0));
        // longe demais: não encaixa
        let d = offset(&moving, &refs, 1.0, (true, true));
        assert_eq!(d, P::new(0.0, 0.0));
        // só o eixo pedido
        let d = offset(&moving, &refs, 8.0, (false, true));
        assert_eq!(d, P::new(0.0, -2.0));
    }

    #[test]
    fn guides_join_aligned_points() {
        let refs = box_points(Rect::from_two(P::new(0.0, 0.0), P::new(100.0, 100.0)));
        let moving = box_points(Rect::from_two(P::new(100.0, 200.0), P::new(140.0, 240.0)));
        let g = guides(&moving, &refs);
        assert_eq!(g.len(), 1);
        assert!(g[0].vertical);
        assert_eq!(g[0].from, P::new(100.0, 0.0));
        assert_eq!(g[0].to, P::new(100.0, 240.0));
    }

    #[test]
    fn references_skip_moving_and_followers() {
        let a = rect("a", 0.0, 0.0, 10.0, 10.0);
        let b = rect("b", 50.0, 0.0, 10.0, 10.0);
        let mut label = Element::new("text", 0.0, 0.0, &Style::default());
        label.id = "t".into();
        label.set_field("containerId", serde_json::json!("b"));
        let mut arrow = Element::new("arrow", 0.0, 0.0, &Style::default());
        arrow.id = "arr".into();
        arrow.set_field("startBinding", serde_json::json!({"elementId": "a"}));
        let off = rect("far", 5000.0, 0.0, 10.0, 10.0);
        let els = vec![a, b, label, arrow, off];
        let skip: HashSet<String> = ["a".to_string()].into();
        let view = Rect::from_two(P::new(-100.0, -100.0), P::new(100.0, 100.0));
        let refs = reference_points(&els, &skip, view);
        assert_eq!(refs.len(), 5);
        assert!(refs.contains(&P::new(55.0, 5.0)));
        assert_eq!(reference_points(&els, &skip, big_view()).len(), 10);
    }
}
