//! Geometria: vetores, caixas, conversão de elementos em primitivas de
//! desenho (independentes do backend) e teste de acerto (hit-test).

use crate::model::{Element, parse_color};
use crate::rough::{self, Opts, Rng};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct P {
    pub x: f32,
    pub y: f32,
}

impl P {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn len_sq(self) -> f32 {
        self.x * self.x + self.y * self.y
    }
    pub fn len(self) -> f32 {
        self.len_sq().sqrt()
    }
    pub fn dot(self, o: P) -> f32 {
        self.x * o.x + self.y * o.y
    }
    pub fn normalized(self) -> P {
        let l = self.len();
        if l < 1e-6 {
            P::new(0.0, 0.0)
        } else {
            self * (1.0 / l)
        }
    }
}

impl Add for P {
    type Output = P;
    fn add(self, o: P) -> P {
        P::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for P {
    type Output = P;
    fn sub(self, o: P) -> P {
        P::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for P {
    type Output = P;
    fn mul(self, s: f32) -> P {
        P::new(self.x * s, self.y * s)
    }
}
impl Neg for P {
    type Output = P;
    fn neg(self) -> P {
        P::new(-self.x, -self.y)
    }
}

/// Gira `p` em torno de `c` por `a` radianos.
pub fn rotate(p: P, c: P, a: f32) -> P {
    if a == 0.0 {
        return p;
    }
    let (s, co) = a.sin_cos();
    let d = p - c;
    P::new(c.x + d.x * co - d.y * s, c.y + d.x * s + d.y * co)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: P,
    pub max: P,
}

impl Rect {
    pub fn from_two(a: P, b: P) -> Self {
        Self {
            min: P::new(a.x.min(b.x), a.y.min(b.y)),
            max: P::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }
    pub fn from_points(pts: &[P]) -> Option<Self> {
        let first = *pts.first()?;
        let mut r = Rect {
            min: first,
            max: first,
        };
        for p in pts {
            r.min = P::new(r.min.x.min(p.x), r.min.y.min(p.y));
            r.max = P::new(r.max.x.max(p.x), r.max.y.max(p.y));
        }
        Some(r)
    }
    pub fn union(self, o: Rect) -> Rect {
        Rect {
            min: P::new(self.min.x.min(o.min.x), self.min.y.min(o.min.y)),
            max: P::new(self.max.x.max(o.max.x), self.max.y.max(o.max.y)),
        }
    }
    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }
    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }
    pub fn center(&self) -> P {
        (self.min + self.max) * 0.5
    }
    pub fn expand(self, m: f32) -> Rect {
        Rect {
            min: self.min - P::new(m, m),
            max: self.max + P::new(m, m),
        }
    }
    pub fn contains(&self, p: P) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
    pub fn contains_rect(&self, o: &Rect) -> bool {
        self.contains(o.min) && self.contains(o.max)
    }
    /// Parte em comum, se houver.
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let r = Rect {
            min: P::new(self.min.x.max(o.min.x), self.min.y.max(o.min.y)),
            max: P::new(self.max.x.min(o.max.x), self.max.y.min(o.max.y)),
        };
        (r.min.x <= r.max.x && r.min.y <= r.max.y).then_some(r)
    }
}

// ---------------------------------------------------------------------------
// Caixa local, centro e caixa envolvente de um elemento
// ---------------------------------------------------------------------------

/// Caixa do elemento sem rotação (coordenadas de mundo).
pub fn local_box(el: &Element) -> Rect {
    let (x, y) = (el.x as f32, el.y as f32);
    if el.has_points() {
        let (a, b, c, d) = el.points_extent();
        Rect::from_two(
            P::new(x + a as f32, y + b as f32),
            P::new(x + c as f32, y + d as f32),
        )
    } else {
        Rect::from_two(
            P::new(x, y),
            P::new(x + el.width as f32, y + el.height as f32),
        )
    }
}

pub fn center(el: &Element) -> P {
    local_box(el).center()
}

/// Cantos da caixa local já rotacionados (nw, ne, se, sw).
pub fn corners(el: &Element) -> [P; 4] {
    let b = local_box(el);
    let c = b.center();
    let a = el.angle as f32;
    [
        rotate(b.min, c, a),
        rotate(P::new(b.max.x, b.min.y), c, a),
        rotate(b.max, c, a),
        rotate(P::new(b.min.x, b.max.y), c, a),
    ]
}

/// Caixa envolvente alinhada aos eixos, considerando rotação.
pub fn aabb(el: &Element) -> Rect {
    if el.has_points() && el.angle != 0.0 {
        let c = center(el);
        let a = el.angle as f32;
        let pts: Vec<P> = abs_points(el)
            .into_iter()
            .map(|p| rotate(p, c, a))
            .collect();
        return Rect::from_points(&pts).unwrap_or(local_box(el));
    }
    Rect::from_points(&corners(el)).unwrap()
}

pub fn abs_points(el: &Element) -> Vec<P> {
    let (x, y) = (el.x as f32, el.y as f32);
    el.points()
        .iter()
        .map(|p| P::new(x + p[0] as f32, y + p[1] as f32))
        .collect()
}

pub fn scene_bounds<'a>(els: impl IntoIterator<Item = &'a Element>) -> Option<Rect> {
    let mut r: Option<Rect> = None;
    for e in els {
        let b = aabb(e).expand(e.stroke_width as f32);
        r = Some(r.map_or(b, |r| r.union(b)));
    }
    r
}

// ---------------------------------------------------------------------------
// Primitivas de desenho
// ---------------------------------------------------------------------------

pub type Rgba = [u8; 4];

#[derive(Clone, Debug)]
pub enum Prim {
    /// Polilinha com espessura e (opcionalmente) tracejado `(traço, espaço)`.
    Stroke {
        pts: Vec<P>,
        color: Rgba,
        width: f32,
        dash: Option<(f32, f32)>,
    },
    /// Polígono preenchido.
    Fill { pts: Vec<P>, color: Rgba },
    /// Bloco de texto. `origin` é o canto superior esquerdo sem rotação.
    Text {
        origin: P,
        width: f32,
        text: String,
        size: f32,
        family: i64,
        color: Rgba,
        align: u8,
        line_height: f32,
        angle: f32,
        center: P,
    },
    /// Imagem de `files`. `min`/`max` é a caixa sem rotação; `uv` o recorte
    /// (0..1) da imagem original, já com o espelhamento aplicado.
    Image {
        file_id: String,
        min: P,
        max: P,
        uv: [f32; 4],
        angle: f32,
        center: P,
        opacity: f32,
    },
}

/// Recorte (0..1) de um elemento `image`, invertido se a escala for negativa.
fn image_uv(el: &Element) -> [f32; 4] {
    let mut uv = [0.0, 0.0, 1.0, 1.0];
    if let Some(c) = el.extra.get("crop").filter(|c| c.is_object()) {
        let f = |k: &str| c.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let (nw, nh) = (f("naturalWidth"), f("naturalHeight"));
        if nw > 0.0 && nh > 0.0 && f("width") > 0.0 && f("height") > 0.0 {
            uv = [
                f("x") / nw,
                f("y") / nh,
                (f("x") + f("width")) / nw,
                (f("y") + f("height")) / nh,
            ];
        }
    }
    let scale = el.extra.get("scale").and_then(|v| v.as_array());
    let neg = |i: usize| {
        scale
            .and_then(|s| s.get(i))
            .and_then(|v| v.as_f64())
            .is_some_and(|v| v < 0.0)
    };
    if neg(0) {
        uv.swap(0, 2);
    }
    if neg(1) {
        uv.swap(1, 3);
    }
    uv
}

/// O que desenhar no lugar de uma imagem que não está em `files` (ou não decodifica).
pub fn image_placeholder(prim: &Prim) -> Vec<Prim> {
    let Prim::Image {
        min,
        max,
        angle,
        center,
        opacity,
        ..
    } = prim
    else {
        return vec![];
    };
    let a = (255.0 * opacity) as u8;
    let pts: Vec<P> = [*min, P::new(max.x, min.y), *max, P::new(min.x, max.y), *min]
        .iter()
        .map(|p| rotate(*p, *center, *angle))
        .collect();
    vec![
        Prim::Fill {
            pts: pts[..4].to_vec(),
            color: [233, 236, 239, a],
        },
        Prim::Stroke {
            pts,
            color: [134, 142, 150, a],
            width: 1.5,
            dash: Some((6.0, 4.0)),
        },
    ]
}

fn with_alpha(c: Option<Rgba>, opacity: f32) -> Option<Rgba> {
    c.map(|mut c| {
        c[3] = (c[3] as f32 * opacity).round() as u8;
        c
    })
}

/// Rugosidade ajustada para elementos pequenos (como o Excalidraw faz).
fn adjust_roughness(el: &Element) -> f32 {
    let r = el.roughness as f32;
    let max = el.width.max(el.height) as f32;
    let min = el.width.min(el.height) as f32;
    if (min >= 20.0 && max >= 50.0) || min >= 15.0 || el.is_linear() {
        return r;
    }
    (r / if max < 10.0 { 3.0 } else { 2.0 }).min(2.5)
}

pub fn corner_radius(el: &Element) -> f32 {
    let m = el.width.abs().min(el.height.abs()) as f32;
    let ty = el
        .roundness
        .as_ref()
        .and_then(|r| r.get("type"))
        .and_then(|t| t.as_i64())
        .unwrap_or(3);
    if ty == 3 {
        if m <= 128.0 { m * 0.25 } else { 32.0 }
    } else {
        m * 0.25
    }
}

fn arc_points(c: P, r: f32, a0: f32, a1: f32, n: usize) -> Vec<P> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            P::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

fn rounded_rect_outline(b: &Rect, r: f32) -> Vec<P> {
    use std::f32::consts::PI;
    let (x0, y0, x1, y1) = (b.min.x, b.min.y, b.max.x, b.max.y);
    let mut v = Vec::new();
    v.extend(arc_points(P::new(x1 - r, y0 + r), r, -PI / 2.0, 0.0, 6));
    v.extend(arc_points(P::new(x1 - r, y1 - r), r, 0.0, PI / 2.0, 6));
    v.extend(arc_points(P::new(x0 + r, y1 - r), r, PI / 2.0, PI, 6));
    v.extend(arc_points(P::new(x0 + r, y0 + r), r, PI, 1.5 * PI, 6));
    v
}

fn ellipse_outline(c: P, rx: f32, ry: f32) -> Vec<P> {
    (0..64)
        .map(|i| {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            P::new(c.x + rx * a.cos(), c.y + ry * a.sin())
        })
        .collect()
}

fn diamond_points(b: &Rect) -> Vec<P> {
    let c = b.center();
    vec![
        P::new(c.x, b.min.y),
        P::new(b.max.x, c.y),
        P::new(c.x, b.max.y),
        P::new(b.min.x, c.y),
    ]
}

/// Contorno fechado de uma forma (sem rotação, coordenadas de mundo), usado
/// para ligar setas. `None` para elementos que não têm contorno fechado.
pub fn outline(el: &Element) -> Option<Vec<P>> {
    let b = local_box(el);
    Some(match el.kind.as_str() {
        "rectangle" if el.is_round() => {
            let r = corner_radius(el).min(b.width() / 2.0).min(b.height() / 2.0);
            rounded_rect_outline(&b, r)
        }
        "rectangle" | "text" | "image" => vec![
            b.min,
            P::new(b.max.x, b.min.y),
            b.max,
            P::new(b.min.x, b.max.y),
        ],
        "diamond" => diamond_points(&b),
        "ellipse" => ellipse_outline(b.center(), b.width() / 2.0, b.height() / 2.0),
        _ => return None,
    })
}

/// Converte um elemento em primitivas (coordenadas de mundo, já rotacionadas).
pub fn element_prims(el: &Element) -> Vec<Prim> {
    let opacity = (el.opacity as f32 / 100.0).clamp(0.0, 1.0);
    let stroke = with_alpha(parse_color(&el.stroke_color), opacity);
    let bg = with_alpha(parse_color(&el.background_color), opacity);
    let solid = el.stroke_style == "solid";
    let o = Opts::new(adjust_roughness(el), solid);
    let mut rng = Rng::new(el.seed);
    let sw = el.stroke_width as f32 + if solid { 0.0 } else { 0.5 };
    let dash = match el.stroke_style.as_str() {
        "dashed" => Some((8.0, 8.0 + sw)),
        "dotted" => Some((1.5, 6.0 + sw)),
        _ => None,
    };
    let b = local_box(el);
    let mut prims: Vec<Prim> = Vec::new();

    let push_strokes = |prims: &mut Vec<Prim>, lines: Vec<Vec<P>>, width: f32, dash| {
        if let Some(color) = stroke {
            for pts in lines {
                prims.push(Prim::Stroke {
                    pts,
                    color,
                    width,
                    dash,
                });
            }
        }
    };

    // Preenchimento (sólido / hachura / hachura cruzada) de um contorno.
    let fill = |prims: &mut Vec<Prim>, outline: &[P], rng: &mut Rng| {
        let Some(color) = bg else { return };
        match el.fill_style.as_str() {
            "solid" => prims.push(Prim::Fill {
                pts: outline.to_vec(),
                color,
            }),
            style => {
                let gap = (el.stroke_width as f32 * 4.0).max(4.0);
                let fw = (el.stroke_width as f32 / 2.0).max(0.5);
                let mut angles = vec![-41.0];
                if style == "cross-hatch" {
                    angles.push(49.0);
                }
                let fo = Opts::new(o.roughness, o.roughness > 0.0);
                for ang in angles {
                    for (a, bb) in rough::hachure(outline, gap, ang) {
                        for pts in rough::line(a, bb, &fo, rng) {
                            prims.push(Prim::Stroke {
                                pts,
                                color,
                                width: fw,
                                dash: None,
                            });
                        }
                    }
                }
            }
        }
    };

    match el.kind.as_str() {
        "rectangle" => {
            if el.is_round() {
                let r = corner_radius(el).min(b.width() / 2.0).min(b.height() / 2.0);
                let outline = rounded_rect_outline(&b, r);
                fill(&mut prims, &outline, &mut rng);
                let (x0, y0, x1, y1) = (b.min.x, b.min.y, b.max.x, b.max.y);
                let mut lines = Vec::new();
                for (a, bb) in [
                    (P::new(x0 + r, y0), P::new(x1 - r, y0)),
                    (P::new(x1, y0 + r), P::new(x1, y1 - r)),
                    (P::new(x1 - r, y1), P::new(x0 + r, y1)),
                    (P::new(x0, y1 - r), P::new(x0, y0 + r)),
                ] {
                    lines.extend(rough::line(a, bb, &o, &mut rng));
                }
                use std::f32::consts::PI;
                for (c, a0) in [
                    (P::new(x1 - r, y0 + r), -PI / 2.0),
                    (P::new(x1 - r, y1 - r), 0.0),
                    (P::new(x0 + r, y1 - r), PI / 2.0),
                    (P::new(x0 + r, y0 + r), PI),
                ] {
                    let arc = arc_points(c, r, a0, a0 + PI / 2.0, 4);
                    lines.extend(rough::curve(&arc, &o, &mut rng));
                }
                push_strokes(&mut prims, lines, sw, dash);
            } else {
                let pts = [
                    b.min,
                    P::new(b.max.x, b.min.y),
                    b.max,
                    P::new(b.min.x, b.max.y),
                ];
                fill(&mut prims, &pts, &mut rng);
                push_strokes(
                    &mut prims,
                    rough::linear_path(&pts, true, &o, &mut rng),
                    sw,
                    dash,
                );
            }
        }
        "diamond" => {
            let pts = diamond_points(&b);
            fill(&mut prims, &pts, &mut rng);
            push_strokes(
                &mut prims,
                rough::linear_path(&pts, true, &o, &mut rng),
                sw,
                dash,
            );
        }
        "ellipse" => {
            let c = b.center();
            let outline = ellipse_outline(c, b.width() / 2.0, b.height() / 2.0);
            fill(&mut prims, &outline, &mut rng);
            let lines = rough::ellipse(c, b.width(), b.height(), &o, &mut rng);
            push_strokes(&mut prims, lines, sw, dash);
        }
        "line" | "arrow" => {
            let pts = abs_points(el);
            if pts.len() >= 2 {
                let closed = pts.len() > 2 && (pts[0] - pts[pts.len() - 1]).len() < 1.0;
                if closed && el.kind == "line" {
                    fill(&mut prims, &pts, &mut rng);
                }
                let curved = el.is_round() && pts.len() > 2;
                let elbow = el.is_elbow();
                let lines = if curved {
                    rough::curve(&pts, &o, &mut rng)
                } else if elbow {
                    rough::linear_path(&crate::elbow::rounded_path(&pts), false, &o, &mut rng)
                } else {
                    rough::linear_path(&pts, false, &o, &mut rng)
                };
                // direção das pontas: a partir do caminho ideal (suavizado, se curvo)
                let ideal = if curved {
                    rough::smooth_through(&pts)
                } else {
                    pts.clone()
                };
                push_strokes(&mut prims, lines, sw, dash);
                if el.kind == "arrow" {
                    let ho = Opts::new(o.roughness, true);
                    for end in [false, true] {
                        if let Some(kind) = el.arrowhead(end) {
                            // ponto de referência ~20px antes da ponta (curvas têm
                            // segmentos finais muito curtos)
                            let path: Vec<P> = if end {
                                ideal.iter().rev().copied().collect()
                            } else {
                                ideal.clone()
                            };
                            let tip = path[0];
                            // em cotovelo, a ponta segue o último segmento
                            let prev = if elbow {
                                path[1]
                            } else {
                                path.iter()
                                    .skip(1)
                                    .find(|p| (**p - tip).len() >= 20.0)
                                    .copied()
                                    .unwrap_or(path[path.len() - 1])
                            };
                            let total: f32 = path.windows(2).map(|w| (w[1] - w[0]).len()).sum();
                            arrowhead(
                                &mut prims, &kind, tip, prev, total, sw, stroke, &ho, &mut rng,
                            );
                        }
                    }
                }
            }
        }
        "freedraw" => {
            let pts = abs_points(el);
            let mut smooth = pts.clone();
            if smooth.len() > 3 {
                smooth = chaikin(&smooth);
            }
            if let Some(color) = stroke {
                if smooth.len() == 1 {
                    smooth.push(smooth[0] + P::new(0.01, 0.0));
                }
                prims.push(Prim::Stroke {
                    pts: smooth,
                    color,
                    width: el.stroke_width as f32 * 1.75 + 0.5,
                    dash: None,
                });
            }
        }
        "text" => {
            if let Some(color) = stroke {
                prims.push(Prim::Text {
                    origin: b.min,
                    width: b.width(),
                    text: el.text().to_string(),
                    size: el.font_size() as f32,
                    family: el.font_family(),
                    color,
                    align: match el.text_align() {
                        "center" => 1,
                        "right" => 2,
                        _ => 0,
                    },
                    line_height: el.line_height() as f32,
                    angle: el.angle as f32,
                    center: b.center(),
                });
            }
        }
        "image" => prims.push(Prim::Image {
            file_id: el.str_field("fileId").unwrap_or_default().to_string(),
            min: b.min,
            max: b.max,
            uv: image_uv(el),
            angle: el.angle as f32,
            center: b.center(),
            opacity,
        }),
        "frame" | "magicframe" => {
            // só a borda; o nome é desenhado à parte (ver `frame`)
            if let Some(color) = stroke {
                let r = crate::frame::RADIUS
                    .min(b.width() / 2.0)
                    .min(b.height() / 2.0);
                let mut pts = rounded_rect_outline(&b, r);
                pts.push(pts[0]);
                prims.push(Prim::Stroke {
                    pts,
                    color,
                    width: 1.0,
                    dash: None,
                });
            }
        }
        _ => {
            // Tipos ainda não suportados (embed...): moldura tracejada.
            let pts = [
                b.min,
                P::new(b.max.x, b.min.y),
                b.max,
                P::new(b.min.x, b.max.y),
                b.min,
            ];
            prims.push(Prim::Stroke {
                pts: pts.to_vec(),
                color: [134, 142, 150, (255.0 * opacity) as u8],
                width: 1.5,
                dash: Some((6.0, 4.0)),
            });
            if let Some(name) = el.str_field("name").or(Some(el.kind.as_str())) {
                prims.push(Prim::Text {
                    origin: b.min + P::new(6.0, 4.0),
                    width: b.width(),
                    text: name.to_string(),
                    size: 14.0,
                    family: 2,
                    color: [134, 142, 150, 255],
                    align: 0,
                    line_height: 1.25,
                    angle: 0.0,
                    center: b.center(),
                });
            }
        }
    }

    // aplica a rotação do elemento
    let a = el.angle as f32;
    if a != 0.0 {
        let c = b.center();
        for p in prims.iter_mut() {
            match p {
                Prim::Stroke { pts, .. } | Prim::Fill { pts, .. } => {
                    for q in pts.iter_mut() {
                        *q = rotate(*q, c, a);
                    }
                }
                Prim::Text { .. } | Prim::Image { .. } => {}
            }
        }
    }
    prims
}

#[allow(clippy::too_many_arguments)]
fn arrowhead(
    prims: &mut Vec<Prim>,
    kind: &str,
    tip: P,
    prev: P,
    total_len: f32,
    sw: f32,
    stroke: Option<Rgba>,
    o: &Opts,
    rng: &mut Rng,
) {
    let Some(color) = stroke else { return };
    let seg = total_len;
    if seg < 0.5 || (tip - prev).len() < 0.01 {
        return;
    }
    let dir = (tip - prev).normalized();
    let (size, angle_deg, mult) = match kind {
        "arrow" => (25.0, 20.0, 0.5),
        "bar" => (15.0, 90.0, 0.5),
        "diamond" | "diamond_outline" => (12.0, 25.0, 0.25),
        _ => (15.0, 25.0, 0.5),
    };
    let size = f32::min(size, seg * mult);
    let back = tip - dir * size;
    let a = (angle_deg as f32).to_radians();
    let w1 = rotate(back, tip, a);
    let w2 = rotate(back, tip, -a);
    match kind {
        "triangle" | "triangle_outline" => {
            let tri = vec![tip, w1, w2];
            if kind == "triangle" {
                prims.push(Prim::Fill {
                    pts: tri.clone(),
                    color,
                });
            }
            for pts in rough::linear_path(&tri, true, o, rng) {
                prims.push(Prim::Stroke {
                    pts,
                    color,
                    width: sw,
                    dash: None,
                });
            }
        }
        "dot" | "circle" | "circle_outline" => {
            let r = ((back - tip).len() + sw - 2.0).max(2.0) / 2.0;
            let c = tip - dir * r;
            let circle = ellipse_outline(c, r, r);
            if kind != "circle_outline" {
                prims.push(Prim::Fill {
                    pts: circle.clone(),
                    color,
                });
            }
            let mut closed = circle;
            closed.push(closed[0]);
            prims.push(Prim::Stroke {
                pts: closed,
                color,
                width: sw,
                dash: None,
            });
        }
        "diamond" | "diamond_outline" => {
            let far = tip - dir * (size * 2.0);
            let mid = tip - dir * size;
            let side = P::new(-dir.y, dir.x) * (size * 0.6);
            let poly = vec![tip, mid + side, far, mid - side];
            if kind == "diamond" {
                prims.push(Prim::Fill {
                    pts: poly.clone(),
                    color,
                });
            }
            for pts in rough::linear_path(&poly, true, o, rng) {
                prims.push(Prim::Stroke {
                    pts,
                    color,
                    width: sw,
                    dash: None,
                });
            }
        }
        "bar" => {
            for pts in rough::line(w1, w2, o, rng) {
                prims.push(Prim::Stroke {
                    pts,
                    color,
                    width: sw,
                    dash: None,
                });
            }
        }
        _ => {
            for pts in rough::linear_path(&[w1, tip, w2], false, o, rng) {
                prims.push(Prim::Stroke {
                    pts,
                    color,
                    width: sw,
                    dash: None,
                });
            }
        }
    }
}

/// Suavização de Chaikin (para traço livre).
fn chaikin(pts: &[P]) -> Vec<P> {
    let mut out = vec![pts[0]];
    for w in pts.windows(2) {
        out.push(w[0] * 0.75 + w[1] * 0.25);
        out.push(w[0] * 0.25 + w[1] * 0.75);
    }
    out.push(pts[pts.len() - 1]);
    out
}

// ---------------------------------------------------------------------------
// Hit-test
// ---------------------------------------------------------------------------

pub fn dist_to_segment(p: P, a: P, b: P) -> f32 {
    let ab = b - a;
    let l2 = ab.len_sq();
    if l2 < 1e-9 {
        return (p - a).len();
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    (p - (a + ab * t)).len()
}

pub fn dist_to_polyline(p: P, pts: &[P], closed: bool) -> f32 {
    let mut d = f32::MAX;
    for w in pts.windows(2) {
        d = d.min(dist_to_segment(p, w[0], w[1]));
    }
    if closed && pts.len() > 2 {
        d = d.min(dist_to_segment(p, pts[pts.len() - 1], pts[0]));
    }
    if pts.len() == 1 {
        d = (p - pts[0]).len();
    }
    d
}

pub fn point_in_polygon(p: P, poly: &[P]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// O ponto `p` (mundo) acerta o elemento? `tol` em unidades de mundo.
pub fn hit_test(el: &Element, p: P, tol: f32) -> bool {
    let c = center(el);
    let q = rotate(p, c, -(el.angle as f32));
    let b = local_box(el);
    let tol = tol + el.stroke_width as f32 / 2.0;
    let filled = parse_color(&el.background_color).is_some();
    match el.kind.as_str() {
        "rectangle" => {
            let outline = if el.is_round() {
                let r = corner_radius(el).min(b.width() / 2.0).min(b.height() / 2.0);
                rounded_rect_outline(&b, r)
            } else {
                vec![
                    b.min,
                    P::new(b.max.x, b.min.y),
                    b.max,
                    P::new(b.min.x, b.max.y),
                ]
            };
            dist_to_polyline(q, &outline, true) <= tol || (filled && point_in_polygon(q, &outline))
        }
        "diamond" => {
            let pts = diamond_points(&b);
            dist_to_polyline(q, &pts, true) <= tol || (filled && point_in_polygon(q, &pts))
        }
        "ellipse" => {
            let outline = ellipse_outline(b.center(), b.width() / 2.0, b.height() / 2.0);
            dist_to_polyline(q, &outline, true) <= tol || (filled && point_in_polygon(q, &outline))
        }
        "line" | "arrow" | "freedraw" => {
            let pts = abs_points(el);
            let pts = if el.is_linear() && el.is_round() && pts.len() > 2 {
                rough::smooth_through(&pts)
            } else {
                pts
            };
            let closed = pts.len() > 2 && (pts[0] - pts[pts.len() - 1]).len() < 1.0;
            let extra = if el.kind == "freedraw" {
                el.stroke_width as f32
            } else {
                0.0
            };
            dist_to_polyline(q, &pts, false) <= tol + extra
                || (closed && filled && point_in_polygon(q, &pts))
        }
        // frames: só a borda (o nome é tratado no app); dentro ficam os filhos
        "frame" | "magicframe" => {
            let pts = [
                b.min,
                P::new(b.max.x, b.min.y),
                b.max,
                P::new(b.min.x, b.max.y),
            ];
            dist_to_polyline(q, &pts, true) <= tol
        }
        _ => b.expand(tol * 0.5).contains(q),
    }
}

/// Aproximação do contorno de um elemento para seleção por laço/caixa e borracha.
pub fn segment_hits(el: &Element, a: P, b: P, tol: f32) -> bool {
    // amostra o segmento do cursor (borracha arrastada rapidamente)
    let n = (((b - a).len() / (tol.max(1.0))) as usize).clamp(1, 64);
    (0..=n).any(|i| hit_test(el, a + (b - a) * (i as f32 / n as f32), tol))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Element, Style};

    #[test]
    fn hit_rect_outline_only_when_transparent() {
        let mut e = Element::new("rectangle", 0.0, 0.0, &Style::default());
        e.width = 100.0;
        e.height = 50.0;
        e.roundness = None;
        assert!(hit_test(&e, P::new(0.0, 25.0), 2.0));
        assert!(!hit_test(&e, P::new(50.0, 25.0), 2.0));
        e.background_color = "#ffc9c9".into();
        assert!(hit_test(&e, P::new(50.0, 25.0), 2.0));
    }

    #[test]
    fn frame_hits_only_on_its_border() {
        let mut f = Element::new("frame", 0.0, 0.0, &Style::default());
        f.width = 100.0;
        f.height = 100.0;
        assert!(hit_test(&f, P::new(0.0, 50.0), 2.0));
        assert!(!hit_test(&f, P::new(50.0, 50.0), 2.0));
    }

    #[test]
    fn rotated_bounds() {
        let mut e = Element::new("rectangle", 0.0, 0.0, &Style::default());
        e.width = 100.0;
        e.height = 0.0;
        e.angle = std::f64::consts::FRAC_PI_2;
        let b = aabb(&e);
        assert!((b.width()).abs() < 0.01);
        assert!((b.height() - 100.0).abs() < 0.01);
    }

    #[test]
    fn prims_for_all_kinds() {
        let s = Style::default();
        for k in [
            "rectangle",
            "diamond",
            "ellipse",
            "arrow",
            "line",
            "freedraw",
            "text",
            "image",
            "frame",
        ] {
            let mut e = Element::new(k, 10.0, 10.0, &s);
            e.width = 80.0;
            e.height = 40.0;
            if e.has_points() {
                e.set_points(vec![[0.0, 0.0], [40.0, 20.0], [80.0, 0.0]]);
            }
            if k == "text" {
                e.set_text("olá");
            }
            assert!(!element_prims(&e).is_empty(), "{k}");
        }
    }
}
