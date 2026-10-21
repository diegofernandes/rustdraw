//! Desenho das primitivas na tela com o `egui::Painter`.

use crate::fonts::Family;
use crate::geom::{P, Prim, Rgba, rotate};
use crate::model::dark_filter;
use egui::epaint::TextShape;
use egui::{Color32, FontFamily, FontId, Painter, Pos2, Shape, Stroke, TextureHandle};
use std::collections::HashMap;

/// Câmera: `tela = origem + (mundo + scroll) * zoom`.
#[derive(Clone, Copy, Debug)]
pub struct Cam {
    pub scroll: P,
    pub zoom: f32,
    pub origin: Pos2,
}

impl Cam {
    pub fn s(&self, p: P) -> Pos2 {
        Pos2::new(
            self.origin.x + (p.x + self.scroll.x) * self.zoom,
            self.origin.y + (p.y + self.scroll.y) * self.zoom,
        )
    }
    pub fn w(&self, s: Pos2) -> P {
        P::new(
            (s.x - self.origin.x) / self.zoom - self.scroll.x,
            (s.y - self.origin.y) / self.zoom - self.scroll.y,
        )
    }
}

pub fn c32(c: Rgba, dark: bool, alpha_mul: f32) -> Color32 {
    let c = if dark { dark_filter(c) } else { c };
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (c[3] as f32 * alpha_mul).round() as u8)
}

pub fn font_family(fam: i64, hand_ok: bool) -> FontFamily {
    match Family::from_excalidraw(fam) {
        Family::Hand if hand_ok => FontFamily::Name("virgil".into()),
        Family::Mono => FontFamily::Monospace,
        _ => FontFamily::Proportional,
    }
}

fn dash_split(pts: &[Pos2], on: f32, off: f32) -> Vec<Vec<Pos2>> {
    let mut out = Vec::new();
    if pts.len() < 2 || on <= 0.0 {
        return out;
    }
    let mut cur = vec![pts[0]];
    let mut drawing = true;
    let mut remain = on;
    for w in pts.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = (b - a).length();
        while seg > remain {
            let m = a + (b - a) * (remain / seg);
            if drawing {
                cur.push(m);
                out.push(std::mem::take(&mut cur));
            } else {
                cur = vec![m];
            }
            seg -= remain;
            drawing = !drawing;
            remain = if drawing { on } else { off.max(0.1) };
            a = m;
        }
        remain -= seg;
        if drawing {
            cur.push(b);
        }
    }
    if drawing && cur.len() > 1 {
        out.push(cur);
    }
    out
}

pub struct PaintCtx<'a> {
    pub painter: &'a Painter,
    pub cam: Cam,
    pub dark: bool,
    pub alpha: f32,
    pub hand_ok: bool,
    /// Texturas das imagens já carregadas, por `fileId`.
    pub images: &'a HashMap<String, TextureHandle>,
}

pub fn paint_prims(pc: &PaintCtx, prims: &[Prim]) {
    let z = pc.cam.zoom;
    for prim in prims {
        match prim {
            Prim::Fill { pts, color } => {
                if pts.len() < 3 {
                    continue;
                }
                let spts: Vec<Pos2> = pts.iter().map(|p| pc.cam.s(*p)).collect();
                pc.painter.add(Shape::convex_polygon(
                    spts,
                    c32(*color, pc.dark, pc.alpha),
                    Stroke::NONE,
                ));
            }
            Prim::Stroke {
                pts,
                color,
                width,
                dash,
            } => {
                if pts.is_empty() {
                    continue;
                }
                let spts: Vec<Pos2> = pts.iter().map(|p| pc.cam.s(*p)).collect();
                let col = c32(*color, pc.dark, pc.alpha);
                let w = (width * z).max(0.5);
                match dash {
                    Some((on, off)) if *on < 2.0 => {
                        // pontilhado: bolinhas
                        for d in dash_split(&spts, (on * z).max(0.5), off * z) {
                            pc.painter.circle_filled(d[0], w / 2.0, col);
                        }
                    }
                    Some((on, off)) => {
                        for d in dash_split(&spts, on * z, off * z) {
                            pc.painter.add(Shape::line(d, Stroke::new(w, col)));
                        }
                    }
                    None => {
                        if w > 2.5 {
                            // pontas arredondadas
                            pc.painter.circle_filled(spts[0], w / 2.0, col);
                            pc.painter.circle_filled(spts[spts.len() - 1], w / 2.0, col);
                        }
                        if spts.len() >= 2 {
                            pc.painter.add(Shape::line(spts, Stroke::new(w, col)));
                        }
                    }
                }
            }
            Prim::Image {
                file_id,
                min,
                max,
                uv,
                angle,
                center,
                opacity,
            } => {
                let Some(tex) = pc.images.get(file_id) else {
                    paint_prims(pc, &crate::geom::image_placeholder(prim));
                    continue;
                };
                let a = (opacity * pc.alpha * 255.0).round() as u8;
                let mut mesh = egui::Mesh::with_texture(tex.id());
                let corners = [*min, P::new(max.x, min.y), *max, P::new(min.x, max.y)];
                let uvs = [
                    (uv[0], uv[1]),
                    (uv[2], uv[1]),
                    (uv[2], uv[3]),
                    (uv[0], uv[3]),
                ];
                for (p, (u, v)) in corners.iter().zip(uvs) {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: pc.cam.s(rotate(*p, *center, *angle)),
                        uv: Pos2::new(u, v),
                        color: Color32::from_white_alpha(a),
                    });
                }
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                pc.painter.add(Shape::mesh(mesh));
            }
            Prim::Text {
                origin,
                width,
                text,
                size,
                family,
                color,
                align,
                line_height,
                angle,
                center,
            } => {
                let px = size * z;
                if px < 1.0 {
                    continue;
                }
                let font = FontId::new(px, font_family(*family, pc.hand_ok));
                let col = c32(*color, pc.dark, pc.alpha);
                let lh = size * line_height;
                for (i, line) in text.split('\n').enumerate() {
                    let galley = pc
                        .painter
                        .layout_no_wrap(line.to_string(), font.clone(), col);
                    let gw = galley.size().x / z;
                    let gh = galley.size().y / z;
                    let dx = match align {
                        1 => (width - gw) / 2.0,
                        2 => width - gw,
                        _ => 0.0,
                    };
                    let top_left =
                        P::new(origin.x + dx, origin.y + i as f32 * lh + (lh - gh) / 2.0);
                    let pos = pc.cam.s(rotate(top_left, *center, *angle));
                    pc.painter
                        .add(TextShape::new(pos, galley, col).with_angle(*angle));
                }
            }
        }
    }
}

/// Mede um texto no tamanho de mundo: (largura, altura) usando a regra de
/// altura de linha do Excalidraw.
/// Tamanho, família e se a fonte à mão carregou.
pub type TextFont = (f32, i64, bool);

/// Largura de uma linha de texto.
pub fn text_width(painter: &Painter, line: &str, (size, fam, hand_ok): TextFont) -> f64 {
    let font = FontId::new(size, font_family(fam, hand_ok));
    painter
        .layout_no_wrap(line.to_string(), font, Color32::BLACK)
        .size()
        .x as f64
}

/// `text` com quebras de linha onde o egui quebraria na largura `max`.
pub fn wrap_text(
    painter: &Painter,
    text: &str,
    (size, fam, hand_ok): TextFont,
    max: f32,
) -> String {
    let font = FontId::new(size, font_family(fam, hand_ok));
    let galley = painter.layout(text.to_string(), font, Color32::BLACK, max.max(1.0));
    let n = galley.rows.len();
    let mut out = String::with_capacity(text.len() + n);
    for (i, row) in galley.rows.iter().enumerate() {
        let line = row.text();
        // numa quebra automática, o espaço que sobra no fim da linha some
        let soft = !row.ends_with_newline && i + 1 < n;
        out.push_str(if soft { line.trim_end() } else { &line });
        if i + 1 < n {
            out.push('\n');
        }
    }
    out
}

pub fn measure_text(
    painter: &Painter,
    text: &str,
    size: f32,
    fam: i64,
    lh: f32,
    hand_ok: bool,
) -> (f32, f32) {
    let font = FontId::new(size, font_family(fam, hand_ok));
    let mut w: f32 = 0.0;
    let mut n = 0;
    for line in text.split('\n') {
        let g = painter.layout_no_wrap(line.to_string(), font.clone(), Color32::BLACK);
        w = w.max(g.size().x);
        n += 1;
    }
    (w.max(1.0), (n as f32 * size * lh).max(size * lh))
}
