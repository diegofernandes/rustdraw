//! Notas adesivas: um retângulo amarelo com o texto dentro, começando no
//! canto de cima à esquerda.
//!
//! No arquivo a nota é um retângulo comum com rótulo (o Excalidraw abre sem
//! saber de notas); a marca `customData.rustdrawNote` só lembra que é uma
//! nota, para o texto escrito nela de novo nascer alinhado como nota.

use crate::model::Element;
use serde_json::{Value, json};

/// Tamanho de uma nota criada com um clique.
pub const SIZE: f64 = 200.0;
/// Amarelo e laranja das cores rápidas do Excalidraw.
const BACKGROUND: &str = "#ffec99";
const STROKE: &str = "#f08c00";
const MARK: &str = "rustdrawNote";

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
    e.stroke_color = STROKE.into();
    e.fill_style = "solid".into();
    e.stroke_width = 1.0;
    e.stroke_style = "solid".into();
    let mut data = match e.extra.remove("customData") {
        Some(Value::Object(m)) => m,
        _ => Default::default(),
    };
    data.insert(MARK.into(), json!(true));
    e.set_field("customData", Value::Object(data));
}

/// Alinha o texto `t` de uma nota em cima à esquerda.
pub fn align(t: &mut Element) {
    t.set_field("textAlign", json!("left"));
    t.set_field("verticalAlign", json!("top"));
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
        assert_eq!(r.extra["customData"]["foo"], 1);

        // ida e volta pelo JSON
        let back: Element = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(is_note(&back));

        let mut e = Element::new("ellipse", 0.0, 0.0, &Style::default());
        make(&mut e);
        assert!(!is_note(&e), "só retângulo é nota");
    }
}
