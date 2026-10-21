//! Notas adesivas: um papel amarelo sem borda, com sombra e a ponta de
//! baixo à direita dobrada, e o texto começando no canto de cima à esquerda.
//!
//! No arquivo a nota é um retângulo comum com rótulo (o Excalidraw abre sem
//! saber de notas, e mostra um retângulo amarelo); a marca
//! `customData.rustdrawNote` diz ao RustDraw para desenhar a dobra e a sombra
//! e para o texto escrito nela de novo nascer alinhado como nota.

use crate::geom::{P, Prim, Rect, Rgba};
use crate::model::{Element, parse_color};
use serde_json::{Value, json};

/// Tamanho de uma nota criada com um clique.
pub const SIZE: f64 = 200.0;
/// Folga do texto: dos lados e em cima/embaixo.
pub const PADDING: (f64, f64) = (16.0, 14.0);
/// Amarelo das cores rápidas do Excalidraw.
const BACKGROUND: &str = "#ffec99";
const MARK: &str = "rustdrawNote";
/// A dobra tem 12% do lado menor (24 px numa nota de 200).
const FOLD: f32 = 0.12;
/// Sombra: deslocamento para baixo, desfoque e opacidade.
const SHADOW_Y: f32 = 3.0;
const SHADOW_BLUR: f32 = 10.0;
const SHADOW_ALPHA: f32 = 0.12;

pub fn is_note(e: &Element) -> bool {
    e.kind == "rectangle"
        && e.extra
            .get("customData")
            .and_then(|d| d.get(MARK))
            .and_then(Value::as_bool)
            == Some(true)
}

/// Pinta o retângulo `e` como nota e marca que ele é uma.
pub fn make(e: &mut Element) {
    e.background_color = BACKGROUND.into();
    e.stroke_color = "transparent".into();
    e.fill_style = "solid".into();
    e.stroke_width = 1.0;
    e.stroke_style = "solid".into();
    e.roundness = None;
    let mut data = match e.extra.remove("customData") {
        Some(Value::Object(m)) => m,
        _ => Default::default(),
    };
    data.insert(MARK.into(), json!(true));
    e.set_field("customData", Value::Object(data));
}

/// Texto `t` de uma nota: Virgil 20, quase preto, em cima à esquerda.
pub fn style_text(t: &mut Element) {
    t.set_field("textAlign", json!("left"));
    t.set_field("verticalAlign", json!("top"));
    t.set_field("fontFamily", json!(1));
    t.set_field("fontSize", json!(20.0));
    t.stroke_color = "#1e1e1e".into();
}

/// Contorno da nota na caixa `b` (sem rotação), com o canto de baixo à
/// direita cortado na diagonal. Convexo.
pub fn outline(b: &Rect) -> Vec<P> {
    let f = fold_size(b);
    vec![
        b.min,
        P::new(b.max.x, b.min.y),
        P::new(b.max.x, b.max.y - f),
        P::new(b.max.x - f, b.max.y),
        P::new(b.min.x, b.max.y),
    ]
}

fn fold_size(b: &Rect) -> f32 {
    b.width().min(b.height()) * FOLD
}

/// A aba dobrada: o canto cortado, rebatido para dentro da nota.
fn flap(b: &Rect) -> Vec<P> {
    let f = fold_size(b);
    vec![
        P::new(b.max.x, b.max.y - f),
        P::new(b.max.x - f, b.max.y),
        P::new(b.max.x - f, b.max.y - f),
    ]
}

/// Sombra (camadas cada vez maiores e mais fracas, imitando o desfoque) e
/// aba da nota na caixa `b`. `bg` é a cor do papel, `opacity` de 0 a 1.
/// A sombra vai antes do papel e a aba depois.
pub fn decorations(b: &Rect, bg: Option<Rgba>, opacity: f32) -> (Vec<Prim>, Vec<Prim>) {
    const LAYERS: usize = 6;
    let alpha = 1.0 - (1.0 - SHADOW_ALPHA).powf(1.0 / LAYERS as f32);
    let alpha = (alpha * opacity * 255.0).round() as u8;
    let shadow = (0..LAYERS)
        .map(|i| {
            // de meio desfoque para dentro até meio desfoque para fora
            let d = SHADOW_BLUR * ((i as f32 + 0.5) / LAYERS as f32 - 0.5);
            let r = Rect {
                min: P::new(b.min.x - d, b.min.y - d + SHADOW_Y),
                max: P::new(b.max.x + d, b.max.y + d + SHADOW_Y),
            };
            Prim::Fill {
                pts: outline(&r),
                color: [0, 0, 0, alpha],
            }
        })
        .collect();
    let flap = bg
        .map(|c| Prim::Fill {
            pts: flap(b),
            color: fold_color(c),
        })
        .into_iter()
        .collect();
    (shadow, flap)
}

/// Cor da aba: o tom mais escuro do design para as cores rápidas, ou a
/// mesma cor com 12% a menos de luminosidade.
pub fn fold_color(c: Rgba) -> Rgba {
    let known = [
        ("#ffec99", "#f2d06b"),
        ("#ffc9c9", "#f2a7a7"),
        ("#a5d8ff", "#82bfef"),
        ("#b2f2bb", "#8edc9a"),
    ];
    for (bg, fold) in known {
        if parse_color(bg).is_some_and(|k| k[..3] == c[..3]) {
            let mut f = parse_color(fold).unwrap();
            f[3] = c[3];
            return f;
        }
    }
    let (h, s, l) = to_hsl(c);
    let [r, g, b] = from_hsl(h, s, (l - 0.12).max(0.0));
    [r, g, b, c[3]]
}

fn to_hsl(c: Rgba) -> (f32, f32, f32) {
    let [r, g, b] = [c[0], c[1], c[2]].map(|v| v as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Style;

    #[test]
    fn a_note_is_a_marked_yellow_rectangle_that_keeps_custom_data() {
        let mut r = Element::new("rectangle", 0.0, 0.0, &Style::default());
        assert!(!is_note(&r));
        r.set_field("customData", json!({"foo": 1}));
        make(&mut r);
        assert!(is_note(&r));
        assert_eq!(r.background_color, BACKGROUND);
        assert_eq!(r.fill_style, "solid");
        assert_eq!(r.stroke_color, "transparent");
        assert_eq!(r.roundness, None);
        assert_eq!(r.extra["customData"]["foo"], 1);

        // ida e volta pelo JSON
        let back: Element = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(is_note(&back));

        let mut e = Element::new("ellipse", 0.0, 0.0, &Style::default());
        make(&mut e);
        assert!(!is_note(&e), "só retângulo é nota");
    }

    #[test]
    fn fold_is_the_design_shade_or_twelve_percent_darker() {
        let c = |s| parse_color(s).unwrap();
        assert_eq!(fold_color(c("#ffec99")), c("#f2d06b"));
        assert_eq!(fold_color(c("#a5d8ff")), c("#82bfef"));
        // cinza 50% vira 38%
        assert_eq!(fold_color(c("#808080")), c("#616161"));
        // cor qualquer: mesmo matiz, mais escura
        let (h0, _, l0) = to_hsl(c("#d0bfff"));
        let (h1, _, l1) = to_hsl(fold_color(c("#d0bfff")));
        assert!((h0 - h1).abs() < 1.0 && (l0 - l1 - 0.12).abs() < 0.01);
    }

    #[test]
    fn the_folded_corner_is_twelve_percent_of_the_shorter_side() {
        let b = Rect {
            min: P::new(0.0, 0.0),
            max: P::new(300.0, 200.0),
        };
        let o = outline(&b);
        assert_eq!(o[2], P::new(300.0, 176.0));
        assert_eq!(o[3], P::new(276.0, 200.0));
    }
}
