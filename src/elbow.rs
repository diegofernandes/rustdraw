//! Setas em cotovelo (`elbowed` no Excalidraw): só segmentos horizontais e
//! verticais, com a rota calculada automaticamente em volta das formas ligadas.
//!
//! Cada ponta sai da forma por um lado ([`Heading`]), anda uma "folga"
//! ([`PADDING`]) para fora dela e o caminho entre essas duas saídas é procurado
//! numa grade esparsa (as bordas das formas com folga), com o menor número de
//! dobras e, entre esses, o mais curto — a mesma ideia do `elbowArrow.ts` do
//! Excalidraw.
//!
//! Segmentos arrastados pelo usuário ficam fixos (`fixedSegments`) e a rota
//! passa por eles.

use crate::binding::{self, Binding};
use crate::geom::{self, P, Rect, rotate};
use crate::model::Element;
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Quanto a rota se afasta das formas ligadas antes de dobrar.
pub const PADDING: f32 = 40.0;
/// Raio dos cantos arredondados.
pub const CORNER: f32 = 16.0;
const EPS: f32 = 0.01;
/// Custo de uma dobra: grande o bastante para que menos dobras sempre vença.
const BEND: f64 = 1e6;
/// Desempate: evita dobrar logo na saída ou na chegada (prefere o "Z" no meio).
const SOFT: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heading {
    Up,
    Right,
    Down,
    Left,
}

impl Heading {
    const ALL: [Heading; 4] = [Heading::Up, Heading::Right, Heading::Down, Heading::Left];

    pub fn vec(self) -> P {
        match self {
            Heading::Up => P::new(0.0, -1.0),
            Heading::Right => P::new(1.0, 0.0),
            Heading::Down => P::new(0.0, 1.0),
            Heading::Left => P::new(-1.0, 0.0),
        }
    }

    /// Direção dominante de um vetor (para a direita se for nulo).
    pub fn of(v: P) -> Heading {
        if v.x.abs() >= v.y.abs() {
            if v.x >= 0.0 {
                Heading::Right
            } else {
                Heading::Left
            }
        } else if v.y >= 0.0 {
            Heading::Down
        } else {
            Heading::Up
        }
    }

    pub fn opposite(self) -> Heading {
        match self {
            Heading::Up => Heading::Down,
            Heading::Right => Heading::Left,
            Heading::Down => Heading::Up,
            Heading::Left => Heading::Right,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Uma ponta da rota: onde fica, para onde a seta sai dela e a caixa da forma
/// ligada (que a rota contorna).
#[derive(Clone, Copy, Debug)]
pub struct End {
    pub p: P,
    pub heading: Heading,
    pub shape: Option<Rect>,
}

/// Segmento fixo (posição absoluta). `index` é o do ponto final do segmento,
/// como no Excalidraw: o segmento vai de `points[index - 1]` a `points[index]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub index: usize,
    pub start: P,
    pub end: P,
}

impl Segment {
    fn horizontal(&self) -> bool {
        (self.start.y - self.end.y).abs() < 0.5
    }
}

pub fn fixed_segments(el: &Element) -> Vec<Segment> {
    let pt = |v: &Value| {
        let a = v.as_array()?;
        Some(P::new(
            (el.x + a.first()?.as_f64()?) as f32,
            (el.y + a.get(1)?.as_f64()?) as f32,
        ))
    };
    el.extra
        .get("fixedSegments")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some(Segment {
                index: s.get("index")?.as_u64()? as usize,
                start: pt(s.get("start")?)?,
                end: pt(s.get("end")?)?,
            })
        })
        .filter(|s| {
            let d = s.end - s.start;
            d.len() > EPS && (d.x.abs() < 0.5 || d.y.abs() < 0.5)
        })
        .collect()
}

/// Grava os segmentos fixos (relativos a `el.x`, `el.y`); `null` se não houver.
pub fn set_fixed_segments(el: &mut Element, segs: &[Segment]) {
    let (ox, oy) = (el.x, el.y);
    let rel = |p: P| json!([p.x as f64 - ox, p.y as f64 - oy]);
    let v = if segs.is_empty() {
        Value::Null
    } else {
        Value::Array(
            segs.iter()
                .map(|s| json!({"start": rel(s.start), "end": rel(s.end), "index": s.index}))
                .collect(),
        )
    };
    el.set_field("fixedSegments", v);
}

/// Espelha os segmentos fixos junto com os pontos (relativos a `x`, `y`).
pub fn mirror_segments(el: &mut Element, horizontal: bool) {
    let Some(Value::Array(segs)) = el.extra.get_mut("fixedSegments") else {
        return;
    };
    let k = if horizontal { 0 } else { 1 };
    for seg in segs.iter_mut() {
        for key in ["start", "end"] {
            if let Some(v) = seg.get_mut(key).and_then(|p| p.get_mut(k))
                && let Some(c) = v.as_f64()
            {
                *v = json!(-c);
            }
        }
    }
}

/// Lado de saída de um ponto dado como fração da caixa da forma.
pub fn side_heading(target: &Element, f: [f64; 2]) -> Heading {
    let (dx, dy) = (f[0] - 0.5, f[1] - 0.5);
    let local = if dx.abs() >= dy.abs() {
        P::new(dx.signum() as f32, 0.0)
    } else {
        P::new(0.0, dy.signum() as f32)
    };
    Heading::of(rotate(local, P::new(0.0, 0.0), target.angle as f32))
}

/// Meio do lado da forma virado para `toward`.
pub fn auto_side(target: &Element, toward: P) -> [f64; 2] {
    let b = geom::local_box(target);
    let c = b.center();
    let q = rotate(toward, c, -(target.angle as f32)) - c;
    let (hw, hh) = (b.width().max(1.0) / 2.0, b.height().max(1.0) / 2.0);
    if (q.x / hw).abs() >= (q.y / hh).abs() {
        if q.x >= 0.0 { [1.0, 0.5] } else { [0.0, 0.5] }
    } else if q.y >= 0.0 {
        [0.5, 1.0]
    } else {
        [0.5, 0.0]
    }
}

/// Ponta ligada a `target`; devolve também o ponto da forma usado.
fn bound_end(target: &Element, b: &Binding, toward: P) -> (End, [f64; 2]) {
    let f = b.fixed.unwrap_or_else(|| auto_side(target, toward));
    let heading = side_heading(target, f);
    let p = binding::anchor_world(target, f) + heading.vec() * b.gap as f32;
    let end = End {
        p,
        heading,
        shape: Some(geom::aabb(target)),
    };
    (end, f)
}

// ---------------------------------------------------------------------------
// Rota
// ---------------------------------------------------------------------------

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

fn inside(r: &Rect, p: P) -> bool {
    p.x > r.min.x + EPS && p.x < r.max.x - EPS && p.y > r.min.y + EPS && p.y < r.max.y - EPS
}

/// Caixas com folga em volta das formas. Quando duas formas estão perto, a
/// folga entre elas é dividida ao meio, para sobrar um corredor.
fn padded(a: Option<Rect>, b: Option<Rect>) -> (Option<Rect>, Option<Rect>) {
    // [esquerda, cima, direita, baixo]
    let grow = |r: Rect, p: [f32; 4]| Rect {
        min: P::new(r.min.x - p[0], r.min.y - p[1]),
        max: P::new(r.max.x + p[2], r.max.y + p[3]),
    };
    let full = [PADDING; 4];
    match (a, b) {
        (Some(a), Some(b)) => {
            if overlaps(&a, &b) {
                // formas sobrepostas: não há como contornar as duas
                return (None, None);
            }
            let (mut pa, mut pb) = (full, full);
            if overlaps(&grow(a, pa), &grow(b, pb)) {
                if a.max.x <= b.min.x {
                    let g = (b.min.x - a.max.x) / 2.0;
                    pa[2] = pa[2].min(g);
                    pb[0] = pb[0].min(g);
                } else if b.max.x <= a.min.x {
                    let g = (a.min.x - b.max.x) / 2.0;
                    pb[2] = pb[2].min(g);
                    pa[0] = pa[0].min(g);
                }
                if a.max.y <= b.min.y {
                    let g = (b.min.y - a.max.y) / 2.0;
                    pa[3] = pa[3].min(g);
                    pb[1] = pb[1].min(g);
                } else if b.max.y <= a.min.y {
                    let g = (a.min.y - b.max.y) / 2.0;
                    pb[3] = pb[3].min(g);
                    pa[1] = pa[1].min(g);
                }
            }
            (Some(grow(a, pa)), Some(grow(b, pb)))
        }
        (a, b) => (a.map(|r| grow(r, full)), b.map(|r| grow(r, full))),
    }
}

/// Onde a ponta sai da caixa com folga, andando na sua direção.
fn dongle(e: &End, r: Option<Rect>) -> P {
    let Some(r) = r else { return e.p };
    match e.heading {
        Heading::Right => P::new(e.p.x.max(r.max.x), e.p.y),
        Heading::Left => P::new(e.p.x.min(r.min.x), e.p.y),
        Heading::Down => P::new(e.p.x, e.p.y.max(r.max.y)),
        Heading::Up => P::new(e.p.x, e.p.y.min(r.min.y)),
    }
}

#[derive(PartialEq)]
struct Node {
    cost: f64,
    state: usize,
}

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost)
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn coords(mut v: Vec<f32>) -> Vec<f32> {
    v.sort_by(f32::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < EPS);
    v
}

/// Menor caminho ortogonal de `from` (saindo na direção `h0`) até `to`
/// (chegando no sentido contrário de `h1`), sem entrar nos obstáculos.
fn search(from: P, h0: Heading, to: P, h1: Heading, obs: &[Rect]) -> Option<Vec<P>> {
    if obs.iter().any(|r| inside(r, from) || inside(r, to)) {
        return None;
    }
    let mut xs = vec![from.x, to.x, (from.x + to.x) / 2.0];
    let mut ys = vec![from.y, to.y, (from.y + to.y) / 2.0];
    for r in obs {
        xs.extend([r.min.x, r.max.x]);
        ys.extend([r.min.y, r.max.y]);
    }
    if let [a, b] = obs {
        for (lo, hi) in [(a, b), (b, a)] {
            if lo.max.x < hi.min.x {
                xs.push((lo.max.x + hi.min.x) / 2.0);
            }
            if lo.max.y < hi.min.y {
                ys.push((lo.max.y + hi.min.y) / 2.0);
            }
        }
    }
    let (xs, ys) = (coords(xs), coords(ys));
    let (nx, ny) = (xs.len(), ys.len());
    let at = |x: f32, v: &[f32]| v.iter().position(|c| (c - x).abs() < EPS);
    let start = at(from.x, &xs)? + at(from.y, &ys)? * nx;
    let goal = at(to.x, &xs)? + at(to.y, &ys)? * nx;
    let pos = |n: usize| P::new(xs[n % nx], ys[n / nx]);
    let blocked: Vec<bool> = (0..nx * ny)
        .map(|n| obs.iter().any(|r| inside(r, pos(n))))
        .collect();

    let states = nx * ny * 4;
    let mut dist = vec![f64::INFINITY; states];
    let mut prev = vec![usize::MAX; states];
    let mut heap = BinaryHeap::new();
    let s0 = start * 4 + h0.index();
    dist[s0] = 0.0;
    heap.push(Node {
        cost: 0.0,
        state: s0,
    });
    while let Some(Node { cost, state }) = heap.pop() {
        if cost > dist[state] {
            continue;
        }
        let (n, dir) = (state / 4, Heading::ALL[state % 4]);
        if n == goal {
            let mut path = vec![pos(n)];
            let mut s = state;
            while prev[s] != usize::MAX {
                s = prev[s];
                path.push(pos(s / 4));
            }
            path.reverse();
            return Some(path);
        }
        let (i, j) = ((n % nx) as isize, (n / nx) as isize);
        for d in Heading::ALL {
            if d == dir.opposite() {
                continue;
            }
            let v = d.vec();
            let (ni, nj) = (i + v.x as isize, j + v.y as isize);
            if ni < 0 || nj < 0 || ni >= nx as isize || nj >= ny as isize {
                continue;
            }
            let m = ni as usize + nj as usize * nx;
            if blocked[m] {
                continue;
            }
            let (a, b) = (pos(n), pos(m));
            let mid = (a + b) * 0.5;
            if obs.iter().any(|r| inside(r, mid)) {
                continue;
            }
            let mut c = cost + (b - a).len() as f64;
            if d != dir {
                c += BEND;
                if n == start {
                    c += SOFT;
                }
            }
            if m == goal {
                // chegar no sentido da ponta conta como dobra; voltando, duas
                if d == h1 {
                    c += 2.0 * BEND;
                } else if d != h1.opposite() {
                    c += BEND + SOFT;
                }
            }
            let ns = m * 4 + d.index();
            if c < dist[ns] {
                dist[ns] = c;
                prev[ns] = state;
                heap.push(Node { cost: c, state: ns });
            }
        }
    }
    None
}

/// Tira pontos repetidos e pontos no meio de um trecho reto.
pub fn simplify(pts: Vec<P>) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_some_and(|q| (*q - p).len() < 0.5) {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let same_x = (a.x - b.x).abs() < 0.5 && (b.x - p.x).abs() < 0.5;
            let same_y = (a.y - b.y).abs() < 0.5 && (b.y - p.y).abs() < 0.5;
            if same_x || same_y {
                out.pop();
            }
        }
        out.push(p);
    }
    if out.len() == 1 {
        out.push(out[0]);
    }
    out
}

/// Rota ortogonal entre duas pontas.
pub fn route(a: &End, b: &End) -> Vec<P> {
    let (ra, rb) = padded(a.shape, b.shape);
    let (da, db) = (dongle(a, ra), dongle(b, rb));
    let obs: Vec<Rect> = [ra, rb].into_iter().flatten().collect();
    let middle = search(da, a.heading, db, b.heading, &obs)
        .or_else(|| search(da, a.heading, db, b.heading, &[]))
        .unwrap_or_else(|| vec![da, db]);
    let mut pts = vec![a.p];
    pts.extend(middle);
    pts.push(b.p);
    simplify(pts)
}

/// Rota passando pelos segmentos fixos, na ordem.
fn route_through(a: &End, segs: &[Segment], b: &End) -> Vec<P> {
    let mut pts = Vec::new();
    let mut cur = *a;
    for s in segs {
        let d = Heading::of(s.end - s.start);
        let to = End {
            p: s.start,
            heading: d.opposite(),
            shape: None,
        };
        pts.extend(route(&cur, &to));
        cur = End {
            p: s.end,
            heading: d,
            shape: None,
        };
    }
    pts.extend(route(&cur, b));
    simplify(pts)
}

/// Encontra cada segmento fixo na rota nova (mesma reta, trechos que se cruzam).
fn reindex(pts: &[P], segs: &[Segment]) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for s in segs {
        let hz = s.horizontal();
        let found = (1..pts.len()).find(|&i| {
            let (a, b) = (pts[i - 1], pts[i]);
            let (along, lo, hi, slo, shi) = if hz {
                (
                    (a.y - s.start.y).abs() < 0.5 && (b.y - s.start.y).abs() < 0.5,
                    a.x,
                    b.x,
                    s.start.x,
                    s.end.x,
                )
            } else {
                (
                    (a.x - s.start.x).abs() < 0.5 && (b.x - s.start.x).abs() < 0.5,
                    a.y,
                    b.y,
                    s.start.y,
                    s.end.y,
                )
            };
            along && lo.min(hi) <= slo.max(shi) && slo.min(shi) <= lo.max(hi)
        });
        // o primeiro e o último segmentos saem das formas: não ficam fixos
        if let Some(i) = found.filter(|&i| i > 1 && i + 1 < pts.len())
            && out.iter().all(|o| o.index != i)
        {
            out.push(Segment {
                index: i,
                start: pts[i - 1],
                end: pts[i],
            });
        }
    }
    out.sort_by_key(|s| s.index);
    out
}

/// A seta em cotovelo recalculada a partir das pontas e ligações atuais.
pub fn rerouted<'a>(arrow: &Element, find: &impl Fn(&str) -> Option<&'a Element>) -> Element {
    let c = geom::center(arrow);
    let a = arrow.angle as f32;
    let pts: Vec<P> = geom::abs_points(arrow)
        .into_iter()
        .map(|p| rotate(p, c, a))
        .collect();
    let (p0, p1) = match pts.as_slice() {
        [] => (
            P::new(arrow.x as f32, arrow.y as f32),
            P::new(arrow.x as f32, arrow.y as f32),
        ),
        [p] => (*p, *p),
        [p, .., q] => (*p, *q),
    };
    let resolve = |end: bool| {
        let b = binding::get(arrow, end)?;
        let t = find(&b.element_id).filter(|t| binding::is_bindable(t))?;
        Some((t, b))
    };
    let (sb, eb) = (resolve(false), resolve(true));
    let reference = |x: &Option<(&Element, Binding)>, raw: P| match x {
        Some((t, b)) => b
            .fixed
            .map(|f| binding::anchor_world(t, f))
            .unwrap_or_else(|| geom::center(t)),
        None => raw,
    };
    let (r0, r1) = (reference(&sb, p0), reference(&eb, p1));
    let start = sb.as_ref().map(|(t, b)| bound_end(t, b, r1));
    let end = eb.as_ref().map(|(t, b)| bound_end(t, b, r0));
    let s_pt = start.map(|(e, _)| e.p).unwrap_or(p0);
    let e_pt = end.map(|(e, _)| e.p).unwrap_or(p1);
    let start_end = start.map(|(e, _)| e).unwrap_or(End {
        p: p0,
        heading: Heading::of(e_pt - p0),
        shape: None,
    });
    let end_end = end.map(|(e, _)| e).unwrap_or(End {
        p: p1,
        heading: Heading::of(s_pt - p1),
        shape: None,
    });

    let mut segs = fixed_segments(arrow);
    segs.sort_by_key(|s| s.index);
    let path = if segs.is_empty() {
        route(&start_end, &end_end)
    } else {
        route_through(&start_end, &segs, &end_end)
    };
    let segs = reindex(&path, &segs);

    let mut el = arrow.clone();
    el.angle = 0.0;
    el.roundness = None;
    el.x = path[0].x as f64;
    el.y = path[0].y as f64;
    el.set_points(
        path.iter()
            .map(|p| [(p.x - path[0].x) as f64, (p.y - path[0].y) as f64])
            .collect(),
    );
    set_fixed_segments(&mut el, &segs);
    for (is_end, side, b) in [(false, start, &sb), (true, end, &eb)] {
        if let (Some((_, f)), Some((_, b))) = (side, b)
            && b.fixed.is_none()
        {
            let mut b = b.clone();
            b.side = Some(f);
            binding::set(&mut el, is_end, Some(&b));
        }
    }
    el
}

/// Caminho com os cantos arredondados, para desenhar.
pub fn rounded_path(pts: &[P]) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for i in 1..pts.len() - 1 {
        let (a, p, b) = (pts[i - 1], pts[i], pts[i + 1]);
        let r = CORNER.min((p - a).len() / 2.0).min((b - p).len() / 2.0);
        if r < 0.5 {
            out.push(p);
            continue;
        }
        let s = p + (a - p).normalized() * r;
        let e = p + (b - p).normalized() * r;
        for k in 0..=4 {
            let t = k as f32 / 4.0;
            let u = 1.0 - t;
            out.push(s * (u * u) + p * (2.0 * u * t) + e * (t * t));
        }
    }
    out.push(pts[pts.len() - 1]);
    out
}

/// Segmentos que podem ser arrastados (todos menos o primeiro e o último),
/// pelo índice do ponto onde começam.
pub fn movable_segments(n_points: usize) -> std::ops::Range<usize> {
    1..n_points.saturating_sub(2)
}

/// Move o segmento que começa no ponto `i` para passar por `p` (na
/// perpendicular) e o marca como fixo. Pontos em coordenadas absolutas.
pub fn drag_segment(el: &mut Element, i: usize, p: P) {
    let mut pts = geom::abs_points(el);
    if i + 1 >= pts.len() {
        return;
    }
    let horizontal = (pts[i].y - pts[i + 1].y).abs() < (pts[i].x - pts[i + 1].x).abs();
    if horizontal {
        pts[i].y = p.y;
        pts[i + 1].y = p.y;
    } else {
        pts[i].x = p.x;
        pts[i + 1].x = p.x;
    }
    let mut idx: Vec<usize> = fixed_segments(el).iter().map(|s| s.index).collect();
    if !idx.contains(&(i + 1)) {
        idx.push(i + 1);
    }
    idx.sort();
    let segs: Vec<Segment> = idx
        .into_iter()
        .filter(|&k| k >= 1 && k < pts.len())
        .map(|k| Segment {
            index: k,
            start: pts[k - 1],
            end: pts[k],
        })
        .collect();
    el.set_abs_points(
        &pts.iter()
            .map(|p| [p.x as f64, p.y as f64])
            .collect::<Vec<_>>(),
    );
    set_fixed_segments(el, &segs);
}

/// Solta o segmento fixo que começa no ponto `i` (volta a ser automático).
pub fn release_segment(el: &mut Element, i: usize) -> bool {
    let segs = fixed_segments(el);
    let keep: Vec<Segment> = segs.iter().copied().filter(|s| s.index != i + 1).collect();
    if keep.len() == segs.len() {
        return false;
    }
    set_fixed_segments(el, &keep);
    true
}

/// Converte uma seta para cotovelo (ou de volta), mantendo as pontas.
pub fn set_elbowed(el: &mut Element, on: bool) {
    let pts = geom::abs_points(el);
    el.set_field("elbowed", Value::Bool(on));
    el.set_field("fixedSegments", Value::Null);
    if on {
        el.roundness = None;
        for k in ["startIsSpecial", "endIsSpecial"] {
            el.extra.entry(k).or_insert(Value::Null);
        }
    } else {
        // o lado escolhido só vale para cotovelo
        for end in [false, true] {
            if let Some(mut b) = binding::get(el, end).filter(|b| b.side.is_some()) {
                b.side = None;
                binding::set(el, end, Some(&b));
            }
        }
    }
    // os pontos do meio não fazem sentido no outro tipo
    if let (Some(a), Some(b)) = (pts.first(), pts.last())
        && pts.len() > 2
    {
        let ends = [[a.x as f64, a.y as f64], [b.x as f64, b.y as f64]];
        el.set_abs_points(&ends);
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

    fn elbow(a: [f64; 2], b: [f64; 2]) -> Element {
        let style = Style {
            arrow_type: "elbow".into(),
            ..Style::default()
        };
        let mut e = Element::new("arrow", a[0], a[1], &style);
        e.set_points(vec![[0.0, 0.0], [b[0] - a[0], b[1] - a[1]]]);
        e
    }

    fn orthogonal(pts: &[P]) -> bool {
        pts.windows(2)
            .all(|w| (w[0].x - w[1].x).abs() < 0.01 || (w[0].y - w[1].y).abs() < 0.01)
    }

    fn crosses(pts: &[P], r: &Rect) -> bool {
        pts.windows(2).any(|w| {
            (0..=20).any(|k| {
                let p = w[0] + (w[1] - w[0]) * (k as f32 / 20.0);
                inside(r, p)
            })
        })
    }

    #[test]
    fn free_elbow_is_a_z() {
        let e = elbow([0.0, 0.0], [200.0, 100.0]);
        let r = rerouted(&e, &|_: &str| None);
        let pts = geom::abs_points(&r);
        assert!(orthogonal(&pts), "{pts:?}");
        assert_eq!(pts.len(), 4, "{pts:?}");
        assert_eq!(pts[0], P::new(0.0, 0.0));
        assert_eq!(pts[3], P::new(200.0, 100.0));
        assert!((pts[1].x - 100.0).abs() < 0.01, "dobra no meio: {pts:?}");
        assert!(!r.is_round());
    }

    #[test]
    fn bound_elbow_goes_around_shapes() {
        // seta da direita de r1 até a direita de r2, que está à esquerda e abaixo
        let r1 = rect(300.0, 0.0);
        let r2 = rect(0.0, 200.0);
        let mut e = elbow([405.0, 25.0], [105.0, 225.0]);
        binding::set(&mut e, false, Some(&Binding::new(&r1.id, Some([1.0, 0.5]))));
        binding::set(&mut e, true, Some(&Binding::new(&r2.id, Some([1.0, 0.5]))));
        let els = [r1.clone(), r2.clone()];
        let find = |id: &str| els.iter().find(|e| e.id == id);
        let r = rerouted(&e, &find);
        let pts = geom::abs_points(&r);
        assert!(orthogonal(&pts), "{pts:?}");
        assert!(close(pts[0], P::new(405.0, 25.0)), "{pts:?}");
        assert!(close(*pts.last().unwrap(), P::new(105.0, 225.0)), "{pts:?}");
        // sai para a direita e chega pela direita
        assert!(pts[1].x > pts[0].x, "{pts:?}");
        assert!(pts[pts.len() - 2].x > pts[pts.len() - 1].x, "{pts:?}");
        for s in [&r1, &r2] {
            assert!(!crosses(&pts, &geom::aabb(s)), "{pts:?}");
        }
    }

    fn close(a: P, b: P) -> bool {
        (a - b).len() < 0.05
    }

    #[test]
    fn free_binding_picks_facing_side_and_saves_it() {
        let r1 = rect(0.0, 0.0);
        let r2 = rect(400.0, 0.0);
        let mut e = elbow([50.0, 25.0], [450.0, 25.0]);
        binding::set(&mut e, false, Some(&Binding::new(&r1.id, None)));
        binding::set(&mut e, true, Some(&Binding::new(&r2.id, None)));
        let els = [r1.clone(), r2.clone()];
        let find = |id: &str| els.iter().find(|e| e.id == id);
        let r = rerouted(&e, &find);
        let pts = geom::abs_points(&r);
        assert_eq!(pts.len(), 2, "reta entre lados de frente: {pts:?}");
        assert!(close(pts[0], P::new(105.0, 25.0)), "{pts:?}");
        assert!(close(pts[1], P::new(395.0, 25.0)), "{pts:?}");
        let v = r.extra["startBinding"].clone();
        assert_eq!(v["mode"], json!("orbit"));
        assert_eq!(v["fixedPoint"], json!([1.0, 0.5]));
        // continua livre ao reabrir
        assert_eq!(binding::get(&r, false).unwrap().fixed, None);
    }

    #[test]
    fn dragged_segment_stays_fixed() {
        let mut e = elbow([0.0, 0.0], [200.0, 100.0]);
        e = rerouted(&e, &|_: &str| None);
        assert_eq!(movable_segments(e.points().len()), 1..2);
        drag_segment(&mut e, 1, P::new(150.0, 50.0));
        let pts = geom::abs_points(&e);
        assert!((pts[1].x - 150.0).abs() < 0.01 && (pts[2].x - 150.0).abs() < 0.01);
        assert_eq!(fixed_segments(&e).len(), 1);
        // mover a ponta final mantém o segmento em x = 150
        let mut pts: Vec<[f64; 2]> = pts.iter().map(|p| [p.x as f64, p.y as f64]).collect();
        let n = pts.len();
        pts[n - 1] = [300.0, 200.0];
        e.set_abs_points(&pts);
        let r = rerouted(&e, &|_: &str| None);
        let pts = geom::abs_points(&r);
        assert!(orthogonal(&pts), "{pts:?}");
        assert!(close(*pts.last().unwrap(), P::new(300.0, 200.0)), "{pts:?}");
        let segs = fixed_segments(&r);
        assert_eq!(segs.len(), 1, "{pts:?}");
        assert!((segs[0].start.x - 150.0).abs() < 0.01, "{segs:?}");
        // soltar volta à rota automática
        let mut r = r;
        assert!(release_segment(&mut r, segs[0].index - 1));
        let pts = geom::abs_points(&rerouted(&r, &|_: &str| None));
        assert!((pts[1].x - 150.0).abs() < 0.01, "{pts:?}");
    }

    #[test]
    fn segments_survive_moving_the_whole_arrow() {
        let mut e = rerouted(&elbow([0.0, 0.0], [200.0, 100.0]), &|_: &str| None);
        drag_segment(&mut e, 1, P::new(150.0, 50.0));
        e.x += 10.0;
        e.y += 10.0;
        let segs = fixed_segments(&e);
        assert!((segs[0].start.x - 160.0).abs() < 0.01, "{segs:?}");
    }

    #[test]
    fn rounded_corners() {
        let pts = [P::new(0.0, 0.0), P::new(100.0, 0.0), P::new(100.0, 100.0)];
        let r = rounded_path(&pts);
        assert!(r.len() > 3);
        assert_eq!(r[0], pts[0]);
        assert_eq!(*r.last().unwrap(), pts[2]);
        assert!(!r.contains(&pts[1]), "o canto é cortado pela curva");
    }
}
