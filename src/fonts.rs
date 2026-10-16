//! Fontes embutidas (o app funciona 100% offline).
//!
//! * Virgil — a fonte "manuscrita" do Excalidraw (SIL OFL 1.1, © Ellinor Rapp).
//!   É distribuída sem modificações, no formato WOFF2 original, e descompactada
//!   em memória na inicialização.
//! * Ubuntu Light e Hack — fontes padrão do egui (normal e monoespaçada).

use std::sync::OnceLock;

static VIRGIL_WOFF2: &[u8] = include_bytes!("../assets/fonts/Virgil.woff2");

pub fn virgil_woff2() -> &'static [u8] {
    VIRGIL_WOFF2
}

/// TTF/OTF descompactado da Virgil (ou `None` se falhar — usa a fonte normal).
pub fn virgil_ttf() -> Option<&'static [u8]> {
    static CELL: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    CELL.get_or_init(|| wuff::decompress_woff2(VIRGIL_WOFF2).ok())
        .as_deref()
}

pub fn sans_ttf() -> &'static [u8] {
    epaint_default_fonts::UBUNTU_LIGHT
}

pub fn mono_ttf() -> &'static [u8] {
    epaint_default_fonts::HACK_REGULAR
}

/// Categoria de fonte a partir do `fontFamily` do Excalidraw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Hand,
    Sans,
    Mono,
}

impl Family {
    pub fn from_excalidraw(id: i64) -> Self {
        match id {
            1 | 5 => Family::Hand, // Virgil / Excalifont
            3 | 8 => Family::Mono, // Cascadia / Comic Shanns
            _ => Family::Sans,     // Helvetica, Nunito, Lilita...
        }
    }

    pub fn svg_font_family(self) -> &'static str {
        match self {
            Family::Hand => "Virgil, Segoe UI Emoji",
            Family::Sans => "Helvetica, Arial, sans-serif",
            Family::Mono => "Cascadia, Consolas, monospace",
        }
    }
}

/// Fontes do fontdue (para exportar PNG), com fallback para a fonte normal.
pub struct RasterFonts {
    pub hand: Option<fontdue::Font>,
    pub sans: fontdue::Font,
    pub mono: fontdue::Font,
}

pub fn raster_fonts() -> &'static RasterFonts {
    static CELL: OnceLock<RasterFonts> = OnceLock::new();
    CELL.get_or_init(|| {
        let load = |b: &[u8]| fontdue::Font::from_bytes(b, fontdue::FontSettings::default());
        RasterFonts {
            hand: virgil_ttf().and_then(|b| load(b).ok()),
            sans: load(sans_ttf()).expect("fonte padrão"),
            mono: load(mono_ttf()).expect("fonte mono"),
        }
    })
}

impl RasterFonts {
    pub fn primary(&self, f: Family) -> &fontdue::Font {
        match f {
            Family::Hand => self.hand.as_ref().unwrap_or(&self.sans),
            Family::Sans => &self.sans,
            Family::Mono => &self.mono,
        }
    }

    /// Fonte que tem o glifo `c` (primária, senão a normal).
    pub fn for_char(&self, f: Family, c: char) -> &fontdue::Font {
        let p = self.primary(f);
        if p.lookup_glyph_index(c) != 0 || c.is_whitespace() {
            p
        } else {
            &self.sans
        }
    }

    pub fn line_width(&self, f: Family, line: &str, px: f32) -> f32 {
        line.chars()
            .map(|c| self.for_char(f, c).metrics(c, px).advance_width)
            .sum()
    }

    /// Deslocamento da linha de base dentro de uma linha de altura `line_h`.
    pub fn baseline(&self, f: Family, px: f32, line_h: f32) -> f32 {
        let (asc, desc) = self
            .primary(f)
            .horizontal_line_metrics(px)
            .map(|m| (m.ascent, m.descent))
            .unwrap_or((px * 0.8, -px * 0.2));
        (line_h + asc + desc) / 2.0
    }
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virgil_decodes_and_has_latin_glyphs() {
        let f = raster_fonts();
        let hand = f.hand.as_ref().expect("Virgil deve descompactar");
        assert_ne!(hand.lookup_glyph_index('A'), 0);
        assert!(f.line_width(Family::Hand, "Olá", 20.0) > 10.0);
    }

    #[test]
    fn b64() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
