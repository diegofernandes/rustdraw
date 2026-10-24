//! Exportação da cena para PNG (tiny-skia + fontdue) e SVG. O TikZ fica em
//! `tikz.rs`, a partir do mesmo plano.

use crate::fonts::{self, Family};
use crate::frame;
use crate::geom::{P, Prim, Rect, Rgba, element_prims, image_placeholder, local_box, scene_bounds};
use crate::model::{Element, dark_filter, parse_color};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt::Write as _;
use tiny_skia::{
    FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, StrokeDash,
    Transform,
};

#[derive(Clone, Debug)]
pub struct ExportOpts {
    pub scale: f32,
    pub background: bool,
    pub dark: bool,
    pub padding: f32,
    pub view_background: String,
    /// Exporta só o conteúdo deste frame (id), recortado pela área dele.
    pub frame: Option<String>,
    /// Mapa `files` da cena (as imagens).
    pub files: Map<String, Value>,
}

impl Default for ExportOpts {
    fn default() -> Self {
        Self {
            scale: 2.0,
            background: true,
            dark: false,
            padding: 10.0,
            view_background: "#ffffff".into(),
            frame: None,
            files: Map::new(),
        }
    }
}

/// Formato do arquivo exportado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Svg,
    Tikz,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Svg => "svg",
            Format::Tikz => "tex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Svg => "SVG",
            Format::Tikz => "TikZ",
        }
    }

    /// Extensões aceitas na caixa de salvar.
    pub fn exts(self) -> &'static [&'static str] {
        match self {
            Format::Png => &["png"],
            Format::Svg => &["svg"],
            Format::Tikz => &["tex", "tikz"],
        }
    }

    /// Pela extensão do caminho: .png, .svg, .tex ou .tikz.
    pub fn from_path(path: &str) -> Option<Self> {
        let ext = std::path::Path::new(path).extension()?.to_str()?;
        Some(match ext.to_ascii_lowercase().as_str() {
            "png" => Format::Png,
            "svg" => Format::Svg,
            "tex" | "tikz" => Format::Tikz,
            _ => return None,
        })
    }
}

/// Grava a exportação em `path`. O TikZ grava também as imagens, numa pasta
/// ao lado do `.tex`.
pub fn write_file(
    path: &std::path::Path,
    format: Format,
    els: &[Element],
    opts: &ExportOpts,
) -> Result<usize, String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let data = match format {
        Format::Png => render_png(els, opts)?,
        Format::Svg => render_svg(els, opts)?.into_bytes(),
        Format::Tikz => return crate::tikz::write_file(path, els, opts),
    };
    std::fs::write(path, &data).map_err(err)?;
    Ok(data.len())
}

pub(crate) fn map_color(c: Rgba, dark: bool) -> Rgba {
    if dark { dark_filter(c) } else { c }
}

fn element_bounds(e: &Element) -> Option<Rect> {
    let r = scene_bounds([e])?;
    Some(if e.kind == "text" {
        r.union(local_box(e))
    } else {
        r
    })
}

/// O que desenhar, em ordem: as primitivas de um elemento e o recorte do
/// frame dele (`clip` = id do frame).
pub(crate) struct Item<'a> {
    pub id: &'a str,
    pub prims: Vec<Prim>,
    pub clip: Option<&'a str>,
}

pub(crate) struct Plan<'a> {
    pub bounds: Rect,
    pub padding: f32,
    pub items: Vec<Item<'a>>,
    pub clips: HashMap<&'a str, Rect>,
}

/// Nome do frame, acima da borda de cima.
fn frame_name_prim(f: &Element) -> Prim {
    let r = frame::rect(f);
    let lh = 1.25;
    Prim::Text {
        origin: P::new(r.min.x, r.min.y - frame::NAME_GAP - frame::NAME_SIZE * lh),
        width: r.width(),
        text: frame::name(f),
        size: frame::NAME_SIZE,
        family: 2,
        color: frame::NAME_COLOR,
        align: 0,
        line_height: lh,
        angle: 0.0,
        center: r.center(),
    }
}

/// Monta a exportação: a cena inteira (frames com borda e nome, filhos
/// recortados) ou, com `opts.frame`, só o conteúdo de um frame, sem margem.
pub(crate) fn plan<'a>(els: &'a [Element], opts: &ExportOpts) -> Result<Plan<'a>, String> {
    let clips = frame::rects(els);
    if let Some(fid) = &opts.frame {
        let (&id, &r) = clips
            .get_key_value(fid.as_str())
            .ok_or("frame não encontrado")?;
        let items = els
            .iter()
            .filter(|e| e.frame_id.as_deref() == Some(id) && !frame::is_frame(e))
            .map(|e| Item {
                id: &e.id,
                prims: element_prims(e),
                clip: Some(id),
            })
            .collect();
        return Ok(Plan {
            bounds: r,
            padding: 0.0,
            items,
            clips,
        });
    }
    let rf = fonts::raster_fonts();
    let mut bounds: Option<Rect> = None;
    let mut items = Vec::new();
    for e in els {
        let mut prims = element_prims(e);
        let mut b = element_bounds(e);
        let clip = e
            .frame_id
            .as_deref()
            .and_then(|f| clips.get_key_value(f))
            .filter(|_| !frame::is_frame(e));
        if let Some((_, r)) = clip {
            b = b.and_then(|b| b.intersect(r));
        }
        if frame::is_frame(e) {
            let name = frame_name_prim(e);
            if let Prim::Text {
                origin, text, size, ..
            } = &name
            {
                let w = rf.line_width(Family::Sans, text, *size);
                let nb = Rect::from_two(*origin, *origin + P::new(w, size * 1.25));
                b = Some(b.map_or(nb, |b| b.union(nb)));
            }
            prims.push(name);
        }
        if let Some(b) = b {
            bounds = Some(bounds.map_or(b, |r| r.union(b)));
        }
        items.push(Item {
            id: &e.id,
            prims,
            clip: clip.map(|(k, _)| *k),
        });
    }
    Ok(Plan {
        bounds: bounds.ok_or("nada para exportar")?,
        padding: opts.padding,
        items,
        clips,
    })
}

pub fn bounds_with_text(els: &[Element]) -> Option<Rect> {
    export_bounds(els, &ExportOpts::default())
}

/// Área que a exportação ocupa (sem a margem), para a pré-visualização.
pub fn export_bounds(els: &[Element], opts: &ExportOpts) -> Option<Rect> {
    plan(els, opts).ok().map(|p| p.bounds)
}

// ---------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------

pub fn render_png(els: &[Element], opts: &ExportOpts) -> Result<Vec<u8>, String> {
    let pixmap = render_pixmap(els, opts)?;
    pixmap.encode_png().map_err(|e| e.to_string())
}

pub fn render_pixmap(els: &[Element], opts: &ExportOpts) -> Result<Pixmap, String> {
    let plan = plan(els, opts)?;
    let b = plan.bounds;
    let s = opts.scale.max(0.1);
    let pad = plan.padding;
    let w = ((b.width() + pad * 2.0) * s).ceil().max(1.0) as u32;
    let h = ((b.height() + pad * 2.0) * s).ceil().max(1.0) as u32;
    if w > 16384 || h > 16384 {
        return Err(format!("imagem grande demais ({w}×{h}); reduza a escala"));
    }
    let mut pm = Pixmap::new(w, h).ok_or("falha ao criar imagem")?;
    if opts.background {
        let bg = parse_color(&opts.view_background).unwrap_or([255, 255, 255, 255]);
        let bg = map_color(bg, opts.dark);
        pm.fill(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
    }
    let base = Transform::from_scale(s, s).pre_translate(pad - b.min.x, pad - b.min.y);
    let mut masks: HashMap<&str, Mask> = HashMap::new();
    let mut images = HashMap::new();
    for item in &plan.items {
        let mask = match item.clip {
            Some(f) => {
                if !masks.contains_key(f) {
                    let r = plan.clips[f];
                    let mut m = Mask::new(w, h).ok_or("falha ao criar imagem")?;
                    if let Some(rect) =
                        tiny_skia::Rect::from_ltrb(r.min.x, r.min.y, r.max.x, r.max.y)
                    {
                        m.fill_path(&PathBuilder::from_rect(rect), FillRule::Winding, true, base);
                    }
                    masks.insert(f, m);
                }
                masks.get(f)
            }
            None => None,
        };
        for prim in &item.prims {
            if let Prim::Image { file_id, .. } = prim {
                let img = images
                    .entry(file_id.clone())
                    .or_insert_with(|| image_pixmap(&opts.files, file_id));
                if let Some(img) = img {
                    draw_image(&mut pm, prim, img, base, mask);
                    continue;
                }
                for p in image_placeholder(prim) {
                    draw_prim(&mut pm, &p, base, s, opts.dark, mask);
                }
                continue;
            }
            draw_prim(&mut pm, prim, base, s, opts.dark, mask);
        }
    }
    Ok(pm)
}

/// Decodifica uma imagem de `files` para RGBA pré-multiplicado.
fn image_pixmap(files: &Map<String, Value>, id: &str) -> Option<Pixmap> {
    let img = crate::files::decode(files, id)?;
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height())?;
    let mut data = img.into_raw();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = (*c as u32 * a / 255) as u8;
        }
    }
    Pixmap::from_vec(data, size)
}

fn draw_image(pm: &mut Pixmap, prim: &Prim, img: &Pixmap, base: Transform, mask: Option<&Mask>) {
    let Prim::Image {
        min,
        max,
        uv,
        angle,
        center,
        opacity,
        ..
    } = prim
    else {
        return;
    };
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    // recorte em pixels da imagem; u0 > u1 quando está espelhada
    let (u0, v0, u1, v1) = (uv[0] * iw, uv[1] * ih, uv[2] * iw, uv[3] * ih);
    if (u1 - u0).abs() < 0.5 || (v1 - v0).abs() < 0.5 {
        return;
    }
    let (sx, sy) = ((max.x - min.x) / (u1 - u0), (max.y - min.y) / (v1 - v0));
    // o padrão é posicionado nas coordenadas do retângulo (antes de `rt`)
    let t = Transform::from_translate(min.x, min.y)
        .pre_scale(sx, sy)
        .pre_translate(-u0, -v0);
    let pattern = tiny_skia::Pattern::new(
        img.as_ref(),
        tiny_skia::SpreadMode::Pad,
        tiny_skia::FilterQuality::Bilinear,
        *opacity,
        t,
    );
    let paint = Paint {
        shader: pattern,
        anti_alias: true,
        ..Default::default()
    };
    let rect = tiny_skia::Rect::from_ltrb(min.x, min.y, max.x, max.y);
    let Some(rect) = rect else { return };
    let rt = base
        .pre_translate(center.x, center.y)
        .pre_rotate(angle.to_degrees())
        .pre_translate(-center.x, -center.y);
    pm.fill_rect(rect, &paint, rt, mask);
}

fn paint_for(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

/// Maior lado (px) do bloco onde um texto é rasterizado antes de ser composto.
const MAX_TEXT_BLOCK: f32 = 8192.0;
/// Maior tamanho (px) em que um caractere é rasterizado.
const MAX_GLYPH_PX: f32 = 2048.0;

fn draw_prim(
    pm: &mut Pixmap,
    prim: &Prim,
    base: Transform,
    s: f32,
    dark: bool,
    mask: Option<&Mask>,
) {
    match prim {
        Prim::Stroke {
            pts,
            color,
            width,
            dash,
        } => {
            if pts.len() < 2 {
                return;
            }
            let mut pb = PathBuilder::new();
            pb.move_to(pts[0].x, pts[0].y);
            for p in &pts[1..] {
                pb.line_to(p.x, p.y);
            }
            let Some(path) = pb.finish() else { return };
            let stroke = Stroke {
                width: *width,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                dash: dash.and_then(|(on, off)| StrokeDash::new(vec![on, off], 0.0)),
                ..Default::default()
            };
            pm.stroke_path(
                &path,
                &paint_for(map_color(*color, dark)),
                &stroke,
                base,
                mask,
            );
        }
        Prim::Fill { pts, color } => {
            if pts.len() < 3 {
                return;
            }
            let mut pb = PathBuilder::new();
            pb.move_to(pts[0].x, pts[0].y);
            for p in &pts[1..] {
                pb.line_to(p.x, p.y);
            }
            pb.close();
            let Some(path) = pb.finish() else { return };
            pm.fill_path(
                &path,
                &paint_for(map_color(*color, dark)),
                FillRule::Winding,
                base,
                mask,
            );
        }
        // desenhadas por `draw_image` em `render_pixmap`
        Prim::Image { .. } => {}
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
            let fam = Family::from_excalidraw(*family);
            let rf = fonts::raster_fonts();
            let lines: Vec<&str> = text.split('\n').collect();
            // o bloco é rasterizado na escala `s`, a não ser que fique grande
            // demais (um fontSize de 100000 pediria 150 GB): aí é rasterizado
            // menor e ampliado ao compor
            let block = |s: f32| {
                let widths: Vec<f32> = lines
                    .iter()
                    .map(|l| rf.line_width(fam, l, size * s))
                    .collect();
                let w = widths.iter().cloned().fold(width * s, f32::max).ceil() + 4.0;
                let h = (size * line_height * s * lines.len() as f32).ceil() + 4.0;
                (widths, w, h)
            };
            let (_, w, h) = block(s);
            let k = (MAX_TEXT_BLOCK / w)
                .min(MAX_TEXT_BLOCK / h)
                .min(MAX_GLYPH_PX / (size * s))
                .min(1.0);
            let s = s * k;
            let (widths, block_w, block_h) = block(s);
            let (px, lh) = (size * s, size * line_height * s);
            let Some(mut tmp) = Pixmap::new(block_w as u32, block_h as u32) else {
                return;
            };
            let col = map_color(*color, dark);
            let baseline_off = rf.baseline(fam, px, lh);
            for (i, line) in lines.iter().enumerate() {
                let mut pen = match align {
                    1 => (width * s - widths[i]) / 2.0,
                    2 => width * s - widths[i],
                    _ => 0.0,
                };
                let baseline = i as f32 * lh + baseline_off;
                for ch in line.chars() {
                    let font = rf.for_char(fam, ch);
                    let (m, bitmap) = font.rasterize(ch, px);
                    let gx = (pen + m.xmin as f32).round() as i32;
                    let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
                    blit_coverage(&mut tmp, gx, gy, m.width, m.height, &bitmap, col);
                    pen += m.advance_width;
                }
            }
            let t = base
                .pre_translate(center.x, center.y)
                .pre_rotate(angle.to_degrees())
                .pre_translate(-center.x, -center.y)
                .pre_translate(origin.x, origin.y)
                .pre_scale(1.0 / s, 1.0 / s);
            pm.draw_pixmap(0, 0, tmp.as_ref(), &PixmapPaint::default(), t, mask);
        }
    }
}

fn blit_coverage(pm: &mut Pixmap, x0: i32, y0: i32, w: usize, h: usize, cov: &[u8], c: Rgba) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let (px, py) = (x0 + x, y0 + y);
            if px < 0 || py < 0 || px >= pw || py >= ph {
                continue;
            }
            let a = cov[(y as usize) * w + x as usize] as u32 * c[3] as u32 / 255;
            if a == 0 {
                continue;
            }
            let i = ((py * pw + px) * 4) as usize;
            // "source over" em RGBA pré-multiplicado
            let inv = 255 - a;
            for k in 0..3 {
                let src = c[k] as u32 * a / 255;
                data[i + k] = (src + data[i + k] as u32 * inv / 255) as u8;
            }
            data[i + 3] = (a + data[i + 3] as u32 * inv / 255) as u8;
        }
    }
}

// ---------------------------------------------------------------------------
// SVG
// ---------------------------------------------------------------------------

fn hex(c: Rgba) -> (String, f32) {
    (
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]),
        c[3] as f32 / 255.0,
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn path_d(pts: &[crate::geom::P], close: bool) -> String {
    let mut d = String::new();
    for (i, p) in pts.iter().enumerate() {
        let _ = write!(
            d,
            "{}{:.2} {:.2} ",
            if i == 0 { "M" } else { "L" },
            p.x,
            p.y
        );
    }
    if close {
        d.push('Z');
    }
    d.trim_end().to_string()
}

pub fn render_svg(els: &[Element], opts: &ExportOpts) -> Result<String, String> {
    let plan = plan(els, opts)?;
    let b = plan.bounds;
    let pad = plan.padding;
    let w = b.width() + pad * 2.0;
    let h = b.height() + pad * 2.0;
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.2} {h:.2}">"#
    );
    let _ = writeln!(out, "  <!-- svg-source:rustdraw -->");
    let uses_hand = plan.items.iter().flat_map(|i| &i.prims).any(
        |p| matches!(p, Prim::Text { family, .. } if Family::from_excalidraw(*family) == Family::Hand),
    );
    if uses_hand {
        let _ = writeln!(
            out,
            "  <defs><style>@font-face {{ font-family: \"Virgil\"; src: url(data:font/woff2;base64,{}); }}</style></defs>",
            fonts::base64(fonts::virgil_woff2())
        );
    }
    if opts.background {
        let bg = parse_color(&opts.view_background).unwrap_or([255, 255, 255, 255]);
        let (c, a) = hex(map_color(bg, opts.dark));
        let _ = writeln!(
            out,
            r#"  <rect x="0" y="0" width="{w:.2}" height="{h:.2}" fill="{c}" fill-opacity="{a:.3}"/>"#
        );
    }
    let _ = writeln!(
        out,
        r#"  <g transform="translate({:.2} {:.2})">"#,
        pad - b.min.x,
        pad - b.min.y
    );
    let rf = fonts::raster_fonts();
    let mut clip_ids: HashMap<&str, usize> = HashMap::new();
    for item in &plan.items {
        let clip = match item.clip {
            Some(f) => {
                let n = clip_ids.len();
                let n = *clip_ids.entry(f).or_insert_with(|| {
                    let r = plan.clips[f];
                    let _ = writeln!(
                        out,
                        r#"    <clipPath id="frame-clip-{n}"><rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}"/></clipPath>"#,
                        r.min.x,
                        r.min.y,
                        r.width(),
                        r.height()
                    );
                    n
                });
                format!(r#" clip-path="url(#frame-clip-{n})""#)
            }
            None => String::new(),
        };
        let _ = writeln!(out, "    <g data-id=\"{}\"{clip}>", esc(item.id));
        let image_url = |id: &str| {
            opts.files
                .get(id)
                .and_then(|f| f.get("dataURL"))
                .and_then(|u| u.as_str())
                .filter(|u| u.starts_with("data:image/"))
        };
        let prims: Vec<Prim> = item
            .prims
            .iter()
            .cloned()
            .flat_map(|p| match &p {
                Prim::Image { file_id, .. } if image_url(file_id).is_none() => {
                    image_placeholder(&p)
                }
                _ => vec![p],
            })
            .collect();
        for prim in prims {
            match prim {
                Prim::Image {
                    file_id,
                    min,
                    max,
                    uv,
                    angle,
                    center,
                    opacity,
                } => {
                    let url = image_url(&file_id).unwrap_or_default();
                    let fx = if uv[0] > uv[2] { -1 } else { 1 };
                    let fy = if uv[1] > uv[3] { -1 } else { 1 };
                    let _ = writeln!(
                        out,
                        r#"      <g transform="rotate({:.3} {cx:.2} {cy:.2}) translate({cx:.2} {cy:.2}) scale({fx} {fy}) translate({:.2} {:.2})" opacity="{opacity:.3}">"#,
                        angle.to_degrees(),
                        -center.x,
                        -center.y,
                        cx = center.x,
                        cy = center.y,
                    );
                    let _ = writeln!(
                        out,
                        r#"        <svg x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" viewBox="{:.5} {:.5} {:.5} {:.5}" preserveAspectRatio="none"><image href="{}" x="0" y="0" width="1" height="1" preserveAspectRatio="none"/></svg>"#,
                        min.x,
                        min.y,
                        max.x - min.x,
                        max.y - min.y,
                        uv[0].min(uv[2]),
                        uv[1].min(uv[3]),
                        (uv[2] - uv[0]).abs(),
                        (uv[3] - uv[1]).abs(),
                        esc(url)
                    );
                    let _ = writeln!(out, "      </g>");
                }
                Prim::Stroke {
                    pts,
                    color,
                    width,
                    dash,
                } => {
                    if pts.len() < 2 {
                        continue;
                    }
                    let (c, a) = hex(map_color(color, opts.dark));
                    let dash = dash
                        .map(|(on, off)| format!(r#" stroke-dasharray="{on:.1} {off:.1}""#))
                        .unwrap_or_default();
                    let _ = writeln!(
                        out,
                        r#"      <path d="{}" fill="none" stroke="{c}" stroke-opacity="{a:.3}" stroke-width="{width:.2}" stroke-linecap="round" stroke-linejoin="round"{dash}/>"#,
                        path_d(&pts, false)
                    );
                }
                Prim::Fill { pts, color } => {
                    let (c, a) = hex(map_color(color, opts.dark));
                    let _ = writeln!(
                        out,
                        r#"      <path d="{}" fill="{c}" fill-opacity="{a:.3}" stroke="none"/>"#,
                        path_d(&pts, true)
                    );
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
                    let fam = Family::from_excalidraw(family);
                    let (c, a) = hex(map_color(color, opts.dark));
                    let lh = size * line_height;
                    let base = rf.baseline(fam, size, lh);
                    let (anchor, x) = match align {
                        1 => ("middle", origin.x + width / 2.0),
                        2 => ("end", origin.x + width),
                        _ => ("start", origin.x),
                    };
                    let _ = writeln!(
                        out,
                        r#"      <g transform="rotate({:.3} {:.2} {:.2})">"#,
                        angle.to_degrees(),
                        center.x,
                        center.y
                    );
                    for (i, line) in text.split('\n').enumerate() {
                        let _ = writeln!(
                            out,
                            r#"        <text x="{x:.2}" y="{:.2}" font-family="{}" font-size="{size:.1}px" fill="{c}" fill-opacity="{a:.3}" text-anchor="{anchor}" style="white-space: pre;" xml:space="preserve">{}</text>"#,
                            origin.y + i as f32 * lh + base,
                            fam.svg_font_family(),
                            esc(line)
                        );
                    }
                    let _ = writeln!(out, "      </g>");
                }
            }
        }
        let _ = writeln!(out, "    </g>");
    }
    let _ = writeln!(out, "  </g>\n</svg>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Element, Style};

    fn sample() -> Vec<Element> {
        let s = Style::default();
        let mut r = Element::new("rectangle", 0.0, 0.0, &s);
        r.width = 120.0;
        r.height = 60.0;
        r.background_color = "#a5d8ff".into();
        r.fill_style = "hachure".into();
        let mut t = Element::new("text", 10.0, 80.0, &s);
        t.set_text("Olá\nmundo");
        t.width = 60.0;
        t.height = 50.0;
        vec![r, t]
    }

    /// Frame 100×80 em (0, 0) com um retângulo que passa da borda e outro fora.
    fn framed() -> Vec<Element> {
        let s = Style::default();
        let mut f = Element::new("frame", 0.0, 0.0, &s);
        f.width = 100.0;
        f.height = 80.0;
        f.set_field("name", serde_json::json!("Tela 1"));
        let mut child = Element::new("rectangle", 50.0, 40.0, &s);
        child.width = 200.0;
        child.height = 200.0;
        child.background_color = "#e03131".into();
        child.frame_id = Some(f.id.clone());
        let mut out = Element::new("rectangle", 400.0, 0.0, &s);
        out.width = 50.0;
        out.height = 50.0;
        vec![child, out, f]
    }

    #[test]
    fn single_frame_export_is_clipped_to_the_frame() {
        let els = framed();
        let opts = ExportOpts {
            scale: 1.0,
            background: false,
            frame: Some(els[2].id.clone()),
            ..Default::default()
        };
        let pm = render_pixmap(&els, &opts).unwrap();
        assert_eq!((pm.width(), pm.height()), (100, 80));
        // o filho aparece (vermelho) no canto de baixo à direita; fora dele, nada
        let px = |x, y| pm.pixel(x, y).unwrap();
        assert!(px(90, 70).red() > 200 && px(90, 70).alpha() == 255);
        assert_eq!(px(10, 10).alpha(), 0);
        let svg = render_svg(&els, &opts).unwrap();
        assert!(svg.contains("clipPath") && svg.contains(&els[0].id));
        assert!(!svg.contains(&els[1].id), "fora do frame não entra");
        assert!(!svg.contains("Tela 1"), "sem a borda e o nome do frame");

        let missing = ExportOpts {
            frame: Some("nada".into()),
            ..Default::default()
        };
        assert!(render_png(&els, &missing).is_err());
    }

    #[test]
    fn full_export_shows_frame_names_and_clips_children() {
        let els = framed();
        let b = export_bounds(&els, &ExportOpts::default()).unwrap();
        assert!(b.min.y < -10.0, "o nome fica acima do frame");
        assert!(b.max.y < 100.0, "o filho é cortado na borda do frame");
        let svg = render_svg(&els, &ExportOpts::default()).unwrap();
        assert!(svg.contains("Tela 1") && svg.contains("clip-path"));
        assert_eq!(crate::frame::find(&els, "Tela 1"), Some(els[2].id.clone()));
    }

    #[test]
    fn png_and_svg() {
        let els = sample();
        let png = render_png(&els, &ExportOpts::default()).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let svg = render_svg(&els, &ExportOpts::default()).unwrap();
        assert!(svg.contains("<path") && svg.contains("Olá") && svg.contains("@font-face"));
    }

    #[test]
    fn huge_text_is_drawn_without_running_out_of_memory() {
        // antes: o bloco de um texto com fontSize 100000 pedia 150 GB
        let text = |size: f64| {
            let mut els = vec![];
            let spec = serde_json::json!({"type": "text", "text": "oi", "fontSize": size});
            crate::edit::add(&mut els, &mut Map::new(), &[spec]).unwrap();
            els
        };
        let opts = |scale| ExportOpts {
            scale,
            padding: 0.0,
            ..Default::default()
        };
        let mut huge = text(100000.0);
        (huge[0].width, huge[0].height) = (100.0, 50.0);
        assert!(render_pixmap(&huge, &opts(1.0)).is_ok());
        // acima do limite por caractere: rasterizado menor e ampliado, mas aparece
        let big = text(8000.0);
        let pm = render_pixmap(&big, &opts(0.3)).unwrap();
        let dark = pm.pixels().iter().filter(|p| p.red() < 128).count();
        assert!(
            dark > pm.pixels().len() / 50,
            "o texto foi desenhado ({dark} px)"
        );
    }

    #[test]
    fn images_are_drawn() {
        let mut opts = ExportOpts {
            scale: 1.0,
            padding: 0.0,
            background: false,
            ..Default::default()
        };
        let red = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let a = crate::files::add_bytes(&mut opts.files, &crate::files::encode_png(&red).unwrap())
            .unwrap();
        let mut el = crate::files::image_element(&a, 10.0, 10.0, 20.0);
        el.width = 20.0;
        el.height = 20.0;
        el.x = 0.0;
        el.y = 0.0;
        let pm = render_pixmap(std::slice::from_ref(&el), &opts).unwrap();
        let px = pm.pixel(10, 10).unwrap();
        assert_eq!((px.red(), px.green(), px.alpha()), (255, 0, 255));
        let svg = render_svg(std::slice::from_ref(&el), &opts).unwrap();
        assert!(svg.contains("<image href=\"data:image/png;base64,"));
        // sem o arquivo: um quadro cinza no lugar
        opts.files.clear();
        let pm = render_pixmap(&[el], &opts).unwrap();
        let px = pm.pixel(10, 10).unwrap();
        assert_eq!((px.red(), px.green()), (233, 236));
    }
}
