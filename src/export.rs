//! Exportação da cena para PNG (tiny-skia + fontdue) e SVG.

use crate::fonts::{self, Family};
use crate::geom::{Prim, Rgba, element_prims, image_placeholder, local_box, scene_bounds};
use crate::model::{Element, dark_filter, parse_color};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt::Write as _;
use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, StrokeDash,
    Transform,
};

#[derive(Clone, Debug)]
pub struct ExportOpts {
    pub scale: f32,
    pub background: bool,
    pub dark: bool,
    pub padding: f32,
    pub view_background: String,
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
            files: Map::new(),
        }
    }
}

fn map_color(c: Rgba, dark: bool) -> Rgba {
    if dark { dark_filter(c) } else { c }
}

pub fn bounds_with_text(els: &[Element]) -> Option<crate::geom::Rect> {
    let mut r = scene_bounds(els.iter())?;
    for e in els.iter().filter(|e| e.kind == "text") {
        r = r.union(local_box(e));
    }
    Some(r)
}

// ---------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------

pub fn render_png(els: &[Element], opts: &ExportOpts) -> Result<Vec<u8>, String> {
    let pixmap = render_pixmap(els, opts)?;
    pixmap.encode_png().map_err(|e| e.to_string())
}

pub fn render_pixmap(els: &[Element], opts: &ExportOpts) -> Result<Pixmap, String> {
    let b = bounds_with_text(els).ok_or("nada para exportar")?;
    let s = opts.scale.max(0.1);
    let pad = opts.padding;
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
    let mut images = HashMap::new();
    for el in els {
        for prim in element_prims(el) {
            if let Prim::Image { file_id, .. } = &prim {
                let img = images
                    .entry(file_id.clone())
                    .or_insert_with(|| image_pixmap(&opts.files, file_id));
                if let Some(img) = img {
                    draw_image(&mut pm, &prim, img, base);
                    continue;
                }
                for p in image_placeholder(&prim) {
                    draw_prim(&mut pm, &p, base, s, opts.dark);
                }
                continue;
            }
            draw_prim(&mut pm, &prim, base, s, opts.dark);
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

fn draw_image(pm: &mut Pixmap, prim: &Prim, img: &Pixmap, base: Transform) {
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
    pm.fill_rect(rect, &paint, rt, None);
}

fn paint_for(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn draw_prim(pm: &mut Pixmap, prim: &Prim, base: Transform, s: f32, dark: bool) {
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
                None,
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
                None,
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
            let px = size * s;
            let lh = size * line_height * s;
            let lines: Vec<&str> = text.split('\n').collect();
            let widths: Vec<f32> = lines.iter().map(|l| rf.line_width(fam, l, px)).collect();
            let block_w = widths.iter().cloned().fold(width * s, f32::max).ceil() + 4.0;
            let block_h = (lh * lines.len() as f32).ceil() + 4.0;
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
            pm.draw_pixmap(0, 0, tmp.as_ref(), &PixmapPaint::default(), t, None);
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
    let b = bounds_with_text(els).ok_or("nada para exportar")?;
    let pad = opts.padding;
    let w = b.width() + pad * 2.0;
    let h = b.height() + pad * 2.0;
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.2} {h:.2}">"#
    );
    let _ = writeln!(out, "  <!-- svg-source:rustdraw -->");
    let uses_hand = els
        .iter()
        .any(|e| e.kind == "text" && Family::from_excalidraw(e.font_family()) == Family::Hand);
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
    for el in els {
        let _ = writeln!(out, "    <g data-id=\"{}\">", esc(&el.id));
        let image_url = |id: &str| {
            opts.files
                .get(id)
                .and_then(|f| f.get("dataURL"))
                .and_then(|u| u.as_str())
                .filter(|u| u.starts_with("data:image/"))
        };
        let prims: Vec<Prim> = element_prims(el)
            .into_iter()
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

    #[test]
    fn png_and_svg() {
        let els = sample();
        let png = render_png(&els, &ExportOpts::default()).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let svg = render_svg(&els, &ExportOpts::default()).unwrap();
        assert!(svg.contains("<path") && svg.contains("Olá") && svg.contains("@font-face"));
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
