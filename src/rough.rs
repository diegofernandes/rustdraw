//! Geração de traços "desenhados à mão", seguindo os algoritmos do rough.js
//! (a biblioteca usada pelo Excalidraw). Tudo é determinístico a partir da
//! `seed` do elemento, então o desenho não "treme" entre quadros.
//!
//! A saída são polilinhas em coordenadas de mundo, que depois são desenhadas
//! pelo egui (tela), pelo tiny-skia (PNG) ou viram `<path>` (SVG).

use crate::geom::P;
use std::f32::consts::PI;

/// Gerador pseudoaleatório com semente (LCG de Park–Miller, como o rough.js).
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: i64) -> Self {
        let s = (seed.unsigned_abs() % 2_147_483_646) as u32 + 1;
        Rng(s)
    }
    /// Valor em [0, 1).
    pub fn next(&mut self) -> f32 {
        self.0 = ((self.0 as u64 * 48271) % 2_147_483_647) as u32;
        self.0 as f32 / 2_147_483_647.0
    }
}

#[derive(Clone, Copy)]
pub struct Opts {
    pub roughness: f32,
    pub bowing: f32,
    pub max_offset: f32,
    /// Desenha cada traço duas vezes (o "duplo traço" característico).
    pub multi: bool,
}

impl Opts {
    pub fn new(roughness: f32, multi: bool) -> Self {
        Self {
            roughness,
            bowing: 1.0,
            max_offset: 2.0,
            multi: multi && roughness > 0.0,
        }
    }
}

fn offset(min: f32, max: f32, o: &Opts, rng: &mut Rng, gain: f32) -> f32 {
    o.roughness * gain * (rng.next() * (max - min) + min)
}

fn offset_opt(x: f32, o: &Opts, rng: &mut Rng, gain: f32) -> f32 {
    offset(-x, x, o, rng, gain)
}

fn bezier(p0: P, p1: P, p2: P, p3: P, out: &mut Vec<P>, steps: usize) {
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let u = 1.0 - t;
        let a = u * u * u;
        let b = 3.0 * u * u * t;
        let c = 3.0 * u * t * t;
        let d = t * t * t;
        out.push(P::new(
            a * p0.x + b * p1.x + c * p2.x + d * p3.x,
            a * p0.y + b * p1.y + c * p2.y + d * p3.y,
        ));
    }
}

/// Um único traço irregular de `a` até `b` (equivale ao `_line` do rough.js).
fn rough_line(a: P, b: P, o: &Opts, rng: &mut Rng, overlay: bool) -> Vec<P> {
    let len_sq = (a - b).len_sq();
    let len = len_sq.sqrt();
    if o.roughness <= 0.0 {
        return vec![a, b];
    }
    let gain = if len < 200.0 {
        1.0
    } else if len > 500.0 {
        0.4
    } else {
        -0.0016668 * len + 1.233334
    };
    let mut off = o.max_offset;
    if off * off * 100.0 > len_sq {
        off = len / 10.0;
    }
    let half = off / 2.0;
    let diverge = 0.2 + rng.next() * 0.2;
    let mut mid_x = o.bowing * o.max_offset * (b.y - a.y) / 200.0;
    let mut mid_y = o.bowing * o.max_offset * (a.x - b.x) / 200.0;
    mid_x = offset_opt(mid_x, o, rng, gain);
    mid_y = offset_opt(mid_y, o, rng, gain);
    let r = |rng: &mut Rng| {
        if overlay {
            offset_opt(half, o, rng, gain)
        } else {
            offset_opt(off, o, rng, gain)
        }
    };
    let start = P::new(a.x + r(rng), a.y + r(rng));
    let c1 = P::new(
        mid_x + a.x + (b.x - a.x) * diverge + r(rng),
        mid_y + a.y + (b.y - a.y) * diverge + r(rng),
    );
    let c2 = P::new(
        mid_x + a.x + 2.0 * (b.x - a.x) * diverge + r(rng),
        mid_y + a.y + 2.0 * (b.y - a.y) * diverge + r(rng),
    );
    let end = P::new(b.x + r(rng), b.y + r(rng));
    let steps = ((len / 12.0) as usize).clamp(4, 24);
    let mut out = vec![start];
    bezier(start, c1, c2, end, &mut out, steps);
    out
}

/// Linha irregular (uma ou duas passadas).
pub fn line(a: P, b: P, o: &Opts, rng: &mut Rng) -> Vec<Vec<P>> {
    let mut v = vec![rough_line(a, b, o, rng, false)];
    if o.multi {
        v.push(rough_line(a, b, o, rng, true));
    }
    v
}

/// Polilinha (aberta ou fechada) com cada segmento irregular.
pub fn linear_path(pts: &[P], closed: bool, o: &Opts, rng: &mut Rng) -> Vec<Vec<P>> {
    if pts.len() < 2 {
        return vec![];
    }
    let mut segs: Vec<(P, P)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
    if closed {
        segs.push((pts[pts.len() - 1], pts[0]));
    }
    let passes = if o.multi { 2 } else { 1 };
    let mut out = Vec::new();
    for pass in 0..passes {
        let mut poly: Vec<P> = Vec::new();
        for (a, b) in &segs {
            let l = rough_line(*a, *b, o, rng, pass == 1);
            if poly.is_empty() {
                poly.extend(l);
            } else {
                poly.extend(l.into_iter().skip(1));
            }
        }
        out.push(poly);
    }
    out
}

/// Curva Catmull-Rom passando pelos pontos (como `_curve` do rough.js).
fn catmull(points: &[P]) -> Vec<P> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[1]];
    for i in 1..n - 2 {
        let b0 = points[i];
        let b1 = points[i] + (points[i + 1] - points[i - 1]) * (1.0 / 6.0);
        let b2 = points[i + 1] - (points[i + 2] - points[i]) * (1.0 / 6.0);
        let b3 = points[i + 1];
        let steps = (((b3 - b0).len() / 6.0) as usize).clamp(3, 16);
        bezier(b0, b1, b2, b3, &mut out, steps);
    }
    out
}

/// Curva suave passando por todos os pontos (linhas/setas curvas).
pub fn smooth_through(pts: &[P]) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut ext = Vec::with_capacity(pts.len() + 2);
    ext.push(pts[0]);
    ext.extend_from_slice(pts);
    ext.push(pts[pts.len() - 1]);
    let mut out = vec![pts[0]];
    out.extend(catmull(&ext).into_iter().skip(1));
    out
}

/// Curva irregular passando pelos pontos, com duas passadas.
pub fn curve(pts: &[P], o: &Opts, rng: &mut Rng) -> Vec<Vec<P>> {
    let passes = if o.multi { 2 } else { 1 };
    let mut out = Vec::new();
    for pass in 0..passes {
        let amt = if pass == 0 { 1.0 } else { 1.5 };
        let jittered: Vec<P> = pts
            .iter()
            .map(|p| {
                P::new(
                    p.x + offset_opt(amt, o, rng, 1.0),
                    p.y + offset_opt(amt, o, rng, 1.0),
                )
            })
            .collect();
        out.push(smooth_through(&jittered));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn ellipse_points(
    inc: f32,
    c: P,
    rx: f32,
    ry: f32,
    off: f32,
    overlap: f32,
    o: &Opts,
    rng: &mut Rng,
) -> Vec<P> {
    let mut pts = Vec::new();
    if o.roughness <= 0.0 {
        let n = (2.0 * PI / inc).ceil() as usize;
        for i in 0..n {
            let a = i as f32 * inc;
            pts.push(P::new(c.x + rx * a.cos(), c.y + ry * a.sin()));
        }
        // fecha com sobreposição para a Catmull-Rom
        let (p0, p1, last) = (pts[0], pts[1], pts[n - 1]);
        pts.insert(0, last);
        pts.push(p0);
        pts.push(p1);
        return pts;
    }
    let rad_off = offset_opt(0.5, o, rng, 1.0) - PI / 2.0;
    let jit = |rng: &mut Rng| offset_opt(off, o, rng, 1.0);
    pts.push(P::new(
        jit(rng) + c.x + 0.9 * rx * (rad_off - inc).cos(),
        jit(rng) + c.y + 0.9 * ry * (rad_off - inc).sin(),
    ));
    let mut angle = rad_off;
    while angle < 2.0 * PI + rad_off - 0.01 {
        pts.push(P::new(
            jit(rng) + c.x + rx * angle.cos(),
            jit(rng) + c.y + ry * angle.sin(),
        ));
        angle += inc;
    }
    pts.push(P::new(
        jit(rng) + c.x + rx * (rad_off + 2.0 * PI + overlap * 0.5).cos(),
        jit(rng) + c.y + ry * (rad_off + 2.0 * PI + overlap * 0.5).sin(),
    ));
    pts.push(P::new(
        jit(rng) + c.x + 0.98 * rx * (rad_off + overlap).cos(),
        jit(rng) + c.y + 0.98 * ry * (rad_off + overlap).sin(),
    ));
    pts.push(P::new(
        jit(rng) + c.x + 0.9 * rx * (rad_off + overlap * 0.5).cos(),
        jit(rng) + c.y + 0.9 * ry * (rad_off + overlap * 0.5).sin(),
    ));
    pts
}

/// Elipse irregular centrada em `c` com largura `w` e altura `h`.
pub fn ellipse(c: P, w: f32, h: f32, o: &Opts, rng: &mut Rng) -> Vec<Vec<P>> {
    let mut rx = (w / 2.0).abs();
    let mut ry = (h / 2.0).abs();
    let psq = (2.0 * PI * ((rx * rx + ry * ry) / 2.0).sqrt()).sqrt();
    let steps = (9.0f32).max((9.0 / 200f32.sqrt()) * psq).ceil();
    let inc = 2.0 * PI / steps;
    rx += offset_opt(rx * 0.05, o, rng, 1.0);
    ry += offset_opt(ry * 0.05, o, rng, 1.0);
    let overlap = inc * offset(0.1, offset(0.4, 1.0, o, rng, 1.0), o, rng, 1.0);
    let mut out = vec![catmull(&ellipse_points(
        inc, c, rx, ry, 1.0, overlap, o, rng,
    ))];
    if o.multi {
        out.push(catmull(&ellipse_points(inc, c, rx, ry, 1.5, 0.0, o, rng)));
    }
    out
}

/// Linhas de hachura dentro de um polígono (pares de pontos), no ângulo dado (graus).
pub fn hachure(polygon: &[P], gap: f32, angle_deg: f32) -> Vec<(P, P)> {
    if polygon.len() < 3 || gap <= 0.1 {
        return vec![];
    }
    let a = angle_deg.to_radians();
    let (s, c) = a.sin_cos();
    // gira o polígono por -a para que as hachuras fiquem horizontais
    let rot: Vec<P> = polygon
        .iter()
        .map(|p| P::new(p.x * c + p.y * s, -p.x * s + p.y * c))
        .collect();
    let (mut miny, mut maxy) = (f32::MAX, f32::MIN);
    for p in &rot {
        miny = miny.min(p.y);
        maxy = maxy.max(p.y);
    }
    let mut lines = Vec::new();
    let mut y = miny + gap / 2.0;
    let n = rot.len();
    while y < maxy {
        let mut xs: Vec<f32> = Vec::new();
        for i in 0..n {
            let p1 = rot[i];
            let p2 = rot[(i + 1) % n];
            if (p1.y <= y && p2.y > y) || (p2.y <= y && p1.y > y) {
                let t = (y - p1.y) / (p2.y - p1.y);
                xs.push(p1.x + t * (p2.x - p1.x));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks(2) {
            if pair.len() == 2 {
                let un = |x: f32| P::new(x * c - y * s, x * s + y * c);
                lines.push((un(pair[0]), un(pair[1])));
            }
        }
        y += gap;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let o = Opts::new(1.0, true);
        let a = line(P::new(0.0, 0.0), P::new(100.0, 0.0), &o, &mut Rng::new(7));
        let b = line(P::new(0.0, 0.0), P::new(100.0, 0.0), &o, &mut Rng::new(7));
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn hachure_square() {
        let sq = [
            P::new(0.0, 0.0),
            P::new(100.0, 0.0),
            P::new(100.0, 100.0),
            P::new(0.0, 100.0),
        ];
        let l = hachure(&sq, 10.0, -41.0);
        assert!(l.len() > 10);
        for (a, b) in l {
            for p in [a, b] {
                assert!(p.x > -0.5 && p.x < 100.5 && p.y > -0.5 && p.y < 100.5);
            }
        }
    }
}
