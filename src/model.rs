//! Modelo de dados compatível com o formato `.excalidraw` (JSON).
//!
//! Os campos comuns a todos os elementos são tipados; os campos específicos de
//! cada tipo (pontos, texto, pontas de seta...) e quaisquer campos desconhecidos
//! ficam em `extra`, o que garante que um arquivo aberto e salvo de novo preserve
//! tudo o que o Excalidraw original escreveu.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Aleatoriedade simples (sem dependências) para ids, seeds e nonces
// ---------------------------------------------------------------------------

static RNG_STATE: AtomicU64 = AtomicU64::new(0);

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn rand_u64() -> u64 {
    if RNG_STATE.load(Ordering::Relaxed) == 0 {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42)
            | 1;
        let _ = RNG_STATE.compare_exchange(0, seed, Ordering::Relaxed, Ordering::Relaxed);
    }
    let s = RNG_STATE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    splitmix(s)
}

/// Inteiro aleatório positivo de 31 bits, como o Excalidraw usa para `seed`.
pub fn rand_seed() -> i64 {
    (rand_u64() % (1u64 << 31)) as i64
}

pub fn new_id() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_-";
    (0..21)
        .map(|_| ALPHABET[(rand_u64() % 64) as usize] as char)
        .collect()
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Elemento
// ---------------------------------------------------------------------------

fn d_stroke() -> String {
    "#1e1e1e".into()
}
fn d_transparent() -> String {
    "transparent".into()
}
fn d_hachure() -> String {
    "solid".into()
}
fn d_solid() -> String {
    "solid".into()
}
fn d_one() -> f64 {
    1.0
}
fn d_two() -> f64 {
    2.0
}
fn d_hundred() -> f64 {
    100.0
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    #[serde(default)]
    pub angle: f64,
    #[serde(default = "d_stroke")]
    pub stroke_color: String,
    #[serde(default = "d_transparent")]
    pub background_color: String,
    #[serde(default = "d_hachure")]
    pub fill_style: String,
    #[serde(default = "d_two")]
    pub stroke_width: f64,
    #[serde(default = "d_solid")]
    pub stroke_style: String,
    #[serde(default = "d_one")]
    pub roughness: f64,
    #[serde(default = "d_hundred")]
    pub opacity: f64,
    #[serde(default)]
    pub group_ids: Vec<String>,
    #[serde(default)]
    pub frame_id: Option<String>,
    #[serde(default)]
    pub roundness: Option<Value>,
    #[serde(default)]
    pub seed: i64,
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub version_nonce: i64,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(default)]
    pub bound_elements: Option<Value>,
    #[serde(default)]
    pub updated: i64,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub locked: bool,
    /// Campos específicos do tipo + qualquer campo desconhecido.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Estilo "atual" usado para novos elementos (equivalente ao `currentItem*` do Excalidraw).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub stroke_color: String,
    pub background_color: String,
    pub fill_style: String,
    pub stroke_width: f64,
    pub stroke_style: String,
    pub roughness: f64,
    pub opacity: f64,
    pub font_size: f64,
    pub font_family: i64,
    pub round: bool,
    pub start_arrowhead: Option<String>,
    pub end_arrowhead: Option<String>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            stroke_color: "#1e1e1e".into(),
            background_color: "transparent".into(),
            fill_style: "solid".into(),
            stroke_width: 2.0,
            stroke_style: "solid".into(),
            roughness: 1.0,
            opacity: 100.0,
            font_size: 20.0,
            font_family: 1,
            round: true,
            start_arrowhead: None,
            end_arrowhead: Some("arrow".into()),
        }
    }
}

pub const LINE_HEIGHT: f64 = 1.25;

impl Element {
    pub fn new(kind: &str, x: f64, y: f64, style: &Style) -> Self {
        let mut extra = Map::new();
        let linear = matches!(kind, "line" | "arrow");
        match kind {
            "line" | "arrow" => {
                extra.insert("points".into(), json!([[0.0, 0.0]]));
                extra.insert("lastCommittedPoint".into(), Value::Null);
                extra.insert("startBinding".into(), Value::Null);
                extra.insert("endBinding".into(), Value::Null);
                let (sa, ea) = if kind == "arrow" {
                    (style.start_arrowhead.clone(), style.end_arrowhead.clone())
                } else {
                    (None, None)
                };
                extra.insert("startArrowhead".into(), opt_str(sa));
                extra.insert("endArrowhead".into(), opt_str(ea));
                if kind == "arrow" {
                    extra.insert("elbowed".into(), Value::Bool(false));
                }
            }
            "freedraw" => {
                extra.insert("points".into(), json!([[0.0, 0.0]]));
                extra.insert("pressures".into(), json!([]));
                extra.insert("simulatePressure".into(), Value::Bool(true));
                extra.insert("lastCommittedPoint".into(), Value::Null);
            }
            "text" => {
                extra.insert("text".into(), json!(""));
                extra.insert("fontSize".into(), json!(style.font_size));
                extra.insert("fontFamily".into(), json!(style.font_family));
                extra.insert("textAlign".into(), json!("left"));
                extra.insert("verticalAlign".into(), json!("top"));
                extra.insert("containerId".into(), Value::Null);
                extra.insert("originalText".into(), json!(""));
                extra.insert("autoResize".into(), Value::Bool(true));
                extra.insert("lineHeight".into(), json!(LINE_HEIGHT));
            }
            "image" => {
                extra.insert("fileId".into(), Value::Null);
                extra.insert("status".into(), json!("saved"));
                extra.insert("scale".into(), json!([1.0, 1.0]));
                extra.insert("crop".into(), Value::Null);
            }
            _ => {}
        }
        let roundness = if style.round {
            match kind {
                "rectangle" => Some(json!({"type": 3})),
                "diamond" | "line" | "arrow" => Some(json!({"type": 2})),
                _ => None,
            }
        } else {
            None
        };
        let is_text = kind == "text";
        let is_image = kind == "image";
        Self {
            id: new_id(),
            kind: kind.into(),
            x,
            y,
            width: 0.0,
            height: 0.0,
            angle: 0.0,
            stroke_color: if is_image {
                "transparent".into()
            } else {
                style.stroke_color.clone()
            },
            background_color: if is_text || is_image || linear && kind == "arrow" {
                "transparent".into()
            } else {
                style.background_color.clone()
            },
            fill_style: style.fill_style.clone(),
            stroke_width: style.stroke_width,
            stroke_style: style.stroke_style.clone(),
            roughness: style.roughness,
            opacity: style.opacity,
            group_ids: vec![],
            frame_id: None,
            roundness,
            seed: rand_seed(),
            version: 1,
            version_nonce: rand_seed(),
            is_deleted: false,
            bound_elements: None,
            updated: now_ms(),
            link: None,
            locked: false,
            extra,
        }
    }

    /// Marca o elemento como alterado (necessário para invalidar o cache de desenho).
    pub fn touch(&mut self) {
        self.version += 1;
        self.version_nonce = rand_seed();
        self.updated = now_ms();
    }

    pub fn is_linear(&self) -> bool {
        matches!(self.kind.as_str(), "line" | "arrow")
    }

    pub fn has_points(&self) -> bool {
        matches!(self.kind.as_str(), "line" | "arrow" | "freedraw")
    }

    pub fn points(&self) -> Vec<[f64; 2]> {
        self.extra
            .get("points")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let a = p.as_array()?;
                        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Define pontos relativos a (x, y) e recalcula width/height.
    pub fn set_points(&mut self, pts: Vec<[f64; 2]>) {
        let (mut minx, mut miny, mut maxx, mut maxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for (i, p) in pts.iter().enumerate() {
            if i == 0 {
                (minx, miny, maxx, maxy) = (p[0], p[1], p[0], p[1]);
            } else {
                minx = minx.min(p[0]);
                miny = miny.min(p[1]);
                maxx = maxx.max(p[0]);
                maxy = maxy.max(p[1]);
            }
        }
        self.width = maxx - minx;
        self.height = maxy - miny;
        self.extra.insert(
            "points".into(),
            Value::Array(pts.iter().map(|p| json!([p[0], p[1]])).collect()),
        );
    }

    /// Extensão dos pontos relativa a (x, y): (minx, miny, maxx, maxy).
    pub fn points_extent(&self) -> (f64, f64, f64, f64) {
        let pts = self.points();
        if pts.is_empty() {
            return (0.0, 0.0, self.width, self.height);
        }
        let mut e = (pts[0][0], pts[0][1], pts[0][0], pts[0][1]);
        for p in &pts {
            e.0 = e.0.min(p[0]);
            e.1 = e.1.min(p[1]);
            e.2 = e.2.max(p[0]);
            e.3 = e.3.max(p[1]);
        }
        e
    }

    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.extra.get(key).and_then(|v| v.as_str())
    }

    pub fn num_field(&self, key: &str) -> Option<f64> {
        self.extra.get(key).and_then(|v| v.as_f64())
    }

    pub fn set_field(&mut self, key: &str, v: Value) {
        self.extra.insert(key.into(), v);
    }

    pub fn text(&self) -> &str {
        self.str_field("text").unwrap_or("")
    }

    pub fn set_text(&mut self, t: &str) {
        self.set_field("text", json!(t));
        self.set_field("originalText", json!(t));
    }

    pub fn font_size(&self) -> f64 {
        self.num_field("fontSize").unwrap_or(20.0)
    }

    pub fn font_family(&self) -> i64 {
        self.extra
            .get("fontFamily")
            .and_then(|v| v.as_i64())
            .unwrap_or(1)
    }

    pub fn line_height(&self) -> f64 {
        self.num_field("lineHeight").unwrap_or(LINE_HEIGHT)
    }

    pub fn text_align(&self) -> &str {
        self.str_field("textAlign").unwrap_or("left")
    }

    pub fn arrowhead(&self, end: bool) -> Option<String> {
        let key = if end {
            "endArrowhead"
        } else {
            "startArrowhead"
        };
        self.str_field(key).map(|s| s.to_string())
    }

    pub fn is_round(&self) -> bool {
        self.roundness.as_ref().is_some_and(|v| !v.is_null())
    }
}

fn opt_str(s: Option<String>) -> Value {
    s.map(Value::String).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// Arquivo / cena
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneFile {
    #[serde(rename = "type", default = "d_type")]
    pub kind: String,
    #[serde(default = "d_version")]
    pub version: i64,
    #[serde(default = "d_source")]
    pub source: String,
    #[serde(default)]
    pub elements: Vec<Element>,
    #[serde(default = "d_obj")]
    pub app_state: Value,
    #[serde(default = "d_obj")]
    pub files: Value,
}

fn d_type() -> String {
    "excalidraw".into()
}
fn d_version() -> i64 {
    2
}
fn d_source() -> String {
    "https://github.com/diegofernandes/rustdraw".into()
}
fn d_obj() -> Value {
    json!({})
}

impl SceneFile {
    pub fn new(elements: Vec<Element>, view_background: &str, grid: bool) -> Self {
        Self {
            kind: d_type(),
            version: 2,
            source: d_source(),
            elements,
            app_state: json!({
                "gridSize": 20,
                "gridStep": 5,
                "gridModeEnabled": grid,
                "viewBackgroundColor": view_background,
            }),
            files: json!({}),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("JSON inválido: {e}"))?;
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if !matches!(kind, "excalidraw" | "excalidraw/clipboard" | "") {
            return Err(format!("tipo de arquivo não suportado: {kind}"));
        }
        let mut f: SceneFile =
            serde_json::from_value(v).map_err(|e| format!("estrutura inválida: {e}"))?;
        f.elements.retain(|e| !e.is_deleted);
        Ok(f)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Entradas de `files` como mapa (vazio se o arquivo não tiver).
    pub fn files_map(&self) -> Map<String, Value> {
        self.files.as_object().cloned().unwrap_or_default()
    }

    /// Guarda só os arquivos que algum elemento usa.
    pub fn set_files(&mut self, files: &Map<String, Value>) {
        self.files = Value::Object(crate::files::subset(files, &self.elements));
    }

    pub fn view_background(&self) -> Option<String> {
        self.app_state
            .get("viewBackgroundColor")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn grid_enabled(&self) -> bool {
        self.app_state
            .get("gridModeEnabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

/// JSON no formato da área de transferência do Excalidraw — cola direto no excalidraw.com.
pub fn clipboard_json(elements: &[Element], files: &Map<String, Value>) -> String {
    serde_json::to_string(&json!({
        "type": "excalidraw/clipboard",
        "elements": elements,
        "files": crate::files::subset(files, elements),
    }))
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Cores
// ---------------------------------------------------------------------------

/// Converte uma cor CSS simples em RGBA. `None` = transparente.
pub fn parse_color(s: &str) -> Option<[u8; 4]> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("transparent") || s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix('#') {
        let h = |i: usize, len: usize| u8::from_str_radix(&hex[i..i + len], 16).ok();
        return match hex.len() {
            3 | 4 => {
                let c = |i| h(i, 1).map(|v| v * 17);
                Some([
                    c(0)?,
                    c(1)?,
                    c(2)?,
                    if hex.len() == 4 { c(3)? } else { 255 },
                ])
            }
            6 => Some([h(0, 2)?, h(2, 2)?, h(4, 2)?, 255]),
            8 => Some([h(0, 2)?, h(2, 2)?, h(4, 2)?, h(6, 2)?]),
            _ => None,
        };
    }
    if let Some(inner) = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<f64> = inner
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if parts.len() >= 3 {
            let a = parts
                .get(3)
                .map(|a| (a * 255.0).round() as u8)
                .unwrap_or(255);
            return Some([parts[0] as u8, parts[1] as u8, parts[2] as u8, a]);
        }
        return None;
    }
    let named = match s.to_ascii_lowercase().as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "orange" => [255, 165, 0],
        "gray" | "grey" => [128, 128, 128],
        "purple" => [128, 0, 128],
        "pink" => [255, 192, 203],
        _ => return Some([0, 0, 0, 255]),
    };
    Some([named[0], named[1], named[2], 255])
}

pub fn color_to_hex(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

/// Filtro do modo escuro do Excalidraw: `invert(93%) hue-rotate(180deg)`.
pub fn dark_filter(c: [u8; 4]) -> [u8; 4] {
    let inv = |v: u8| {
        let v = v as f32 / 255.0;
        0.93 * (1.0 - v) + 0.07 * v
    };
    let (r, g, b) = (inv(c[0]), inv(c[1]), inv(c[2]));
    let nr = -0.574 * r + 1.43 * g + 0.144 * b;
    let ng = 0.426 * r + 0.43 * g + 0.144 * b;
    let nb = 0.426 * r + 1.43 * g - 0.856 * b;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [q(nr), q(ng), q(nb), c[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_roundtrip_preserves_unknown_fields() {
        let src = r##"{
          "type": "excalidraw", "version": 2, "source": "https://excalidraw.com",
          "elements": [
            {"id":"a","type":"rectangle","x":10,"y":20,"width":100,"height":50,"angle":0,
             "strokeColor":"#1e1e1e","backgroundColor":"transparent","fillStyle":"solid",
             "strokeWidth":2,"strokeStyle":"solid","roughness":1,"opacity":100,"groupIds":[],
             "frameId":null,"roundness":{"type":3},"seed":1,"version":3,"versionNonce":4,
             "isDeleted":false,"boundElements":null,"updated":1,"link":null,"locked":false,
             "index":"a0","customData":{"foo":1}},
            {"id":"b","type":"arrow","x":0,"y":0,"width":10,"height":10,"points":[[0,0],[10,10]],
             "startArrowhead":null,"endArrowhead":"arrow","isDeleted":false},
            {"id":"c","type":"text","x":0,"y":0,"text":"oi","fontSize":20,"fontFamily":1,"isDeleted":true}
          ],
          "appState": {"viewBackgroundColor":"#ffffff","gridSize":null},
          "files": {}
        }"##;
        let f = SceneFile::parse(src).unwrap();
        assert_eq!(f.elements.len(), 2, "elementos apagados são descartados");
        assert_eq!(f.elements[0].extra["index"], json!("a0"));
        assert_eq!(f.elements[1].points(), vec![[0.0, 0.0], [10.0, 10.0]]);
        let out = f.to_json();
        let back = SceneFile::parse(&out).unwrap();
        assert_eq!(back.elements, f.elements);
        assert!(out.contains("\"customData\""));
        assert!(out.contains("\"startArrowhead\": null"));
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_color("#1e1e1e"), Some([30, 30, 30, 255]));
        assert_eq!(parse_color("transparent"), None);
        assert_eq!(dark_filter([255, 255, 255, 255]), [18, 18, 18, 255]);
    }
}
