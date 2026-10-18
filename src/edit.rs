//! Edição da cena por comandos (usada pelo servidor MCP).
//!
//! Os elementos são descritos em JSON com os mesmos nomes de campo do
//! `.excalidraw` (`strokeColor`, `fontSize`...), com algumas facilidades:
//! `angle` em graus, `rounded` em vez de `roundness`, `label` para escrever um
//! texto centralizado dentro de uma forma e `start`/`end` para ligar as pontas
//! de uma seta em formas pelo id.

use crate::binding::{self, ANCHORS, Binding};
use crate::files;
use crate::fonts::{self, Family};
use crate::geom;
use crate::model::{Element, SceneFile, Style, new_id};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::Path;

const KINDS: [&str; 8] = [
    "rectangle",
    "diamond",
    "ellipse",
    "text",
    "line",
    "arrow",
    "freedraw",
    "image",
];
const ANCHOR_NAMES: [&str; 4] = ["top", "right", "bottom", "left"];

pub fn load(path: &Path) -> Result<SceneFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    SceneFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Grava num arquivo temporário e renomeia, para nunca deixar o arquivo pela metade.
pub fn save(path: &Path, scene: &SceneFile) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, scene.to_json()).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Leitura dos campos
// ---------------------------------------------------------------------------

type Spec = Map<String, Value>;

fn num(s: &Spec, k: &str) -> Result<Option<f64>, String> {
    match s.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .map(Some)
            .ok_or_else(|| format!("`{k}` precisa ser um número")),
    }
}

fn string<'a>(s: &'a Spec, k: &str) -> Result<Option<&'a str>, String> {
    match s.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("`{k}` precisa ser um texto")),
    }
}

fn boolean(s: &Spec, k: &str) -> Result<Option<bool>, String> {
    match s.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("`{k}` precisa ser true ou false")),
    }
}

fn one_of<'a>(s: &'a Spec, k: &str, allowed: &[&str]) -> Result<Option<&'a str>, String> {
    let v = string(s, k)?;
    match v {
        Some(v) if !allowed.contains(&v) => Err(format!(
            "`{k}` inválido: {v:?} (use {})",
            allowed.join(", ")
        )),
        _ => Ok(v),
    }
}

fn points(s: &Spec) -> Result<Option<Vec<[f64; 2]>>, String> {
    let Some(v) = s.get("points").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let err = || "`points` precisa ser uma lista de pares [x, y]".to_string();
    let pts = v
        .as_array()
        .ok_or_else(err)?
        .iter()
        .map(|p| {
            let a = p.as_array().filter(|a| a.len() == 2).ok_or_else(err)?;
            Ok([
                a[0].as_f64().ok_or_else(err)?,
                a[1].as_f64().ok_or_else(err)?,
            ])
        })
        .collect::<Result<Vec<_>, String>>()?;
    if pts.len() < 2 {
        return Err("`points` precisa de pelo menos 2 pontos".into());
    }
    Ok(Some(pts))
}

fn font_family(s: &Spec) -> Result<Option<i64>, String> {
    match s.get("fontFamily") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(f)) => match f.as_str() {
            "hand" => Ok(Some(1)),
            "normal" => Ok(Some(2)),
            "code" => Ok(Some(3)),
            _ => Err(format!(
                "`fontFamily` inválido: {f:?} (use hand, normal ou code)"
            )),
        },
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| "`fontFamily` precisa ser hand, normal, code ou um número".into()),
    }
}

// ---------------------------------------------------------------------------
// Resumo legível de um elemento
// ---------------------------------------------------------------------------

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn label_of<'a>(els: &'a [Element], container: &str) -> Option<&'a Element> {
    els.iter()
        .find(|e| e.kind == "text" && e.str_field("containerId") == Some(container))
}

fn end_summary(el: &Element, end: bool) -> Value {
    match binding::get(el, end) {
        Some(b) => {
            let anchor = b
                .fixed
                .and_then(|f| {
                    ANCHORS
                        .iter()
                        .position(|a| (a[0] - f[0]).abs() < 0.01 && (a[1] - f[1]).abs() < 0.01)
                        .map(|i| json!(ANCHOR_NAMES[i]))
                        .or(Some(json!([r2(f[0]), r2(f[1])])))
                })
                .unwrap_or(json!("center"));
            json!({"element": b.element_id, "anchor": anchor})
        }
        None => Value::Null,
    }
}

pub fn summary(el: &Element, all: &[Element]) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(el.id));
    o.insert("type".into(), json!(el.kind));
    for (k, v) in [
        ("x", el.x),
        ("y", el.y),
        ("width", el.width),
        ("height", el.height),
    ] {
        o.insert(k.into(), json!(r2(v)));
    }
    if el.angle != 0.0 {
        o.insert("angle".into(), json!(r2(el.angle.to_degrees())));
    }
    o.insert("strokeColor".into(), json!(el.stroke_color));
    if el.kind != "text" {
        o.insert("backgroundColor".into(), json!(el.background_color));
        o.insert("fillStyle".into(), json!(el.fill_style));
        o.insert("strokeWidth".into(), json!(el.stroke_width));
        o.insert("strokeStyle".into(), json!(el.stroke_style));
        o.insert("roughness".into(), json!(el.roughness));
    }
    if el.opacity != 100.0 {
        o.insert("opacity".into(), json!(el.opacity));
    }
    if matches!(el.kind.as_str(), "rectangle" | "diamond" | "line" | "arrow") {
        o.insert("rounded".into(), json!(el.is_round()));
    }
    match el.kind.as_str() {
        "text" => {
            o.insert("text".into(), json!(el.text()));
            o.insert("fontSize".into(), json!(el.font_size()));
            o.insert("fontFamily".into(), json!(el.font_family()));
            o.insert("textAlign".into(), json!(el.text_align()));
            if let Some(c) = el.str_field("containerId") {
                o.insert("containerId".into(), json!(c));
            }
        }
        "image" => {
            o.insert("fileId".into(), json!(el.str_field("fileId")));
        }
        "line" | "arrow" | "freedraw" => {
            let pts: Vec<Value> = el
                .points()
                .iter()
                .map(|p| json!([r2(p[0]), r2(p[1])]))
                .collect();
            o.insert("points".into(), Value::Array(pts));
            if el.is_linear() {
                o.insert("startArrowhead".into(), json!(el.arrowhead(false)));
                o.insert("endArrowhead".into(), json!(el.arrowhead(true)));
            }
            if el.kind == "arrow" {
                o.insert("start".into(), end_summary(el, false));
                o.insert("end".into(), end_summary(el, true));
            }
        }
        _ => {
            if let Some(l) = label_of(all, &el.id) {
                o.insert("label".into(), json!(l.text()));
            }
        }
    }
    if !el.group_ids.is_empty() {
        o.insert("groupIds".into(), json!(el.group_ids));
    }
    if el.locked {
        o.insert("locked".into(), json!(true));
    }
    if let Some(l) = &el.link {
        o.insert("link".into(), json!(l));
    }
    Value::Object(o)
}

// ---------------------------------------------------------------------------
// Aplicação dos campos
// ---------------------------------------------------------------------------

/// Campos de estilo e posição comuns a criar e alterar.
fn apply_common(el: &mut Element, s: &Spec) -> Result<(), String> {
    if let Some(v) = num(s, "x")? {
        el.x = v;
    }
    if let Some(v) = num(s, "y")? {
        el.y = v;
    }
    if !el.has_points() && el.kind != "text" {
        if let Some(v) = num(s, "width")? {
            el.width = v.abs().max(1.0);
        }
        if let Some(v) = num(s, "height")? {
            el.height = v.abs().max(1.0);
        }
    }
    if let Some(v) = num(s, "angle")? {
        el.angle = v.to_radians().rem_euclid(std::f64::consts::TAU);
    }
    if let Some(v) = string(s, "strokeColor")? {
        el.stroke_color = v.into();
    }
    if let Some(v) = string(s, "backgroundColor")? {
        el.background_color = v.into();
    }
    if let Some(v) = one_of(s, "fillStyle", &["hachure", "cross-hatch", "solid"])? {
        el.fill_style = v.into();
    }
    if let Some(v) = num(s, "strokeWidth")? {
        el.stroke_width = v.max(0.5);
    }
    if let Some(v) = one_of(s, "strokeStyle", &["solid", "dashed", "dotted"])? {
        el.stroke_style = v.into();
    }
    if let Some(v) = num(s, "roughness")? {
        el.roughness = v.clamp(0.0, 2.0);
    }
    if let Some(v) = num(s, "opacity")? {
        el.opacity = v.clamp(0.0, 100.0);
    }
    if let Some(v) = boolean(s, "rounded")? {
        el.roundness = match (v, el.kind.as_str()) {
            (true, "rectangle") => Some(json!({"type": 3})),
            (true, "diamond" | "line" | "arrow") => Some(json!({"type": 2})),
            _ => None,
        };
    }
    if let Some(v) = boolean(s, "locked")? {
        el.locked = v;
    }
    if s.contains_key("link") {
        el.link = string(s, "link")?.map(String::from);
    }
    if let Some(g) = s.get("groupIds").filter(|v| !v.is_null()) {
        el.group_ids = serde_json::from_value(g.clone())
            .map_err(|_| "`groupIds` precisa ser uma lista de textos")?;
    }
    if el.kind == "text" {
        if let Some(v) = num(s, "fontSize")? {
            el.set_field("fontSize", json!(v.max(1.0)));
        }
        if let Some(v) = font_family(s)? {
            el.set_field("fontFamily", json!(v));
        }
        if let Some(v) = one_of(s, "textAlign", &["left", "center", "right"])? {
            el.set_field("textAlign", json!(v));
        }
        if let Some(v) = string(s, "text")? {
            el.set_text(v);
        }
    }
    if el.is_linear() {
        for (k, end) in [("startArrowhead", false), ("endArrowhead", true)] {
            if s.contains_key(k) {
                let v = one_of(s, k, &["arrow", "triangle", "bar", "dot"])?;
                let key = if end {
                    "endArrowhead"
                } else {
                    "startArrowhead"
                };
                el.set_field(key, json!(v));
            }
        }
    }
    Ok(())
}

/// Mede o texto com as fontes embutidas e ajusta largura e altura.
fn measure(el: &mut Element) {
    let f = fonts::raster_fonts();
    let fam = Family::from_excalidraw(el.font_family());
    let size = el.font_size();
    let text = el.text().to_string();
    let w = text
        .split('\n')
        .map(|l| f.line_width(fam, l, size as f32) as f64)
        .fold(0.0, f64::max);
    el.width = w.ceil().max(1.0);
    el.height = text.split('\n').count() as f64 * size * el.line_height();
}

/// Ponta de seta: `{"element": id, "anchor": ...}` ou `{"x": .., "y": ..}`.
enum EndSpec {
    Free(geom::P),
    Bound(Binding),
}

fn end_spec(v: &Value, key: &str, els: &[Element]) -> Result<EndSpec, String> {
    let o = v
        .as_object()
        .ok_or_else(|| format!("`{key}` precisa ser {{\"element\": id}} ou {{\"x\", \"y\"}}"))?;
    if let Some(id) = string(o, "element")? {
        let target = els
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("`{key}`: elemento {id:?} não existe"))?;
        if !binding::is_bindable(target) {
            return Err(format!(
                "`{key}`: setas só ligam em retângulo, losango, elipse ou texto solto ({id:?} é {})",
                target.kind
            ));
        }
        let fixed = match one_of(o, "anchor", &["center", "top", "right", "bottom", "left"])? {
            None | Some("center") => None,
            Some(a) => Some(ANCHORS[ANCHOR_NAMES.iter().position(|n| *n == a).unwrap()]),
        };
        return Ok(EndSpec::Bound(Binding::new(id, fixed)));
    }
    match (num(o, "x")?, num(o, "y")?) {
        (Some(x), Some(y)) => Ok(EndSpec::Free(geom::P::new(x as f32, y as f32))),
        _ => Err(format!(
            "`{key}` precisa de `element` (id da forma) ou de `x` e `y`"
        )),
    }
}

fn end_pos(e: &EndSpec, els: &[Element]) -> geom::P {
    match e {
        EndSpec::Free(p) => *p,
        EndSpec::Bound(b) => {
            let t = els.iter().find(|e| e.id == b.element_id).unwrap();
            match b.fixed {
                Some(f) => binding::anchor_world(t, f),
                None => geom::center(t),
            }
        }
    }
}

/// Pontos e ligações de linhas/setas (`points`, `start`, `end`).
fn apply_linear(el: &mut Element, s: &Spec, els: &[Element], creating: bool) -> Result<(), String> {
    let new_pts = points(s)?;
    let mut ends = [None, None];
    for (i, key) in ["start", "end"].into_iter().enumerate() {
        match s.get(key) {
            None => {}
            Some(Value::Null) => ends[i] = Some(None),
            Some(v) => ends[i] = Some(Some(end_spec(v, key, els)?)),
        }
    }
    let binds = ends
        .iter()
        .flatten()
        .flatten()
        .any(|e| matches!(e, EndSpec::Bound(_)));
    if binds && el.kind != "arrow" {
        return Err("só setas (`arrow`) podem ligar as pontas em formas".into());
    }
    if el.kind == "freedraw" && ends.iter().any(Option::is_some) {
        return Err("`start`/`end` só valem para linhas e setas".into());
    }
    let mut world: Vec<geom::P> = match &new_pts {
        Some(p) => p
            .iter()
            .map(|p| geom::P::new((el.x + p[0]) as f32, (el.y + p[1]) as f32))
            .collect(),
        None if creating => {
            let (Some(Some(_)), Some(Some(_))) = (&ends[0], &ends[1]) else {
                return Err(format!(
                    "`{}` precisa de `points` ou de `start` e `end`",
                    el.kind
                ));
            };
            vec![geom::P::new(0.0, 0.0); 2]
        }
        None => geom::abs_points(el),
    };
    if world.len() < 2 {
        return Err(format!("`{}` precisa de pelo menos 2 pontos", el.kind));
    }
    let n = world.len();
    for (i, e) in ends.iter().enumerate() {
        let idx = if i == 0 { 0 } else { n - 1 };
        if let Some(Some(e)) = e {
            world[idx] = end_pos(e, els);
        }
        if el.kind == "arrow"
            && let Some(e) = e
        {
            let b = match e {
                Some(EndSpec::Bound(b)) => Some(b),
                _ => None,
            };
            binding::set(el, i == 1, b);
        }
    }
    if new_pts.is_some() || ends.iter().any(Option::is_some) {
        el.angle = 0.0;
        el.x = world[0].x as f64;
        el.y = world[0].y as f64;
        el.set_points(
            world
                .iter()
                .map(|p| [(p.x - world[0].x) as f64, (p.y - world[0].y) as f64])
                .collect(),
        );
    }
    Ok(())
}

fn spec_of(v: &Value, i: usize) -> Result<&Spec, String> {
    v.as_object()
        .ok_or_else(|| format!("item {i}: precisa ser um objeto"))
}

/// Cria, troca ou remove o rótulo (texto centralizado) de uma forma.
fn set_label(
    els: &mut Vec<Element>,
    container: &str,
    text: Option<&str>,
    s: &Spec,
) -> Result<(), String> {
    let existing = els
        .iter()
        .position(|e| e.kind == "text" && e.str_field("containerId") == Some(container));
    let Some(text) = text.filter(|t| !t.is_empty()) else {
        if let Some(i) = existing {
            els.remove(i);
        }
        return Ok(());
    };
    let ci = els.iter().position(|e| e.id == container).unwrap();
    let i = match existing {
        Some(i) => i,
        None => {
            let shape = &els[ci];
            let mut t = Element::new("text", shape.x, shape.y, &Style::default());
            t.stroke_color = shape.stroke_color.clone();
            t.opacity = shape.opacity;
            t.set_field("textAlign", json!("center"));
            t.set_field("verticalAlign", json!("middle"));
            t.set_field("containerId", json!(container));
            // agrupado com a forma, para que no editor os dois andem juntos
            let group = new_id();
            t.group_ids = std::iter::once(group.clone())
                .chain(shape.group_ids.iter().cloned())
                .collect();
            let shape = &mut els[ci];
            shape.group_ids.insert(0, group);
            let mut bound = shape
                .bound_elements
                .as_ref()
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            bound.push(json!({"id": t.id, "type": "text"}));
            shape.bound_elements = Some(Value::Array(bound));
            shape.touch();
            els.insert(ci + 1, t);
            ci + 1
        }
    };
    let t = &mut els[i];
    t.set_text(text);
    let mut style = Spec::new();
    for k in ["fontSize", "fontFamily"] {
        if let Some(v) = s.get(k) {
            style.insert(k.into(), v.clone());
        }
    }
    apply_common(t, &style)?;
    t.touch();
    Ok(())
}

/// Recoloca o rótulo de `container` no centro dela.
fn center_label(els: &mut [Element], container: &str) {
    let Some(c) = els.iter().find(|e| e.id == container) else {
        return;
    };
    let (center, angle) = (geom::center(c), c.angle);
    if let Some(t) = els
        .iter_mut()
        .find(|e| e.kind == "text" && e.str_field("containerId") == Some(container))
    {
        measure(t);
        t.x = center.x as f64 - t.width / 2.0;
        t.y = center.y as f64 - t.height / 2.0;
        t.angle = angle;
        t.touch();
    }
}

/// Depois de mudanças: arruma as ligações, recentraliza rótulos e faz as
/// setas ligadas acompanharem as formas alteradas.
fn finish(els: &mut [Element], changed: &HashSet<String>) {
    binding::tidy(els);
    let containers: Vec<String> = els
        .iter()
        .filter(|e| e.kind == "text")
        .filter_map(|e| e.str_field("containerId").map(String::from))
        .filter(|c| els.iter().any(|e| e.id == *c))
        .collect();
    for c in containers {
        let label_changed = label_of(els, &c).is_some_and(|l| changed.contains(&l.id));
        if changed.contains(&c) || label_changed {
            center_label(els, &c);
        }
    }
    let arrows: Vec<String> = els
        .iter()
        .filter(|e| e.kind == "arrow")
        .filter(|e| {
            changed.contains(&e.id)
                || [false, true].iter().any(|end| {
                    binding::get(e, *end).is_some_and(|b| changed.contains(&b.element_id))
                })
        })
        .map(|e| e.id.clone())
        .collect();
    for id in arrows {
        binding::refresh_arrow(els, &id);
    }
}

// ---------------------------------------------------------------------------
// Operações
// ---------------------------------------------------------------------------

/// Imagem de um item `image`: o arquivo em `path` (guardado em `files`) ou um
/// `fileId` que já está na cena. Devolve o id e o tamanho natural.
fn image_source(files: &mut Map<String, Value>, s: &Spec) -> Result<files::Added, String> {
    if let Some(path) = string(s, "path")? {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        return files::add_bytes(files, &bytes).map_err(|e| format!("{path}: {e}"));
    }
    let id = string(s, "fileId")?.ok_or("imagem precisa de `path` (arquivo PNG/JPG)")?;
    let img = files::decode(files, id).ok_or_else(|| format!("fileId {id:?} não está na cena"))?;
    Ok(files::Added {
        id: id.into(),
        width: img.width(),
        height: img.height(),
    })
}

/// Adiciona elementos (na ordem dada, por cima dos existentes). Devolve os ids.
/// Imagens novas entram em `files`.
pub fn add(
    els: &mut Vec<Element>,
    files: &mut Map<String, Value>,
    specs: &[Value],
) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for (i, v) in specs.iter().enumerate() {
        let s = spec_of(v, i)?;
        let ctx = |e: String| format!("item {i}: {e}");
        let kind = one_of(s, "type", &KINDS)
            .map_err(ctx)?
            .ok_or_else(|| ctx(format!("falta `type` ({})", KINDS.join(", "))))?;
        let mut el = Element::new(kind, 0.0, 0.0, &Style::default());
        if let Some(id) = string(s, "id").map_err(ctx)? {
            if id.is_empty() || els.iter().any(|e| e.id == id) {
                return Err(ctx(format!("id {id:?} vazio ou já usado")));
            }
            el.id = id.into();
        }
        if matches!(kind, "rectangle" | "diamond" | "ellipse") {
            el.width = 160.0;
            el.height = 80.0;
        }
        let image = if kind == "image" {
            let a = image_source(files, s).map_err(ctx)?;
            let mut img = files::image_element(&a, 0.0, 0.0, f64::MAX);
            img.id = el.id.clone();
            (img.x, img.y) = (0.0, 0.0);
            el = img;
            Some((a.width as f64, a.height as f64))
        } else {
            None
        };
        apply_common(&mut el, s).map_err(ctx)?;
        if let Some((nw, nh)) = image {
            // só um dos lados dado: o outro segue a proporção da imagem
            match (s.contains_key("width"), s.contains_key("height")) {
                (true, false) => el.height = (el.width * nh / nw).max(1.0),
                (false, true) => el.width = (el.height * nw / nh).max(1.0),
                _ => {}
            }
        }
        if kind == "text" {
            if el.text().is_empty() {
                return Err(ctx("texto precisa de `text`".into()));
            }
            measure(&mut el);
            match el.text_align() {
                "center" => el.x -= el.width / 2.0,
                "right" => el.x -= el.width,
                _ => {}
            }
        }
        if el.has_points() {
            apply_linear(&mut el, s, els, true).map_err(ctx)?;
        }
        let id = el.id.clone();
        els.push(el);
        if let Some(label) = string(s, "label").map_err(ctx)? {
            if !matches!(kind, "rectangle" | "diamond" | "ellipse") {
                return Err(ctx(
                    "`label` só vale para retângulo, losango e elipse".into()
                ));
            }
            set_label(els, &id, Some(label), s).map_err(ctx)?;
        }
        ids.push(id);
    }
    finish(els, &ids.iter().cloned().collect());
    Ok(ids)
}

/// Altera elementos existentes; cada item tem `id` e os campos a mudar.
pub fn update(els: &mut Vec<Element>, specs: &[Value]) -> Result<Vec<String>, String> {
    let mut changed = Vec::new();
    for (i, v) in specs.iter().enumerate() {
        let s = spec_of(v, i)?;
        let ctx = |e: String| format!("item {i}: {e}");
        let id = string(s, "id")
            .map_err(ctx)?
            .ok_or_else(|| ctx("falta `id`".into()))?
            .to_string();
        let idx = els
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| ctx(format!("elemento {id:?} não existe")))?;
        if s.get("type")
            .is_some_and(|t| t.as_str() != Some(&els[idx].kind))
        {
            return Err(ctx("não dá para mudar o `type` de um elemento".into()));
        }
        let mut el = els[idx].clone();
        let anchor_center = (el.kind == "text").then(|| el.x + el.width / 2.0);
        apply_common(&mut el, s).map_err(ctx)?;
        if el.kind == "text" && el.str_field("containerId").is_none() {
            measure(&mut el);
            if el.text_align() == "center" && num(s, "x").map_err(ctx)?.is_none() {
                el.x = anchor_center.unwrap_or(el.x) - el.width / 2.0;
            }
        }
        if el.has_points() {
            apply_linear(&mut el, s, els, false).map_err(ctx)?;
        }
        el.touch();
        els[idx] = el;
        if s.contains_key("label") {
            if !matches!(els[idx].kind.as_str(), "rectangle" | "diamond" | "ellipse") {
                return Err(ctx(
                    "`label` só vale para retângulo, losango e elipse".into()
                ));
            }
            let label = string(s, "label").map_err(ctx)?.map(String::from);
            set_label(els, &id, label.as_deref(), s).map_err(ctx)?;
        }
        changed.push(id);
    }
    finish(els, &changed.iter().cloned().collect());
    Ok(changed)
}

/// Apaga elementos (e os rótulos deles). Devolve quantos saíram.
pub fn delete(els: &mut Vec<Element>, ids: &[String]) -> Result<usize, String> {
    if let Some(id) = ids.iter().find(|id| !els.iter().any(|e| e.id == **id)) {
        return Err(format!("elemento {id:?} não existe"));
    }
    let gone: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let before = els.len();
    els.retain(|e| {
        !gone.contains(e.id.as_str())
            && !e.str_field("containerId").is_some_and(|c| gone.contains(c))
    });
    let removed = before - els.len();
    for el in els.iter_mut() {
        if let Some(Value::Array(b)) = &el.bound_elements {
            let kept: Vec<Value> = b
                .iter()
                .filter(|v| {
                    !v.get("id")
                        .and_then(|i| i.as_str())
                        .is_some_and(|i| gone.contains(i))
                })
                .cloned()
                .collect();
            if kept.len() != b.len() {
                el.bound_elements = Some(Value::Array(kept));
                el.touch();
            }
        }
    }
    finish(els, &HashSet::new());
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Vec<Element> {
        let mut els = Vec::new();
        add(
            &mut els,
            &mut Map::new(),
            &[
                json!({"type": "rectangle", "id": "a", "x": 0, "y": 0, "width": 100, "height": 60, "label": "Cliente"}),
                json!({"type": "ellipse", "id": "b", "x": 300, "y": 0, "width": 100, "height": 60}),
                json!({"type": "arrow", "id": "s", "start": {"element": "a"}, "end": {"element": "b", "anchor": "left"}}),
            ],
        )
        .unwrap();
        els
    }

    #[test]
    fn arrow_binds_and_clips_to_shapes() {
        let els = scene();
        let s = els.iter().find(|e| e.id == "s").unwrap();
        let pts = geom::abs_points(s);
        // começa na borda direita do retângulo (100) + folga, termina no ponto fixo da elipse
        assert!((pts[0].x - 105.0).abs() < 0.5, "{pts:?}");
        assert!((pts[1].x - 295.0).abs() < 0.5, "{pts:?}");
        assert!((pts[1].y - 30.0).abs() < 0.5, "{pts:?}");
        let a = els.iter().find(|e| e.id == "a").unwrap();
        let bound = a.bound_elements.as_ref().unwrap().to_string();
        assert!(
            bound.contains("\"s\"") && bound.contains("\"text\""),
            "{bound}"
        );
    }

    #[test]
    fn label_follows_shape_and_arrow_follows_too() {
        let mut els = scene();
        update(&mut els, &[json!({"id": "a", "y": 200})]).unwrap();
        let label = label_of(&els, "a").unwrap();
        assert!((geom::center(label).y - 230.0).abs() < 0.5);
        assert_eq!(label.text(), "Cliente");
        let s = els.iter().find(|e| e.id == "s").unwrap();
        assert!(
            geom::abs_points(s)[0].y > 150.0,
            "a seta saiu de onde a forma estava"
        );
        let a = els.iter().find(|e| e.id == "a").unwrap();
        assert_eq!(a.group_ids, label.group_ids);
    }

    #[test]
    fn delete_removes_label_and_bindings() {
        let mut els = scene();
        assert_eq!(delete(&mut els, &["a".into()]).unwrap(), 2);
        let s = els.iter().find(|e| e.id == "s").unwrap();
        assert!(binding::get(s, false).is_none());
        assert!(binding::get(s, true).is_some());
        assert!(delete(&mut els, &["nada".into()]).is_err());
    }

    #[test]
    fn errors_are_reported_and_text_is_measured() {
        let mut els = Vec::new();
        let e = add(&mut els, &mut Map::new(), &[json!({"type": "star"})]).unwrap_err();
        assert!(e.contains("item 0"), "{e}");
        assert!(add(&mut els, &mut Map::new(), &[json!({"type": "arrow"})]).is_err());
        assert!(
            add(
                &mut els,
                &mut Map::new(),
                &[json!({"type": "line", "start": {"x": 0, "y": 0}, "end": {"element": "x"}})]
            )
            .is_err()
        );
        let ids = add(
            &mut els,
            &mut Map::new(),
            &[json!({"type": "text", "text": "Olá\nmundo", "fontSize": 20})],
        )
        .unwrap();
        let t = els.iter().find(|e| e.id == ids[0]).unwrap();
        assert!(t.width > 10.0);
        assert_eq!(t.height, 50.0);
        let sum = summary(t, &els);
        assert_eq!(sum["text"], json!("Olá\nmundo"));
    }

    #[test]
    fn adds_images_from_a_path() {
        let dir = std::env::temp_dir().join(format!("rustdraw-img-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([0, 0, 255, 255]));
        std::fs::write(&path, files::encode_png(&img).unwrap()).unwrap();
        let mut els = Vec::new();
        let mut fs = Map::new();
        let p = path.to_str().unwrap();
        let ids = add(
            &mut els,
            &mut fs,
            &[
                json!({"type": "image", "path": p, "x": 10, "y": 5}),
                json!({"type": "image", "path": p, "width": 100}),
            ],
        )
        .unwrap();
        assert_eq!(fs.len(), 1, "a mesma imagem entra uma vez só");
        let (a, b) = (&els[0], &els[1]);
        assert_eq!((a.x, a.y, a.width, a.height), (10.0, 5.0, 40.0, 20.0));
        assert_eq!((b.width, b.height), (100.0, 50.0));
        assert_eq!(a.str_field("fileId"), b.str_field("fileId"));
        assert_eq!(summary(a, &els)["fileId"], json!(fs.keys().next().unwrap()));
        let fid = a.str_field("fileId").unwrap().to_string();
        add(
            &mut els,
            &mut fs,
            &[json!({"type": "image", "fileId": fid})],
        )
        .unwrap();
        assert!(add(&mut els, &mut fs, &[json!({"type": "image"})]).is_err());
        assert_eq!(ids.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
