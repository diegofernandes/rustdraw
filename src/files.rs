//! Imagens da cena: o mapa `files` do `.excalidraw` e os elementos `image`.
//!
//! Como no Excalidraw, cada imagem vira uma entrada em `files` (chave = `fileId`,
//! com `mimeType` e o conteúdo inteiro num `dataURL` base64) e o elemento
//! `image` só aponta para ela pelo `fileId`. Assim o arquivo continua um único
//! JSON, que abre no excalidraw.com e funciona sem internet.

use crate::fonts;
use crate::model::{Element, Style, now_ms};
use image::{ImageFormat, RgbaImage};
use serde_json::{Map, Value, json};
use std::io::Cursor;

/// Lado máximo de uma imagem inserida; maiores são reduzidas (como o Excalidraw faz).
pub const MAX_SIDE: u32 = 1440;

/// Formatos que sabemos decodificar.
pub fn mime_of(bytes: &[u8]) -> Option<&'static str> {
    Some(match image::guess_format(bytes).ok()? {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => return None,
    })
}

pub fn is_image_path(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp"
        )
    })
}

pub fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", fonts::base64(bytes))
}

/// Conteúdo de um `data:...;base64,...`.
pub fn parse_data_url(url: &str) -> Option<Vec<u8>> {
    let (head, data) = url.strip_prefix("data:")?.split_once(',')?;
    if !head.ends_with(";base64") {
        return None;
    }
    base64_decode(data)
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        match chunk.len() {
            4 => out.extend_from_slice(&bytes),
            3 => out.extend_from_slice(&bytes[..2]),
            2 => out.push(bytes[0]),
            _ => return None,
        }
    }
    Some(out)
}

/// SHA-1 em hexadecimal: o Excalidraw usa o hash do arquivo como `fileId`, o que
/// faz a mesma imagem colada duas vezes ocupar uma entrada só.
pub fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(v);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

pub fn decode_bytes(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes).ok().map(|i| i.to_rgba8())
}

/// Decodifica a imagem de uma entrada de `files`.
pub fn decode(files: &Map<String, Value>, id: &str) -> Option<RgbaImage> {
    let url = files.get(id)?.get("dataURL")?.as_str()?;
    decode_bytes(&parse_data_url(url)?)
}

pub fn encode_png(img: &RgbaImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .ok()?;
    Some(out)
}

/// Uma imagem pronta para entrar na cena.
pub struct Added {
    pub id: String,
    pub width: u32,
    pub height: u32,
}

/// Guarda a imagem em `files` (reduzindo se passar de [`MAX_SIDE`]) e devolve o `fileId`.
pub fn add_bytes(files: &mut Map<String, Value>, bytes: &[u8]) -> Result<Added, String> {
    let mime =
        mime_of(bytes).ok_or("formato de imagem não suportado (use PNG, JPG, GIF ou WebP)")?;
    let img = image::load_from_memory(bytes).map_err(|e| format!("imagem inválida: {e}"))?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err("imagem vazia".into());
    }
    let (mime, bytes, w, h) = if w.max(h) > MAX_SIDE {
        let small = img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
        let mut out = Vec::new();
        let (fmt, mime) = if mime == "image/jpeg" {
            (ImageFormat::Jpeg, "image/jpeg")
        } else {
            (ImageFormat::Png, "image/png")
        };
        let small = if fmt == ImageFormat::Jpeg {
            image::DynamicImage::ImageRgb8(small.to_rgb8())
        } else {
            small
        };
        small
            .write_to(&mut Cursor::new(&mut out), fmt)
            .map_err(|e| e.to_string())?;
        (mime, out, small.width(), small.height())
    } else {
        (mime, bytes.to_vec(), w, h)
    };
    let id = sha1_hex(&bytes);
    if !files.contains_key(&id) {
        let now = now_ms();
        files.insert(
            id.clone(),
            json!({
                "mimeType": mime,
                "id": id,
                "dataURL": data_url(mime, &bytes),
                "created": now,
                "lastRetrieved": now,
            }),
        );
    }
    Ok(Added {
        id,
        width: w,
        height: h,
    })
}

/// Elemento `image` com o centro em (cx, cy), lado maior limitado a `max_side`.
pub fn image_element(a: &Added, cx: f64, cy: f64, max_side: f64) -> Element {
    let mut el = Element::new("image", 0.0, 0.0, &Style::default());
    let (w, h) = (a.width as f64, a.height as f64);
    let k = (max_side / w.max(h)).min(1.0);
    el.width = (w * k).round().max(1.0);
    el.height = (h * k).round().max(1.0);
    el.x = (cx - el.width / 2.0).round();
    el.y = (cy - el.height / 2.0).round();
    el.set_field("fileId", json!(a.id));
    el
}

/// Ids de arquivo usados pelos elementos.
pub fn used_ids(els: &[Element]) -> std::collections::HashSet<&str> {
    els.iter()
        .filter(|e| e.kind == "image")
        .filter_map(|e| e.str_field("fileId"))
        .collect()
}

/// Só as entradas de `files` usadas por `els` (o que vai para o arquivo ou a área de transferência).
pub fn subset(files: &Map<String, Value>, els: &[Element]) -> Map<String, Value> {
    let used = used_ids(els);
    files
        .iter()
        .filter(|(k, _)| used.contains(k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        encode_png(&RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]))).unwrap()
    }

    #[test]
    fn sha1_matches_known_vectors() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn base64_roundtrip() {
        for n in 0..10 {
            let data: Vec<u8> = (0..n as u8).map(|i| i.wrapping_mul(37)).collect();
            let url = data_url("image/png", &data);
            assert_eq!(parse_data_url(&url).unwrap(), data);
        }
    }

    #[test]
    fn add_dedupes_and_shrinks() {
        let mut files = Map::new();
        let a = add_bytes(&mut files, &png(4, 2)).unwrap();
        let b = add_bytes(&mut files, &png(4, 2)).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(files.len(), 1);
        assert_eq!(files[&a.id]["mimeType"], "image/png");
        let big = add_bytes(&mut files, &png(2000, 1000)).unwrap();
        assert_eq!((big.width, big.height), (MAX_SIDE, MAX_SIDE / 2));
        let img = decode(&files, &big.id).unwrap();
        assert_eq!(img.dimensions(), (MAX_SIDE, MAX_SIDE / 2));
        assert!(add_bytes(&mut files, b"not an image").is_err());
        let el = image_element(&a, 100.0, 100.0, 1000.0);
        assert_eq!((el.x, el.y, el.width, el.height), (98.0, 99.0, 4.0, 2.0));
        assert_eq!(subset(&files, &[el]).len(), 1);
    }
}
