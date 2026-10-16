//! Estado do editor, entrada de ponteiro/teclado e ações.

use crate::export::{self, ExportOpts};
use crate::geom::{self, P, Prim, Rect, rotate};
use crate::model::{Element, SceneFile, Style, clipboard_json, new_id, rand_seed};
use crate::render::{self, Cam, PaintCtx};
use egui::{Color32, CursorIcon, Key, Modifiers, PointerButton, Pos2, Sense, Stroke, StrokeKind};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub const SELECTION: Color32 = Color32::from_rgb(0x69, 0x65, 0xdb);
const HANDLE_PX: f32 = 7.0;
const GRID: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Hand,
    Select,
    Rect,
    Diamond,
    Ellipse,
    Arrow,
    Line,
    Draw,
    Text,
    Eraser,
}

impl Tool {
    pub fn kind(self) -> Option<&'static str> {
        Some(match self {
            Tool::Rect => "rectangle",
            Tool::Diamond => "diamond",
            Tool::Ellipse => "ellipse",
            Tool::Arrow => "arrow",
            Tool::Line => "line",
            Tool::Draw => "freedraw",
            Tool::Text => "text",
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
    Rotate,
    Point(usize),
}

pub enum Drag {
    None,
    Pan {
        last: Pos2,
    },
    Create {
        id: String,
        origin: P,
        before: Vec<Element>,
    },
    LinearMulti {
        id: String,
        before: Vec<Element>,
    },
    FreeDraw {
        id: String,
        before: Vec<Element>,
    },
    Move {
        start: P,
        originals: Vec<(String, f64, f64)>,
        before: Vec<Element>,
        moved: bool,
    },
    BoxSelect {
        start: P,
        cur: P,
        base: HashSet<String>,
    },
    Resize {
        handle: Handle,
        orig: Element,
        before: Vec<Element>,
    },
    Rotate {
        orig: Element,
        before: Vec<Element>,
    },
    Erase {
        last: P,
        marked: HashSet<String>,
    },
}

pub struct TextEditing {
    pub id: String,
    pub buf: String,
    pub before: Vec<Element>,
    pub focus: bool,
    /// Centro fixo para texto centralizado (criado com duplo clique numa forma).
    pub anchor_center: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    New,
    Open,
    /// Limpar o canvas (Ctrl+Delete).
    Clear,
}

struct Cached {
    version: i64,
    nonce: i64,
    prims: Vec<Prim>,
    bounds: Rect,
}

pub struct App {
    pub elements: Vec<Element>,
    pub selected: HashSet<String>,
    pub tool: Tool,
    pub tool_locked: bool,
    /// Ferramenta anterior, para H e E alternarem de volta (como no Excalidraw).
    prev_tool: Tool,
    pub style: Style,
    pub cam: Cam,
    pub drag: Drag,
    undo: Vec<Vec<Element>>,
    redo: Vec<Vec<Element>>,
    cache: HashMap<String, Cached>,
    pub editing: Option<TextEditing>,
    pub file_path: Option<PathBuf>,
    pub dirty: bool,
    pub dark: bool,
    pub grid: bool,
    pub view_bg: String,
    pub show_menu: bool,
    pub show_export: bool,
    pub show_help: bool,
    pub show_stats: bool,
    pub zen_mode: bool,
    pub view_mode: bool,
    /// Abre o seletor de cor do traço (`Some(false)`) ou do fundo (`Some(true)`) — atalhos S e G.
    pub open_picker: Option<bool>,
    /// Estilo copiado com Ctrl+Alt+C.
    copied_style: Option<Element>,
    pub export_opts: ExportOpts,
    pub export_selection_only: bool,
    pub export_preview: Option<(String, egui::TextureHandle)>,
    pub confirm: Option<Pending>,
    pub toast: Option<(String, f64)>,
    pub hand_ok: bool,
    pointer_world: P,
    last_change: f64,
    autosave_pending: bool,
    last_title: String,
    prop_coalesce: Option<(&'static str, f64)>,
    pub now: f64,
    pending_fit: bool,
    needs_measure: HashSet<String>,
    last_prefs: Option<(bool, bool, String)>,
}

/// Caminho livre em ~/Documentos (usado quando não há diálogo de arquivos no sistema).
fn fallback_path(stem: &str, ext: &str) -> Option<PathBuf> {
    let dir = dirs::document_dir().or_else(dirs::home_dir)?;
    let _ = std::fs::create_dir_all(&dir);
    (0..1000)
        .map(|i| {
            if i == 0 {
                dir.join(format!("{stem}.{ext}"))
            } else {
                dir.join(format!("{stem}-{i}.{ext}"))
            }
        })
        .find(|p| !p.exists())
}

/// Abre um diálogo nativo. Retorna `Err(())` se o sistema não tem diálogo
/// disponível (ex.: Linux sem xdg-desktop-portal nem zenity).
fn run_dialog<T>(f: impl FnOnce() -> Option<T>) -> Result<Option<T>, ()> {
    let t = std::time::Instant::now();
    let r = f();
    if r.is_none() && t.elapsed() < std::time::Duration::from_millis(120) {
        Err(())
    } else {
        Ok(r)
    }
}

const NO_DIALOG: &str =
    "Diálogo de arquivos indisponível (no Linux, instale xdg-desktop-portal ou zenity)";

pub fn autosave_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("rustdraw").join("autosave.excalidraw"))
}

/// Consome uma tecla pressionada com exatamente estes modificadores (Shift e Alt
/// precisam bater, ao contrário de `consume_key`). Com `physical`, compara a
/// posição física da tecla.
fn take_key(i: &mut egui::InputState, m: Modifiers, key: Key, physical: bool) -> bool {
    let mut hit = false;
    i.events.retain(|ev| {
        let is = !hit
            && matches!(ev, egui::Event::Key { key: k, physical_key, modifiers, pressed: true, .. }
                if modifiers.matches_exact(m)
                    && if physical { physical_key.unwrap_or(*k) == key } else { *k == key });
        hit |= is;
        !is
    });
    hit
}

fn snap(v: f32, on: bool) -> f32 {
    if on { (v / GRID).round() * GRID } else { v }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let hand_ok = setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut app = App {
            elements: vec![],
            selected: HashSet::new(),
            tool: Tool::Select,
            tool_locked: false,
            prev_tool: Tool::Select,
            style: Style::default(),
            cam: Cam {
                scroll: P::new(0.0, 0.0),
                zoom: 1.0,
                origin: Pos2::ZERO,
            },
            drag: Drag::None,
            undo: vec![],
            redo: vec![],
            cache: HashMap::new(),
            editing: None,
            file_path: None,
            dirty: false,
            dark: false,
            grid: false,
            view_bg: "#ffffff".into(),
            show_menu: false,
            show_export: false,
            show_help: false,
            show_stats: false,
            zen_mode: false,
            view_mode: false,
            open_picker: None,
            copied_style: None,
            export_opts: ExportOpts::default(),
            export_selection_only: false,
            export_preview: None,
            confirm: None,
            toast: None,
            hand_ok,
            pointer_world: P::new(0.0, 0.0),
            last_change: 0.0,
            autosave_pending: false,
            last_title: String::new(),
            prop_coalesce: None,
            now: 0.0,
            pending_fit: false,
            needs_measure: HashSet::new(),
            last_prefs: None,
        };
        if let Some(p) = open {
            app.load_path(&p);
        } else if let Some(p) = autosave_path().filter(|p| p.exists())
            && let Ok(txt) = std::fs::read_to_string(&p)
            && let Ok(f) = SceneFile::parse(&txt)
        {
            app.file_path = f
                .app_state
                .get("rustdrawFilePath")
                .and_then(|v| v.as_str())
                .map(PathBuf::from);
            app.dirty = f
                .app_state
                .get("rustdrawDirty")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            app.dark = f
                .app_state
                .get("theme")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t == "dark");
            app.apply_scene(f);
        }
        app
    }

    // ------------------------------------------------------------------
    // Utilidades
    // ------------------------------------------------------------------

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Element> {
        self.elements.iter_mut().find(|e| e.id == id)
    }

    pub fn selected_elements(&self) -> Vec<&Element> {
        self.elements
            .iter()
            .filter(|e| self.selected.contains(&e.id))
            .collect()
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), self.now + 3.0));
    }

    /// Registra um passo no histórico (`before` = estado antes da ação).
    pub fn commit(&mut self, before: Vec<Element>) {
        if before == self.elements {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > 300 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.mark_changed();
    }

    fn mark_changed(&mut self) {
        self.dirty = true;
        self.autosave_pending = true;
        self.last_change = self.now;
        self.export_preview = None;
    }

    pub fn undo(&mut self) {
        self.finish_editing();
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.elements, prev));
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        self.finish_editing();
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.elements, next));
            self.after_history();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn after_history(&mut self) {
        let ids: HashSet<String> = self.elements.iter().map(|e| e.id.clone()).collect();
        self.selected.retain(|s| ids.contains(s));
        self.drag = Drag::None;
        self.mark_changed();
    }

    pub fn scene_file(&self) -> SceneFile {
        SceneFile::new(self.elements.clone(), &self.view_bg, self.grid)
    }

    fn apply_scene(&mut self, f: SceneFile) {
        self.view_bg = f.view_background().unwrap_or_else(|| "#ffffff".into());
        self.grid = f.grid_enabled();
        self.elements = f.elements;
        self.selected.clear();
        self.undo.clear();
        self.redo.clear();
        self.cache.clear();
        self.drag = Drag::None;
        self.editing = None;
        self.export_preview = None;
        self.cam.zoom = 1.0;
        self.cam.scroll = P::new(0.0, 0.0);
        self.pending_fit = true;
    }

    // ------------------------------------------------------------------
    // Arquivos
    // ------------------------------------------------------------------

    pub fn load_path(&mut self, p: &std::path::Path) {
        match std::fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| SceneFile::parse(&t))
        {
            Ok(f) => {
                self.apply_scene(f);
                self.file_path = Some(p.to_path_buf());
                self.dirty = false;
                self.autosave_pending = true;
                self.toast(format!("Aberto: {}", file_name(p)));
            }
            Err(e) => self.toast(format!("Não foi possível abrir: {e}")),
        }
    }

    pub fn open_dialog(&mut self) {
        if self.dirty && !self.elements.is_empty() && self.confirm.is_none() {
            self.confirm = Some(Pending::Open);
            return;
        }
        let dlg = rfd::FileDialog::new()
            .set_title("Abrir desenho")
            .add_filter("Excalidraw", &["excalidraw", "json"]);
        match run_dialog(|| dlg.pick_file()) {
            Ok(Some(p)) => self.load_path(&p),
            Ok(None) => {}
            Err(()) => self.toast(format!(
                "{NO_DIALOG}. Dica: arraste o arquivo para a janela."
            )),
        }
    }

    pub fn new_scene(&mut self) {
        if self.dirty && !self.elements.is_empty() && self.confirm.is_none() {
            self.confirm = Some(Pending::New);
            return;
        }
        let bg = self.view_bg.clone();
        self.apply_scene(SceneFile::new(vec![], &bg, self.grid));
        self.file_path = None;
        self.dirty = false;
        self.autosave_pending = true;
    }

    pub fn save(&mut self) {
        match self.file_path.clone() {
            Some(p) => self.write_to(&p),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) {
        let name = self
            .file_path
            .as_deref()
            .map(file_name)
            .unwrap_or_else(|| "desenho.excalidraw".into());
        let dlg = rfd::FileDialog::new()
            .set_title("Salvar desenho")
            .add_filter("Excalidraw", &["excalidraw"])
            .set_file_name(name);
        let picked = match run_dialog(|| dlg.save_file()) {
            Ok(p) => p,
            Err(()) => fallback_path("desenho", "excalidraw"),
        };
        if let Some(mut p) = picked {
            if p.extension().is_none() {
                p.set_extension("excalidraw");
            }
            self.write_to(&p);
        }
    }

    fn write_to(&mut self, p: &std::path::Path) {
        match std::fs::write(p, self.scene_file().to_json()) {
            Ok(()) => {
                self.file_path = Some(p.to_path_buf());
                self.dirty = false;
                self.autosave_pending = true;
                self.toast(format!("Salvo: {}", p.display()));
            }
            Err(e) => self.toast(format!("Erro ao salvar: {e}")),
        }
    }

    fn write_autosave(&mut self) {
        let Some(path) = autosave_path() else { return };
        let mut f = self.scene_file();
        if let Some(obj) = f.app_state.as_object_mut() {
            if let Some(fp) = &self.file_path {
                obj.insert("rustdrawFilePath".into(), fp.to_string_lossy().into());
            }
            obj.insert("rustdrawDirty".into(), self.dirty.into());
            obj.insert(
                "theme".into(),
                if self.dark { "dark" } else { "light" }.into(),
            );
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, f.to_json()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
        self.autosave_pending = false;
    }

    pub fn flush_autosave(&mut self) {
        if self.autosave_pending {
            self.write_autosave();
        }
    }

    pub fn export_elements(&self) -> Vec<Element> {
        if self.export_selection_only && !self.selected.is_empty() {
            self.selected_elements().into_iter().cloned().collect()
        } else {
            self.elements.clone()
        }
    }

    pub fn export_file(&mut self, svg: bool) {
        let els = self.export_elements();
        if els.is_empty() {
            self.toast("Nada para exportar");
            return;
        }
        let mut opts = self.export_opts.clone();
        opts.view_background = self.view_bg.clone();
        let base = self
            .file_path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "desenho".into());
        let (ext, label) = if svg { ("svg", "SVG") } else { ("png", "PNG") };
        let dlg = rfd::FileDialog::new()
            .set_title(format!("Exportar {label}"))
            .add_filter(label, &[ext])
            .set_file_name(format!("{base}.{ext}"));
        let picked = match run_dialog(|| dlg.save_file()) {
            Ok(p) => p,
            Err(()) => fallback_path(&base, ext),
        };
        let Some(path) = picked else { return };
        let res = if svg {
            export::render_svg(&els, &opts)
                .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()))
        } else {
            export::render_png(&els, &opts)
                .and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string()))
        };
        match res {
            Ok(()) => self.toast(format!("Exportado: {}", path.display())),
            Err(e) => self.toast(format!("Erro ao exportar: {e}")),
        }
    }

    pub fn copy_svg(&mut self, ctx: &egui::Context) {
        let mut opts = self.export_opts.clone();
        opts.view_background = self.view_bg.clone();
        match export::render_svg(&self.export_elements(), &opts) {
            Ok(s) => {
                ctx.copy_text(s);
                self.toast("SVG copiado para a área de transferência");
            }
            Err(e) => self.toast(e),
        }
    }

    // ------------------------------------------------------------------
    // Edição
    // ------------------------------------------------------------------

    pub fn set_tool(&mut self, t: Tool) {
        self.finish_editing();
        self.finish_linear();
        if t != self.tool {
            self.prev_tool = self.tool;
        }
        self.tool = t;
        if t != Tool::Select {
            self.selected.clear();
        }
    }

    pub fn delete_selected(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let before = self.elements.clone();
        self.elements.retain(|e| !self.selected.contains(&e.id));
        self.selected.clear();
        self.commit(before);
    }

    pub fn select_all(&mut self) {
        self.finish_editing();
        self.tool = Tool::Select;
        self.selected = self.elements.iter().map(|e| e.id.clone()).collect();
    }

    /// Copia elementos com novos ids/seeds, remapeando grupos.
    fn clone_with_new_ids(els: &[Element], offset: P) -> Vec<Element> {
        let mut groups: HashMap<String, String> = HashMap::new();
        els.iter()
            .map(|e| {
                let mut n = e.clone();
                n.id = new_id();
                n.seed = rand_seed();
                n.x += offset.x as f64;
                n.y += offset.y as f64;
                n.group_ids = e
                    .group_ids
                    .iter()
                    .map(|g| groups.entry(g.clone()).or_insert_with(new_id).clone())
                    .collect();
                n.bound_elements = None;
                for k in ["startBinding", "endBinding", "containerId"] {
                    if n.extra.contains_key(k) {
                        n.extra.insert(k.into(), serde_json::Value::Null);
                    }
                }
                n.touch();
                n
            })
            .collect()
    }

    pub fn duplicate_selected(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let before = self.elements.clone();
        let src: Vec<Element> = self.selected_elements().into_iter().cloned().collect();
        let copies = Self::clone_with_new_ids(&src, P::new(10.0, 10.0));
        self.selected = copies.iter().map(|e| e.id.clone()).collect();
        self.elements.extend(copies);
        self.commit(before);
    }

    pub fn copy_selection(&self, ctx: &egui::Context) {
        let sel: Vec<Element> = self.selected_elements().into_iter().cloned().collect();
        if !sel.is_empty() {
            ctx.copy_text(clipboard_json(&sel));
        }
    }

    pub fn paste(&mut self, text: &str) {
        let at = self.pointer_world;
        let before = self.elements.clone();
        if let Ok(f) = SceneFile::parse(text) {
            if f.elements.is_empty() {
                return;
            }
            let b = geom::scene_bounds(f.elements.iter()).unwrap();
            let off = at - b.center();
            let copies = Self::clone_with_new_ids(&f.elements, off);
            self.selected = copies.iter().map(|e| e.id.clone()).collect();
            self.elements.extend(copies);
        } else if !text.trim().is_empty() {
            let mut el = Element::new("text", at.x as f64, at.y as f64, &self.style);
            el.set_text(text.trim_end());
            // largura estimada; ajustada no próximo quadro pela medição real
            let text = el.text().to_string();
            let lines: Vec<&str> = text.split('\n').collect();
            let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1);
            el.width = longest as f64 * el.font_size() * 0.55;
            el.height = lines.len() as f64 * el.font_size() * el.line_height();
            self.needs_measure.insert(el.id.clone());
            self.selected = [el.id.clone()].into();
            self.elements.push(el);
        } else {
            return;
        }
        self.tool = Tool::Select;
        self.commit(before);
    }

    pub fn reorder(&mut self, to_front: bool, all_the_way: bool) {
        if self.selected.is_empty() {
            return;
        }
        let before = self.elements.clone();
        let sel = self.selected.clone();
        if all_the_way {
            let (mut a, b): (Vec<Element>, Vec<Element>) =
                self.elements.drain(..).partition(|e| !sel.contains(&e.id));
            if to_front {
                a.extend(b);
                self.elements = a;
            } else {
                let mut b = b;
                b.extend(a);
                self.elements = b;
            }
        } else if to_front {
            for i in (0..self.elements.len().saturating_sub(1)).rev() {
                if sel.contains(&self.elements[i].id) && !sel.contains(&self.elements[i + 1].id) {
                    self.elements.swap(i, i + 1);
                }
            }
        } else {
            for i in 1..self.elements.len() {
                if sel.contains(&self.elements[i].id) && !sel.contains(&self.elements[i - 1].id) {
                    self.elements.swap(i, i - 1);
                }
            }
        }
        self.commit(before);
    }

    pub fn group_selected(&mut self, group: bool) {
        if self.selected.len() < if group { 2 } else { 1 } {
            return;
        }
        let before = self.elements.clone();
        let gid = new_id();
        let sel = self.selected.clone();
        for e in self.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
            if group {
                e.group_ids.push(gid.clone());
            } else {
                e.group_ids.pop();
            }
            e.touch();
        }
        self.commit(before);
    }

    /// Ctrl+Delete: apaga todos os elementos (pode ser desfeito).
    pub fn clear_canvas(&mut self) {
        self.finish_editing();
        self.finish_linear();
        let before = std::mem::take(&mut self.elements);
        self.selected.clear();
        self.commit(before);
    }

    /// Ctrl+Shift+L: trava ou destrava os elementos selecionados.
    pub fn toggle_lock(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let before = self.elements.clone();
        let sel = self.selected.clone();
        let lock = self
            .elements
            .iter()
            .filter(|e| sel.contains(&e.id))
            .any(|e| !e.locked);
        for e in self.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
            e.locked = lock;
            e.touch();
        }
        self.commit(before);
        self.toast(if lock { "Travado" } else { "Destravado" });
    }

    /// Shift+H / Shift+V: espelha a seleção em torno do centro dela.
    pub fn flip_selected(&mut self, horizontal: bool) {
        let Some(b) = self.selection_bounds() else {
            return;
        };
        let before = self.elements.clone();
        let sel = self.selected.clone();
        let mid = if horizontal {
            (b.min.x + b.max.x) as f64
        } else {
            (b.min.y + b.max.y) as f64
        };
        for e in self.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
            flip(e, mid, horizontal);
        }
        self.commit(before);
    }

    /// Ctrl+Shift+setas: alinha a seleção (grupos se movem como uma unidade).
    pub fn align_selected(&mut self, dx: i8, dy: i8) {
        let Some(all) = self.selection_bounds() else {
            return;
        };
        let mut units: Vec<(String, Vec<String>)> = vec![];
        for e in self
            .elements
            .iter()
            .filter(|e| self.selected.contains(&e.id))
        {
            let key = e.group_ids.last().cloned().unwrap_or_else(|| e.id.clone());
            match units.iter_mut().find(|(k, _)| *k == key) {
                Some((_, ids)) => ids.push(e.id.clone()),
                None => units.push((key, vec![e.id.clone()])),
            }
        }
        if units.len() < 2 {
            return;
        }
        let before = self.elements.clone();
        for (_, ids) in units {
            let Some(b) = geom::scene_bounds(self.elements.iter().filter(|e| ids.contains(&e.id)))
            else {
                continue;
            };
            let ox = match dx {
                -1 => all.min.x - b.min.x,
                1 => all.max.x - b.max.x,
                _ => 0.0,
            } as f64;
            let oy = match dy {
                -1 => all.min.y - b.min.y,
                1 => all.max.y - b.max.y,
                _ => 0.0,
            } as f64;
            for e in self.elements.iter_mut().filter(|e| ids.contains(&e.id)) {
                e.x += ox;
                e.y += oy;
                e.touch();
            }
        }
        self.commit(before);
    }

    /// Ctrl+Shift+< / >: diminui ou aumenta a fonte em 10%.
    pub fn change_font_size(&mut self, increase: bool) {
        let resize = move |fs: f64| {
            if increase {
                (fs * 1.1).round()
            } else {
                (fs / 1.1).round().max(1.0)
            }
        };
        let first = self
            .selected_elements()
            .into_iter()
            .find(|e| e.kind == "text")
            .map(|e| e.font_size());
        let Some(first) = first else { return };
        self.style.font_size = resize(first);
        self.apply_prop("font", move |e| {
            if e.kind == "text" {
                let fs = resize(e.font_size());
                e.set_field("fontSize", serde_json::json!(fs));
            }
        });
    }

    /// Tab / Shift+Tab: troca o tipo das formas selecionadas
    /// (retângulo → losango → elipse; linha ↔ seta).
    pub fn convert_selected(&mut self, forward: bool) {
        const SHAPES: [&str; 3] = ["rectangle", "diamond", "ellipse"];
        let sel = self.selected.clone();
        let kinds: Vec<&str> = self
            .elements
            .iter()
            .filter(|e| sel.contains(&e.id))
            .map(|e| e.kind.as_str())
            .collect();
        let shapes = !kinds.is_empty() && kinds.iter().all(|k| SHAPES.contains(k));
        let linear = !kinds.is_empty() && kinds.iter().all(|k| matches!(*k, "line" | "arrow"));
        if !shapes && !linear {
            return;
        }
        let before = self.elements.clone();
        let style = self.style.clone();
        for e in self.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
            let round = e.is_round();
            if shapes {
                let i = SHAPES.iter().position(|k| *k == e.kind).unwrap_or(0);
                let next = if forward { (i + 1) % 3 } else { (i + 2) % 3 };
                e.kind = SHAPES[next].into();
                e.roundness = match (round, SHAPES[next]) {
                    (true, "rectangle") => Some(serde_json::json!({"type": 3})),
                    (true, "diamond") => Some(serde_json::json!({"type": 2})),
                    _ => None,
                };
            } else {
                let to_arrow = e.kind == "line";
                e.kind = if to_arrow { "arrow" } else { "line" }.into();
                let (sa, ea) = if to_arrow {
                    (style.start_arrowhead.clone(), style.end_arrowhead.clone())
                } else {
                    (None, None)
                };
                let v = |a: Option<String>| a.map(serde_json::Value::String).unwrap_or_default();
                e.set_field("startArrowhead", v(sa));
                e.set_field("endArrowhead", v(ea));
                if to_arrow {
                    e.background_color = "transparent".into();
                    e.set_field("elbowed", serde_json::Value::Bool(false));
                } else {
                    e.extra.remove("elbowed");
                }
            }
            e.touch();
        }
        self.commit(before);
    }

    /// Ctrl+Alt+C: copia o estilo do primeiro elemento selecionado.
    pub fn copy_styles(&mut self) {
        let first = self
            .elements
            .iter()
            .find(|e| self.selected.contains(&e.id))
            .cloned();
        if first.is_some() {
            self.copied_style = first;
            self.toast("Estilo copiado");
        }
    }

    /// Ctrl+Alt+V: aplica o estilo copiado à seleção.
    pub fn paste_styles(&mut self) {
        let Some(src) = self.copied_style.clone() else {
            return;
        };
        if self.selected.is_empty() {
            return;
        }
        let before = self.elements.clone();
        let sel = self.selected.clone();
        for e in self.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
            e.stroke_color = src.stroke_color.clone();
            if e.kind != "text" && !(e.kind == "arrow" && src.kind != "arrow") {
                e.background_color = src.background_color.clone();
            }
            e.fill_style = src.fill_style.clone();
            e.stroke_width = src.stroke_width;
            e.stroke_style = src.stroke_style.clone();
            e.roughness = src.roughness;
            e.opacity = src.opacity;
            if matches!(e.kind.as_str(), "rectangle" | "diamond" | "line" | "arrow") {
                e.roundness = src.is_round().then(
                    || serde_json::json!({"type": if e.kind == "rectangle" { 3 } else { 2 }}),
                );
            }
            if e.kind == "text" && src.kind == "text" {
                for k in ["fontSize", "fontFamily", "textAlign"] {
                    if let Some(v) = src.extra.get(k) {
                        e.extra.insert(k.into(), v.clone());
                    }
                }
                self.needs_measure.insert(e.id.clone());
            }
            if e.kind == "arrow" && src.kind == "arrow" {
                for k in ["startArrowhead", "endArrowhead"] {
                    let v = src.extra.get(k).cloned().unwrap_or_default();
                    e.extra.insert(k.into(), v);
                }
            }
            e.touch();
        }
        self.commit(before);
    }

    /// Shift+Alt+C: copia a seleção (ou tudo) como imagem PNG.
    pub fn copy_png(&mut self, ctx: &egui::Context) {
        let els: Vec<Element> = if self.selected.is_empty() {
            self.elements.clone()
        } else {
            self.selected_elements().into_iter().cloned().collect()
        };
        if els.is_empty() {
            return;
        }
        let mut opts = self.export_opts.clone();
        opts.view_background = self.view_bg.clone();
        match export::render_pixmap(&els, &opts) {
            Ok(pm) => {
                let size = [pm.width() as usize, pm.height() as usize];
                ctx.copy_image(egui::ColorImage::from_rgba_premultiplied(size, pm.data()));
                self.toast("Imagem PNG copiada para a área de transferência");
            }
            Err(e) => self.toast(e),
        }
    }

    /// Alt+R: modo de visualização (só navegar, sem editar).
    pub fn toggle_view_mode(&mut self) {
        self.finish_editing();
        self.finish_linear();
        self.view_mode = !self.view_mode;
        if self.view_mode {
            self.selected.clear();
            self.show_menu = false;
        }
    }

    /// Aplica uma mudança de propriedade à seleção (com agrupamento no histórico).
    pub fn apply_prop(&mut self, key: &'static str, f: impl Fn(&mut Element)) {
        let ids = self.selected.clone();
        let editing = self.editing.as_ref().map(|e| e.id.clone());
        let targets: Vec<String> = ids.into_iter().chain(editing).collect();
        if targets.is_empty() {
            return;
        }
        let before = self.elements.clone();
        for e in self.elements.iter_mut().filter(|e| targets.contains(&e.id)) {
            f(e);
            e.touch();
        }
        for id in targets.iter().filter(|_| key == "font") {
            self.needs_measure.insert(id.clone());
        }
        let coalesce = matches!(self.prop_coalesce, Some((k, t)) if k == key && self.now - t < 0.8);
        if coalesce {
            self.mark_changed();
        } else {
            self.commit(before);
        }
        self.prop_coalesce = Some((key, self.now));
    }

    // ------------------------------------------------------------------
    // Texto
    // ------------------------------------------------------------------

    pub fn start_text(&mut self, at: P, center_x: Option<f64>) {
        let before = self.elements.clone();
        let mut el = Element::new("text", at.x as f64, at.y as f64, &self.style);
        if center_x.is_some() {
            el.set_field("textAlign", serde_json::json!("center"));
        }
        el.height = el.font_size() * el.line_height();
        el.y -= el.height / 2.0;
        el.width = 1.0;
        let id = el.id.clone();
        self.elements.push(el);
        self.selected = [id.clone()].into();
        self.editing = Some(TextEditing {
            id,
            buf: String::new(),
            before,
            focus: true,
            anchor_center: center_x,
        });
    }

    pub fn edit_text(&mut self, id: &str) {
        let Some(el) = self.elements.iter().find(|e| e.id == id) else {
            return;
        };
        let center = (el.text_align() == "center").then(|| el.x + el.width / 2.0);
        self.editing = Some(TextEditing {
            id: id.to_string(),
            buf: el.text().to_string(),
            before: self.elements.clone(),
            focus: true,
            anchor_center: center,
        });
        self.selected = [id.to_string()].into();
    }

    pub fn finish_editing(&mut self) {
        let Some(ed) = self.editing.take() else {
            return;
        };
        if ed.buf.trim().is_empty() {
            self.elements.retain(|e| e.id != ed.id);
            self.selected.remove(&ed.id);
        }
        self.commit(ed.before);
        if !self.tool_locked && self.tool == Tool::Text {
            self.tool = Tool::Select;
        }
    }

    /// Atualiza o elemento com o texto em edição e re-mede.
    pub fn update_editing_text(&mut self, painter: &egui::Painter) {
        let Some(ed) = &self.editing else { return };
        let (id, buf, anchor) = (ed.id.clone(), ed.buf.clone(), ed.anchor_center);
        let hand_ok = self.hand_ok;
        if let Some(el) = self.get_mut(&id) {
            if el.text() != buf {
                el.set_text(&buf);
                el.touch();
            }
            let (w, h) = render::measure_text(
                painter,
                &buf,
                el.font_size() as f32,
                el.font_family(),
                el.line_height() as f32,
                hand_ok,
            );
            if (el.width - w as f64).abs() > 0.01 || (el.height - h as f64).abs() > 0.01 {
                el.width = w as f64;
                el.height = h as f64;
                if let Some(cx) = anchor {
                    el.x = cx - el.width / 2.0;
                }
                el.touch();
            }
        }
    }

    // ------------------------------------------------------------------
    // Linhas com vários pontos (clique a clique)
    // ------------------------------------------------------------------

    pub fn finish_linear(&mut self) {
        let Drag::LinearMulti { id, before } = std::mem::replace(&mut self.drag, Drag::None) else {
            return;
        };
        let tol = 4.0 / self.cam.zoom as f64;
        if let Some(el) = self.get_mut(&id) {
            let mut pts = el.points();
            while pts.len() > 2 {
                let n = pts.len();
                let (a, b) = (pts[n - 1], pts[n - 2]);
                if (a[0] - b[0]).hypot(a[1] - b[1]) < tol {
                    pts.pop();
                } else {
                    break;
                }
            }
            let len = pts
                .windows(2)
                .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
                .sum::<f64>();
            if pts.len() < 2 || len < tol {
                self.elements.retain(|e| e.id != id);
                self.selected.clear();
            } else {
                el.set_points(pts);
                el.touch();
            }
        }
        self.commit(before);
        if !self.tool_locked {
            self.tool = Tool::Select;
        }
    }

    // ------------------------------------------------------------------
    // Câmera
    // ------------------------------------------------------------------

    pub fn zoom_at(&mut self, screen: Pos2, factor: f32) {
        let w = self.cam.w(screen);
        let z = (self.cam.zoom * factor).clamp(0.1, 30.0);
        self.cam.zoom = z;
        self.cam.scroll = P::new(
            (screen.x - self.cam.origin.x) / z - w.x,
            (screen.y - self.cam.origin.y) / z - w.y,
        );
    }

    /// Zoom em passos de 10% (como o `ZOOM_STEP` do Excalidraw), centrado na tela.
    pub fn zoom_step(&mut self, view: egui::Rect, dir: f32) {
        let z = ((self.cam.zoom + 0.1 * dir) * 10.0).round() / 10.0;
        let z = z.clamp(0.1, 30.0);
        self.zoom_at(view.center(), z / self.cam.zoom);
    }

    pub fn reset_zoom(&mut self, view: egui::Rect) {
        self.zoom_at(view.center(), 1.0 / self.cam.zoom);
    }

    /// Shift+1: enquadra todos os elementos (sem passar de 100%).
    pub fn zoom_to_fit(&mut self, view: egui::Rect) {
        let els: Vec<&Element> = self.elements.iter().collect();
        self.fit_bounds(view, geom::scene_bounds(els), false);
    }

    /// Shift+2 (`contain = false`, sem passar de 100%) e Shift+3 (`contain = true`,
    /// preenche a tela): enquadra a seleção, ou tudo se nada estiver selecionado.
    pub fn zoom_to_selection(&mut self, view: egui::Rect, contain: bool) {
        let els: Vec<&Element> = if self.selected.is_empty() {
            self.elements.iter().collect()
        } else {
            self.selected_elements()
        };
        self.fit_bounds(view, geom::scene_bounds(els), contain);
    }

    fn fit_bounds(&mut self, view: egui::Rect, b: Option<Rect>, contain: bool) {
        let Some(b) = b else {
            self.cam.zoom = 1.0;
            self.cam.scroll = P::new(0.0, 0.0);
            return;
        };
        let max = if contain { 30.0 } else { 1.0 };
        let z = ((view.width() - 160.0) / b.width().max(1.0))
            .min((view.height() - 160.0) / b.height().max(1.0))
            .clamp(0.1, max);
        self.cam.zoom = z;
        let c = b.center();
        self.cam.scroll = P::new(
            (view.center().x - view.min.x) / z - c.x,
            (view.center().y - view.min.y) / z - c.y,
        );
    }

    /// PgUp/PgDn: rola uma tela na vertical (com Shift, na horizontal).
    pub fn scroll_page(&mut self, view: egui::Rect, down: bool, horizontal: bool) {
        let sign = if down { -1.0 } else { 1.0 };
        if horizontal {
            self.cam.scroll.x += sign * view.width() / self.cam.zoom;
        } else {
            self.cam.scroll.y += sign * view.height() / self.cam.zoom;
        }
    }

    // ------------------------------------------------------------------
    // Hit-test e alças
    // ------------------------------------------------------------------

    fn element_at(&self, p: P) -> Option<usize> {
        let tol = 8.0 / self.cam.zoom;
        (0..self.elements.len())
            .rev()
            .find(|&i| geom::hit_test(&self.elements[i], p, tol))
    }

    fn group_of(&self, i: usize) -> HashSet<String> {
        let el = &self.elements[i];
        match el.group_ids.last() {
            Some(g) => self
                .elements
                .iter()
                .filter(|e| e.group_ids.contains(g))
                .map(|e| e.id.clone())
                .collect(),
            None => [el.id.clone()].into(),
        }
    }

    pub fn single_selected(&self) -> Option<&Element> {
        if self.selected.len() != 1 {
            return None;
        }
        let id = self.selected.iter().next()?;
        self.elements.iter().find(|e| &e.id == id)
    }

    /// Posições (mundo) das alças da seleção única.
    pub fn handles(&self) -> Vec<(Handle, P)> {
        let Some(el) = self.single_selected() else {
            return vec![];
        };
        if self.editing.is_some() || el.locked {
            return vec![];
        }
        let z = self.cam.zoom;
        let c = geom::center(el);
        let a = el.angle as f32;
        if el.is_linear() {
            return geom::abs_points(el)
                .into_iter()
                .enumerate()
                .map(|(i, p)| (Handle::Point(i), rotate(p, c, a)))
                .collect();
        }
        let b = geom::local_box(el);
        let pad = 4.0 / z;
        let (x0, y0, x1, y1) = (b.min.x - pad, b.min.y - pad, b.max.x + pad, b.max.y + pad);
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let mut v = vec![
            (Handle::NW, P::new(x0, y0)),
            (Handle::NE, P::new(x1, y0)),
            (Handle::SE, P::new(x1, y1)),
            (Handle::SW, P::new(x0, y1)),
        ];
        let small = (x1 - x0) * z < 30.0 || (y1 - y0) * z < 30.0;
        if el.kind != "text" && !small {
            v.extend([
                (Handle::N, P::new(mx, y0)),
                (Handle::S, P::new(mx, y1)),
                (Handle::E, P::new(x1, my)),
                (Handle::W, P::new(x0, my)),
            ]);
        }
        v.push((Handle::Rotate, P::new(mx, y0 - 22.0 / z)));
        v.into_iter().map(|(h, p)| (h, rotate(p, c, a))).collect()
    }

    fn handle_at(&self, screen: Pos2) -> Option<Handle> {
        self.handles()
            .into_iter()
            .find(|(_, p)| (self.cam.s(*p) - screen).length() <= HANDLE_PX + 2.0)
            .map(|(h, _)| h)
    }

    fn selection_bounds(&self) -> Option<Rect> {
        if self.selected.is_empty() {
            return None;
        }
        geom::scene_bounds(self.selected_elements())
    }

    // ------------------------------------------------------------------
    // Ponteiro
    // ------------------------------------------------------------------

    pub fn handle_pointer(&mut self, ctx: &egui::Context, resp: &egui::Response) {
        let (pos, pressed, released, down, mid_pressed, mid_down, scroll, zoom_delta, mods, dbl) =
            ctx.input(|i| {
                (
                    i.pointer.latest_pos(),
                    i.pointer.primary_pressed(),
                    i.pointer.primary_released(),
                    i.pointer.primary_down(),
                    i.pointer.button_pressed(PointerButton::Middle),
                    i.pointer.middle_down(),
                    i.smooth_scroll_delta,
                    i.zoom_delta(),
                    i.modifiers,
                    i.pointer.button_double_clicked(PointerButton::Primary),
                )
            });
        let space = ctx.input(|i| i.key_down(Key::Space)) && !ctx.egui_wants_keyboard_input();
        let over = resp.contains_pointer();
        if over {
            if zoom_delta != 1.0 {
                if let Some(p) = pos {
                    self.zoom_at(p, zoom_delta);
                }
            } else if scroll != egui::Vec2::ZERO {
                self.cam.scroll.x += scroll.x / self.cam.zoom;
                self.cam.scroll.y += scroll.y / self.cam.zoom;
            }
        }
        let Some(pos) = pos else { return };
        let wp = self.cam.w(pos);
        self.pointer_world = wp;

        // pan com botão do meio
        if over && mid_pressed {
            self.drag = Drag::Pan { last: pos };
        }

        if over && pressed {
            if self.editing.is_some() {
                self.finish_editing();
                if self.tool == Tool::Text || dbl {
                    return;
                }
            }
            if space || self.tool == Tool::Hand || self.view_mode {
                self.drag = Drag::Pan { last: pos };
            } else {
                self.on_press(wp, pos, mods);
            }
        }
        if over && dbl && self.tool == Tool::Select && self.editing.is_none() && !self.view_mode {
            self.on_double_click(wp);
        }
        if over && dbl && matches!(self.drag, Drag::LinearMulti { .. }) {
            self.finish_linear();
        }

        let active = down || mid_down || matches!(self.drag, Drag::LinearMulti { .. });
        if active {
            self.on_move(wp, pos, mods);
        }
        if (released || (!down && !mid_down))
            && !matches!(self.drag, Drag::None | Drag::LinearMulti { .. })
        {
            self.on_release(wp);
        }

        // cursor
        let cursor = match (&self.drag, self.tool) {
            (Drag::Pan { .. }, _) => CursorIcon::Grabbing,
            (_, Tool::Hand) => CursorIcon::Grab,
            _ if space || self.view_mode => CursorIcon::Grab,
            (Drag::Move { .. }, _) => CursorIcon::Move,
            (_, Tool::Select) if over => match self.handle_at(pos) {
                Some(Handle::N | Handle::S) => CursorIcon::ResizeVertical,
                Some(Handle::E | Handle::W) => CursorIcon::ResizeHorizontal,
                Some(Handle::NW | Handle::SE) => CursorIcon::ResizeNwSe,
                Some(Handle::NE | Handle::SW) => CursorIcon::ResizeNeSw,
                Some(Handle::Rotate) => CursorIcon::Alias,
                Some(Handle::Point(_)) => CursorIcon::PointingHand,
                None if self.element_at(wp).is_some() => CursorIcon::Move,
                None => CursorIcon::Default,
            },
            (_, Tool::Text) if over => CursorIcon::Text,
            (_, Tool::Select) => CursorIcon::Default,
            _ if over => CursorIcon::Crosshair,
            _ => CursorIcon::Default,
        };
        if over || !matches!(self.drag, Drag::None) {
            ctx.set_cursor_icon(cursor);
        }
    }

    fn on_press(&mut self, p: P, screen: Pos2, mods: Modifiers) {
        let before = self.elements.clone();
        let g = self.grid;
        match self.tool {
            Tool::Hand => {}
            Tool::Select => {
                if let Some(h) = self.handle_at(screen) {
                    let orig = self.single_selected().unwrap().clone();
                    self.drag = if h == Handle::Rotate {
                        Drag::Rotate { orig, before }
                    } else {
                        Drag::Resize {
                            handle: h,
                            orig,
                            before,
                        }
                    };
                    return;
                }
                let in_sel_box = self.selected.len() > 1
                    && self
                        .selection_bounds()
                        .is_some_and(|b| b.expand(4.0 / self.cam.zoom).contains(p));
                let hit = self.element_at(p);
                match hit {
                    Some(i) if !in_sel_box || mods.shift => {
                        let group = self.group_of(i);
                        if mods.shift {
                            if self.selected.contains(&self.elements[i].id) {
                                for id in &group {
                                    self.selected.remove(id);
                                }
                                return;
                            }
                            self.selected.extend(group);
                        } else if !self.selected.contains(&self.elements[i].id) {
                            self.selected = group;
                        }
                    }
                    Some(_) => {}
                    None if in_sel_box => {}
                    None => {
                        let base = if mods.shift {
                            self.selected.clone()
                        } else {
                            HashSet::new()
                        };
                        self.selected = base.clone();
                        self.drag = Drag::BoxSelect {
                            start: p,
                            cur: p,
                            base,
                        };
                        return;
                    }
                }
                // começa a mover (Alt = duplicar e mover a cópia)
                if mods.alt {
                    let src: Vec<Element> = self.selected_elements().into_iter().cloned().collect();
                    let copies = Self::clone_with_new_ids(&src, P::new(0.0, 0.0));
                    self.selected = copies.iter().map(|e| e.id.clone()).collect();
                    self.elements.extend(copies);
                }
                let originals = self
                    .elements
                    .iter()
                    .filter(|e| self.selected.contains(&e.id) && !e.locked)
                    .map(|e| (e.id.clone(), e.x, e.y))
                    .collect();
                self.drag = Drag::Move {
                    start: p,
                    originals,
                    before,
                    moved: false,
                };
            }
            Tool::Rect | Tool::Diamond | Tool::Ellipse | Tool::Arrow | Tool::Line => {
                if let Drag::LinearMulti { id, .. } = &self.drag {
                    // adiciona um ponto
                    let id = id.clone();
                    if let Some(el) = self.get_mut(&id) {
                        let mut pts = el.points();
                        let rel = [snap(p.x, g) as f64 - el.x, snap(p.y, g) as f64 - el.y];
                        if let Some(last) = pts.last_mut() {
                            *last = rel;
                        }
                        pts.push(rel);
                        el.set_points(pts);
                        el.touch();
                    }
                    return;
                }
                let origin = P::new(snap(p.x, g), snap(p.y, g));
                let kind = self.tool.kind().unwrap();
                let mut el = Element::new(kind, origin.x as f64, origin.y as f64, &self.style);
                if el.is_linear() {
                    el.set_points(vec![[0.0, 0.0], [0.0, 0.0]]);
                }
                let id = el.id.clone();
                self.elements.push(el);
                self.selected = [id.clone()].into();
                self.drag = Drag::Create { id, origin, before };
            }
            Tool::Draw => {
                let mut el = Element::new("freedraw", p.x as f64, p.y as f64, &self.style);
                el.set_points(vec![[0.0, 0.0]]);
                let id = el.id.clone();
                self.elements.push(el);
                self.selected.clear();
                self.drag = Drag::FreeDraw { id, before };
            }
            Tool::Text => {
                if let Some(i) = self
                    .element_at(p)
                    .filter(|&i| self.elements[i].kind == "text")
                {
                    let id = self.elements[i].id.clone();
                    self.edit_text(&id);
                } else {
                    self.start_text(P::new(snap(p.x, g), p.y), None);
                }
            }
            Tool::Eraser => {
                let mut marked = HashSet::new();
                if let Some(i) = self.element_at(p) {
                    marked.extend(self.group_of(i));
                }
                self.drag = Drag::Erase { last: p, marked };
            }
        }
    }

    fn on_double_click(&mut self, p: P) {
        match self.element_at(p) {
            Some(i) if self.elements[i].kind == "text" => {
                let id = self.elements[i].id.clone();
                self.drag = Drag::None;
                self.edit_text(&id);
            }
            Some(i)
                if matches!(
                    self.elements[i].kind.as_str(),
                    "rectangle" | "ellipse" | "diamond"
                ) =>
            {
                let c = geom::center(&self.elements[i]);
                self.drag = Drag::None;
                self.start_text(c, Some(c.x as f64));
            }
            Some(_) => {}
            None => {
                self.drag = Drag::None;
                self.start_text(p, None);
            }
        }
    }

    fn on_move(&mut self, p: P, screen: Pos2, mods: Modifiers) {
        let g = self.grid;
        let zoom = self.cam.zoom;
        match &mut self.drag {
            Drag::None => {}
            Drag::Pan { last } => {
                let d = screen - *last;
                *last = screen;
                self.cam.scroll.x += d.x / zoom;
                self.cam.scroll.y += d.y / zoom;
            }
            Drag::Create { id, origin, .. } => {
                let (id, origin) = (id.clone(), *origin);
                let mut q = P::new(snap(p.x, g), snap(p.y, g));
                let Some(el) = self.get_mut(&id) else { return };
                if el.is_linear() {
                    if mods.shift {
                        q = snap_angle(origin, p);
                    }
                    el.set_points(vec![
                        [0.0, 0.0],
                        [(q.x - origin.x) as f64, (q.y - origin.y) as f64],
                    ]);
                } else {
                    let mut d = q - origin;
                    if mods.shift {
                        let m = d.x.abs().max(d.y.abs());
                        d = P::new(m * d.x.signum(), m * d.y.signum());
                    }
                    let (a, b) = if mods.alt {
                        (origin - d, origin + d)
                    } else {
                        (origin, origin + d)
                    };
                    let r = Rect::from_two(a, b);
                    el.x = r.min.x as f64;
                    el.y = r.min.y as f64;
                    el.width = r.width() as f64;
                    el.height = r.height() as f64;
                }
                el.touch();
            }
            Drag::LinearMulti { id, .. } => {
                let id = id.clone();
                let Some(el) = self.get_mut(&id) else { return };
                let mut pts = el.points();
                if pts.len() >= 2 {
                    let prev = pts[pts.len() - 2];
                    let mut q = P::new(snap(p.x, g), snap(p.y, g));
                    if mods.shift {
                        q = snap_angle(
                            P::new(el.x as f32 + prev[0] as f32, el.y as f32 + prev[1] as f32),
                            p,
                        );
                    }
                    let n = pts.len();
                    pts[n - 1] = [q.x as f64 - el.x, q.y as f64 - el.y];
                    el.set_points(pts);
                    el.touch();
                }
            }
            Drag::FreeDraw { id, .. } => {
                let id = id.clone();
                let Some(el) = self.get_mut(&id) else { return };
                let mut pts = el.points();
                let rel = [p.x as f64 - el.x, p.y as f64 - el.y];
                let last = pts.last().copied().unwrap_or([0.0, 0.0]);
                if (rel[0] - last[0]).hypot(rel[1] - last[1]) * zoom as f64 > 1.5 {
                    pts.push(rel);
                    el.set_points(pts);
                    el.touch();
                }
            }
            Drag::Move {
                start,
                originals,
                moved,
                ..
            } => {
                let mut d = p - *start;
                if mods.shift {
                    if d.x.abs() > d.y.abs() {
                        d.y = 0.0
                    } else {
                        d.x = 0.0
                    }
                }
                if d.len() * zoom > 1.0 {
                    *moved = true;
                }
                if !*moved {
                    return;
                }
                let originals = originals.clone();
                if let Some((_, ox, oy)) = originals.first()
                    && g
                {
                    d.x = snap(*ox as f32 + d.x, true) - *ox as f32;
                    d.y = snap(*oy as f32 + d.y, true) - *oy as f32;
                }
                for (id, ox, oy) in originals {
                    if let Some(el) = self.get_mut(&id) {
                        el.x = ox + d.x as f64;
                        el.y = oy + d.y as f64;
                        el.touch();
                    }
                }
            }
            Drag::BoxSelect { start, cur, base } => {
                *cur = p;
                let r = Rect::from_two(*start, *cur);
                let base = base.clone();
                let mut sel = base;
                for e in &self.elements {
                    if r.contains_rect(&geom::aabb(e)) {
                        sel.insert(e.id.clone());
                    }
                }
                // grupos inteiros
                let groups: HashSet<String> = self
                    .elements
                    .iter()
                    .filter(|e| sel.contains(&e.id))
                    .filter_map(|e| e.group_ids.last().cloned())
                    .collect();
                for e in &self.elements {
                    if e.group_ids.last().is_some_and(|g| groups.contains(g)) {
                        sel.insert(e.id.clone());
                    }
                }
                self.selected = sel;
            }
            Drag::Resize { handle, orig, .. } => {
                let (handle, orig) = (*handle, orig.clone());
                let updated = resize(&orig, handle, p, mods.shift, g);
                if let Some(el) = self.get_mut(&orig.id) {
                    *el = updated;
                    el.touch();
                }
            }
            Drag::Rotate { orig, .. } => {
                let c = geom::center(orig);
                let mut a = (p.y - c.y).atan2(p.x - c.x) + std::f32::consts::FRAC_PI_2;
                if mods.shift {
                    let step = 15f32.to_radians();
                    a = (a / step).round() * step;
                }
                let a = a.rem_euclid(std::f32::consts::TAU);
                let id = orig.id.clone();
                if let Some(el) = self.get_mut(&id) {
                    el.angle = a as f64;
                    el.touch();
                }
            }
            Drag::Erase { last, marked } => {
                let a = *last;
                *last = p;
                let tol = 6.0 / zoom;
                let mut hits = Vec::new();
                for (i, e) in self.elements.iter().enumerate() {
                    if !marked.contains(&e.id) && geom::segment_hits(e, a, p, tol) {
                        hits.push(i);
                    }
                }
                for i in hits {
                    let gset = self.group_of(i);
                    if let Drag::Erase { marked, .. } = &mut self.drag {
                        marked.extend(gset);
                    }
                }
            }
        }
    }

    fn on_release(&mut self, _p: P) {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        let tol = 3.0 / self.cam.zoom as f64;
        match drag {
            Drag::Create { id, before, .. } => {
                let Some(el) = self.elements.iter().find(|e| e.id == id) else {
                    return;
                };
                let tiny = if el.is_linear() {
                    let pts = el.points();
                    pts.len() < 2 || (pts[1][0]).hypot(pts[1][1]) < tol
                } else {
                    el.width < tol && el.height < tol
                };
                if tiny && el.is_linear() {
                    // modo clique a clique
                    self.drag = Drag::LinearMulti { id, before };
                    return;
                }
                if tiny {
                    self.elements.retain(|e| e.id != id);
                    self.selected.clear();
                    return;
                }
                self.commit(before);
                if !self.tool_locked {
                    self.tool = Tool::Select;
                }
            }
            Drag::FreeDraw { before, .. } => self.commit(before),
            Drag::Move { before, moved, .. } => {
                if moved {
                    self.commit(before);
                }
            }
            Drag::Resize { before, .. } | Drag::Rotate { before, .. } => self.commit(before),
            Drag::Erase { marked, .. } => {
                if !marked.is_empty() {
                    let before = self.elements.clone();
                    self.elements.retain(|e| !marked.contains(&e.id));
                    self.selected.retain(|s| !marked.contains(s));
                    self.commit(before);
                }
            }
            Drag::LinearMulti { id, before } => self.drag = Drag::LinearMulti { id, before },
            Drag::None | Drag::Pan { .. } | Drag::BoxSelect { .. } => {}
        }
    }

    // ------------------------------------------------------------------
    // Teclado
    // ------------------------------------------------------------------

    /// Atalhos de teclado, iguais aos do Excalidraw (ver `HelpDialog.tsx` e `shortcuts.ts`).
    pub fn handle_keys(&mut self, ctx: &egui::Context, view: egui::Rect) {
        // eventos de área de transferência
        let events = ctx.input(|i| i.events.clone());
        let typing = ctx.egui_wants_keyboard_input();
        for ev in &events {
            match ev {
                _ if typing || self.view_mode => {}
                egui::Event::Copy => self.copy_selection(ctx),
                egui::Event::Cut => {
                    self.copy_selection(ctx);
                    self.delete_selected();
                }
                egui::Event::Paste(t) => self.paste(t),
                _ => {}
            }
        }
        if let Some(ed) = &self.editing {
            let _ = ed;
            let esc = ctx.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Escape)
                    || i.consume_key(Modifiers::COMMAND, Key::Enter)
            });
            if esc {
                self.finish_editing();
            }
            return;
        }
        if typing {
            return;
        }
        let n = Modifiers::NONE;
        let sh = Modifiers::SHIFT;
        let alt = Modifiers::ALT;
        let cmd = Modifiers::COMMAND;
        let cs = cmd | sh;
        let ca = cmd | alt;
        // `k` compara a tecla lógica (`event.key` no Excalidraw);
        // `kc` compara a posição física (`event.code`), que independe do layout.
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| take_key(i, m, key, false));
        let kc = |m: Modifiers, key: Key| ctx.input_mut(|i| take_key(i, m, key, true));

        // --- visualização e câmera (valem também no modo de visualização) ---
        if kc(alt, Key::R) {
            self.toggle_view_mode();
        }
        if kc(alt, Key::Z) {
            self.zen_mode = !self.zen_mode;
        }
        if kc(alt, Key::Slash) {
            self.show_stats = !self.show_stats;
        }
        if kc(alt | sh, Key::D) {
            self.dark = !self.dark;
        }
        if kc(cmd, Key::Quote) {
            self.grid = !self.grid;
        }
        for m in [cmd, sh, cs] {
            if kc(m, Key::Equals) || k(m, Key::Plus) {
                self.zoom_step(view, 1.0);
            }
            if kc(m, Key::Minus) {
                self.zoom_step(view, -1.0);
            }
            if kc(m, Key::Num0) {
                self.reset_zoom(view);
            }
        }
        if kc(sh, Key::Num1) {
            self.zoom_to_fit(view);
        }
        if kc(sh, Key::Num2) {
            self.zoom_to_selection(view, false);
        }
        if kc(sh, Key::Num3) {
            self.zoom_to_selection(view, true);
        }
        for m in [n, sh] {
            if k(m, Key::PageUp) {
                self.scroll_page(view, false, m == sh);
            }
            if k(m, Key::PageDown) {
                self.scroll_page(view, true, m == sh);
            }
        }
        if ctx.input_mut(|i| i.consume_key(n, Key::Questionmark)) || k(n, Key::F1) {
            self.show_help = !self.show_help;
        }
        if self.view_mode {
            return;
        }

        // --- arquivo e histórico ---
        if k(cs, Key::Z) || k(cmd, Key::Y) {
            self.redo();
        }
        if k(cmd, Key::Z) {
            self.undo();
        }
        if k(cs, Key::S) {
            self.save_as();
        }
        if k(cmd, Key::S) {
            self.save();
        }
        if k(cmd, Key::O) {
            self.open_dialog();
        }
        if k(cs, Key::E) {
            self.show_export = true;
        }
        if (k(cmd, Key::Delete) || k(cmd, Key::Backspace)) && !self.elements.is_empty() {
            self.confirm = Some(Pending::Clear);
        }

        // --- edição ---
        if k(cmd, Key::A) {
            self.select_all();
        }
        if k(cmd, Key::D) {
            self.duplicate_selected();
        }
        if k(cs, Key::G) {
            self.group_selected(false);
        }
        if k(cmd, Key::G) {
            self.group_selected(true);
        }
        // no macOS, "até o fim" é Cmd+Alt+[ ]; nos demais, Ctrl+Shift+[ ]
        let all_the_way = if cfg!(target_os = "macos") { ca } else { cs };
        if kc(all_the_way, Key::CloseBracket) {
            self.reorder(true, true);
        }
        if kc(cmd, Key::CloseBracket) {
            self.reorder(true, false);
        }
        if kc(all_the_way, Key::OpenBracket) {
            self.reorder(false, true);
        }
        if kc(cmd, Key::OpenBracket) {
            self.reorder(false, false);
        }
        if kc(ca, Key::C) {
            self.copy_styles();
        }
        if kc(ca, Key::V) {
            self.paste_styles();
        }
        if kc(alt | sh, Key::C) {
            self.copy_png(ctx);
        }
        if k(cs, Key::L) {
            self.toggle_lock();
        }
        if kc(sh, Key::H) {
            self.flip_selected(true);
        }
        if kc(sh, Key::V) {
            self.flip_selected(false);
        }
        if kc(cs, Key::Comma) {
            self.change_font_size(false);
        }
        if kc(cs, Key::Period) {
            self.change_font_size(true);
        }
        if !self.selected.is_empty() {
            if k(n, Key::Tab) {
                self.convert_selected(true);
            }
            if k(sh, Key::Tab) {
                self.convert_selected(false);
            }
        }
        for (key, dx, dy) in [
            (Key::ArrowLeft, -1, 0),
            (Key::ArrowRight, 1, 0),
            (Key::ArrowUp, 0, -1),
            (Key::ArrowDown, 0, 1),
        ] {
            if k(cs, key) {
                self.align_selected(dx, dy);
            }
        }
        if k(n, Key::Delete) || k(n, Key::Backspace) {
            self.delete_selected();
        }
        if k(n, Key::Escape) {
            if matches!(self.drag, Drag::LinearMulti { .. }) {
                self.finish_linear();
            } else {
                self.selected.clear();
                self.show_menu = false;
                self.tool = Tool::Select;
            }
        }
        if k(n, Key::Enter) {
            if matches!(self.drag, Drag::LinearMulti { .. }) {
                self.finish_linear();
            } else if let Some(el) = self.single_selected().filter(|e| e.kind == "text") {
                let id = el.id.clone();
                self.edit_text(&id);
            }
        }
        // setas: mover a seleção 1px (Shift: 5px); com a grade, 1 célula (Shift: 1px)
        for (key, dx, dy) in [
            (Key::ArrowLeft, -1.0, 0.0),
            (Key::ArrowRight, 1.0, 0.0),
            (Key::ArrowUp, 0.0, -1.0),
            (Key::ArrowDown, 0.0, 1.0),
        ] {
            let shift = k(sh, key);
            if shift || k(n, key) {
                if self.selected.is_empty() {
                    continue;
                }
                let step = match (self.grid, shift) {
                    (true, false) => GRID as f64,
                    (true, true) | (false, false) => 1.0,
                    (false, true) => 5.0,
                };
                let before = self.elements.clone();
                let sel = self.selected.clone();
                for e in self.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
                    e.x += dx * step;
                    e.y += dy * step;
                    e.touch();
                }
                self.commit(before);
            }
        }
        // S / G: abrem o seletor de cor do traço / do fundo
        let has_bg =
            |k: &str| matches!(k, "rectangle" | "diamond" | "ellipse" | "line" | "freedraw");
        let sel_kinds: Vec<String> = self
            .selected_elements()
            .iter()
            .map(|e| e.kind.clone())
            .collect();
        let tool_kind = self.tool.kind();
        let styled = !sel_kinds.is_empty() || tool_kind.is_some();
        if k(n, Key::S) && styled {
            self.open_picker = Some(false);
        }
        if k(n, Key::G) && (sel_kinds.iter().any(|k| has_bg(k)) || tool_kind.is_some_and(has_bg)) {
            self.open_picker = Some(true);
        }
        // ferramentas
        let tools = [
            (Key::H, Tool::Hand),
            (Key::V, Tool::Select),
            (Key::Num1, Tool::Select),
            (Key::R, Tool::Rect),
            (Key::Num2, Tool::Rect),
            (Key::D, Tool::Diamond),
            (Key::Num3, Tool::Diamond),
            (Key::O, Tool::Ellipse),
            (Key::Num4, Tool::Ellipse),
            (Key::A, Tool::Arrow),
            (Key::Num5, Tool::Arrow),
            (Key::L, Tool::Line),
            (Key::Num6, Tool::Line),
            (Key::P, Tool::Draw),
            (Key::X, Tool::Draw),
            (Key::Num7, Tool::Draw),
            (Key::T, Tool::Text),
            (Key::Num8, Tool::Text),
            (Key::E, Tool::Eraser),
            (Key::Num0, Tool::Eraser),
        ];
        for (key, tool) in tools {
            if k(n, key) {
                // mão e borracha alternam de volta para a ferramenta anterior
                if matches!(tool, Tool::Hand | Tool::Eraser) && self.tool == tool {
                    self.set_tool(self.prev_tool);
                } else {
                    self.set_tool(tool);
                }
            }
        }
        if k(n, Key::Q) {
            self.tool_locked = !self.tool_locked;
        }
    }

    // ------------------------------------------------------------------
    // Desenho do canvas
    // ------------------------------------------------------------------

    pub fn begin_canvas(&mut self, ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
        let rect = ui.max_rect();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        self.cam.origin = rect.min;
        if self.pending_fit {
            self.pending_fit = false;
            self.zoom_to_fit(rect);
        }
        (rect, resp)
    }

    pub fn paint_canvas(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let painter = ui.painter_at(rect);

        // mede textos colados
        if !self.needs_measure.is_empty() {
            let ids: Vec<String> = self.needs_measure.drain().collect();
            for id in ids {
                let hand_ok = self.hand_ok;
                if let Some(el) = self.get_mut(&id) {
                    let (w, h) = render::measure_text(
                        &painter,
                        el.text(),
                        el.font_size() as f32,
                        el.font_family(),
                        el.line_height() as f32,
                        hand_ok,
                    );
                    let cx = el.x + el.width / 2.0;
                    el.width = w as f64;
                    el.height = h as f64;
                    if el.text_align() == "center" {
                        el.x = cx - el.width / 2.0;
                    }
                    el.touch();
                }
            }
        }
        self.update_editing_text(&painter);

        let bg = crate::model::parse_color(&self.view_bg).unwrap_or([255, 255, 255, 255]);
        painter.rect_filled(rect, 0.0, render::c32(bg, self.dark, 1.0));

        // grade
        if self.grid {
            let z = self.cam.zoom;
            let step = if GRID * z < 8.0 { GRID * 5.0 } else { GRID };
            let tl = self.cam.w(rect.min);
            let br = self.cam.w(rect.max);
            let line_col = if self.dark {
                Color32::from_gray(32)
            } else {
                Color32::from_gray(236)
            };
            let bold_col = if self.dark {
                Color32::from_gray(44)
            } else {
                Color32::from_gray(222)
            };
            let mut x = (tl.x / step).floor() * step;
            while x <= br.x {
                let bold = (x / (step * 5.0)).fract().abs() < 1e-3;
                let sx = self.cam.s(P::new(x, 0.0)).x;
                painter.vline(
                    sx,
                    rect.y_range(),
                    Stroke::new(1.0, if bold { bold_col } else { line_col }),
                );
                x += step;
            }
            let mut y = (tl.y / step).floor() * step;
            while y <= br.y {
                let bold = (y / (step * 5.0)).fract().abs() < 1e-3;
                let sy = self.cam.s(P::new(0.0, y)).y;
                painter.hline(
                    rect.x_range(),
                    sy,
                    Stroke::new(1.0, if bold { bold_col } else { line_col }),
                );
                y += step;
            }
        }

        // elementos
        let view_world = Rect::from_two(self.cam.w(rect.min), self.cam.w(rect.max));
        let editing_id = self.editing.as_ref().map(|e| e.id.clone());
        let marked: HashSet<String> = match &self.drag {
            Drag::Erase { marked, .. } => marked.clone(),
            _ => HashSet::new(),
        };
        let mut live: HashSet<&str> = HashSet::new();
        for el in &self.elements {
            live.insert(el.id.as_str());
            let entry = self.cache.entry(el.id.clone()).or_insert_with(|| Cached {
                version: i64::MIN,
                nonce: 0,
                prims: vec![],
                bounds: Rect::from_two(P::default(), P::default()),
            });
            if entry.version != el.version || entry.nonce != el.version_nonce {
                entry.prims = geom::element_prims(el);
                entry.bounds = geom::aabb(el).expand(el.stroke_width as f32 * 2.0 + 30.0);
                entry.version = el.version;
                entry.nonce = el.version_nonce;
            }
            if Some(&el.id) == editing_id.as_ref() {
                continue;
            }
            let b = &entry.bounds;
            if b.max.x < view_world.min.x
                || b.min.x > view_world.max.x
                || b.max.y < view_world.min.y
                || b.min.y > view_world.max.y
            {
                continue;
            }
            let pc = PaintCtx {
                painter: &painter,
                cam: self.cam,
                dark: self.dark,
                alpha: if marked.contains(&el.id) { 0.25 } else { 1.0 },
                hand_ok: self.hand_ok,
            };
            render::paint_prims(&pc, &entry.prims);
        }
        if self.cache.len() > self.elements.len() + 64 {
            let keep: HashSet<String> = live.iter().map(|s| s.to_string()).collect();
            self.cache.retain(|k, _| keep.contains(k));
        }

        // seleção
        let sel_stroke = Stroke::new(1.0, SELECTION);
        if self.tool == Tool::Select || matches!(self.drag, Drag::BoxSelect { .. }) {
            let sel: Vec<&Element> = self.selected_elements();
            let multi = sel.len() > 1;
            for el in &sel {
                if Some(&el.id) == editing_id.as_ref() {
                    continue;
                }
                let pad = 4.0 / self.cam.zoom;
                let c = geom::center(el);
                let b = geom::local_box(el).expand(pad);
                let a = el.angle as f32;
                let pts: Vec<Pos2> = [
                    b.min,
                    P::new(b.max.x, b.min.y),
                    b.max,
                    P::new(b.min.x, b.max.y),
                ]
                .iter()
                .map(|p| self.cam.s(rotate(*p, c, a)))
                .collect();
                if el.is_linear() && !multi {
                    continue;
                }
                let stroke = if multi {
                    Stroke::new(1.0, SELECTION.gamma_multiply(0.5))
                } else {
                    sel_stroke
                };
                painter.add(egui::Shape::closed_line(pts, stroke));
            }
            if multi && let Some(b) = self.selection_bounds() {
                let r = egui::Rect::from_two_pos(self.cam.s(b.min), self.cam.s(b.max)).expand(4.0);
                painter.add(egui::Shape::dashed_line(
                    &[
                        r.left_top(),
                        r.right_top(),
                        r.right_bottom(),
                        r.left_bottom(),
                        r.left_top(),
                    ],
                    sel_stroke,
                    5.0,
                    4.0,
                ));
            }
            let handle_fill = if self.dark {
                Color32::from_gray(30)
            } else {
                Color32::WHITE
            };
            for (h, p) in self.handles() {
                let s = self.cam.s(p);
                match h {
                    Handle::Rotate | Handle::Point(_) => {
                        painter.circle(s, HANDLE_PX / 2.0 + 1.0, handle_fill, sel_stroke);
                    }
                    _ => {
                        let r =
                            egui::Rect::from_center_size(s, egui::vec2(HANDLE_PX, HANDLE_PX) * 1.2);
                        painter.rect(r, 2.0, handle_fill, sel_stroke, StrokeKind::Middle);
                    }
                }
            }
        }
        if let Drag::BoxSelect { start, cur, .. } = &self.drag {
            let r = egui::Rect::from_two_pos(self.cam.s(*start), self.cam.s(*cur));
            painter.rect(
                r,
                0.0,
                SELECTION.gamma_multiply(0.08),
                sel_stroke,
                StrokeKind::Middle,
            );
        }
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        self.now = ctx.input(|i| i.time);
        let prefs = (self.dark, self.grid, self.view_bg.clone());
        if self.last_prefs.as_ref() != Some(&prefs) {
            if self.last_prefs.is_some() {
                self.autosave_pending = true;
                self.last_change = self.now;
                self.export_preview = None;
            }
            self.last_prefs = Some(prefs);
        }
        if self.autosave_pending && self.now - self.last_change > 1.0 {
            self.write_autosave();
        }
        if self.autosave_pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(1100));
        }
        let title = format!(
            "{}{} — RustDraw",
            if self.dirty { "• " } else { "" },
            self.file_path
                .as_deref()
                .map(file_name)
                .unwrap_or_else(|| "Sem título".into())
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        // arquivos arrastados para a janela
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(p) = dropped.iter().map(|f| f.path().to_path_buf()).next() {
            self.load_path(&p);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.flush_autosave();
    }
}

/// Espelha um elemento em torno da reta x = mid/2 (ou y = mid/2).
fn flip(e: &mut Element, mid: f64, horizontal: bool) {
    if e.has_points() {
        let pts = e
            .points()
            .into_iter()
            .map(|[x, y]| if horizontal { [-x, y] } else { [x, -y] })
            .collect();
        if horizontal {
            e.x = mid - e.x;
        } else {
            e.y = mid - e.y;
        }
        e.set_points(pts);
    } else if horizontal {
        e.x = mid - e.x - e.width;
    } else {
        e.y = mid - e.y - e.height;
    }
    if e.angle != 0.0 {
        e.angle = std::f64::consts::TAU - e.angle;
    }
    e.touch();
}

pub fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn snap_angle(origin: P, p: P) -> P {
    let d = p - origin;
    let step = 15f32.to_radians();
    let a = (d.y.atan2(d.x) / step).round() * step;
    let l = d.len();
    origin + P::new(a.cos() * l, a.sin() * l)
}

/// Redimensiona `orig` arrastando a alça `h` até `p` (mundo).
pub fn resize(orig: &Element, h: Handle, p: P, keep_ratio: bool, grid: bool) -> Element {
    let mut el = orig.clone();
    let c0 = geom::center(orig);
    let a = orig.angle as f32;
    let b = geom::local_box(orig);
    let mut q = rotate(p, c0, -a);
    if grid && a == 0.0 {
        q = P::new(snap(q.x, true), snap(q.y, true));
    }
    if let Handle::Point(i) = h {
        let mut pts = orig.points();
        if i < pts.len() {
            pts[i] = [q.x as f64 - orig.x, q.y as f64 - orig.y];
            let off = pts[0];
            for pt in pts.iter_mut() {
                pt[0] -= off[0];
                pt[1] -= off[1];
            }
            el.x = orig.x + off[0];
            el.y = orig.y + off[1];
            el.set_points(pts);
        }
        return el;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (b.min.x, b.min.y, b.max.x, b.max.y);
    let text = orig.kind == "text";
    let keep_ratio = keep_ratio || text;
    match h {
        Handle::N => y0 = q.y,
        Handle::S => y1 = q.y,
        Handle::E => x1 = q.x,
        Handle::W => x0 = q.x,
        Handle::NE => {
            x1 = q.x;
            y0 = q.y
        }
        Handle::NW => {
            x0 = q.x;
            y0 = q.y
        }
        Handle::SE => {
            x1 = q.x;
            y1 = q.y
        }
        Handle::SW => {
            x0 = q.x;
            y1 = q.y
        }
        _ => {}
    }
    let corner = matches!(h, Handle::NE | Handle::NW | Handle::SE | Handle::SW);
    if keep_ratio && corner && b.width() > 0.0 && b.height() > 0.0 {
        let (ow, oh) = (b.width(), b.height());
        let s = ((x1 - x0) / ow).abs().max(((y1 - y0) / oh).abs());
        // âncora = canto oposto
        let (ax, ay) = (
            if matches!(h, Handle::NE | Handle::SE) {
                b.min.x
            } else {
                b.max.x
            },
            if matches!(h, Handle::SE | Handle::SW) {
                b.min.y
            } else {
                b.max.y
            },
        );
        let sx = if q.x >= ax { 1.0 } else { -1.0 };
        let sy = if q.y >= ay { 1.0 } else { -1.0 };
        x0 = ax;
        x1 = ax + sx * ow * s;
        y0 = ay;
        y1 = ay + sy * oh * s;
    }
    let nb = Rect::from_two(P::new(x0, y0), P::new(x1, y1));
    let (w, hgt) = (nb.width().max(1.0), nb.height().max(1.0));
    let cw = rotate(nb.center(), c0, a);
    let new_min = P::new(cw.x - w / 2.0, cw.y - hgt / 2.0);
    if orig.has_points() {
        let sx = if b.width() > 0.0 { w / b.width() } else { 1.0 };
        let sy = if b.height() > 0.0 {
            hgt / b.height()
        } else {
            1.0
        };
        let flip_x = x1 < x0;
        let flip_y = y1 < y0;
        let pts: Vec<P> = geom::abs_points(orig)
            .into_iter()
            .map(|p| {
                let mut rx = (p.x - b.min.x) * sx;
                let mut ry = (p.y - b.min.y) * sy;
                if flip_x {
                    rx = w - rx;
                }
                if flip_y {
                    ry = hgt - ry;
                }
                P::new(new_min.x + rx, new_min.y + ry)
            })
            .collect();
        el.x = pts[0].x as f64;
        el.y = pts[0].y as f64;
        el.set_points(
            pts.iter()
                .map(|p| [(p.x - pts[0].x) as f64, (p.y - pts[0].y) as f64])
                .collect(),
        );
    } else {
        if text && b.height() > 0.0 {
            let s = hgt / b.height();
            el.set_field(
                "fontSize",
                serde_json::json!((orig.font_size() * s as f64).max(1.0)),
            );
        }
        el.x = new_min.x as f64;
        el.y = new_min.y as f64;
        el.width = w as f64;
        el.height = hgt as f64;
    }
    el
}

/// Registra as fontes no egui. Retorna se a Virgil foi carregada.
fn setup_fonts(ctx: &egui::Context) -> bool {
    let mut defs = egui::FontDefinitions::default();
    let Some(ttf) = crate::fonts::virgil_ttf() else {
        return false;
    };
    defs.font_data.insert(
        "virgil".into(),
        std::sync::Arc::new(egui::FontData::from_static(ttf)),
    );
    let mut fam = vec!["virgil".to_string()];
    fam.extend(
        defs.families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    defs.families
        .insert(egui::FontFamily::Name("virgil".into()), fam);
    ctx.set_fonts(defs);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(angle: f64) -> Element {
        let mut e = Element::new("rectangle", 0.0, 0.0, &Style::default());
        e.width = 100.0;
        e.height = 50.0;
        e.angle = angle;
        e
    }

    #[test]
    fn resize_keeps_opposite_corner() {
        let e = rect(0.0);
        let r = resize(&e, Handle::SE, P::new(150.0, 80.0), false, false);
        assert_eq!((r.x, r.y, r.width, r.height), (0.0, 0.0, 150.0, 80.0));
        // arrastar além da âncora espelha
        let r = resize(&e, Handle::E, P::new(-20.0, 10.0), false, false);
        assert_eq!((r.x, r.width), (-20.0, 20.0));
    }

    #[test]
    fn resize_rotated_anchor_is_fixed() {
        let e = rect(0.7);
        let before = geom::corners(&e)[0]; // nw
        let target = rotate(P::new(160.0, 90.0), geom::center(&e), 0.7);
        let r = resize(&e, Handle::SE, target, false, false);
        let after = geom::corners(&r)[0];
        assert!((before - after).len() < 0.01, "{before:?} vs {after:?}");
    }

    #[test]
    fn resize_line_point() {
        let mut e = Element::new("arrow", 10.0, 10.0, &Style::default());
        e.set_points(vec![[0.0, 0.0], [100.0, 0.0]]);
        let r = resize(&e, Handle::Point(0), P::new(0.0, 0.0), false, false);
        assert_eq!((r.x, r.y), (0.0, 0.0));
        assert_eq!(r.points(), vec![[0.0, 0.0], [110.0, 10.0]]);
    }

    #[test]
    fn flip_shape_and_line() {
        // retângulo em [0,100] espelhado em torno de x = 150 vai para [200,300]
        let mut e = rect(0.3);
        flip(&mut e, 300.0, true);
        assert_eq!((e.x, e.y), (200.0, 0.0));
        assert!((e.angle - (std::f64::consts::TAU - 0.3)).abs() < 1e-9);
        // seta de (10,10) a (110,30), invertida na vertical em torno de y = 20
        let mut a = Element::new("arrow", 10.0, 10.0, &Style::default());
        a.set_points(vec![[0.0, 0.0], [100.0, 20.0]]);
        flip(&mut a, 40.0, false);
        assert_eq!((a.x, a.y), (10.0, 30.0));
        assert_eq!(a.points(), vec![[0.0, 0.0], [100.0, -20.0]]);
    }

    fn key_event(key: Key, physical: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(physical),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn take_key_matches_modifiers_exactly() {
        let mut i = egui::InputState::default();
        i.events.push(key_event(Key::H, Key::H, Modifiers::SHIFT));
        // H sem Shift (ferramenta mão) não pode disparar com Shift+H (inverter)
        assert!(!take_key(&mut i, Modifiers::NONE, Key::H, false));
        assert!(take_key(&mut i, Modifiers::SHIFT, Key::H, true));
        assert!(i.events.is_empty());
        // Shift+1 chega como "!" na tecla lógica; a posição física é o 1
        i.events
            .push(key_event(Key::Exclamationmark, Key::Num1, Modifiers::SHIFT));
        assert!(!take_key(&mut i, Modifiers::SHIFT, Key::Num1, false));
        assert!(take_key(&mut i, Modifiers::SHIFT, Key::Num1, true));
    }
}
