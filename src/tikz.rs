//! Exportação da cena para TikZ (LaTeX).
//!
//! Formas, setas e textos viram comandos do TikZ com traço limpo (sem o
//! efeito à mão do rough.js), para editar e compilar junto com o documento:
//! retângulos e losangos são caminhos, elipses são `ellipse`, setas curvas
//! são Béziers (a mesma curva da tela) com as pontas do `arrows.meta`, a
//! hachura usa o `patterns.meta` e cada linha de texto é um `\node`. O texto
//! entre `$...$` passa direto, como fórmula. O que não tem equivalente
//! (traço livre, tipos desconhecidos) sai das mesmas primitivas do PNG/SVG.
//!
//! Coordenadas em pt: 1 px do desenho = 0,75 pt (96 px por polegada, como no
//! CSS), com o eixo y para cima, como o TikZ espera.

use crate::elbow;
use crate::export::{ExportOpts, Item, map_color, plan};
use crate::fonts::{self, Family};
use crate::frame;
use crate::geom::{P, Prim, Rgba, abs_points, corner_radius, image_placeholder, local_box, rotate};
use crate::model::{Element, parse_color};
use crate::note;
use crate::rough;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

/// Um px do desenho, em pt.
const PT: f32 = 0.75;
/// Maior lado da figura, em pt. O TeX não passa de 16383 pt; desenhos maiores
/// saem reduzidos por inteiro (traços e letras também).
const MAX_SIDE: f32 = 8000.0;

pub struct Tikz {
    pub code: String,
    /// Imagens que o código inclui: (caminho relativo, conteúdo PNG/JPG).
    pub images: Vec<(String, Vec<u8>)>,
}

/// Traço de um caminho: cor, espessura (px) e tracejado `(traço, espaço)`.
struct Line {
    color: Rgba,
    width: f32,
    dash: Option<(f32, f32)>,
}

enum Paint {
    Solid(Rgba),
    /// Hachura: cor, distância e espessura das linhas (px), cruzada ou não.
    Hatch(Rgba, f32, f32, bool),
}

struct Writer<'a> {
    opts: &'a ExportOpts,
    /// Canto de cima à esquerda da área exportada (com a margem).
    origin: P,
    /// Um px do desenho, em pt ([`PT`], ou menos num desenho enorme).
    unit: f32,
    body: String,
    colors: HashMap<[u8; 3], usize>,
    libs: BTreeSet<&'static str>,
    /// Pasta (relativa) onde as imagens vão; `None` desenha um quadro no lugar.
    image_dir: Option<&'a str>,
    images: Vec<(String, Vec<u8>)>,
    image_names: HashMap<(String, [u32; 4]), Option<String>>,
}

/// Número curto: até 2 casas, sem zeros à direita.
fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn dim(px: f32, unit: f32) -> String {
    format!("{}pt", num(px * unit))
}

fn opacity(c: Rgba) -> f32 {
    c[3] as f32 / 255.0
}

impl Writer<'_> {
    fn dim(&self, px: f32) -> String {
        dim(px, self.unit)
    }

    fn xy(&self, p: P) -> String {
        format!(
            "({},{})",
            num((p.x - self.origin.x) * self.unit),
            num((self.origin.y - p.y) * self.unit)
        )
    }

    /// Nome da cor (definida no começo da figura) e a opacidade dela.
    fn color(&mut self, c: Rgba) -> String {
        let c = map_color(c, self.opts.dark);
        let n = self.colors.len();
        let i = *self.colors.entry([c[0], c[1], c[2]]).or_insert(n);
        format!("rd{i}")
    }

    /// Opções de um caminho com este traço e preenchimento.
    fn style(&mut self, line: Option<&Line>, fill: Option<&Paint>) -> Vec<String> {
        let mut v = vec![];
        if let Some(l) = line {
            v.push(format!("draw={}", self.color(l.color)));
            if l.color[3] < 255 {
                v.push(format!("draw opacity={}", num(opacity(l.color))));
            }
            v.push(format!("line width={}", self.dim(l.width)));
            if let Some((on, off)) = l.dash {
                v.push(format!(
                    "dash pattern=on {} off {}",
                    self.dim(on),
                    self.dim(off)
                ));
            }
        }
        let c = match fill {
            Some(Paint::Solid(c)) => {
                v.push(format!("fill={}", self.color(*c)));
                Some(*c)
            }
            Some(Paint::Hatch(c, gap, width, cross)) => {
                self.libs.insert("patterns.meta");
                // as linhas da hachura sobem a 41° (-41° na tela, com y para baixo)
                v.push(format!(
                    "pattern={{{}[angle=41, distance={}, line width={}]}}",
                    if *cross { "Hatch" } else { "Lines" },
                    self.dim(*gap),
                    self.dim(*width)
                ));
                v.push(format!("pattern color={}", self.color(*c)));
                Some(*c)
            }
            None => None,
        };
        if let Some(c) = c.filter(|c| c[3] < 255) {
            v.push(format!("fill opacity={}", num(opacity(c))));
        }
        v
    }

    /// `\draw` (ou `\path`, sem traço) com as opções e o caminho.
    fn path(&mut self, line: Option<&Line>, fill: Option<&Paint>, extra: &[String], path: &str) {
        if line.is_none() && fill.is_none() {
            return;
        }
        let mut style = self.style(line, fill);
        style.extend_from_slice(extra);
        let cmd = if line.is_some() { "\\draw" } else { "\\path" };
        let _ = writeln!(self.body, "  {cmd}[{}] {path};", style.join(", "));
    }

    /// Polígono fechado. Começa no meio do primeiro lado para que
    /// `rounded corners` arredonde todos os cantos.
    fn polygon(&self, pts: &[P]) -> String {
        let n = pts.len();
        let start = (pts[n - 1] + pts[0]) * 0.5;
        let mut s = self.xy(start);
        for p in pts {
            let _ = write!(s, " -- {}", self.xy(*p));
        }
        s.push_str(" -- cycle");
        s
    }

    /// Polilinha, quebrando a linha a cada tantos pontos.
    fn polyline(&self, pts: &[P]) -> String {
        let mut s = String::new();
        let mut last: Option<P> = None;
        let mut n = 0;
        for (i, p) in pts.iter().enumerate() {
            // pontos quase repetidos (traço livre) não mudam o desenho
            if i + 1 < pts.len() && last.is_some_and(|l| (*p - l).len() < 0.5) {
                continue;
            }
            if last.is_some() {
                s.push_str(if n % 8 == 0 { "\n      -- " } else { " -- " });
            }
            s.push_str(&self.xy(*p));
            last = Some(*p);
            n += 1;
        }
        s
    }

    fn element(&mut self, e: &Element, item: &Item) {
        let b = local_box(e);
        let c = b.center();
        let a = e.angle as f32;
        let turn = |pts: Vec<P>| -> Vec<P> { pts.into_iter().map(|p| rotate(p, c, a)).collect() };
        let line = stroke_of(e);
        let fill = fill_of(e);
        match e.kind.as_str() {
            "rectangle" if note::is_note(e) => {
                let paper = turn(note::outline(&b));
                let path = self.polygon(&paper);
                self.path(line.as_ref(), fill.as_ref(), &[], &path);
                let bg = parse_color(&e.background_color).map(|mut c| {
                    c[3] = (c[3] as f32 * element_opacity(e)).round() as u8;
                    c
                });
                for p in note::decorations(&b, bg, element_opacity(e)).1 {
                    if let Prim::Fill { pts, color } = p {
                        let path = self.polygon(&turn(pts));
                        self.path(None, Some(&Paint::Solid(color)), &[], &path);
                    }
                }
            }
            "rectangle" => {
                let pts = turn(vec![
                    b.min,
                    P::new(b.max.x, b.min.y),
                    b.max,
                    P::new(b.min.x, b.max.y),
                ]);
                let mut extra = vec![];
                if e.is_round() {
                    let r = corner_radius(e).min(b.width() / 2.0).min(b.height() / 2.0);
                    extra.push(format!("rounded corners={}", self.dim(r)));
                }
                let path = self.polygon(&pts);
                self.path(line.as_ref(), fill.as_ref(), &extra, &path);
            }
            "diamond" => {
                let pts = turn(vec![
                    P::new(c.x, b.min.y),
                    P::new(b.max.x, c.y),
                    P::new(c.x, b.max.y),
                    P::new(b.min.x, c.y),
                ]);
                let path = self.polygon(&pts);
                self.path(line.as_ref(), fill.as_ref(), &[], &path);
            }
            "ellipse" => {
                let mut extra = vec![];
                if a != 0.0 {
                    extra.push(format!(
                        "rotate around={{{}:{}}}",
                        num(-a.to_degrees()),
                        self.xy(c)
                    ));
                }
                let path = format!(
                    "{} ellipse [x radius={}, y radius={}]",
                    self.xy(c),
                    self.dim(b.width() / 2.0),
                    self.dim(b.height() / 2.0)
                );
                self.path(line.as_ref(), fill.as_ref(), &extra, &path);
            }
            "line" | "arrow" => self.linear(e, turn(abs_points(e)), line, fill),
            "frame" | "magicframe" => {
                let r = frame::RADIUS.min(b.width() / 2.0).min(b.height() / 2.0);
                let pts = [
                    b.min,
                    P::new(b.max.x, b.min.y),
                    b.max,
                    P::new(b.min.x, b.max.y),
                ];
                let border = parse_color(&e.stroke_color).map(|color| Line {
                    color,
                    width: 1.0,
                    dash: None,
                });
                let path = self.polygon(&pts);
                self.path(
                    border.as_ref(),
                    None,
                    &[format!("rounded corners={}", self.dim(r))],
                    &path,
                );
                for p in item.prims.iter().filter(|p| matches!(p, Prim::Text { .. })) {
                    self.prim(p);
                }
            }
            _ => {
                for p in &item.prims {
                    self.prim(p);
                }
            }
        }
    }

    fn linear(&mut self, e: &Element, pts: Vec<P>, line: Option<Line>, fill: Option<Paint>) {
        if pts.len() < 2 {
            return;
        }
        let closed = pts.len() > 2 && (pts[0] - pts[pts.len() - 1]).len() < 1.0;
        let fill = fill.filter(|_| closed && e.kind == "line");
        let curved = e.is_round() && pts.len() > 2;
        let mut path = self.xy(pts[0]);
        if curved {
            // a mesma curva de `rough::smooth_through`, em Béziers cúbicas
            let n = pts.len();
            let at = |k: isize| pts[k.clamp(0, n as isize - 1) as usize];
            for i in 0..n - 1 {
                let k = i as isize;
                let b1 = pts[i] + (pts[i + 1] - at(k - 1)) * (1.0 / 6.0);
                let b2 = pts[i + 1] - (at(k + 2) - pts[i]) * (1.0 / 6.0);
                let _ = write!(
                    path,
                    "\n      .. controls {} and {} .. {}",
                    self.xy(b1),
                    self.xy(b2),
                    self.xy(pts[i + 1])
                );
            }
        } else if e.is_elbow() {
            // cantos arredondados como em `elbow::rounded_path`
            for i in 1..pts.len() - 1 {
                let (pa, p, pb) = (pts[i - 1], pts[i], pts[i + 1]);
                let r = elbow::CORNER
                    .min((p - pa).len() / 2.0)
                    .min((pb - p).len() / 2.0);
                if r < 0.5 {
                    let _ = write!(path, " -- {}", self.xy(p));
                    continue;
                }
                let s = p + (pa - p).normalized() * r;
                let t = p + (pb - p).normalized() * r;
                let _ = write!(
                    path,
                    " -- {} .. controls {} and {} .. {}",
                    self.xy(s),
                    self.xy(s + (p - s) * (2.0 / 3.0)),
                    self.xy(t + (p - t) * (2.0 / 3.0)),
                    self.xy(t)
                );
            }
            let _ = write!(path, " -- {}", self.xy(pts[pts.len() - 1]));
        } else {
            path = self.polyline(&pts);
            if closed {
                path.push_str(" -- cycle");
            }
        }
        let mut extra = vec![];
        if e.kind == "arrow" {
            let ideal = if curved {
                rough::smooth_through(&pts)
            } else {
                pts.clone()
            };
            let total: f32 = ideal.windows(2).map(|w| (w[1] - w[0]).len()).sum();
            let sw = line.as_ref().map_or(1.0, |l| l.width);
            let tip = |end| {
                e.arrowhead(end)
                    .map(|k| self.tip(&k, total, sw))
                    .unwrap_or_default()
            };
            let (start, end) = (tip(false), tip(true));
            if !start.is_empty() || !end.is_empty() {
                self.libs.insert("arrows.meta");
                extra.push(format!("arrows={{{start}-{end}}}"));
            }
        }
        self.path(line.as_ref(), fill.as_ref(), &extra, &path);
    }

    /// Primitiva do PNG/SVG (traço livre, nome do frame, tipos desconhecidos).
    fn prim(&mut self, p: &Prim) {
        match p {
            Prim::Stroke {
                pts,
                color,
                width,
                dash,
            } => {
                if pts.len() < 2 {
                    return;
                }
                let line = Line {
                    color: *color,
                    width: *width,
                    dash: *dash,
                };
                let path = self.polyline(pts);
                self.path(Some(&line), None, &[], &path);
            }
            Prim::Fill { pts, color } => {
                if pts.len() < 3 {
                    return;
                }
                let path = self.polygon(pts);
                self.path(None, Some(&Paint::Solid(*color)), &[], &path);
            }
            Prim::Text { .. } => self.text(p),
            Prim::Image { .. } => self.image(p),
        }
    }

    /// Um `\node` por linha, na linha de base, alinhado como no desenho.
    fn text(&mut self, p: &Prim) {
        let Prim::Text {
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
        } = p
        else {
            return;
        };
        let fam = Family::from_excalidraw(*family);
        let lh = size * line_height;
        let base = fonts::raster_fonts().baseline(fam, *size, lh);
        // o Virgil não existe no LaTeX: o texto à mão sai na fonte do documento
        let face = match fam {
            Family::Hand => "",
            Family::Sans => "\\sffamily",
            Family::Mono => "\\ttfamily",
        };
        let (anchor, x) = match align {
            1 => ("base", origin.x + width / 2.0),
            2 => ("base east", origin.x + width),
            _ => ("base west", origin.x),
        };
        let col = self.color(*color);
        let mut style = format!(
            "anchor={anchor}, text={col}, font=\\fontsize{{{}}}{{{}}}\\selectfont{face}",
            num(size * self.unit),
            num(size * 1.2 * self.unit)
        );
        if color[3] < 255 {
            let _ = write!(style, ", text opacity={}", num(opacity(*color)));
        }
        let rotated = *angle != 0.0;
        if rotated {
            let _ = writeln!(
                self.body,
                "  \\begin{{scope}}[rotate around={{{}:{}}}, transform shape]",
                num(-angle.to_degrees()),
                self.xy(*center)
            );
        }
        for (i, line) in text.split('\n').enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let at = P::new(x, origin.y + i as f32 * lh + base);
            let _ = writeln!(
                self.body,
                "  \\node[{style}] at {} {{{}}};",
                self.xy(at),
                latex_text(line.trim_end())
            );
        }
        if rotated {
            let _ = writeln!(self.body, "  \\end{{scope}}");
        }
    }

    fn image(&mut self, p: &Prim) {
        let Prim::Image {
            file_id,
            min,
            max,
            uv,
            angle,
            center,
            opacity,
        } = p
        else {
            return;
        };
        let Some(name) = self.image_file(file_id, *uv) else {
            let _ = writeln!(self.body, "  % imagem {file_id} não incluída");
            for q in image_placeholder(p) {
                self.prim(&q);
            }
            return;
        };
        let mut style = vec!["anchor=center".to_string()];
        if *angle != 0.0 {
            style.push(format!("rotate={}", num(-angle.to_degrees())));
        }
        if *opacity < 1.0 {
            style.push(format!("opacity={}", num(*opacity)));
        }
        let _ = writeln!(
            self.body,
            "  \\node[{}] at {} {{\\includegraphics[width={}, height={}]{{{name}}}}};",
            style.join(", "),
            self.xy(*center),
            self.dim(max.x - min.x),
            self.dim(max.y - min.y)
        );
    }

    /// Caminho da imagem (com o recorte e o espelhamento já aplicados), gravada
    /// uma vez por arquivo e recorte. PNG e JPG sem recorte vão como estão; o
    /// resto vira PNG, que o pdflatex lê.
    fn image_file(&mut self, id: &str, uv: [f32; 4]) -> Option<String> {
        let dir = self.image_dir?;
        let key = (id.to_string(), uv.map(f32::to_bits));
        if let Some(name) = self.image_names.get(&key) {
            return name.clone();
        }
        let name = (|| {
            let url = self.opts.files.get(id)?.get("dataURL")?.as_str()?;
            let bytes = crate::files::parse_data_url(url)?;
            let whole = uv == [0.0, 0.0, 1.0, 1.0];
            let (data, ext) = match crate::files::mime_of(&bytes) {
                Some("image/png") if whole => (bytes, "png"),
                Some("image/jpeg") if whole => (bytes, "jpg"),
                _ => {
                    let img = crate::files::decode_bytes(&bytes)?;
                    let (w, h) = (img.width() as f32, img.height() as f32);
                    let x0 = (uv[0].min(uv[2]) * w).round().max(0.0) as u32;
                    let y0 = (uv[1].min(uv[3]) * h).round().max(0.0) as u32;
                    let x1 = ((uv[0].max(uv[2]) * w).round() as u32).min(img.width());
                    let y1 = ((uv[1].max(uv[3]) * h).round() as u32).min(img.height());
                    let mut img = image::imageops::crop_imm(
                        &img,
                        x0,
                        y0,
                        x1.saturating_sub(x0).max(1),
                        y1.saturating_sub(y0).max(1),
                    )
                    .to_image();
                    if uv[0] > uv[2] {
                        image::imageops::flip_horizontal_in_place(&mut img);
                    }
                    if uv[1] > uv[3] {
                        image::imageops::flip_vertical_in_place(&mut img);
                    }
                    (crate::files::encode_png(&img)?, "png")
                }
            };
            let name = format!("{dir}/imagem-{}.{ext}", self.images.len() + 1);
            self.images.push((name.clone(), data));
            Some(name)
        })();
        self.image_names.insert(key, name.clone());
        name
    }
}

fn element_opacity(e: &Element) -> f32 {
    (e.opacity as f32 / 100.0).clamp(0.0, 1.0)
}

fn with_opacity(c: Option<Rgba>, e: &Element) -> Option<Rgba> {
    c.map(|mut c| {
        c[3] = (c[3] as f32 * element_opacity(e)).round() as u8;
        c
    })
    .filter(|c| c[3] > 0)
}

/// Traço do elemento, com a mesma espessura e tracejado do PNG/SVG.
fn stroke_of(e: &Element) -> Option<Line> {
    let color = with_opacity(parse_color(&e.stroke_color), e)?;
    let solid = e.stroke_style == "solid";
    let width = e.stroke_width as f32 + if solid { 0.0 } else { 0.5 };
    let dash = match e.stroke_style.as_str() {
        "dashed" => Some((8.0, 8.0 + width)),
        "dotted" => Some((1.5, 6.0 + width)),
        _ => None,
    };
    Some(Line { color, width, dash })
}

fn fill_of(e: &Element) -> Option<Paint> {
    let c = with_opacity(parse_color(&e.background_color), e)?;
    Some(match e.fill_style.as_str() {
        "solid" => Paint::Solid(c),
        style => Paint::Hatch(
            c,
            (e.stroke_width as f32 * 4.0).max(4.0),
            (e.stroke_width as f32 / 2.0).max(0.5),
            style == "cross-hatch",
        ),
    })
}

impl Writer<'_> {
    /// Ponta de seta do `arrows.meta` com o tamanho das pontas do desenho
    /// (`geom::arrowhead`).
    fn tip(&self, kind: &str, total: f32, sw: f32) -> String {
        let (size, angle, mult): (f32, f32, f32) = match kind {
            "arrow" => (25.0, 20.0, 0.5),
            "bar" => (15.0, 90.0, 0.5),
            "diamond" | "diamond_outline" => (12.0, 25.0, 0.25),
            _ => (15.0, 25.0, 0.5),
        };
        let size = size.min(total * mult);
        if size < 0.5 {
            return String::new();
        }
        let a = angle.to_radians();
        let open = if kind.ends_with("_outline") {
            ", open"
        } else {
            ""
        };
        match kind {
            "triangle" | "triangle_outline" => format!(
                "Triangle[length={}, width={}{open}]",
                self.dim(size * a.cos()),
                self.dim(2.0 * size * a.sin())
            ),
            "dot" | "circle" | "circle_outline" => {
                let d = self.dim((size + sw - 2.0).max(2.0));
                format!("Circle[length={d}, width={d}{open}]")
            }
            "diamond" | "diamond_outline" => format!(
                "Diamond[length={}, width={}{open}]",
                self.dim(2.0 * size),
                self.dim(1.2 * size)
            ),
            "bar" => format!("Bar[width={}]", self.dim(2.0 * size)),
            _ => format!(
                "Straight Barb[length={}, width={}]",
                self.dim(size * a.cos()),
                self.dim(2.0 * size * a.sin())
            ),
        }
    }
}

/// Escapa o texto para o LaTeX. Trechos entre `$...$` passam como estão
/// (fórmulas), seguindo a regra do Pandoc: o `$` de abertura vem colado no
/// que segue e o de fechamento colado no que vem antes e sem dígito depois,
/// então "R$ 5 e R$ 10" e "custa $5 ou $10" continuam texto.
fn latex_text(s: &str) -> String {
    let ch: Vec<char> = s.chars().collect();
    let n = ch.len();
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        if ch[i] == '$' && i + 1 < n && !ch[i + 1].is_whitespace() && ch[i + 1] != '$' {
            let close = (i + 2..n).find(|&j| {
                ch[j] == '$'
                    && !ch[j - 1].is_whitespace()
                    && ch[j - 1] != '\\'
                    && !ch.get(j + 1).is_some_and(|c| c.is_ascii_digit())
            });
            if let Some(j) = close {
                out.extend(&ch[i..=j]);
                i = j + 1;
                continue;
            }
        }
        match ch[i] {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' | '}' | '$' | '&' | '#' | '%' | '_' => {
                out.push('\\');
                out.push(ch[i]);
            }
            '^' => out.push_str("\\textasciicircum{}"),
            '~' => out.push_str("\\textasciitilde{}"),
            '<' => out.push_str("\\textless{}"),
            '>' => out.push_str("\\textgreater{}"),
            '|' => out.push_str("\\textbar{}"),
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// Gera o TikZ. Com `image_dir`, as imagens são incluídas de lá (e
/// devolvidas em `images` para gravar); sem, viram um quadro cinza.
pub fn render_tikz(
    els: &[Element],
    opts: &ExportOpts,
    image_dir: Option<&str>,
) -> Result<Tikz, String> {
    let plan = plan(els, opts)?;
    let b = plan.bounds;
    let pad = plan.padding;
    let mut w = Writer {
        opts,
        origin: P::new(b.min.x - pad, b.max.y + pad),
        unit: PT.min(MAX_SIDE / (b.width() + pad * 2.0).max(b.height() + pad * 2.0)),
        body: String::new(),
        colors: HashMap::new(),
        libs: BTreeSet::new(),
        image_dir,
        images: vec![],
        image_names: HashMap::new(),
    };
    let (width, height) = (b.width() + pad * 2.0, b.height() + pad * 2.0);
    if opts.background {
        let bg = parse_color(&opts.view_background).unwrap_or([255, 255, 255, 255]);
        let path = format!(
            "(0,0) rectangle ({},{})",
            num(width * w.unit),
            num(height * w.unit)
        );
        w.path(None, Some(&Paint::Solid(bg)), &[], &path);
    }
    let by_id: HashMap<&str, &Element> = els.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut clip: Option<&str> = None;
    for item in &plan.items {
        if item.clip != clip {
            if clip.is_some() {
                let _ = writeln!(w.body, "  \\end{{scope}}");
            }
            if let Some(f) = item.clip {
                let r = plan.clips[f];
                let _ = writeln!(w.body, "  \\begin{{scope}}");
                let _ = writeln!(
                    w.body,
                    "  \\clip {} rectangle {};",
                    w.xy(P::new(r.min.x, r.max.y)),
                    w.xy(P::new(r.max.x, r.min.y))
                );
            }
            clip = item.clip;
        }
        if let Some(e) = by_id.get(item.id) {
            w.element(e, item);
        }
    }
    if clip.is_some() {
        let _ = writeln!(w.body, "  \\end{{scope}}");
    }

    let mut out = String::new();
    let _ = writeln!(out, "% Gerado pelo RustDraw. No preâmbulo do documento:");
    let _ = writeln!(out, "%   \\usepackage{{tikz}}");
    if !w.libs.is_empty() {
        let libs: Vec<&str> = w.libs.iter().copied().collect();
        let _ = writeln!(out, "%   \\usetikzlibrary{{{}}}", libs.join(","));
    }
    let _ = writeln!(
        out,
        "% Tamanho: {} x {}. Para caber na largura do texto:",
        dim(width, w.unit),
        dim(height, w.unit)
    );
    if w.unit < PT {
        let _ = writeln!(
            out,
            "% Reduzido a {}% do tamanho normal, o maior que o TeX aceita.",
            num(w.unit / PT * 100.0)
        );
    }
    let _ = writeln!(
        out,
        "%   \\resizebox{{\\linewidth}}{{!}}{{\\input{{este-arquivo}}}}"
    );
    if !w.images.is_empty() {
        let _ = writeln!(
            out,
            "% As imagens são procuradas em relação à pasta onde o LaTeX roda."
        );
    }
    let _ = writeln!(
        out,
        "\\begin{{tikzpicture}}[x=1pt, y=1pt, line cap=round, line join=round, every node/.append style={{inner sep=0pt}}]"
    );
    let mut colors: Vec<(&[u8; 3], &usize)> = w.colors.iter().collect();
    colors.sort_by_key(|(_, i)| **i);
    for (c, i) in colors {
        let _ = writeln!(
            out,
            "  \\definecolor{{rd{i}}}{{HTML}}{{{:02X}{:02X}{:02X}}}",
            c[0], c[1], c[2]
        );
    }
    out.push_str(&w.body);
    out.push_str("\\end{tikzpicture}\n");
    Ok(Tikz {
        code: out,
        images: w.images,
    })
}

/// Grava o `.tex` e as imagens, numa pasta `<nome>-imagens` ao lado.
pub fn write_file(path: &Path, els: &[Element], opts: &ExportOpts) -> Result<usize, String> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "desenho".into());
    // sem espaços nem acentos, que atrapalham o \includegraphics
    let safe: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let dir = format!("{safe}-imagens");
    let t = render_tikz(els, opts, Some(&dir))?;
    let parent = path.parent().unwrap_or(Path::new(""));
    if !t.images.is_empty() {
        let d = parent.join(&dir);
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    for (name, data) in &t.images {
        let p = parent.join(name);
        std::fs::write(&p, data).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    std::fs::write(path, &t.code).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(t.code.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Style;
    use serde_json::json;

    fn tikz(els: &[Element]) -> String {
        let opts = ExportOpts {
            background: false,
            ..Default::default()
        };
        render_tikz(els, &opts, None).unwrap().code
    }

    #[test]
    fn shapes_are_clean_tikz_paths() {
        let s = Style::default();
        let mut r = Element::new("rectangle", 0.0, 0.0, &s);
        (r.width, r.height) = (120.0, 60.0);
        r.background_color = "#a5d8ff".into();
        r.fill_style = "hachure".into();
        let mut el = Element::new("ellipse", 200.0, 0.0, &s);
        (el.width, el.height) = (80.0, 40.0);
        el.angle = std::f64::consts::FRAC_PI_2;
        el.stroke_style = "dashed".into();
        let code = tikz(&[r, el]);
        assert!(code.starts_with("% Gerado pelo RustDraw"));
        assert!(code.contains("\\begin{tikzpicture}") && code.ends_with("\\end{tikzpicture}\n"));
        assert!(code.contains("\\definecolor{rd0}{HTML}{1E1E1E}"));
        assert!(code.contains("rounded corners=") && code.contains("-- cycle"));
        assert!(code.contains("pattern={Lines[angle=41"));
        assert!(code.contains("\\usetikzlibrary{patterns.meta}"));
        assert!(code.contains("ellipse [x radius=30pt, y radius=15pt]"));
        assert!(code.contains("rotate around={-90:"));
        assert!(code.contains("dash pattern=on 6pt off"));
        // traço limpo: nada das várias passadas do rough.js
        assert_eq!(code.matches("\\draw").count(), 2);
    }

    #[test]
    fn arrows_get_arrows_meta_tips_and_curves() {
        let s = Style::default();
        let mut a = Element::new("arrow", 0.0, 0.0, &s);
        a.set_points(vec![[0.0, 0.0], [100.0, 50.0], [200.0, 0.0]]);
        a.set_field("startArrowhead", json!("dot"));
        let code = tikz(&[a]);
        assert!(code.contains("arrows={Circle[") && code.contains("-Straight Barb["));
        assert!(code.contains(".. controls"));
        assert!(code.contains("\\usetikzlibrary{arrows.meta}"));
    }

    #[test]
    fn text_is_escaped_but_math_passes() {
        assert_eq!(latex_text("50% & #1_a"), "50\\% \\& \\#1\\_a");
        assert_eq!(latex_text("área $x^2$ e {y}"), "área $x^2$ e \\{y\\}");
        assert_eq!(latex_text("R$ 5 e R$ 10"), "R\\$ 5 e R\\$ 10");
        assert_eq!(latex_text("custa $5 ou $10"), "custa \\$5 ou \\$10");
        let s = Style::default();
        let mut t = Element::new("text", 10.0, 80.0, &s);
        t.set_text("Olá\n\nmundo");
        (t.width, t.height) = (60.0, 75.0);
        let code = tikz(&[t]);
        assert_eq!(code.matches("\\node").count(), 2, "linha vazia não vira nó");
        assert!(code.contains("{Olá}") && code.contains("{mundo}"));
        assert!(code.contains("anchor=base west") && code.contains("\\fontsize{15}{18}"));
    }

    #[test]
    fn frames_clip_their_children() {
        let s = Style::default();
        let mut f = Element::new("frame", 0.0, 0.0, &s);
        (f.width, f.height) = (100.0, 80.0);
        f.set_field("name", json!("Tela 1"));
        let mut child = Element::new("rectangle", 50.0, 40.0, &s);
        (child.width, child.height) = (200.0, 200.0);
        child.frame_id = Some(f.id.clone());
        let code = tikz(&[child, f]);
        assert!(code.contains("\\clip") && code.contains("{Tela 1}"));
        assert_eq!(code.matches("\\begin{scope}").count(), 1);
    }

    #[test]
    fn images_are_written_next_to_the_tex() {
        let mut opts = ExportOpts::default();
        let red = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]));
        let a = crate::files::add_bytes(&mut opts.files, &crate::files::encode_png(&red).unwrap())
            .unwrap();
        let mut el = crate::files::image_element(&a, 10.0, 10.0, 40.0);
        (el.width, el.height) = (40.0, 20.0);
        let els = [el];
        // sem pasta (copiar): um quadro no lugar
        let t = render_tikz(&els, &opts, None).unwrap();
        assert!(t.images.is_empty() && !t.code.contains("includegraphics"));
        let dir = std::env::temp_dir().join(format!("rustdraw-tikz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tex = dir.join("Meu desenho.tex");
        write_file(&tex, &els, &opts).unwrap();
        let code = std::fs::read_to_string(&tex).unwrap();
        assert!(code.contains(
            "\\includegraphics[width=30pt, height=15pt]{Meu-desenho-imagens/imagem-1.png}"
        ));
        let png = std::fs::read(dir.join("Meu-desenho-imagens/imagem-1.png")).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
