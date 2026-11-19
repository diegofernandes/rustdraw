//! Mini mapa (Alt+M): a cena inteira em miniatura, com o retângulo da área
//! visível, para navegar em desenhos grandes.
//!
//! A miniatura é rasterizada com o tiny-skia numa thread à parte, com formas
//! simplificadas (contornos, linhas, barras no lugar dos textos e blocos no
//! lugar das imagens), e só de novo quando a cena muda: enquanto ela muda (um
//! arrasto, por exemplo), no máximo quatro vezes por segundo, e num desenho
//! grande só quando ela para de mudar. A cada quadro a
//! janela só desenha uma textura e um retângulo, então o custo do mapa na
//! thread da janela não cresce com o tamanho do desenho.

use crate::export::map_color;
use crate::geom::{self, P, Rect, Rgba, rotate};
use crate::model::{Element, parse_color};
use std::sync::mpsc::{Receiver, Sender, channel};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Tamanho da área do mapa na tela (pontos).
pub const SIZE: egui::Vec2 = egui::vec2(224.0, 144.0);
/// Intervalo mínimo entre duas rasterizações enquanto a cena muda (s).
const MIN_INTERVAL: f64 = 0.25;
/// A partir de quantos elementos o mapa espera a cena parar de mudar antes
/// de redesenhar: copiar 30 mil elementos para a outra thread leva uns 15 ms,
/// o que, a cada quarto de segundo durante um arrasto, apareceria como
/// tropeços na janela.
const BIG_SCENE: usize = 3000;
/// Quanto tempo a cena grande tem que ficar parada (s).
const SETTLE: f64 = 0.3;
/// Desligado, o mapa aparece quando o quadro se move (mão, rolagem, zoom) e
/// fica na tela por este tempo depois que ele para (s)...
const PEEK_HOLD: f64 = 1.2;
/// ... e então some aos poucos, neste tempo (s).
const PEEK_FADE: f64 = 0.4;
/// Maior lado da miniatura (px).
const MAX_PX: f32 = 1024.0;
/// Menor lado do retângulo da área visível no mapa (pontos), para ele não
/// sumir com muito zoom num desenho grande.
const MIN_VIEW_PX: f32 = 6.0;

/// Miniatura pronta, vinda da thread.
struct Rendered {
    key: u64,
    /// Parte da cena que a imagem cobre (`None`: cena vazia).
    bounds: Option<Rect>,
    image: Option<egui::ColorImage>,
}

pub struct Minimap {
    /// Mostrar o mini mapa (guardado no autosave).
    pub show: bool,
    /// Miniatura e a parte da cena que ela cobre.
    texture: Option<(egui::TextureHandle, Rect)>,
    /// Chave (cena, tema, escala da tela) da miniatura pronta ou sendo feita.
    key: Option<u64>,
    busy: bool,
    last_start: f64,
    /// Chave que a cena tem agora e desde quando, para esperar ela assentar.
    seen: Option<(u64, f64)>,
    /// Câmera (scroll, zoom) do último quadro e quando ela mudou por último.
    last_cam: Option<(P, f32)>,
    moved_at: f64,
    done: (Sender<Rendered>, Receiver<Rendered>),
    /// Área da cena mostrada no mapa, parada enquanto se arrasta nele (senão
    /// o mapa mudaria de escala debaixo do ponteiro).
    frozen: Option<Rect>,
    /// Ao arrastar: centro da tela menos o ponto agarrado (cena).
    grab: P,
}

impl Default for Minimap {
    fn default() -> Self {
        Self {
            show: false,
            texture: None,
            key: None,
            busy: false,
            last_start: f64::NEG_INFINITY,
            seen: None,
            last_cam: None,
            moved_at: f64::NEG_INFINITY,
            done: channel(),
            frozen: None,
            grab: P::default(),
        }
    }
}

impl Minimap {
    /// Recebe a miniatura que ficou pronta e, se a cena mudou, pede outra.
    /// `sig` é a assinatura da cena (a mesma do `App::sync_bindings`).
    pub fn update(&mut self, ctx: &egui::Context, els: &[Element], sig: u64, dark: bool, now: f64) {
        while let Ok(r) = self.done.1.try_recv() {
            self.busy = false;
            if Some(r.key) != self.key {
                continue;
            }
            self.texture = match (r.bounds, r.image) {
                (Some(b), Some(img)) => match self.texture.take() {
                    Some((mut tex, _)) => {
                        tex.set(img, egui::TextureOptions::LINEAR);
                        Some((tex, b))
                    }
                    None => {
                        let tex = ctx.load_texture("minimap", img, egui::TextureOptions::LINEAR);
                        Some((tex, b))
                    }
                },
                _ => None,
            };
        }
        let ppp = ctx.pixels_per_point();
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (sig, dark, ppp.to_bits()).hash(&mut h);
            h.finish()
        };
        if self.key == Some(key) || self.busy {
            return;
        }
        if self.seen.map(|(k, _)| k) != Some(key) {
            self.seen = Some((key, now));
        }
        let since = self.seen.map_or(now, |(_, t)| t);
        let wait = if self.key.is_none() {
            0.0
        } else if els.len() >= BIG_SCENE {
            SETTLE - (now - since)
        } else {
            MIN_INTERVAL - (now - self.last_start)
        };
        if wait > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
            return;
        }
        self.key = Some(key);
        self.busy = true;
        self.last_start = now;
        let els = els.to_vec();
        let tx = self.done.0.clone();
        let ctx = ctx.clone();
        let px = SIZE * ppp;
        std::thread::spawn(move || {
            let bounds = content_bounds(&els);
            let image = bounds.and_then(|b| {
                let pm = rasterize(&els, b, (px.x, px.y), ppp, dark)?;
                Some(color_image(&pm))
            });
            let _ = tx.send(Rendered { key, bounds, image });
            ctx.request_repaint();
        });
    }

    /// Parte da cena que o mapa mostra: o desenho e a área visível juntos.
    pub fn area(&self, view: Rect) -> Rect {
        if let Some(a) = self.frozen {
            return a;
        }
        let r = match &self.texture {
            Some((_, b)) => b.union(view),
            None => view,
        };
        let m = r.width().max(r.height()) * 0.04;
        r.expand(m)
    }

    pub fn texture(&self) -> Option<&(egui::TextureHandle, Rect)> {
        self.texture.as_ref()
    }

    /// Começo de um arrasto no mapa: para a escala e guarda onde o ponteiro
    /// pegou. Um clique fora do retângulo da área visível leva o centro da
    /// tela até o ponto; dentro, o retângulo é arrastado de onde foi pego.
    pub fn start_drag(&mut self, area: Rect, view: Rect, at: P) {
        self.frozen = Some(area);
        self.grab = if view.contains(at) {
            view.center() - at
        } else {
            P::default()
        };
    }

    /// Centro da tela pedido pelo ponteiro em `at` durante o arrasto.
    pub fn drag_center(&self, at: P) -> P {
        at + self.grab
    }

    pub fn end_drag(&mut self) {
        self.frozen = None;
        self.grab = P::default();
    }

    pub fn dragging(&self) -> bool {
        self.frozen.is_some()
    }

    /// Anota a câmera deste quadro; se o quadro andou ou mudou de zoom, o
    /// mapa desligado aparece por um tempo.
    pub fn note_camera(&mut self, scroll: P, zoom: f32, now: f64) {
        if self.last_cam.is_some_and(|c| c != (scroll, zoom)) {
            self.moved_at = now;
        }
        self.last_cam = Some((scroll, zoom));
    }

    /// Mantém o mapa que apareceu sozinho na tela (o ponteiro está nele).
    pub fn keep_peeking(&mut self, now: f64) {
        self.moved_at = now;
    }

    /// Opacidade do mapa: ligado, sempre 1; desligado, 1 enquanto o quadro se
    /// move e logo depois, e então cai até 0.
    pub fn opacity(&self, now: f64) -> f32 {
        if self.show || self.dragging() {
            return 1.0;
        }
        let idle = now - self.moved_at;
        (1.0 - (idle - PEEK_HOLD) / PEEK_FADE).clamp(0.0, 1.0) as f32
    }

    /// Quanto falta (s) para o mapa que apareceu sozinho começar a sumir.
    pub fn hold_left(&self, now: f64) -> f64 {
        PEEK_HOLD - (now - self.moved_at)
    }
}

/// Conversão entre a cena e o retângulo do mapa na tela, mantendo a
/// proporção (a área fica centrada no retângulo).
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    area: Rect,
    origin: egui::Pos2,
    k: f32,
}

impl Fit {
    pub fn new(area: Rect, rect: egui::Rect) -> Self {
        let k =
            (rect.width() / area.width().max(1e-3)).min(rect.height() / area.height().max(1e-3));
        let k = if k.is_finite() && k > 0.0 { k } else { 1.0 };
        let used = egui::vec2(area.width() * k, area.height() * k);
        Self {
            area,
            origin: rect.center() - used / 2.0,
            k,
        }
    }

    pub fn screen(&self, p: P) -> egui::Pos2 {
        self.origin
            + egui::vec2(
                (p.x - self.area.min.x) * self.k,
                (p.y - self.area.min.y) * self.k,
            )
    }

    pub fn world(&self, s: egui::Pos2) -> P {
        P::new(
            self.area.min.x + (s.x - self.origin.x) / self.k,
            self.area.min.y + (s.y - self.origin.y) / self.k,
        )
    }

    pub fn rect(&self, r: Rect) -> egui::Rect {
        egui::Rect::from_min_max(self.screen(r.min), self.screen(r.max))
    }

    /// Retângulo da área visível no mapa, com um tamanho mínimo.
    pub fn view_rect(&self, view: Rect) -> egui::Rect {
        let r = self.rect(view);
        let size = egui::vec2(r.width().max(MIN_VIEW_PX), r.height().max(MIN_VIEW_PX));
        egui::Rect::from_center_size(r.center(), size)
    }
}

/// Caixa do desenho, só com elementos de coordenadas finitas (um arquivo com
/// `1e400` não pode deixar o mapa inteiro em NaN).
pub fn content_bounds(els: &[Element]) -> Option<Rect> {
    let mut r: Option<Rect> = None;
    for e in els {
        if e.is_deleted {
            continue;
        }
        let b = geom::aabb(e);
        if ![b.min.x, b.min.y, b.max.x, b.max.y]
            .iter()
            .all(|v| v.is_finite())
        {
            continue;
        }
        r = Some(r.map_or(b, |r| r.union(b)));
    }
    let r = r?;
    // uma linha reta (largura zero) ainda precisa de um retângulo de verdade
    let m = (r.width().max(r.height()) * 0.01).max(1.0);
    Some(r.expand(m))
}

fn with_opacity(c: Option<Rgba>, el: &Element, mul: f32) -> Option<Rgba> {
    let c = c?;
    let o = (el.opacity as f32 / 100.0).clamp(0.0, 1.0) * mul;
    let a = (c[3] as f32 * o).round() as u8;
    (a > 0).then_some([c[0], c[1], c[2], a])
}

fn paint(c: Rgba, dark: bool) -> Paint<'static> {
    let c = map_color(c, dark);
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

/// Tira pontos a menos de `min` um do outro: um desenho livre com milhares
/// de pontos vira poucos pixels no mapa.
fn thin(pts: &[P], min: f32) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(pts.len().min(256));
    for (i, &p) in pts.iter().enumerate() {
        let last = i + 1 == pts.len();
        match out.last() {
            Some(&q) if !last && (p - q).len_sq() < min * min => {}
            _ => out.push(p),
        }
    }
    out
}

fn path(pts: &[P], close: bool) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let (first, rest) = pts.split_first()?;
    pb.move_to(first.x, first.y);
    for p in rest {
        pb.line_to(p.x, p.y);
    }
    if close {
        pb.close();
    }
    pb.finish()
}

/// Desenha a cena simplificada numa imagem de no máximo `max` px (largura,
/// altura), com a caixa `b` da cena ocupando a imagem toda. `ppp` é a escala
/// da tela (os traços ficam com 1 ponto).
pub fn rasterize(
    els: &[Element],
    b: Rect,
    max: (f32, f32),
    ppp: f32,
    dark: bool,
) -> Option<Pixmap> {
    let (mw, mh) = (max.0.clamp(1.0, MAX_PX), max.1.clamp(1.0, MAX_PX));
    let k = (mw / b.width().max(1e-3)).min(mh / b.height().max(1e-3));
    if !k.is_finite() || k <= 0.0 {
        return None;
    }
    let w = (b.width() * k).ceil().clamp(1.0, mw) as u32;
    let h = (b.height() * k).ceil().clamp(1.0, mh) as u32;
    let mut pm = Pixmap::new(w, h)?;
    let t = Transform::from_scale(k, k).pre_translate(-b.min.x, -b.min.y);
    // um pixel da imagem, na escala da cena
    let px = 1.0 / k;
    let line = Stroke {
        width: (ppp * 0.8).max(0.8) * px,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    };
    let frame_col: Rgba = [150, 138, 126, 255];
    let image_col: Rgba = [176, 164, 150, 170];
    for el in els {
        if el.is_deleted {
            continue;
        }
        let a = geom::aabb(el);
        if ![a.min.x, a.min.y, a.max.x, a.max.y]
            .iter()
            .all(|v| v.is_finite())
        {
            continue;
        }
        let stroke = with_opacity(parse_color(&el.stroke_color), el, 1.0);
        let bg = with_opacity(
            parse_color(&el.background_color),
            el,
            if el.fill_style == "solid" { 1.0 } else { 0.6 },
        );
        // menor que um pixel e meio: um ponto
        if a.width() * k < 1.5 && a.height() * k < 1.5 {
            if let Some(c) = stroke.or(bg)
                && let Some(r) = tiny_skia::Rect::from_xywh(
                    a.center().x - line.width / 2.0,
                    a.center().y - line.width / 2.0,
                    line.width,
                    line.width,
                )
            {
                pm.fill_rect(r, &paint(c, dark), t, None);
            }
            continue;
        }
        let c = geom::center(el);
        let angle = el.angle as f32;
        let turn = |pts: Vec<P>| -> Vec<P> {
            if angle == 0.0 {
                pts
            } else {
                pts.into_iter().map(|p| rotate(p, c, angle)).collect()
            }
        };
        let corners = || geom::corners(el).to_vec();
        match el.kind.as_str() {
            "text" => {
                // uma barra por linha, do comprimento da linha, como num
                // mapa de código
                let Some(col) = with_opacity(parse_color(&el.stroke_color), el, 0.55) else {
                    continue;
                };
                let lb = geom::local_box(el);
                let lines: Vec<usize> = el.text().lines().map(|l| l.chars().count()).collect();
                let n = lines.len().max(1);
                let longest = lines.iter().copied().max().unwrap_or(1).max(1) as f32;
                let lh = lb.height() / n as f32;
                let align = el.text_align();
                for (i, len) in lines.iter().enumerate() {
                    if *len == 0 {
                        continue;
                    }
                    let bw = lb.width() * (*len as f32 / longest);
                    let x0 = match align {
                        "center" => lb.center().x - bw / 2.0,
                        "right" => lb.max.x - bw,
                        _ => lb.min.x,
                    };
                    let y0 = lb.min.y + lh * (i as f32 + 0.2);
                    let y1 = y0 + (lh * 0.6).max(px);
                    let bar = vec![
                        P::new(x0, y0),
                        P::new(x0 + bw, y0),
                        P::new(x0 + bw, y1),
                        P::new(x0, y1),
                    ];
                    if let Some(p) = path(&turn(bar), true) {
                        pm.fill_path(&p, &paint(col, dark), FillRule::Winding, t, None);
                    }
                }
            }
            "image" => {
                let Some(p) = path(&corners(), true) else {
                    continue;
                };
                let o = (el.opacity as f32 / 100.0).clamp(0.0, 1.0);
                let col = [
                    image_col[0],
                    image_col[1],
                    image_col[2],
                    (image_col[3] as f32 * o) as u8,
                ];
                pm.fill_path(&p, &paint(col, dark), FillRule::Winding, t, None);
            }
            _ if crate::frame::is_frame(el) => {
                if let Some(p) = path(&corners(), true) {
                    pm.stroke_path(&p, &paint(frame_col, dark), &line, t, None);
                }
            }
            _ if el.has_points() => {
                let pts = thin(&turn(geom::abs_points(el)), px * 0.75);
                if pts.len() < 2 {
                    continue;
                }
                let closed = pts.len() > 2 && (pts[0] - pts[pts.len() - 1]).len() <= px;
                if closed
                    && let Some(col) = bg
                    && let Some(p) = path(&pts, true)
                {
                    pm.fill_path(&p, &paint(col, dark), FillRule::Winding, t, None);
                }
                if let Some(col) = stroke
                    && let Some(p) = path(&pts, false)
                {
                    pm.stroke_path(&p, &paint(col, dark), &line, t, None);
                }
            }
            _ => {
                let pts = geom::outline(el).map(turn).unwrap_or_else(corners);
                let Some(p) = path(&thin(&pts, px * 0.75), true) else {
                    continue;
                };
                if let Some(col) = bg {
                    pm.fill_path(&p, &paint(col, dark), FillRule::Winding, t, None);
                }
                if let Some(col) = stroke {
                    pm.stroke_path(&p, &paint(col, dark), &line, t, None);
                }
            }
        }
    }
    Some(pm)
}

fn color_image(pm: &Pixmap) -> egui::ColorImage {
    let size = [pm.width() as usize, pm.height() as usize];
    let px: Vec<egui::Color32> = pm
        .pixels()
        .iter()
        .map(|p| egui::Color32::from_rgba_premultiplied(p.red(), p.green(), p.blue(), p.alpha()))
        .collect();
    egui::ColorImage::new(size, px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn el(v: serde_json::Value) -> Element {
        serde_json::from_value(v).unwrap()
    }

    fn rect(id: &str, x: f64, y: f64, w: f64, h: f64) -> Element {
        el(
            json!({"id": id, "type": "rectangle", "x": x, "y": y, "width": w, "height": h,
                  "strokeColor": "#1e1e1e", "backgroundColor": "#a5d8ff", "fillStyle": "solid"}),
        )
    }

    fn alpha_at(pm: &Pixmap, x: u32, y: u32) -> u8 {
        pm.pixel(x, y).unwrap().alpha()
    }

    #[test]
    fn fit_maps_both_ways_and_keeps_the_ratio() {
        let area = Rect::from_two(P::new(-100.0, 0.0), P::new(300.0, 100.0));
        let screen = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 200.0));
        let f = Fit::new(area, screen);
        // 400×100 num quadrado de 200: escala 0,5, centrado na vertical
        let tl = f.screen(area.min);
        let br = f.screen(area.max);
        assert!((tl.x - 10.0).abs() < 1e-3 && (br.x - 210.0).abs() < 1e-3);
        assert!((tl.y - 95.0).abs() < 1e-3 && (br.y - 145.0).abs() < 1e-3);
        let p = P::new(37.0, 61.0);
        let back = f.world(f.screen(p));
        assert!((back.x - p.x).abs() < 1e-3 && (back.y - p.y).abs() < 1e-3);
    }

    #[test]
    fn a_tiny_view_still_shows_on_the_map() {
        let area = Rect::from_two(P::new(0.0, 0.0), P::new(1e6, 1e6));
        let f = Fit::new(area, egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE));
        let r = f.view_rect(Rect::from_two(P::new(10.0, 10.0), P::new(20.0, 20.0)));
        assert!(r.width() >= MIN_VIEW_PX && r.height() >= MIN_VIEW_PX);
    }

    #[test]
    fn the_map_takes_in_the_scene_and_the_view() {
        let mut m = Minimap::default();
        let view = Rect::from_two(P::new(1000.0, 1000.0), P::new(1200.0, 1100.0));
        assert!(m.area(view).contains_rect(&view));
        // arrastando, a área fica parada mesmo que a tela saia dela
        m.start_drag(view, view, view.center());
        let away = Rect::from_two(P::new(5000.0, 5000.0), P::new(5200.0, 5100.0));
        assert_eq!(m.area(away).min.x, view.min.x);
        m.end_drag();
        assert!(m.area(away).contains_rect(&away));
    }

    #[test]
    fn dragging_the_view_keeps_where_it_was_grabbed() {
        let mut m = Minimap::default();
        let view = Rect::from_two(P::new(0.0, 0.0), P::new(200.0, 100.0));
        let area = view.expand(1000.0);
        // pegou o retângulo perto do canto: o centro anda junto, sem pular
        m.start_drag(area, view, P::new(10.0, 10.0));
        let c = m.drag_center(P::new(60.0, 10.0));
        assert_eq!((c.x, c.y), (150.0, 50.0));
        // clicou fora dele: o centro vai até o ponto
        m.start_drag(area, view, P::new(800.0, 800.0));
        let c = m.drag_center(P::new(800.0, 800.0));
        assert_eq!((c.x, c.y), (800.0, 800.0));
    }

    #[test]
    fn rasterize_draws_shapes_where_they_are() {
        let els = vec![
            rect("a", 0.0, 0.0, 100.0, 100.0),
            rect("b", 900.0, 400.0, 100.0, 100.0),
        ];
        let b = content_bounds(&els).unwrap();
        let pm = rasterize(&els, b, (224.0, 144.0), 1.0, false).unwrap();
        assert!(pm.width() <= 224 && pm.height() <= 144);
        let k = pm.width() as f32 / b.width();
        let atx = |x: f32| ((x - b.min.x) * k) as u32;
        let aty = |y: f32| ((y - b.min.y) * k) as u32;
        assert!(alpha_at(&pm, atx(50.0), aty(50.0)) > 200);
        assert!(alpha_at(&pm, atx(950.0), aty(450.0)) > 200);
        // entre os dois, nada
        assert_eq!(alpha_at(&pm, atx(500.0), aty(250.0)), 0);
    }

    #[test]
    fn hostile_coordinates_do_not_break_the_map() {
        let mut far = rect("far", 1e30, 1e30, 10.0, 10.0);
        far.width = f64::INFINITY;
        let els = vec![rect("a", 0.0, 0.0, 100.0, 100.0), far];
        let b = content_bounds(&els).unwrap();
        assert!(b.width().is_finite() && b.width() < 1e4);
        assert!(rasterize(&els, b, (224.0, 144.0), 2.0, true).is_some());
        // pontos demais, todos no mesmo lugar: viram um só
        let pts = vec![P::new(1.0, 1.0); 100_000];
        assert_eq!(thin(&pts, 0.5).len(), 2);
    }

    #[test]
    fn moving_the_view_shows_the_map_for_a_while() {
        let mut m = Minimap::default();
        let (z, p) = (1.0, P::new(0.0, 0.0));
        // o primeiro quadro só anota a câmera
        m.note_camera(p, z, 0.0);
        assert_eq!(m.opacity(0.0), 0.0);
        // mão ou rolagem: aparece
        m.note_camera(P::new(30.0, 0.0), z, 10.0);
        assert_eq!(m.opacity(10.0), 1.0);
        assert_eq!(m.opacity(10.0 + PEEK_HOLD), 1.0);
        let mid = m.opacity(10.0 + PEEK_HOLD + PEEK_FADE / 2.0);
        assert!(mid > 0.0 && mid < 1.0);
        assert_eq!(m.opacity(10.0 + PEEK_HOLD + PEEK_FADE + 0.01), 0.0);
        // parado não aparece de novo; zoom aparece
        m.note_camera(P::new(30.0, 0.0), z, 20.0);
        assert_eq!(m.opacity(20.0), 0.0);
        m.note_camera(P::new(30.0, 0.0), 1.1, 21.0);
        assert_eq!(m.opacity(21.0), 1.0);
        // ligado, fica sempre
        m.show = true;
        assert_eq!(m.opacity(100.0), 1.0);
    }

    #[test]
    fn empty_scene_has_no_bounds() {
        assert!(content_bounds(&[]).is_none());
    }
}
