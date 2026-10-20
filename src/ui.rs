//! Interface flutuante sobre o canvas: ferramentas, propriedades, menu,
//! zoom/desfazer, editor de texto, exportação, ajuda e avisos.

use crate::app::{App, Drag, Pending, SELECTION, Tool};
use crate::model::{Element, parse_color};
use crate::render;
use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Painter, Pos2, Rect, Response,
    RichText, Sense, Shadow, Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};
use serde_json::json;

const STROKES: [&str; 5] = ["#1e1e1e", "#e03131", "#2f9e44", "#1971c2", "#f08c00"];
const BACKGROUNDS: [&str; 5] = ["transparent", "#ffc9c9", "#b2f2bb", "#a5d8ff", "#ffec99"];
const CANVAS_BGS: [&str; 5] = ["#ffffff", "#f8f9fa", "#f5faff", "#fffce8", "#fdf8f6"];

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    Hand,
    Select,
    Rect,
    Diamond,
    Ellipse,
    Arrow,
    Line,
    Draw,
    Text,
    Image,
    Eraser,
    Frame,
    Lock(bool),
    Menu,
    Undo,
    Redo,
}

/// Rótulos de atalho com os nomes das teclas do sistema: no macOS, "Ctrl"
/// vira "Cmd" e "Alt" vira "Option", como no Excalidraw. Os atalhos em si já
/// usam `Modifiers::COMMAND`, que é a tecla Command no macOS.
pub fn keys(s: &str) -> String {
    keys_for(s, cfg!(target_os = "macos"))
}

fn keys_for(s: &str, mac: bool) -> String {
    if !mac {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut rest = s;
    while !rest.is_empty() {
        let at_word = !out.chars().next_back().is_some_and(char::is_alphanumeric);
        let found = [("Ctrl", "Cmd"), ("Alt", "Option")]
            .into_iter()
            .find(|(from, _)| {
                at_word
                    && rest.starts_with(from)
                    && !rest[from.len()..]
                        .chars()
                        .next()
                        .is_some_and(char::is_alphanumeric)
            });
        if let Some((from, to)) = found {
            out.push_str(to);
            rest = &rest[from.len()..];
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

fn panel_frame(ui: &Ui) -> Frame {
    let dark = ui.visuals().dark_mode;
    Frame::new()
        .fill(if dark {
            Color32::from_rgb(35, 35, 41)
        } else {
            Color32::WHITE
        })
        .corner_radius(CornerRadius::same(8))
        .stroke(Stroke::new(
            1.0,
            if dark {
                Color32::from_gray(55)
            } else {
                Color32::from_gray(225)
            },
        ))
        .shadow(Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(if dark { 90 } else { 28 }),
        })
        .inner_margin(Margin::same(6))
}

fn accent_bg(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(64, 62, 110)
    } else {
        Color32::from_rgb(224, 223, 255)
    }
}

fn fg(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_gray(225)
    } else {
        Color32::from_gray(30)
    }
}

fn rel(r: Rect, fx: f32, fy: f32) -> Pos2 {
    pos2(r.left() + fx * r.width(), r.top() + fy * r.height())
}

fn paint_icon(p: &Painter, r: Rect, icon: Icon, col: Color32) {
    let s = Stroke::new(1.6, col);
    let pts = |v: &[(f32, f32)]| v.iter().map(|(x, y)| rel(r, *x, *y)).collect::<Vec<_>>();
    match icon {
        Icon::Hand => {
            for (i, x) in [0.3f32, 0.45, 0.6, 0.75].iter().enumerate() {
                let top = if i == 0 || i == 3 { 0.22 } else { 0.1 };
                p.line_segment([rel(r, *x, top), rel(r, *x, 0.55)], Stroke::new(2.0, col));
            }
            p.line_segment(
                [rel(r, 0.12, 0.45), rel(r, 0.3, 0.68)],
                Stroke::new(2.0, col),
            );
            let palm = Rect::from_min_max(rel(r, 0.26, 0.5), rel(r, 0.8, 0.95));
            p.rect_stroke(palm, 4.0, s, StrokeKind::Middle);
        }
        Icon::Select => {
            p.add(Shape::closed_line(
                pts(&[
                    (0.28, 0.1),
                    (0.28, 0.82),
                    (0.45, 0.66),
                    (0.57, 0.92),
                    (0.67, 0.87),
                    (0.55, 0.62),
                    (0.78, 0.6),
                ]),
                s,
            ));
        }
        Icon::Rect => {
            p.rect_stroke(r.shrink(2.0), 3.0, s, StrokeKind::Middle);
        }
        Icon::Diamond => {
            p.add(Shape::closed_line(
                pts(&[(0.5, 0.05), (0.95, 0.5), (0.5, 0.95), (0.05, 0.5)]),
                s,
            ));
        }
        Icon::Ellipse => {
            p.circle_stroke(r.center(), r.width() / 2.0 - 1.0, s);
        }
        Icon::Arrow => {
            p.line_segment([rel(r, 0.1, 0.9), rel(r, 0.9, 0.1)], s);
            p.add(Shape::line(pts(&[(0.45, 0.1), (0.9, 0.1), (0.9, 0.55)]), s));
        }
        Icon::Line => {
            p.line_segment([rel(r, 0.1, 0.9), rel(r, 0.9, 0.1)], s);
        }
        Icon::Draw => {
            p.add(Shape::line(
                pts(&[
                    (0.05, 0.75),
                    (0.2, 0.4),
                    (0.35, 0.7),
                    (0.5, 0.3),
                    (0.65, 0.65),
                    (0.8, 0.25),
                    (0.95, 0.45),
                ]),
                s,
            ));
        }
        Icon::Text => {
            p.text(
                r.center(),
                Align2::CENTER_CENTER,
                "A",
                FontId::proportional(r.height() * 1.1),
                col,
            );
        }
        Icon::Image => {
            p.rect_stroke(r.shrink(1.5), 2.0, s, StrokeKind::Middle);
            p.add(Shape::line(
                pts(&[
                    (0.1, 0.85),
                    (0.4, 0.5),
                    (0.6, 0.7),
                    (0.72, 0.58),
                    (0.9, 0.78),
                ]),
                s,
            ));
            p.circle_stroke(rel(r, 0.68, 0.3), r.width() * 0.09, s);
        }
        Icon::Eraser => {
            p.add(Shape::closed_line(
                pts(&[(0.15, 0.6), (0.55, 0.15), (0.9, 0.5), (0.5, 0.95)]),
                s,
            ));
            p.line_segment([rel(r, 0.33, 0.4), rel(r, 0.7, 0.77)], s);
        }
        Icon::Frame => {
            // cerquilha: as bordas passam um pouco dos cantos
            for t in [0.25, 0.75] {
                p.line_segment([rel(r, 0.05, t), rel(r, 0.95, t)], s);
                p.line_segment([rel(r, t, 0.05), rel(r, t, 0.95)], s);
            }
        }
        Icon::Lock(locked) => {
            let body = Rect::from_min_max(rel(r, 0.18, 0.48), rel(r, 0.82, 0.95));
            p.rect_stroke(body, 2.0, s, StrokeKind::Middle);
            let right_y = if locked { 0.48 } else { 0.3 };
            p.add(Shape::line(
                pts(&[
                    (0.3, 0.48),
                    (0.3, 0.3),
                    (0.35, 0.12),
                    (0.5, 0.05),
                    (0.65, 0.12),
                    (0.7, 0.3),
                    (0.7, right_y),
                ]),
                s,
            ));
            if locked {
                p.circle_filled(rel(r, 0.5, 0.7), 1.6, col);
            }
        }
        Icon::Menu => {
            for y in [0.2, 0.5, 0.8] {
                p.line_segment([rel(r, 0.1, y), rel(r, 0.9, y)], s);
            }
        }
        Icon::Undo | Icon::Redo => {
            let mirror = |v: &[(f32, f32)]| {
                v.iter()
                    .map(|(x, y)| {
                        if icon == Icon::Redo {
                            (1.0 - x, *y)
                        } else {
                            (*x, *y)
                        }
                    })
                    .collect::<Vec<_>>()
            };
            p.add(Shape::line(
                pts(&mirror(&[
                    (0.15, 0.4),
                    (0.6, 0.4),
                    (0.85, 0.55),
                    (0.85, 0.75),
                    (0.6, 0.9),
                    (0.35, 0.9),
                ])),
                s,
            ));
            p.add(Shape::line(
                pts(&mirror(&[(0.35, 0.15), (0.12, 0.4), (0.35, 0.65)])),
                s,
            ));
        }
    }
}

fn icon_button(
    ui: &mut Ui,
    selected: bool,
    size: f32,
    tooltip: &str,
    paint: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let bg = if selected {
        accent_bg(ui)
    } else if resp.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, bg);
    let col = if selected && !ui.visuals().dark_mode {
        Color32::from_rgb(0x43, 0x41, 0x9e)
    } else {
        fg(ui)
    };
    paint(ui.painter(), rect, col);
    resp.on_hover_text(keys(tooltip))
}

fn swatch(ui: &mut Ui, color: &str, selected: bool, dark: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let p = ui.painter();
    let inner = rect.shrink(2.0);
    match parse_color(color) {
        Some(c) => {
            p.rect_filled(inner, 4.0, render::c32(c, dark, 1.0));
        }
        None => {
            p.rect_filled(inner, 4.0, ui.visuals().extreme_bg_color);
            p.line_segment(
                [inner.left_bottom(), inner.right_top()],
                Stroke::new(1.5, Color32::from_rgb(224, 49, 49)),
            );
        }
    }
    p.rect_stroke(
        inner,
        4.0,
        Stroke::new(1.0, Color32::from_black_alpha(40)),
        StrokeKind::Inside,
    );
    if selected {
        p.rect_stroke(
            rect.expand(0.5),
            6.0,
            Stroke::new(2.0, SELECTION),
            StrokeKind::Middle,
        );
    }
    resp.on_hover_text(color)
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(title)
            .size(11.5)
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(2.0);
}

fn color_row(ui: &mut Ui, app: &mut App, bg: bool) {
    let dark = app.dark;
    let current = current_value(app, |e| {
        if bg {
            e.background_color.clone()
        } else {
            e.stroke_color.clone()
        }
    })
    .unwrap_or_else(|| {
        if bg {
            app.style.background_color.clone()
        } else {
            app.style.stroke_color.clone()
        }
    });
    let palette: &[&str] = if bg { &BACKGROUNDS } else { &STROKES };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for c in palette {
            if swatch(ui, c, current.eq_ignore_ascii_case(c), dark).clicked() {
                set_color(app, bg, c.to_string());
            }
        }
        ui.add_space(4.0);
        let mut c32 = parse_color(&current)
            .map(|c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
            .unwrap_or(Color32::TRANSPARENT);
        if app.open_picker == Some(bg) {
            app.open_picker = None;
            egui::Popup::open_id(ui.ctx(), ui.auto_id_with("popup"));
        }
        let resp = egui::widgets::color_picker::color_edit_button_srgba(
            ui,
            &mut c32,
            egui::widgets::color_picker::Alpha::OnlyBlend,
        );
        if resp.changed() {
            let [r, g, b, a] = c32.to_srgba_unmultiplied();
            let hex = if a == 0 {
                "transparent".to_string()
            } else {
                crate::model::color_to_hex([r, g, b, a])
            };
            set_color(app, bg, hex);
        }
    });
}

fn set_color(app: &mut App, bg: bool, c: String) {
    if bg {
        app.style.background_color = c.clone();
        app.apply_prop("bg", move |e| {
            if e.kind != "text" {
                e.background_color = c.clone()
            }
        });
    } else {
        app.style.stroke_color = c.clone();
        app.apply_prop("stroke", move |e| e.stroke_color = c.clone());
    }
}

/// Valor da propriedade no primeiro elemento da seleção (ou em edição).
fn current_value<T>(app: &App, f: impl Fn(&Element) -> T) -> Option<T> {
    let editing = app.editing.as_ref().map(|e| e.id.as_str());
    app.elements
        .iter()
        .find(|e| app.selected.contains(&e.id) || Some(e.id.as_str()) == editing)
        .map(f)
}

fn line_icon(
    p: &Painter,
    r: Rect,
    col: Color32,
    width: f32,
    dash: Option<(f32, f32)>,
    wobble: f32,
) {
    let n = 16;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let y = (t * std::f32::consts::TAU * 1.5).sin() * wobble;
            pos2(r.left() + 3.0 + t * (r.width() - 6.0), r.center().y + y)
        })
        .collect();
    match dash {
        Some((on, off)) => {
            p.extend(Shape::dashed_line(&pts, Stroke::new(width, col), on, off));
        }
        None => {
            p.add(Shape::line(pts, Stroke::new(width, col)));
        }
    }
}

/// Ícones do seletor de tipo de seta (afiada, curva, cotovelo).
fn arrow_type_icon(p: &Painter, b: Rect, col: Color32, kind: &str) {
    let s = Stroke::new(1.6, col);
    let (a, z) = (b.left_bottom(), b.right_top());
    let pts = match kind {
        "sharp" => vec![a, pos2(b.left() + b.width() * 0.45, b.top()), z],
        "round" => (0..=12)
            .map(|i| {
                let t = i as f32 / 12.0;
                let c = pos2(b.left() + b.width() * 0.2, b.top());
                let u = 1.0 - t;
                pos2(
                    u * u * a.x + 2.0 * u * t * c.x + t * t * z.x,
                    u * u * a.y + 2.0 * u * t * c.y + t * t * z.y,
                )
            })
            .collect(),
        _ => {
            let mx = b.center().x;
            vec![a, pos2(mx, a.y), pos2(mx, z.y), z]
        }
    };
    p.add(Shape::line(pts.clone(), s));
    // ponta no fim
    let n = pts.len();
    let dir = (pts[n - 1] - pts[n - 2]).normalized();
    let back = pts[n - 1] - dir * 5.0;
    let side = vec2(-dir.y, dir.x) * 3.5;
    p.line_segment([pts[n - 1], back + side], s);
    p.line_segment([pts[n - 1], back - side], s);
}

fn props_panel(app: &mut App, ctx: &egui::Context) {
    let sel_kinds: Vec<String> = app
        .elements
        .iter()
        .filter(|e| {
            app.selected.contains(&e.id) || app.editing.as_ref().is_some_and(|ed| ed.id == e.id)
        })
        .map(|e| e.kind.clone())
        .filter(|k| !matches!(k.as_str(), "frame" | "magicframe"))
        .collect();
    let tool_kind = app
        .tool
        .kind()
        .filter(|k| *k != "frame")
        .map(|k| k.to_string());
    let kinds: Vec<String> = if sel_kinds.is_empty() {
        tool_kind.into_iter().collect()
    } else {
        sel_kinds.clone()
    };
    if kinds.is_empty() {
        return;
    }
    let has = |ks: &[&str]| kinds.iter().any(|k| ks.contains(&k.as_str()));
    let only_text = kinds.iter().all(|k| k == "text");
    // imagens só têm opacidade e camadas, como no Excalidraw
    let only_image = kinds.iter().all(|k| k == "image");
    let has_selection = !sel_kinds.is_empty();

    egui::Area::new(Id::unique("props"))
        .fixed_pos(pos2(12.0, 64.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            panel_frame(ui).show(ui, |ui| {
                ui.set_width(196.0);
                ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
                if !only_image {
                    section(ui, "Traço");
                    color_row(ui, app, false);
                }
                if has(&["rectangle", "diamond", "ellipse", "line", "freedraw"]) {
                    section(ui, "Fundo");
                    color_row(ui, app, true);
                    let bg = current_value(app, |e| e.background_color.clone())
                        .unwrap_or_else(|| app.style.background_color.clone());
                    if parse_color(&bg).is_some() && !kinds.iter().all(|k| k == "freedraw") {
                        section(ui, "Preenchimento");
                        let cur = current_value(app, |e| e.fill_style.clone())
                            .unwrap_or_else(|| app.style.fill_style.clone());
                        ui.horizontal(|ui| {
                            for (val, tip) in [
                                ("hachure", "Hachura"),
                                ("cross-hatch", "Hachura cruzada"),
                                ("solid", "Sólido"),
                            ] {
                                let r = icon_button(ui, cur == val, 30.0, tip, |p, r, col| {
                                    let b = r.shrink(7.0);
                                    p.rect_stroke(
                                        b,
                                        2.0,
                                        Stroke::new(1.2, col),
                                        StrokeKind::Middle,
                                    );
                                    match val {
                                        "solid" => {
                                            p.rect_filled(b, 2.0, col);
                                        }
                                        _ => {
                                            for i in 0..4 {
                                                let t = i as f32 / 3.0;
                                                let a = pos2(b.left() + t * b.width(), b.bottom());
                                                let c = pos2(b.left(), b.bottom() - t * b.height());
                                                p.line_segment([a, c], Stroke::new(1.0, col));
                                                let a2 = pos2(b.right(), b.top() + t * b.height());
                                                let c2 = pos2(b.left() + t * b.width(), b.top());
                                                p.line_segment([a2, c2], Stroke::new(1.0, col));
                                                if val == "cross-hatch" {
                                                    p.line_segment(
                                                        [
                                                            pos2(b.left() + t * b.width(), b.top()),
                                                            pos2(
                                                                b.right(),
                                                                b.bottom() - (1.0 - t) * b.height(),
                                                            ),
                                                        ],
                                                        Stroke::new(1.0, col),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                });
                                if r.clicked() {
                                    app.style.fill_style = val.into();
                                    app.apply_prop("fill", move |e| e.fill_style = val.into());
                                }
                            }
                        });
                    }
                }
                if !only_text && !only_image {
                    section(ui, "Espessura do traço");
                    let cur =
                        current_value(app, |e| e.stroke_width).unwrap_or(app.style.stroke_width);
                    ui.horizontal(|ui| {
                        for (val, tip) in [(1.0, "Fino"), (2.0, "Negrito"), (4.0, "Extra negrito")]
                        {
                            if icon_button(ui, cur == val, 30.0, tip, |p, r, col| {
                                line_icon(p, r.shrink(4.0), col, val as f32 * 0.9 + 0.4, None, 0.0)
                            })
                            .clicked()
                            {
                                app.style.stroke_width = val;
                                app.apply_prop("sw", move |e| e.stroke_width = val);
                            }
                        }
                    });
                }
                if has(&["rectangle", "diamond", "ellipse", "line", "arrow"]) {
                    section(ui, "Estilo do traço");
                    let cur = current_value(app, |e| e.stroke_style.clone())
                        .unwrap_or_else(|| app.style.stroke_style.clone());
                    ui.horizontal(|ui| {
                        for (val, tip, dash) in [
                            ("solid", "Sólido", None),
                            ("dashed", "Tracejado", Some((4.0, 3.0))),
                            ("dotted", "Pontilhado", Some((1.5, 3.0))),
                        ] {
                            if icon_button(ui, cur == val, 30.0, tip, |p, r, col| {
                                line_icon(p, r.shrink(4.0), col, 1.6, dash, 0.0)
                            })
                            .clicked()
                            {
                                app.style.stroke_style = val.into();
                                app.apply_prop("ss", move |e| e.stroke_style = val.into());
                            }
                        }
                    });
                    section(ui, "Desleixo");
                    let cur = current_value(app, |e| e.roughness).unwrap_or(app.style.roughness);
                    ui.horizontal(|ui| {
                        for (val, tip, wob) in [
                            (0.0, "Arquiteto", 0.0),
                            (1.0, "Artista", 2.0),
                            (2.0, "Cartunista", 4.0),
                        ] {
                            if icon_button(ui, cur == val, 30.0, tip, |p, r, col| {
                                line_icon(p, r.shrink(4.0), col, 1.6, None, wob)
                            })
                            .clicked()
                            {
                                app.style.roughness = val;
                                app.apply_prop("rough", move |e| e.roughness = val);
                            }
                        }
                    });
                }
                if has(&["rectangle", "diamond", "line"]) {
                    section(ui, "Bordas");
                    let cur = current_value(app, |e| e.is_round()).unwrap_or(app.style.round);
                    ui.horizontal(|ui| {
                        for (val, tip) in [(false, "Pontiagudas"), (true, "Arredondadas")] {
                            if icon_button(ui, cur == val, 30.0, tip, |p, r, col| {
                                let b = r.shrink(8.0);
                                let s = Stroke::new(1.6, col);
                                if val {
                                    p.add(Shape::line(
                                        vec![
                                            b.left_bottom(),
                                            pos2(b.left(), b.top() + 6.0),
                                            pos2(b.left() + 2.0, b.top() + 2.0),
                                            pos2(b.left() + 6.0, b.top()),
                                            b.right_top(),
                                        ],
                                        s,
                                    ));
                                } else {
                                    p.add(Shape::line(
                                        vec![b.left_bottom(), b.left_top(), b.right_top()],
                                        s,
                                    ));
                                }
                            })
                            .clicked()
                            {
                                app.style.round = val;
                                app.apply_prop("round", move |e| {
                                    // setas usam o tipo de seta
                                    if e.kind == "arrow" {
                                        return;
                                    }
                                    e.roundness = if !val {
                                        None
                                    } else if e.kind == "rectangle" {
                                        Some(json!({"type": 3}))
                                    } else {
                                        Some(json!({"type": 2}))
                                    }
                                });
                            }
                        }
                    });
                }
                if has(&["arrow"]) {
                    section(ui, "Tipo de seta");
                    let cur = app
                        .elements
                        .iter()
                        .find(|e| e.kind == "arrow" && app.selected.contains(&e.id))
                        .map(|e| e.arrow_type().to_string())
                        .unwrap_or_else(|| app.style.arrow_type.clone());
                    ui.horizontal(|ui| {
                        for (val, tip) in [
                            ("sharp", "Afiada"),
                            ("round", "Curva"),
                            ("elbow", "Cotovelo"),
                        ] {
                            if icon_button(ui, cur == val, 30.0, tip, |p, r, col| {
                                arrow_type_icon(p, r.shrink(7.0), col, val)
                            })
                            .clicked()
                            {
                                app.set_arrow_type(val);
                            }
                        }
                    });
                    section(ui, "Pontas de seta");
                    for end in [false, true] {
                        let cur = current_value(app, |e| e.arrowhead(end)).unwrap_or_else(|| {
                            if end {
                                app.style.end_arrowhead.clone()
                            } else {
                                app.style.start_arrowhead.clone()
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(if end { "Fim" } else { "Início" }).size(11.0));
                            for val in [
                                None,
                                Some("arrow"),
                                Some("triangle"),
                                Some("bar"),
                                Some("dot"),
                            ] {
                                let tip = match val {
                                    None => "Nenhuma",
                                    Some("arrow") => "Seta",
                                    Some("triangle") => "Triângulo",
                                    Some("bar") => "Barra",
                                    _ => "Ponto",
                                };
                                let sel = cur.as_deref() == val;
                                if icon_button(ui, sel, 26.0, tip, |p, r, col| {
                                    let b = r.shrink(6.0);
                                    let s = Stroke::new(1.4, col);
                                    let (a, tipp) = if end {
                                        (b.left_center(), b.right_center())
                                    } else {
                                        (b.right_center(), b.left_center())
                                    };
                                    p.line_segment([a, tipp], s);
                                    let d = if end { -1.0 } else { 1.0 };
                                    match val {
                                        Some("arrow") => {
                                            p.line_segment([tipp, tipp + vec2(5.0 * d, -4.0)], s);
                                            p.line_segment([tipp, tipp + vec2(5.0 * d, 4.0)], s);
                                        }
                                        Some("triangle") => {
                                            p.add(Shape::convex_polygon(
                                                vec![
                                                    tipp,
                                                    tipp + vec2(6.0 * d, -4.0),
                                                    tipp + vec2(6.0 * d, 4.0),
                                                ],
                                                col,
                                                Stroke::NONE,
                                            ));
                                        }
                                        Some("bar") => {
                                            p.line_segment(
                                                [tipp + vec2(0.0, -5.0), tipp + vec2(0.0, 5.0)],
                                                s,
                                            );
                                        }
                                        Some(_) => {
                                            p.circle_filled(tipp + vec2(2.0 * d, 0.0), 3.0, col);
                                        }
                                        None => {}
                                    }
                                })
                                .clicked()
                                {
                                    let v = val.map(|s| s.to_string());
                                    if end {
                                        app.style.end_arrowhead = v.clone();
                                    } else {
                                        app.style.start_arrowhead = v.clone();
                                    }
                                    let key = if end {
                                        "endArrowhead"
                                    } else {
                                        "startArrowhead"
                                    };
                                    app.apply_prop("head", move |e| {
                                        if e.kind == "arrow" {
                                            e.set_field(
                                                key,
                                                v.clone()
                                                    .map(serde_json::Value::String)
                                                    .unwrap_or(serde_json::Value::Null),
                                            );
                                        }
                                    });
                                }
                            }
                        });
                    }
                }
                if has(&["text"]) {
                    section(ui, "Tamanho da fonte");
                    let cur = current_value(app, |e| e.font_size()).unwrap_or(app.style.font_size);
                    ui.horizontal(|ui| {
                        for (val, label, tip) in [
                            (16.0, "P", "Pequena"),
                            (20.0, "M", "Média"),
                            (28.0, "G", "Grande"),
                            (36.0, "GG", "Muito grande"),
                        ] {
                            if icon_button(ui, cur == val, 30.0, tip, |p, r, col| {
                                p.text(
                                    r.center(),
                                    Align2::CENTER_CENTER,
                                    label,
                                    FontId::proportional(13.0),
                                    col,
                                );
                            })
                            .clicked()
                            {
                                app.style.font_size = val;
                                app.apply_prop("font", move |e| {
                                    if e.kind == "text" {
                                        e.set_field("fontSize", json!(val))
                                    }
                                });
                            }
                        }
                    });
                    section(ui, "Família da fonte");
                    let cur =
                        current_value(app, |e| e.font_family()).unwrap_or(app.style.font_family);
                    let hand_ok = app.hand_ok;
                    ui.horizontal(|ui| {
                        for (val, tip) in [(1i64, "À mão"), (2, "Normal"), (3, "Código")] {
                            let sel = crate::fonts::Family::from_excalidraw(cur)
                                == crate::fonts::Family::from_excalidraw(val);
                            if icon_button(ui, sel, 30.0, tip, |p, r, col| {
                                p.text(
                                    r.center(),
                                    Align2::CENTER_CENTER,
                                    "Aa",
                                    FontId::new(14.0, render::font_family(val, hand_ok)),
                                    col,
                                );
                            })
                            .clicked()
                            {
                                app.style.font_family = val;
                                app.apply_prop("font", move |e| {
                                    if e.kind == "text" {
                                        e.set_field("fontFamily", json!(val))
                                    }
                                });
                            }
                        }
                    });
                    section(ui, "Alinhamento");
                    let cur = current_value(app, |e| e.text_align().to_string())
                        .unwrap_or_else(|| "left".into());
                    ui.horizontal(|ui| {
                        for (val, tip) in [
                            ("left", "Esquerda"),
                            ("center", "Centro"),
                            ("right", "Direita"),
                        ] {
                            if icon_button(ui, cur == val, 30.0, tip, |p, r, col| {
                                let b = r.shrink(8.0);
                                for (i, w) in [1.0f32, 0.6, 0.85].iter().enumerate() {
                                    let y = b.top() + i as f32 * b.height() / 2.0;
                                    let lw = b.width() * w;
                                    let x0 = match val {
                                        "center" => b.center().x - lw / 2.0,
                                        "right" => b.right() - lw,
                                        _ => b.left(),
                                    };
                                    p.line_segment(
                                        [pos2(x0, y), pos2(x0 + lw, y)],
                                        Stroke::new(1.6, col),
                                    );
                                }
                            })
                            .clicked()
                            {
                                app.apply_prop("align", move |e| {
                                    if e.kind == "text" {
                                        e.set_field("textAlign", json!(val))
                                    }
                                });
                            }
                        }
                    });
                }
                section(ui, "Opacidade");
                let mut op = current_value(app, |e| e.opacity).unwrap_or(app.style.opacity);
                if ui
                    .add(
                        egui::Slider::new(&mut op, 0.0..=100.0)
                            .step_by(10.0)
                            .show_value(true),
                    )
                    .changed()
                {
                    app.style.opacity = op;
                    app.apply_prop("opacity", move |e| e.opacity = op);
                }
                if has_selection {
                    section(ui, "Camadas");
                    ui.horizontal(|ui| {
                        let (to_back, to_front) = if cfg!(target_os = "macos") {
                            (
                                "Enviar para trás (Ctrl+Alt+[)",
                                "Trazer para frente (Ctrl+Alt+])",
                            )
                        } else {
                            (
                                "Enviar para trás (Ctrl+Shift+[)",
                                "Trazer para frente (Ctrl+Shift+])",
                            )
                        };
                        for (front, all, tip) in [
                            (false, true, to_back),
                            (false, false, "Recuar (Ctrl+[)"),
                            (true, false, "Avançar (Ctrl+])"),
                            (true, true, to_front),
                        ] {
                            if icon_button(ui, false, 30.0, tip, |p, r, col| {
                                let b = r.shrink(8.0);
                                let s = Stroke::new(1.6, col);
                                let (from, to) = if front {
                                    (b.bottom(), b.top() + 2.0)
                                } else {
                                    (b.top(), b.bottom() - 2.0)
                                };
                                let x = b.center().x;
                                p.line_segment([pos2(x, from), pos2(x, to)], s);
                                let d = if front { 4.0 } else { -4.0 };
                                p.line_segment([pos2(x, to), pos2(x - 4.0, to + d)], s);
                                p.line_segment([pos2(x, to), pos2(x + 4.0, to + d)], s);
                                if all {
                                    let y = if front {
                                        b.top() - 1.0
                                    } else {
                                        b.bottom() + 1.0
                                    };
                                    p.line_segment([pos2(b.left(), y), pos2(b.right(), y)], s);
                                }
                            })
                            .clicked()
                            {
                                app.reorder(front, all);
                            }
                        }
                    });
                    section(ui, "Ações");
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button("Duplicar")
                            .on_hover_text(keys("Ctrl+D"))
                            .clicked()
                        {
                            app.duplicate_selected();
                        }
                        if ui.small_button("Excluir").on_hover_text("Delete").clicked() {
                            app.delete_selected();
                        }
                        if app.selected.len() > 1
                            && ui
                                .small_button("Agrupar")
                                .on_hover_text(keys("Ctrl+G"))
                                .clicked()
                        {
                            app.group_selected(true);
                        }
                        let grouped = app
                            .selected_elements()
                            .iter()
                            .any(|e| !e.group_ids.is_empty());
                        if grouped
                            && ui
                                .small_button("Desagrupar")
                                .on_hover_text(keys("Ctrl+Shift+G"))
                                .clicked()
                        {
                            app.group_selected(false);
                        }
                    });
                }
            });
        });
}

fn toolbar(app: &mut App, ctx: &egui::Context, view: Rect) {
    egui::Area::new(Id::unique("toolbar"))
        .fixed_pos(pos2(view.center().x - 246.0, 12.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            panel_frame(ui).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let locked = app.tool_locked;
                    if icon_button(
                        ui,
                        locked,
                        34.0,
                        "Manter ferramenta ativa após desenhar (Q)",
                        |p, r, c| paint_icon(p, r.shrink(10.0), Icon::Lock(locked), c),
                    )
                    .clicked()
                    {
                        app.tool_locked = !app.tool_locked;
                    }
                    ui.separator();
                    let tools = [
                        (Tool::Hand, Icon::Hand, "Mão — mover a tela (H)", ""),
                        (Tool::Select, Icon::Select, "Seleção (V ou 1)", "1"),
                        (Tool::Rect, Icon::Rect, "Retângulo (R ou 2)", "2"),
                        (Tool::Diamond, Icon::Diamond, "Losango (D ou 3)", "3"),
                        (Tool::Ellipse, Icon::Ellipse, "Elipse (O ou 4)", "4"),
                        (Tool::Arrow, Icon::Arrow, "Seta (A ou 5)", "5"),
                        (Tool::Line, Icon::Line, "Linha (L ou 6)", "6"),
                        (Tool::Draw, Icon::Draw, "Desenho livre (P ou 7)", "7"),
                        (Tool::Text, Icon::Text, "Texto (T ou 8)", "8"),
                        // imagem não é um modo: abre o seletor de arquivo
                        (Tool::Select, Icon::Image, "Inserir imagem (9)", "9"),
                        (Tool::Eraser, Icon::Eraser, "Borracha (E ou 0)", "0"),
                        (Tool::Frame, Icon::Frame, "Frame (F)", "F"),
                    ];
                    for (tool, icon, tip, key) in tools {
                        let is_image = icon == Icon::Image;
                        let on = app.tool == tool && !is_image;
                        let resp = icon_button(ui, on, 34.0, tip, |p, r, c| {
                            paint_icon(p, r.shrink(10.0), icon, c);
                            if !key.is_empty() {
                                p.text(
                                    r.right_bottom() - vec2(3.0, 2.0),
                                    Align2::RIGHT_BOTTOM,
                                    key,
                                    FontId::proportional(8.5),
                                    c.gamma_multiply(0.55),
                                );
                            }
                        });
                        if resp.clicked() && is_image {
                            app.image_dialog();
                        } else if resp.clicked() {
                            app.set_tool(tool);
                        }
                    }
                });
            });
        });
}

fn menu(app: &mut App, ctx: &egui::Context) {
    egui::Area::new(Id::unique("menu_btn"))
        .fixed_pos(pos2(12.0, 12.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            panel_frame(ui)
                .inner_margin(Margin::same(3))
                .show(ui, |ui| {
                    let open = app.show_menu;
                    if icon_button(ui, open, 34.0, "Menu", |p, r, c| {
                        paint_icon(p, r.shrink(11.0), Icon::Menu, c)
                    })
                    .clicked()
                    {
                        app.show_menu = !app.show_menu;
                    }
                });
        });
    if !app.show_menu {
        return;
    }
    let resp = egui::Area::new(Id::unique("menu"))
        .fixed_pos(pos2(12.0, 60.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            panel_frame(ui).show(ui, |ui| {
                ui.set_width(230.0);
                let mut close = false;
                let mut item = |ui: &mut Ui, label: &str, shortcut: &str| {
                    let r = ui.add(
                        egui::Button::new(label)
                            .shortcut_text(keys(shortcut))
                            .frame(false)
                            .min_size(vec2(224.0, 26.0)),
                    );
                    if r.clicked() {
                        close = true;
                    }
                    r.clicked()
                };
                if item(ui, "Abrir…", "Ctrl+O") {
                    app.open_dialog();
                }
                if item(ui, "Salvar", "Ctrl+S") {
                    app.save();
                }
                if item(ui, "Salvar como…", "Ctrl+Shift+S") {
                    app.save_as();
                }
                if item(ui, "Exportar imagem…", "Ctrl+Shift+E") {
                    app.show_export = true;
                }
                ui.separator();
                if item(ui, "Novo desenho", "") {
                    app.new_scene();
                }
                if item(ui, "Limpar o canvas", "Ctrl+Delete") && !app.elements.is_empty() {
                    app.confirm = Some(Pending::Clear);
                }
                if item(ui, "Atalhos e ajuda", "?") {
                    app.show_help = true;
                }
                ui.separator();
                ui.checkbox(&mut app.dark, keys("Tema escuro  (Alt+Shift+D)"));
                let mut grid = app.grid;
                if ui.checkbox(&mut grid, keys("Grade  (Ctrl+')")).changed() {
                    app.toggle_grid();
                }
                let mut snap = app.snap;
                if ui
                    .checkbox(&mut snap, keys("Encaixar em objetos  (Alt+S)"))
                    .changed()
                {
                    app.toggle_snap();
                }
                ui.checkbox(&mut app.zen_mode, keys("Modo zen  (Alt+Z)"));
                let mut view_mode = app.view_mode;
                if ui
                    .checkbox(&mut view_mode, keys("Modo de visualização  (Alt+R)"))
                    .changed()
                {
                    app.toggle_view_mode();
                }
                ui.checkbox(&mut app.show_stats, keys("Estatísticas  (Alt+/)"));
                let mut live = app.live.is_some();
                if ui
                    .checkbox(&mut live, "Permitir edição por IA (MCP)")
                    .on_hover_text(
                        "Deixa um assistente de IA ligado ao `rustdraw mcp` editar este \
                         desenho enquanto o editor está aberto. Só aceita conexões deste \
                         computador.",
                    )
                    .changed()
                {
                    app.set_live(live);
                }
                section(ui, "Fundo do canvas");
                ui.horizontal(|ui| {
                    for c in CANVAS_BGS {
                        if swatch(ui, c, app.view_bg.eq_ignore_ascii_case(c), app.dark).clicked() {
                            app.view_bg = c.to_string();
                        }
                    }
                });
                if close {
                    app.show_menu = false;
                }
            });
        })
        .response;
    let pressed_outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx.pointer_interact_pos().is_some_and(|p| {
            !resp.rect.contains(p)
                && !Rect::from_min_size(pos2(12.0, 12.0), vec2(44.0, 44.0)).contains(p)
        });
    if pressed_outside {
        app.show_menu = false;
    }
}

/// Ações do menu de contexto.
#[derive(Clone, Copy)]
enum Act {
    Cut,
    Copy,
    Paste,
    CopyPng,
    CopySvg,
    CopyText,
    SelectAll,
    UnlockAll,
    AddToLibrary,
    Grid,
    Snap,
    Zen,
    ViewMode,
    Stats,
    CopyStyles,
    PasteStyles,
    Group,
    Ungroup,
    SendBackward,
    BringForward,
    SendToBack,
    BringToFront,
    FlipH,
    FlipV,
    Duplicate,
    Lock,
    Delete,
    ExportFrame,
    RenameFrame,
}

enum Entry {
    Sep,
    /// Rótulo, atalho, ação e, para as opções liga/desliga, se está ligada.
    Item(&'static str, &'static str, Act, Option<bool>),
}

/// Itens do menu do botão direito, na mesma ordem do Excalidraw
/// (`App.tsx`, `getContextMenuItems`). Ficam de fora os itens de recursos que o
/// RustDraw ainda não tem (links, recorte de imagem...).
fn context_entries(app: &App, on_element: bool) -> Vec<Entry> {
    use Entry::{Item, Sep};
    let has_elements = !app.elements.is_empty();
    let has_text = !app.selected_text().is_empty();
    let toggles = [
        Item("Grade", "Ctrl+'", Act::Grid, Some(app.grid)),
        Item("Encaixar em objetos", "Alt+S", Act::Snap, Some(app.snap)),
        Item("Modo zen", "Alt+Z", Act::Zen, Some(app.zen_mode)),
        Item(
            "Modo de visualização",
            "Alt+R",
            Act::ViewMode,
            Some(app.view_mode),
        ),
        Item("Estatísticas", "Alt+/", Act::Stats, Some(app.show_stats)),
    ];
    let mut v = vec![];
    if app.view_mode {
        v.extend(toggles);
    } else if !on_element {
        v.push(Item("Colar", "Ctrl+V", Act::Paste, None));
        v.push(Sep);
        if has_elements {
            v.push(Item("Copiar como PNG", "Shift+Alt+C", Act::CopyPng, None));
            v.push(Item("Copiar como SVG", "", Act::CopySvg, None));
        }
        if has_text {
            v.push(Item("Copiar texto", "", Act::CopyText, None));
        }
        v.push(Sep);
        v.push(Item("Selecionar tudo", "Ctrl+A", Act::SelectAll, None));
        if app.has_locked() {
            v.push(Item("Destravar tudo", "", Act::UnlockAll, None));
        }
        v.push(Sep);
        v.extend(toggles);
    } else {
        // no macOS, "até o fim" é Cmd+Alt+[ ]; nos demais, Ctrl+Shift+[ ]
        let (to_back, to_front) = if cfg!(target_os = "macos") {
            ("Ctrl+Alt+[", "Ctrl+Alt+]")
        } else {
            ("Ctrl+Shift+[", "Ctrl+Shift+]")
        };
        v.extend([
            Item("Recortar", "Ctrl+X", Act::Cut, None),
            Item("Copiar", "Ctrl+C", Act::Copy, None),
            Item("Colar", "Ctrl+V", Act::Paste, None),
            Sep,
            Item("Copiar como PNG", "Shift+Alt+C", Act::CopyPng, None),
            Item("Copiar como SVG", "", Act::CopySvg, None),
        ]);
        if has_text {
            v.push(Item("Copiar texto", "", Act::CopyText, None));
        }
        v.extend([
            Sep,
            Item("Copiar estilos", "Ctrl+Alt+C", Act::CopyStyles, None),
            Item("Colar estilos", "Ctrl+Alt+V", Act::PasteStyles, None),
            Sep,
        ]);
        if app.can_group() {
            v.push(Item("Agrupar seleção", "Ctrl+G", Act::Group, None));
        }
        if app.can_ungroup() {
            v.push(Item("Desagrupar", "Ctrl+Shift+G", Act::Ungroup, None));
        }
        if app.selected_frame().is_some() {
            v.extend([
                Sep,
                Item("Renomear frame", "Enter", Act::RenameFrame, None),
                Item("Exportar frame…", "", Act::ExportFrame, None),
            ]);
        }
        let lock = if app.selection_locked() {
            "Destravar"
        } else {
            "Travar"
        };
        v.extend([
            Sep,
            Item("Adicionar à biblioteca", "", Act::AddToLibrary, None),
            Item("Enviar para trás", "Ctrl+[", Act::SendBackward, None),
            Item("Trazer para frente", "Ctrl+]", Act::BringForward, None),
            Item("Enviar para o fundo", to_back, Act::SendToBack, None),
            Item("Trazer ao topo", to_front, Act::BringToFront, None),
            Sep,
            Item("Inverter na horizontal", "Shift+H", Act::FlipH, None),
            Item("Inverter na vertical", "Shift+V", Act::FlipV, None),
            Sep,
            Item("Duplicar", "Ctrl+D", Act::Duplicate, None),
            Item(lock, "Ctrl+Shift+L", Act::Lock, None),
            Sep,
            Item("Apagar", "Delete", Act::Delete, None),
        ]);
    }
    // sem separadores no começo, no fim ou repetidos
    let mut out: Vec<Entry> = vec![];
    for e in v {
        if matches!(e, Sep) && matches!(out.last(), None | Some(Sep)) {
            continue;
        }
        out.push(e);
    }
    if matches!(out.last(), Some(Sep)) {
        out.pop();
    }
    out
}

fn run_act(app: &mut App, ctx: &egui::Context, act: Act, at: Pos2) {
    match act {
        Act::Cut => app.cut_selection(ctx),
        Act::Copy => app.copy_selection(ctx),
        Act::Paste => {
            let p = app.cam.w(at);
            app.paste_at(ctx, p);
        }
        Act::CopyPng => app.copy_png(ctx),
        Act::CopySvg => app.copy_selection_svg(ctx),
        Act::CopyText => app.copy_text(ctx),
        Act::SelectAll => app.select_all(),
        Act::UnlockAll => app.unlock_all(),
        Act::AddToLibrary => app.add_selection_to_library(),
        Act::Grid => app.toggle_grid(),
        Act::Snap => app.toggle_snap(),
        Act::Zen => app.zen_mode = !app.zen_mode,
        Act::ViewMode => app.toggle_view_mode(),
        Act::Stats => app.show_stats = !app.show_stats,
        Act::CopyStyles => app.copy_styles(),
        Act::PasteStyles => app.paste_styles(),
        Act::Group => app.group_selected(true),
        Act::Ungroup => app.group_selected(false),
        Act::SendBackward => app.reorder(false, false),
        Act::BringForward => app.reorder(true, false),
        Act::SendToBack => app.reorder(false, true),
        Act::BringToFront => app.reorder(true, true),
        Act::FlipH => app.flip_selected(true),
        Act::FlipV => app.flip_selected(false),
        Act::Duplicate => app.duplicate_selected(),
        Act::Lock => app.toggle_lock(),
        Act::Delete => app.delete_selected(),
        Act::ExportFrame => app.export_selected_frame(),
        Act::RenameFrame => {
            if let Some(id) = app.selected_frame().map(|f| f.id.clone()) {
                app.rename_frame(&id);
            }
        }
    }
}

/// Menu do botão direito: um para o canvas vazio e outro para elementos.
fn context_menu(app: &mut App, ctx: &egui::Context) {
    let Some(cm) = &app.context_menu else { return };
    let (at, fresh) = (cm.pos, cm.fresh);
    let entries = context_entries(app, cm.on_element);
    let mut chosen = None;
    let resp = egui::Area::new(Id::unique("context_menu"))
        .fixed_pos(at)
        .order(egui::Order::Foreground)
        .constrain(true)
        .show(ctx, |ui| {
            panel_frame(ui).show(ui, |ui| {
                ui.set_width(250.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                let danger = ui.visuals().error_fg_color;
                for e in &entries {
                    match *e {
                        Entry::Sep => {
                            ui.add_space(3.0);
                            ui.separator();
                            ui.add_space(3.0);
                        }
                        Entry::Item(label, shortcut, act, checked) => {
                            let text = match checked {
                                Some(true) => RichText::new(format!("✔ {label}")),
                                _ => RichText::new(label),
                            };
                            let text = if matches!(act, Act::Delete) {
                                text.color(danger)
                            } else {
                                text
                            };
                            let r = ui.add(
                                egui::Button::new(text)
                                    .shortcut_text(keys(shortcut))
                                    .frame(false)
                                    .min_size(vec2(250.0, 26.0)),
                            );
                            if r.clicked() {
                                chosen = Some(act);
                            }
                        }
                    }
                }
            });
        })
        .response;
    if let Some(cm) = &mut app.context_menu {
        cm.fresh = false;
    }
    if let Some(act) = chosen {
        app.context_menu = None;
        run_act(app, ctx, act, at);
        return;
    }
    // clique em outro painel fecha o menu (no canvas, `handle_pointer` cuida disso)
    let pressed_outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .pointer_interact_pos()
            .is_some_and(|p| !resp.rect.contains(p));
    if pressed_outside && !fresh {
        app.context_menu = None;
    }
}

fn bottom_bar(app: &mut App, ctx: &egui::Context, view: Rect) {
    egui::Area::new(Id::unique("zoom"))
        .anchor(Align2::LEFT_BOTTOM, vec2(12.0, -12.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                panel_frame(ui)
                    .inner_margin(Margin::same(3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            if icon_button(ui, false, 30.0, "Diminuir zoom (Ctrl+-)", |p, r, c| {
                                p.line_segment(
                                    [r.center() - vec2(5.0, 0.0), r.center() + vec2(5.0, 0.0)],
                                    Stroke::new(1.6, c),
                                );
                            })
                            .clicked()
                            {
                                app.zoom_step(view, -1.0);
                            }
                            let pct = format!("{:.0}%", app.cam.zoom * 100.0);
                            if ui
                                .add(
                                    egui::Button::new(pct)
                                        .frame(false)
                                        .min_size(vec2(52.0, 30.0)),
                                )
                                .on_hover_text(keys(
                                    "Redefinir zoom (Ctrl+0) · Ajustar à tela: Shift+1",
                                ))
                                .clicked()
                            {
                                app.reset_zoom(view);
                            }
                            if icon_button(ui, false, 30.0, "Aumentar zoom (Ctrl++)", |p, r, c| {
                                p.line_segment(
                                    [r.center() - vec2(5.0, 0.0), r.center() + vec2(5.0, 0.0)],
                                    Stroke::new(1.6, c),
                                );
                                p.line_segment(
                                    [r.center() - vec2(0.0, 5.0), r.center() + vec2(0.0, 5.0)],
                                    Stroke::new(1.6, c),
                                );
                            })
                            .clicked()
                            {
                                app.zoom_step(view, 1.0);
                            }
                        });
                    });
                panel_frame(ui)
                    .inner_margin(Margin::same(3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_enabled_ui(app.can_undo(), |ui| {
                                if icon_button(ui, false, 30.0, "Desfazer (Ctrl+Z)", |p, r, c| {
                                    paint_icon(p, r.shrink(9.0), Icon::Undo, c)
                                })
                                .clicked()
                                {
                                    app.undo();
                                }
                            });
                            ui.add_enabled_ui(app.can_redo(), |ui| {
                                if icon_button(
                                    ui,
                                    false,
                                    30.0,
                                    "Refazer (Ctrl+Shift+Z)",
                                    |p, r, c| paint_icon(p, r.shrink(9.0), Icon::Redo, c),
                                )
                                .clicked()
                                {
                                    app.redo();
                                }
                            });
                        });
                    });
            });
        });
    egui::Area::new(Id::unique("help_btn"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -12.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            panel_frame(ui)
                .inner_margin(Margin::same(3))
                .show(ui, |ui| {
                    if icon_button(ui, app.show_help, 30.0, "Atalhos (?)", |p, r, c| {
                        p.text(
                            r.center(),
                            Align2::CENTER_CENTER,
                            "?",
                            FontId::proportional(16.0),
                            c,
                        );
                    })
                    .clicked()
                    {
                        app.show_help = !app.show_help;
                    }
                });
        });
}

fn text_editor(app: &mut App, ctx: &egui::Context) {
    let Some(ed) = &app.editing else { return };
    let Some(el) = app.elements.iter().find(|e| e.id == ed.id) else {
        app.editing = None;
        return;
    };
    let z = app.cam.zoom;
    let pos = app.cam.s(crate::geom::P::new(el.x as f32, el.y as f32));
    let size = el.font_size() as f32 * z;
    let font = FontId::new(size, render::font_family(el.font_family(), app.hand_ok));
    let color = parse_color(&el.stroke_color)
        .map(|c| render::c32(c, app.dark, 1.0))
        .unwrap_or(Color32::BLACK);
    let focus = ed.focus;
    egui::Area::new(Id::unique("text_editor"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let ed = app.editing.as_mut().unwrap();
            // sem quebra automática: a largura do texto é definida pelas linhas
            let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
                let job = egui::text::LayoutJob::simple(
                    buf.as_str().to_owned(),
                    font.clone(),
                    color,
                    f32::INFINITY,
                );
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let te = egui::TextEdit::multiline(&mut ed.buf)
                .id(Id::unique("text_editor_field"))
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(0.0)
                .desired_rows(1)
                .layouter(&mut layouter)
                .background_color(Color32::TRANSPARENT);
            let r = ui.add(te);
            // o clique que cria o texto também "solta" o foco no canvas;
            // mantemos o foco enquanto o texto estiver em edição
            if focus || !r.has_focus() {
                r.request_focus();
                ed.focus = false;
            }
        });
}

/// Campo para renomear um frame, no lugar do nome dele.
fn frame_name_editor(app: &mut App, ctx: &egui::Context) {
    let Some(r) = &app.renaming else { return };
    let Some(f) = app.elements.iter().find(|e| e.id == r.id) else {
        app.renaming = None;
        return;
    };
    let rect = crate::frame::rect(f);
    let top_left = app.cam.s(rect.min);
    let width = (rect.width() * app.cam.zoom).max(120.0);
    let font = FontId::proportional(crate::frame::NAME_SIZE);
    let pos = top_left - vec2(0.0, crate::frame::NAME_GAP + font.size * 1.25 + 4.0);
    let mut done = false;
    egui::Area::new(Id::unique("frame_name_editor"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let r = app.renaming.as_mut().unwrap();
            let resp = ui.add(
                egui::TextEdit::singleline(&mut r.buf)
                    .id(Id::unique("frame_name_field"))
                    .font(font)
                    .desired_width(width),
            );
            if r.focus {
                resp.request_focus();
                r.focus = false;
            } else if resp.lost_focus() {
                done = true;
            }
        });
    if done {
        app.finish_rename();
    }
}

fn export_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_export {
        return;
    }
    // pré-visualização
    let (els, opts) = app.export_job();
    let key = format!(
        "{}|{}|{}|{}|{:?}|{}|{}",
        app.export_opts.background,
        app.export_opts.dark,
        app.export_selection_only,
        app.view_bg,
        opts.frame,
        els.len(),
        els.iter()
            .map(|e| e.version_nonce)
            .fold(0i64, |a, b| a.wrapping_add(b))
    );
    if app
        .export_preview
        .as_ref()
        .map(|(k, _)| k != &key)
        .unwrap_or(true)
        && !els.is_empty()
    {
        let mut o = opts.clone();
        if let Some(b) = crate::export::export_bounds(&els, &o) {
            o.scale = (340.0 / (b.width() + 20.0))
                .min(200.0 / (b.height() + 20.0))
                .min(2.0);
        }
        if let Ok(pm) = crate::export::render_pixmap(&els, &o) {
            let img = egui::ColorImage::from_rgba_premultiplied(
                [pm.width() as usize, pm.height() as usize],
                pm.data(),
            );
            let tex = ctx.load_texture("export_preview", img, egui::TextureOptions::LINEAR);
            app.export_preview = Some((key, tex));
        }
    }
    let mut open = true;
    egui::Window::new("Exportar imagem")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(360.0);
            let (rect, _) = ui.allocate_exact_size(vec2(360.0, 210.0), Sense::hover());
            // fundo xadrez para mostrar transparência
            let p = ui.painter();
            let cell = 10.0;
            for iy in 0..(rect.height() / cell) as i32 + 1 {
                for ix in 0..(rect.width() / cell) as i32 + 1 {
                    let c = if (ix + iy) % 2 == 0 {
                        Color32::from_gray(235)
                    } else {
                        Color32::from_gray(250)
                    };
                    let r = Rect::from_min_size(
                        rect.min + vec2(ix as f32 * cell, iy as f32 * cell),
                        vec2(cell, cell),
                    )
                    .intersect(rect);
                    p.rect_filled(r, 0.0, c);
                }
            }
            if let Some((_, tex)) = &app.export_preview {
                let s = tex.size_vec2();
                let k = (rect.width() / s.x).min(rect.height() / s.y).min(1.0);
                let r = Rect::from_center_size(rect.center(), s * k);
                p.image(
                    tex.id(),
                    r,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                p.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "Nada para exportar",
                    FontId::proportional(14.0),
                    Color32::GRAY,
                );
            }
            ui.add_space(8.0);
            ui.checkbox(&mut app.export_opts.background, "Incluir fundo");
            ui.checkbox(&mut app.export_opts.dark, "Modo escuro");
            ui.add_enabled_ui(!app.selected.is_empty(), |ui| {
                let label = if app.selected_frame().is_some() {
                    "Somente o frame selecionado"
                } else {
                    "Somente a seleção"
                };
                ui.checkbox(&mut app.export_selection_only, label);
            });
            ui.horizontal(|ui| {
                ui.label("Escala:");
                for s in [1.0, 2.0, 3.0] {
                    ui.selectable_value(&mut app.export_opts.scale, s, format!("{s:.0}×"));
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("  PNG  ").strong()).clicked() {
                    app.export_file(false);
                }
                if ui.button(RichText::new("  SVG  ").strong()).clicked() {
                    app.export_file(true);
                }
                if ui.button("Copiar SVG").clicked() {
                    app.copy_svg(ctx);
                }
            });
            ui.label(
                RichText::new("O SVG embute a fonte Virgil e abre offline em qualquer navegador.")
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
    app.show_export = open && app.show_export;
}

fn help_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_help {
        return;
    }
    let mut open = true;
    egui::Window::new("Atalhos de teclado")
        .collapsible(false)
        .resizable(false)
        .min_width(760.0)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.columns(2, |cols| {
                // mesma organização do diálogo de atalhos do Excalidraw
                let tools: &[(&str, &str)] = &[
                    ("Mão (mover a tela)", "H"),
                    ("Seleção", "V ou 1"),
                    ("Retângulo", "R ou 2"),
                    ("Losango", "D ou 3"),
                    ("Elipse", "O ou 4"),
                    ("Seta", "A ou 5"),
                    ("Linha", "L ou 6"),
                    ("Desenho livre", "P ou 7"),
                    ("Texto", "T ou 8"),
                    ("Inserir imagem", "9"),
                    ("Borracha", "E ou 0"),
                    ("Frame", "F"),
                    ("Editar texto", "Enter"),
                    ("Renomear frame", "Enter · duplo clique no nome"),
                    ("Concluir texto", "Esc · Ctrl+Enter"),
                    ("Manter ferramenta ativa", "Q"),
                    ("Trocar tipo da forma", "Tab · Shift+Tab"),
                ];
                let view: &[(&str, &str)] = &[
                    ("Aumentar / diminuir zoom", "Ctrl++ · Ctrl+-"),
                    ("Redefinir zoom", "Ctrl+0"),
                    ("Ajustar todos os elementos", "Shift+1"),
                    ("Ajustar à seleção", "Shift+2 · Shift+3"),
                    ("Rolar a página", "PgUp/PgDn · Shift+PgUp/PgDn"),
                    ("Modo zen", "Alt+Z"),
                    ("Grade", "Ctrl+'"),
                    ("Encaixar em objetos", "Alt+S"),
                    ("Modo de visualização", "Alt+R"),
                    ("Tema escuro", "Alt+Shift+D"),
                    ("Estatísticas", "Alt+/"),
                    ("Mover a tela", "Espaço+arrastar · roda"),
                    ("Atalhos de teclado", "?"),
                ];
                let (to_back, to_front) = if cfg!(target_os = "macos") {
                    ("Ctrl+[ · Ctrl+Alt+[", "Ctrl+] · Ctrl+Alt+]")
                } else {
                    ("Ctrl+[ · Ctrl+Shift+[", "Ctrl+] · Ctrl+Shift+]")
                };
                let editor: &[(&str, &str)] = &[
                    ("Limpar o canvas", "Ctrl+Delete"),
                    ("Apagar", "Delete"),
                    ("Recortar / Copiar / Colar", "Ctrl+X / C / V"),
                    ("Selecionar tudo", "Ctrl+A"),
                    ("Adicionar à seleção", "Shift+clique"),
                    ("Copiar como PNG", "Shift+Alt+C"),
                    ("Copiar / colar estilos", "Ctrl+Alt+C / V"),
                    ("Enviar para trás / para o fundo", to_back),
                    ("Trazer para frente / ao topo", to_front),
                    ("Alinhar", "Ctrl+Shift+setas"),
                    ("Duplicar", "Ctrl+D · Alt+arrastar"),
                    ("Travar / destravar", "Ctrl+Shift+L"),
                    ("Desfazer / Refazer", "Ctrl+Z / Ctrl+Shift+Z"),
                    ("Agrupar / Desagrupar", "Ctrl+G / Ctrl+Shift+G"),
                    ("Inverter na horizontal / vertical", "Shift+H / Shift+V"),
                    ("Cor do traço / do fundo", "S / G"),
                    ("Diminuir / aumentar fonte", "Ctrl+Shift+< / >"),
                    ("Mover 1px / 5px", "Setas / Shift+setas"),
                    ("Seta sem grudar na forma", "Ctrl+arrastar"),
                    ("Linha: adicionar ponto", "Arrastar o meio · duplo clique"),
                    ("Linha: selecionar pontos", "Clique · Shift+clique"),
                    ("Linha: apagar pontos", "Delete"),
                    ("Cotovelo: soltar segmento", "Duplo clique no meio"),
                    ("Encaixe: inverter enquanto arrasta", "Ctrl+arrastar"),
                    ("Salvar / Salvar como / Abrir", "Ctrl+S / Ctrl+Shift+S / Ctrl+O"),
                    ("Exportar imagem", "Ctrl+Shift+E"),
                    ("Menu de opções", "Botão direito"),
                ];
                let grid = |ui: &mut Ui, id: &str, title: &str, rows: &[(&str, &str)]| {
                    ui.strong(title);
                    egui::Grid::new(id).num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                        for (a, b) in rows {
                            ui.label(*a);
                            ui.label(RichText::new(keys(b)).monospace().size(11.5));
                            ui.end_row();
                        }
                    });
                };
                grid(&mut cols[0], "help_tools", "Ferramentas", tools);
                cols[0].add_space(8.0);
                grid(&mut cols[0], "help_view", "Visualização", view);
                grid(&mut cols[1], "help_editor", "Editor", editor);
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(keys("Arquivos .excalidraw abrem e salvam no mesmo formato do excalidraw.com. Copiar (Ctrl+C) gera JSON que pode ser colado lá também."))
                    .size(11.5)
                    .color(ui.visuals().weak_text_color()),
            );
        });
    app.show_help = open && app.show_help;
}

fn confirm_window(app: &mut App, ctx: &egui::Context) {
    let Some(pending) = app.confirm else { return };
    if pending == Pending::ClearLibrary {
        let some = !app.library_selected.is_empty();
        egui::Window::new(if some {
            "Remover da biblioteca"
        } else {
            "Limpar a biblioteca"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(if some {
                "Remover os itens marcados da biblioteca? Dá para desfazer logo depois, no painel."
            } else {
                "Remover todos os itens da biblioteca? Dá para desfazer logo depois, no painel."
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(if some { "Remover" } else { "Limpar" }).clicked() {
                    app.confirm = None;
                    app.remove_library_items();
                }
                if ui.button("Cancelar").clicked() {
                    app.confirm = None;
                }
            });
        });
        return;
    }
    if pending == Pending::Clear {
        egui::Window::new("Limpar o canvas")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(keys(
                    "Isto vai apagar todo o canvas. Você pode desfazer com Ctrl+Z.",
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Limpar").clicked() {
                        app.confirm = None;
                        app.clear_canvas();
                    }
                    if ui.button("Cancelar").clicked() {
                        app.confirm = None;
                    }
                });
            });
        return;
    }
    if pending == Pending::Reload {
        let name = app
            .file_path
            .as_deref()
            .map(crate::app::file_name)
            .unwrap_or_default();
        egui::Window::new("Arquivo alterado")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!(
                    "Outro programa alterou {name}, mas o desenho aberto tem alterações não salvas."
                ));
                ui.label(keys(
                    "Recarregar troca o desenho pelo do arquivo (Ctrl+Z volta ao seu).",
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Recarregar").clicked() {
                        app.confirm = None;
                        app.reload_from_disk(None);
                    }
                    if ui.button("Manter o meu").clicked() {
                        app.confirm = None;
                    }
                });
            });
        return;
    }
    egui::Window::new("Alterações não salvas")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label("O desenho atual tem alterações não salvas. O que deseja fazer?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Salvar").clicked() {
                    app.confirm = None;
                    app.save_then(Some(pending));
                }
                if ui.button("Descartar").clicked() {
                    app.dirty = false;
                    app.confirm = None;
                    app.run_pending(pending);
                }
                if ui.button("Cancelar").clicked() {
                    app.confirm = None;
                }
            });
        });
}

fn toast(app: &mut App, ctx: &egui::Context) {
    let Some((msg, until)) = app.toast.clone() else {
        return;
    };
    if app.now > until {
        app.toast = None;
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
    egui::Area::new(Id::unique("toast"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -16.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            panel_frame(ui)
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.label(msg);
                });
        });
}

fn hint(app: &App, ctx: &egui::Context) {
    let text = match (&app.drag, app.tool) {
        (Drag::LinearMulti { .. }, _) => {
            "Clique para adicionar pontos · Enter, Esc ou duplo clique para terminar"
        }
        _ if app.editing.is_some() => "Esc ou Ctrl+Enter para concluir o texto",
        (_, Tool::Draw) => "Arraste para desenhar à mão livre",
        (_, Tool::Arrow | Tool::Line) => "Arraste, ou clique ponto a ponto · Shift trava o ângulo",
        (_, Tool::Rect | Tool::Diamond | Tool::Ellipse) => {
            "Arraste para desenhar · Shift mantém a proporção · Alt a partir do centro"
        }
        (_, Tool::Text) => "Clique para escrever",
        (_, Tool::Eraser) => "Clique ou arraste sobre os elementos para apagar",
        (_, Tool::Frame) => {
            "Arraste para criar um frame · o que ficar dentro passa a fazer parte dele"
        }
        (_, Tool::Select) if app.elements.is_empty() => {
            "Escolha uma ferramenta acima ou dê duplo clique para escrever · ? mostra os atalhos"
        }
        _ => return,
    };
    egui::Area::new(Id::unique("hint"))
        .anchor(Align2::CENTER_TOP, vec2(0.0, 70.0))
        .order(egui::Order::Middle)
        .interactable(false)
        .show(ctx, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(keys(text))
                        .size(12.0)
                        .color(ui.visuals().weak_text_color()),
                )
                .wrap_mode(egui::TextWrapMode::Extend),
            );
        });
}

pub fn overlays(app: &mut App, ctx: &egui::Context, view: Rect) {
    if !app.view_mode {
        toolbar(app, ctx, view);
        hint(app, ctx);
    }
    if !app.view_mode && !app.zen_mode {
        props_panel(app, ctx);
        menu(app, ctx);
        library_panel(app, ctx, view);
        library_edit_banner(app, ctx);
    } else {
        app.library_drag = None;
    }
    bottom_bar(app, ctx, view);
    mode_exit_button(app, ctx);
    stats_window(app, ctx);
    app.open_picker = None;
    text_editor(app, ctx);
    frame_name_editor(app, ctx);
    export_window(app, ctx);
    help_window(app, ctx);
    confirm_window(app, ctx);
    context_menu(app, ctx);
    toast(app, ctx);
}

/// Botão para sair do modo zen (Alt+Z) ou de visualização (Alt+R).
fn mode_exit_button(app: &mut App, ctx: &egui::Context) {
    let label = if app.view_mode {
        "Sair do modo de visualização (Alt+R)"
    } else if app.zen_mode {
        "Sair do modo zen (Alt+Z)"
    } else {
        return;
    };
    egui::Area::new(Id::unique("mode_exit"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -14.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            panel_frame(ui)
                .inner_margin(Margin::same(3))
                .show(ui, |ui| {
                    if ui
                        .add(
                            egui::Button::new(keys(label))
                                .frame(false)
                                .min_size(vec2(0.0, 30.0)),
                        )
                        .clicked()
                    {
                        if app.view_mode {
                            app.toggle_view_mode();
                        } else {
                            app.zen_mode = false;
                        }
                    }
                });
        });
}

/// Painel de estatísticas (Alt+/).
fn stats_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_stats {
        return;
    }
    let mut open = true;
    egui::Window::new("Estatísticas")
        .collapsible(false)
        .resizable(false)
        .anchor(
            Align2::RIGHT_TOP,
            vec2(
                if library_visible(app) {
                    -(LIB_W + 24.0)
                } else {
                    -12.0
                },
                64.0,
            ),
        )
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new("stats_grid")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    let row = |ui: &mut Ui, a: &str, b: String| {
                        ui.label(a);
                        ui.label(RichText::new(b).monospace());
                        ui.end_row();
                    };
                    ui.strong("Canvas");
                    ui.end_row();
                    row(ui, "Elementos", app.elements.len().to_string());
                    if let Some(b) = crate::geom::scene_bounds(app.elements.iter()) {
                        row(ui, "Largura", format!("{:.0}", b.width()));
                        row(ui, "Altura", format!("{:.0}", b.height()));
                    }
                    let sel = app.selected_elements();
                    if sel.len() == 1 {
                        let e = sel[0];
                        ui.strong("Elemento");
                        ui.end_row();
                        row(ui, "X", format!("{:.0}", e.x));
                        row(ui, "Y", format!("{:.0}", e.y));
                        row(ui, "Largura", format!("{:.0}", e.width));
                        row(ui, "Altura", format!("{:.0}", e.height));
                        row(ui, "Ângulo", format!("{:.0}°", e.angle.to_degrees()));
                    } else if sel.len() > 1 {
                        ui.strong("Seleção");
                        ui.end_row();
                        row(ui, "Elementos", sel.len().to_string());
                        if let Some(b) = crate::geom::scene_bounds(sel) {
                            row(ui, "X", format!("{:.0}", b.min.x));
                            row(ui, "Y", format!("{:.0}", b.min.y));
                            row(ui, "Largura", format!("{:.0}", b.width()));
                            row(ui, "Altura", format!("{:.0}", b.height()));
                        }
                    }
                });
        });
    app.show_stats = open && app.show_stats;
}

// ---------------------------------------------------------------------------
// Biblioteca
// ---------------------------------------------------------------------------

const LIB_W: f32 = 252.0;
const LIB_TILE: f32 = 76.0;

fn library_visible(app: &App) -> bool {
    app.show_library && !app.view_mode && !app.zen_mode
}

/// Limites e primitivas de um item (calculados uma vez por item).
fn library_thumb(app: &mut App, id: &str) -> Option<(crate::geom::Rect, Vec<crate::geom::Prim>)> {
    if !app.library_thumbs.contains_key(id) {
        let item = app.library.iter().find(|i| i.id == id)?;
        let b = crate::export::bounds_with_text(&item.elements)?;
        let prims = item
            .elements
            .iter()
            .flat_map(crate::geom::element_prims)
            .collect();
        app.library_thumbs.insert(id.to_string(), (b, prims));
    }
    app.library_thumbs.get(id).cloned()
}

/// Desenha `prims` com o centro de `b` em `at`, na escala `zoom`.
fn paint_library_prims(
    app: &App,
    painter: &Painter,
    b: &crate::geom::Rect,
    prims: &[crate::geom::Prim],
    at: Pos2,
    zoom: f32,
    alpha: f32,
) {
    let c = b.center();
    let pc = render::PaintCtx {
        painter,
        cam: render::Cam {
            scroll: crate::geom::P::new(-c.x, -c.y),
            zoom,
            origin: at,
        },
        dark: app.dark,
        alpha,
        hand_ok: app.hand_ok,
        images: &app.textures,
    };
    render::paint_prims(&pc, prims);
}

/// Painel da biblioteca, à direita, como a barra lateral do Excalidraw:
/// clique insere no centro da tela, arrastar posiciona, Shift+clique marca.
fn library_panel(app: &mut App, ctx: &egui::Context, view: Rect) {
    egui::Area::new(Id::unique("library_btn"))
        .anchor(Align2::RIGHT_TOP, vec2(-12.0, 12.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            panel_frame(ui)
                .inner_margin(Margin::same(3))
                .show(ui, |ui| {
                    let open = app.show_library;
                    let text = RichText::new("Biblioteca").color(if open && !app.dark {
                        Color32::from_rgb(0x43, 0x41, 0x9e)
                    } else {
                        fg(ui)
                    });
                    let mut b = egui::Button::new(text).min_size(vec2(0.0, 34.0));
                    b = if open {
                        b.fill(accent_bg(ui))
                    } else {
                        b.frame(false)
                    };
                    if ui.add(b).on_hover_text("Biblioteca de formas").clicked() {
                        app.show_library = !app.show_library;
                    }
                });
        });
    if !app.show_library {
        app.library_drag = None;
        return;
    }
    let height = (view.height() - 60.0 - 64.0).max(160.0);
    let mut insert: Option<String> = None;
    let mut drop_target: Option<String> = None;
    let resp = egui::Area::new(Id::unique("library"))
        .anchor(Align2::RIGHT_TOP, vec2(-12.0, 60.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            panel_frame(ui)
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(LIB_W);
                    ui.set_height(height);
                    ui.horizontal(|ui| {
                        ui.strong("Biblioteca");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(egui::Button::new("✕").frame(false))
                                .on_hover_text("Fechar")
                                .clicked()
                            {
                                app.show_library = false;
                            }
                        });
                    });
                    let marked = app.library_selected.len();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if ui
                            .small_button("Importar…")
                            .on_hover_text("Adicionar itens de um arquivo .excalidrawlib")
                            .clicked()
                        {
                            app.import_library_dialog();
                        }
                        let empty = app.library.is_empty();
                        ui.add_enabled_ui(!empty, |ui| {
                            let (export, remove) = if marked > 0 {
                                ("Exportar marcados…", "Remover marcados")
                            } else {
                                ("Exportar…", "Limpar")
                            };
                            if ui
                                .small_button(export)
                                .on_hover_text("Salvar como .excalidrawlib")
                                .clicked()
                            {
                                app.export_library_dialog();
                            }
                            if ui.small_button(remove).clicked() {
                                app.confirm = Some(Pending::ClearLibrary);
                            }
                        });
                    });
                    if marked > 0 {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if marked == 1 {
                                    "1 item marcado".to_string()
                                } else {
                                    format!("{marked} itens marcados")
                                })
                                .size(11.5)
                                .color(ui.visuals().weak_text_color()),
                            );
                            if ui.small_button("Desmarcar").clicked() {
                                app.library_selected.clear();
                            }
                        });
                    }
                    if let Some((_, what)) = app.library_undo.clone() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(what)
                                    .size(11.5)
                                    .color(ui.visuals().weak_text_color()),
                            );
                            if ui.small_button("Desfazer").clicked() {
                                app.undo_library();
                            }
                            if ui
                                .add(egui::Button::new("✕").frame(false).small())
                                .on_hover_text("Dispensar")
                                .clicked()
                            {
                                app.library_undo = None;
                            }
                        });
                    }
                    if let Some((_, buf)) = &mut app.library_rename {
                        let mut done = None;
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::TextEdit::singleline(buf)
                                    .hint_text("Nome do item")
                                    .desired_width(LIB_W - 90.0),
                            );
                            if !r.has_focus() && !r.lost_focus() {
                                r.request_focus();
                            }
                            if r.lost_focus() {
                                done = Some(!ui.input(|i| i.key_pressed(egui::Key::Escape)));
                            }
                            if ui.small_button("OK").clicked() {
                                done = Some(true);
                            }
                            if ui.small_button("✕").on_hover_text("Cancelar").clicked() {
                                done = Some(false);
                            }
                        });
                        if let Some(commit) = done {
                            app.finish_rename_library_item(commit);
                        }
                    }
                    ui.add_space(4.0);
                    let can_add = !app.selected.is_empty();
                    if app.library.is_empty() && !can_add {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(
                                "Nenhum item ainda.\n\nSelecione formas ou grupos no canvas e \
                             clique em + (ou use \"Adicionar à biblioteca\" no botão direito), \
                             ou importe um arquivo .excalidrawlib.",
                            )
                            .size(12.0)
                            .color(ui.visuals().weak_text_color()),
                        );
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                            let ids: Vec<Option<String>> = can_add
                                .then_some(None)
                                .into_iter()
                                .chain(app.library.iter().map(|i| Some(i.id.clone())))
                                .collect();
                            for row in ids.chunks(3) {
                                ui.horizontal(|ui| {
                                    for cell in row {
                                        library_tile(
                                            app,
                                            ui,
                                            cell.as_deref(),
                                            view,
                                            &mut insert,
                                            &mut drop_target,
                                        );
                                    }
                                });
                            }
                        });
                });
        })
        .response;
    if let Some(id) = insert {
        let at = app.view_center(view);
        app.insert_library_item(&id, at);
    }
    // arrastar um item para o canvas
    let Some(id) = app.library_drag.clone() else {
        return;
    };
    let (pos, released) = ctx.input(|i| (i.pointer.latest_pos(), !i.pointer.primary_down()));
    let on_canvas = pos.filter(|p| view.contains(*p) && !resp.rect.contains(*p));
    if released {
        app.library_drag = None;
        if let Some(p) = on_canvas {
            let at = app.cam.w(p);
            app.insert_library_item(&id, at);
        } else if let Some(before) = drop_target {
            // solto sobre outro item: reordena
            app.move_library_item_before(&id, &before);
        }
        return;
    }
    ctx.set_cursor_icon(if on_canvas.is_some() || drop_target.is_some() {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::NoDrop
    });
    if let (Some(p), Some((b, prims))) = (on_canvas, library_thumb(app, &id)) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            Id::unique("library_ghost"),
        ));
        paint_library_prims(app, &painter, &b, &prims, p, app.cam.zoom, 0.6);
    }
}

/// Um quadradinho da biblioteca: `None` é o botão "+" que adiciona a seleção.
/// Arrastar um item sobre outro marca `drop_target` (reordenar).
fn library_tile(
    app: &mut App,
    ui: &mut Ui,
    id: Option<&str>,
    view: Rect,
    insert: &mut Option<String>,
    drop_target: &mut Option<String>,
) {
    let (rect, resp) = ui.allocate_exact_size(vec2(LIB_TILE, LIB_TILE), Sense::click_and_drag());
    let dragging = app.library_drag.as_deref();
    let drop_here = id.is_some_and(|id| {
        dragging.is_some_and(|d| d != id)
            && ui
                .ctx()
                .pointer_latest_pos()
                .is_some_and(|p| rect.contains(p) && ui.clip_rect().contains(p))
    });
    if drop_here {
        *drop_target = id.map(String::from);
    }
    let marked = id.is_some_and(|id| app.library_selected.contains(id));
    let hovered = resp.hovered();
    let fill = if marked {
        accent_bg(ui)
    } else if hovered && app.library_drag.is_none() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        ui.visuals().faint_bg_color
    };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, fill);
    p.rect_stroke(
        rect,
        6.0,
        if marked {
            Stroke::new(2.0, SELECTION)
        } else {
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        },
        StrokeKind::Inside,
    );
    let Some(id) = id else {
        let c = fg(ui);
        let m = rect.center();
        p.line_segment(
            [m - vec2(10.0, 0.0), m + vec2(10.0, 0.0)],
            Stroke::new(2.0, c),
        );
        p.line_segment(
            [m - vec2(0.0, 10.0), m + vec2(0.0, 10.0)],
            Stroke::new(2.0, c),
        );
        if resp
            .on_hover_text("Adicionar a seleção à biblioteca")
            .clicked()
        {
            app.add_selection_to_library();
        }
        return;
    };
    let name = app
        .library
        .iter()
        .find(|i| i.id == id)
        .and_then(|i| i.name.clone());
    let editing = app.library_edit.as_deref() == Some(id);
    if let Some((b, prims)) = library_thumb(app, id) {
        let inner = rect.shrink(8.0);
        let inner = if name.is_some() {
            Rect::from_min_max(inner.min, inner.max - vec2(0.0, 12.0))
        } else {
            inner
        };
        let zoom = (inner.width() / b.width().max(1.0))
            .min(inner.height() / b.height().max(1.0))
            .min(1.5);
        let painter = ui.painter_at(rect.shrink(2.0));
        paint_library_prims(app, &painter, &b, &prims, inner.center(), zoom, 1.0);
    }
    let p = ui.painter();
    if let Some(n) = &name {
        let short: String = if n.chars().count() > 12 {
            n.chars().take(11).chain(['…']).collect()
        } else {
            n.clone()
        };
        p.text(
            rect.center_bottom() - vec2(0.0, 4.0),
            Align2::CENTER_BOTTOM,
            short,
            FontId::proportional(10.0),
            ui.visuals().weak_text_color(),
        );
    }
    if editing {
        p.rect_stroke(
            rect.expand(1.0),
            7.0,
            Stroke::new(2.0, Color32::from_rgb(0xf0, 0x8c, 0x00)),
            StrokeKind::Middle,
        );
    }
    if drop_here {
        // o item arrastado entra antes deste
        let x = rect.left() - 3.0;
        p.line_segment(
            [pos2(x, rect.top() + 4.0), pos2(x, rect.bottom() - 4.0)],
            Stroke::new(3.0, SELECTION),
        );
    }
    if resp.drag_started() {
        app.library_drag = Some(id.to_string());
    }
    const HELP: &str = "Clique para inserir · arraste para o canvas ou para outro item · \
                        Shift+clique marca · botão direito para mais opções";
    let tip = match &name {
        Some(n) => format!("{n}\n{HELP}"),
        None => HELP.into(),
    };
    let resp = resp.on_hover_text(tip);
    let last = app.library.len().saturating_sub(1);
    let pos = app.library.iter().position(|i| i.id == id).unwrap_or(0);
    resp.context_menu(|ui| {
        ui.set_min_width(190.0);
        if ui.button("Inserir no centro").clicked() {
            *insert = Some(id.to_string());
            ui.close();
        }
        if ui.button("Editar no canvas").clicked() {
            let at = app.view_center(view);
            app.edit_library_item(id, at);
            ui.close();
        }
        if ui.button("Renomear…").clicked() {
            app.start_rename_library_item(id);
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(pos > 0, egui::Button::new("Mover para trás"))
            .clicked()
        {
            app.move_library_item(id, pos - 1);
            ui.close();
        }
        if ui
            .add_enabled(pos < last, egui::Button::new("Mover para frente"))
            .clicked()
        {
            app.move_library_item(id, pos + 1);
            ui.close();
        }
        if ui
            .add_enabled(pos > 0, egui::Button::new("Mover para o início"))
            .clicked()
        {
            app.move_library_item(id, 0);
            ui.close();
        }
        if ui
            .add_enabled(pos < last, egui::Button::new("Mover para o fim"))
            .clicked()
        {
            app.move_library_item(id, usize::MAX);
            ui.close();
        }
        ui.separator();
        if ui.button("Exportar…").clicked() {
            app.export_library_item(id);
            ui.close();
        }
        let danger = ui.visuals().error_fg_color;
        if ui.button(RichText::new("Remover").color(danger)).clicked() {
            app.remove_library_item(id);
            ui.close();
        }
    });
    if resp.clicked() {
        if ui.input(|i| i.modifiers.shift) || !app.library_selected.is_empty() {
            if !app.library_selected.remove(id) {
                app.library_selected.insert(id.to_string());
            }
        } else {
            *insert = Some(id.to_string());
        }
    }
}

/// Faixa mostrada enquanto um item da biblioteca é editado no canvas.
fn library_edit_banner(app: &mut App, ctx: &egui::Context) {
    let Some(id) = app.library_edit.clone() else {
        return;
    };
    let name = app
        .library
        .iter()
        .find(|i| i.id == id)
        .and_then(|i| i.name.clone());
    egui::Area::new(Id::unique("library_edit"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -60.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            panel_frame(ui)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(match name {
                            Some(n) => format!("Editando \u{201c}{n}\u{201d} da biblioteca."),
                            None => "Editando um item da biblioteca.".into(),
                        });
                        ui.label(
                            RichText::new("Selecione o que deve ficar no item e salve.")
                                .color(ui.visuals().weak_text_color()),
                        );
                        if ui.button("Salvar no item").clicked() {
                            app.save_library_edit();
                        }
                        if ui.button("Cancelar").clicked() {
                            app.cancel_library_edit();
                        }
                    });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::keys_for;

    #[test]
    fn keys_use_mac_names_only_on_mac() {
        assert_eq!(keys_for("Ctrl+Shift+Z", false), "Ctrl+Shift+Z");
        assert_eq!(keys_for("Ctrl+Shift+Z", true), "Cmd+Shift+Z");
        assert_eq!(keys_for("Ctrl+Alt+C / V", true), "Cmd+Option+C / V");
        assert_eq!(keys_for("Ctrl++ · Ctrl+-", true), "Cmd++ · Cmd+-");
        assert_eq!(
            keys_for("Shift mantém a proporção · Alt a partir do centro", true),
            "Shift mantém a proporção · Option a partir do centro"
        );
        // palavras que só começam com "Alt" ficam como estão
        assert_eq!(keys_for("Altera o desenho", true), "Altera o desenho");
    }
}
