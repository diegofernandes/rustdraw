//! Estado do editor, entrada de ponteiro/teclado e ações.

use crate::binding::{self, Binding, Candidate};
use crate::elbow;
use crate::export::{self, ExportOpts};
use crate::files;
use crate::frame;
use crate::geom::{self, P, Prim, Rect, rotate};
use crate::library::{self, LibraryItem};
use crate::live;
use crate::mcp;
use crate::model::{Element, SceneFile, Style, clipboard_json, new_id, rand_seed};
use crate::render::{self, Cam, PaintCtx};
use crate::snap;
use crate::watch::Watch;
use egui::{Color32, CursorIcon, Key, Modifiers, PointerButton, Pos2, Sense, Stroke, StrokeKind};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

pub const SELECTION: Color32 = Color32::from_rgb(0x69, 0x65, 0xdb);
const HANDLE_PX: f32 = 7.0;
/// Segmentos mais curtos que isso (pixels de tela) não mostram a alça do meio.
const MID_MIN_PX: f32 = 28.0;
const GRID: f32 = 20.0;
/// De quanto em quanto tempo o arquivo aberto é consultado no disco.
const WATCH_INTERVAL_MS: u64 = 700;

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
    Frame,
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
            Tool::Frame => "frame",
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
    /// Meio do segmento que começa no ponto `i` de uma linha/seta: arrastar
    /// insere um ponto (em cotovelo, move o segmento).
    Mid(usize),
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
        /// Caixa da seleção no início, para o encaixe em objetos.
        bounds: Option<Rect>,
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
    /// Arrastando pontos de uma linha/seta (todos os selecionados juntos).
    Points {
        orig: Element,
        indices: Vec<usize>,
        start: P,
        before: Vec<Element>,
        /// O ponto foi criado agora (alça do meio): um clique sem arrastar desfaz.
        inserted: bool,
    },
    /// Arrastando um segmento de uma seta em cotovelo.
    Segment {
        orig: Element,
        index: usize,
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

/// Nome de frame em edição (duplo clique no nome, ou Enter com o frame selecionado).
pub struct FrameRename {
    pub id: String,
    pub buf: String,
    pub before: Vec<Element>,
    pub focus: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    New,
    Open,
    /// Limpar o canvas (Ctrl+Delete).
    Clear,
    /// Esvaziar a biblioteca (ou remover os itens marcados).
    ClearLibrary,
    /// O arquivo aberto mudou no disco e há alterações não salvas.
    Reload,
}

struct Cached {
    version: i64,
    nonce: i64,
    prims: Vec<Prim>,
    bounds: Rect,
}

/// Menu de opções do botão direito, como o do Excalidraw.
pub struct ContextMenu {
    /// Posição na tela onde o menu abre.
    pub pos: Pos2,
    /// Clique sobre um elemento (menu do elemento) ou no canvas vazio.
    pub on_element: bool,
    /// Aberto neste quadro: o próprio clique que o abriu não deve fechá-lo.
    pub fresh: bool,
}

pub struct App {
    pub elements: Vec<Element>,
    /// Mapa `files` do `.excalidraw` (imagens). Guarda também imagens já
    /// apagadas, para o desfazer; só as usadas vão para o arquivo.
    pub files: Map<String, Value>,
    /// Texturas das imagens, por `fileId`.
    pub textures: HashMap<String, egui::TextureHandle>,
    /// Imagens que não decodificaram (não tenta de novo a cada quadro).
    bad_images: HashSet<String>,
    /// Área do canvas na tela, do último quadro.
    view: egui::Rect,
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
    /// Acompanha o arquivo aberto, para recarregar quando outro programa o altera.
    watch: Option<Watch>,
    last_watch: f64,
    /// Escuta o `rustdraw mcp` ("Permitir edição por IA" ligado).
    pub live: Option<live::Host>,
    /// Pedidos da IA esperando o fim de um arrasto ou de um texto.
    live_queue: Vec<live::Request>,
    pub dark: bool,
    pub grid: bool,
    /// Encaixe em objetos (Alt+S), como o "Snap to objects" do Excalidraw.
    pub snap: bool,
    /// Guias de alinhamento do encaixe em andamento.
    pub snap_guides: Vec<snap::Guide>,
    pub view_bg: String,
    pub show_menu: bool,
    pub context_menu: Option<ContextMenu>,
    /// Onde colar quando "Colar" vem do menu de contexto (e quando foi pedido).
    paste_at: Option<(P, f64)>,
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
    last_prefs: Option<(bool, bool, bool, String)>,
    /// Diálogo de arquivos aberto, aguardando a escolha do usuário.
    dialog: Option<DialogTask>,
    ctx: egui::Context,
    /// Forma (e ponto de encaixe) onde a ponta da seta vai ligar, para destacar.
    pub bind_hint: Option<Candidate>,
    /// Pontos selecionados da linha/seta selecionada (Delete apaga só eles).
    pub point_sel: Option<(String, BTreeSet<usize>)>,
    /// Estado das setas ligadas já vistas (ver `binding::sync`).
    bind_seen: HashMap<String, Vec<i64>>,
    /// Modificadores do último quadro (Ctrl/Cmd desliga a ligação de setas).
    mods: Modifiers,
    /// Onde o nome de cada frame foi desenhado (tela), para clicar nele.
    pub frame_labels: Vec<(String, egui::Rect)>,
    pub renaming: Option<FrameRename>,
    /// Frame onde os elementos arrastados vão entrar, para destacar.
    frame_hint: Option<String>,
    /// Biblioteca pessoal (itens mais recentes primeiro).
    pub library: Vec<LibraryItem>,
    pub show_library: bool,
    /// Itens marcados no painel da biblioteca (Shift+clique).
    pub library_selected: HashSet<String>,
    /// Item sendo arrastado do painel para o canvas.
    pub library_drag: Option<String>,
    /// Miniaturas já calculadas: limites e primitivas de cada item.
    pub library_thumbs: HashMap<String, (Rect, Vec<Prim>)>,
    /// Estado da biblioteca antes da última remoção ou edição, para "Desfazer".
    pub library_undo: Option<(Vec<LibraryItem>, String)>,
    /// Item sendo renomeado e o texto digitado.
    pub library_rename: Option<(String, String)>,
    /// Item sendo editado no canvas (ver [`App::edit_library_item`]).
    pub library_edit: Option<String>,
}

/// O que fazer com o caminho escolhido no diálogo de arquivos.
enum DialogKind {
    Open,
    /// Salvar e, se der certo, seguir com a ação adiada pela confirmação.
    Save(Option<Pending>),
    Export {
        svg: bool,
        els: Vec<Element>,
        opts: ExportOpts,
        base: String,
    },
    ImportLibrary,
    ExportLibrary(Vec<LibraryItem>),
    /// Inserir imagem (ferramenta de imagem, tecla 9).
    Image,
}

/// `Err(())` = o sistema não tem diálogo de arquivos (ver [`NO_DIALOG`]).
type DialogResult = Result<Option<PathBuf>, ()>;

struct DialogTask {
    kind: DialogKind,
    rx: std::sync::mpsc::Receiver<DialogResult>,
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

/// Espera o diálogo nativo numa thread e devolve o resultado pelo canal.
///
/// O diálogo é assíncrono porque, no macOS, o diálogo síncrono roda um loop
/// modal dentro do tratamento de evento do winit, que entra em pânico com
/// "tried to handle event while another event is currently being handled".
fn spawn_dialog(
    ctx: &egui::Context,
    fut: impl std::future::Future<Output = Option<rfd::FileHandle>> + Send + 'static,
) -> std::sync::mpsc::Receiver<DialogResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let r = pollster::block_on(fut).map(|h| h.path().to_path_buf());
        // sem diálogo no sistema, o rfd devolve `None` na hora
        let r = if r.is_none() && t.elapsed() < std::time::Duration::from_millis(120) {
            Err(())
        } else {
            Ok(r)
        };
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    rx
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
            files: Map::new(),
            textures: HashMap::new(),
            bad_images: HashSet::new(),
            view: egui::Rect::NOTHING,
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
            watch: None,
            last_watch: 0.0,
            live: None,
            live_queue: vec![],
            dark: false,
            grid: false,
            snap: false,
            snap_guides: vec![],
            view_bg: "#ffffff".into(),
            show_menu: false,
            context_menu: None,
            paste_at: None,
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
            dialog: None,
            ctx: cc.egui_ctx.clone(),
            bind_hint: None,
            point_sel: None,
            bind_seen: HashMap::new(),
            mods: Modifiers::NONE,
            frame_labels: vec![],
            renaming: None,
            frame_hint: None,
            library: library::path()
                .filter(|p| p.exists())
                .and_then(|p| library::load(&p).ok())
                .unwrap_or_default(),
            show_library: false,
            library_selected: HashSet::new(),
            library_drag: None,
            library_thumbs: HashMap::new(),
            library_undo: None,
            library_rename: None,
            library_edit: None,
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
            // sem alterações locais, o que estiver no disco vale (recarrega no
            // primeiro quadro se mudou com o editor fechado)
            app.watch = app.file_path.as_deref().map(|fp| {
                let known = if app.dirty {
                    std::fs::read(fp).unwrap_or_default()
                } else {
                    vec![]
                };
                Watch::new(fp, &known)
            });
            app.dark = f
                .app_state
                .get("theme")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t == "dark");
            app.snap = f
                .app_state
                .get("objectsSnapModeEnabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if f.app_state.get("rustdrawMcpLive").and_then(|v| v.as_bool()) == Some(true) {
                app.set_live(true);
                app.toast = None;
            }
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
        self.sync_bindings();
        binding::tidy(&mut self.elements);
        frame::tidy(&mut self.elements);
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

    /// Faz as setas ligadas acompanharem as formas que mudaram.
    pub fn sync_bindings(&mut self) {
        binding::sync(&mut self.elements, &mut self.bind_seen);
    }

    /// Solta as pontas de setas de `moved` cuja forma ficou para trás
    /// (a seta foi movida sozinha, como no Excalidraw).
    fn unbind_detached(&mut self, moved: &HashSet<String>) {
        for e in self
            .elements
            .iter_mut()
            .filter(|e| e.kind == "arrow" && moved.contains(&e.id))
        {
            let mut changed = false;
            for end in [false, true] {
                if binding::get(e, end).is_some_and(|b| !moved.contains(&b.element_id)) {
                    binding::set(e, end, None);
                    changed = true;
                }
            }
            if changed {
                e.touch();
            }
        }
    }

    /// Escolhe onde a ponta `end` da seta `id` liga, estando em `p`.
    /// Com Ctrl/Cmd, não liga.
    fn bind_end_at(&mut self, id: &str, end: bool, p: P, mods: Modifiers) {
        let Some(el) = self.elements.iter().find(|e| e.id == id) else {
            return;
        };
        if el.kind != "arrow" {
            return;
        }
        let other = binding::get(el, !end);
        let cand = if mods.command {
            None
        } else {
            binding::candidate(&self.elements, p, self.cam.zoom, id).filter(|c| {
                // as duas pontas na mesma forma só entre pontos fixos
                other.as_ref().is_none_or(|o| {
                    o.element_id != c.id || (o.fixed.is_some() && c.fixed.is_some())
                })
            })
        };
        let b = cand.as_ref().map(|c| Binding::new(&c.id, c.fixed));
        if let Some(el) = self.get_mut(id) {
            binding::set(el, end, b.as_ref());
            el.touch();
        }
        binding::refresh_arrow(&mut self.elements, id);
        self.bind_hint = cand;
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
        self.renaming = None;
        self.mark_changed();
    }

    pub fn scene_file(&self) -> SceneFile {
        let mut f = SceneFile::new(self.elements.clone(), &self.view_bg, self.grid);
        f.set_files(&self.files);
        f
    }

    fn apply_scene(&mut self, f: SceneFile) {
        self.view_bg = f.view_background().unwrap_or_else(|| "#ffffff".into());
        self.grid = f.grid_enabled();
        self.files = f.files_map();
        self.textures.clear();
        self.bad_images.clear();
        self.elements = f.elements;
        self.bind_seen.clear();
        self.bind_hint = None;
        self.selected.clear();
        self.undo.clear();
        self.redo.clear();
        self.cache.clear();
        self.drag = Drag::None;
        self.editing = None;
        self.renaming = None;
        self.export_preview = None;
        self.cam.zoom = 1.0;
        self.cam.scroll = P::new(0.0, 0.0);
        self.pending_fit = true;
    }

    // ------------------------------------------------------------------
    // Arquivos
    // ------------------------------------------------------------------

    pub fn load_path(&mut self, p: &std::path::Path) {
        let text = std::fs::read_to_string(p).map_err(|e| e.to_string());
        if let Ok(t) = &text
            && library::is_library(t)
        {
            self.import_library_text(t);
            return;
        }
        match text.and_then(|t| SceneFile::parse(&t).map(|f| (f, t))) {
            Ok((f, t)) => {
                self.apply_scene(f);
                self.watch = Some(Watch::new(p, t.as_bytes()));
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
        let fut = rfd::AsyncFileDialog::new()
            .set_title("Abrir desenho")
            .add_filter("Excalidraw", &["excalidraw", "json", library::EXT])
            .pick_file();
        self.start_dialog(DialogKind::Open, fut);
    }

    /// Mostra o diálogo, a não ser que já haja um aberto.
    fn start_dialog(
        &mut self,
        kind: DialogKind,
        fut: impl std::future::Future<Output = Option<rfd::FileHandle>> + Send + 'static,
    ) {
        if self.dialog.is_none() {
            let rx = spawn_dialog(&self.ctx, fut);
            self.dialog = Some(DialogTask { kind, rx });
        }
    }

    fn poll_dialog(&mut self) {
        let Some(task) = &self.dialog else { return };
        let r = match task.rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Ok(None),
        };
        let Some(task) = self.dialog.take() else {
            return;
        };
        match task.kind {
            DialogKind::Open => match r {
                Ok(Some(p)) => self.load_path(&p),
                Ok(None) => {}
                Err(()) => self.toast(format!(
                    "{NO_DIALOG}. Dica: arraste o arquivo para a janela."
                )),
            },
            DialogKind::Save(then) => {
                let picked = r.unwrap_or_else(|()| fallback_path("desenho", "excalidraw"));
                if let Some(mut p) = picked {
                    if p.extension().is_none() {
                        p.set_extension("excalidraw");
                    }
                    self.write_to(&p);
                }
                match then {
                    Some(next) if !self.dirty => self.run_pending(next),
                    // cancelado ou erro: volta à pergunta
                    Some(next) => self.confirm = Some(next),
                    None => {}
                }
            }
            DialogKind::Image => match r {
                Ok(Some(p)) => self.insert_image_file(&p, None),
                Ok(None) => {}
                Err(()) => self.toast(crate::ui::keys(&format!(
                    "{NO_DIALOG}. Dica: arraste a imagem para a janela ou cole com Ctrl+V."
                ))),
            },
            DialogKind::Export {
                svg,
                els,
                opts,
                base,
            } => {
                let ext = if svg { "svg" } else { "png" };
                let Some(path) = r.unwrap_or_else(|()| fallback_path(&base, ext)) else {
                    return;
                };
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
            DialogKind::ImportLibrary => match r {
                Ok(Some(p)) => match std::fs::read_to_string(&p) {
                    Ok(t) => self.import_library_text(&t),
                    Err(e) => self.toast(format!("Não foi possível abrir: {e}")),
                },
                Ok(None) => {}
                Err(()) => self.toast(format!(
                    "{NO_DIALOG}. Dica: arraste o arquivo .excalidrawlib para a janela."
                )),
            },
            DialogKind::ExportLibrary(items) => {
                let Some(mut path) =
                    r.unwrap_or_else(|()| fallback_path("biblioteca", library::EXT))
                else {
                    return;
                };
                if path.extension().is_none() {
                    path.set_extension(library::EXT);
                }
                match std::fs::write(&path, library::to_json(&items)) {
                    Ok(()) => self.toast(format!("Biblioteca exportada: {}", path.display())),
                    Err(e) => self.toast(format!("Erro ao exportar: {e}")),
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Biblioteca
    // ------------------------------------------------------------------

    fn save_library(&mut self) {
        let ids: HashSet<&str> = self.library.iter().map(|i| i.id.as_str()).collect();
        self.library_selected.retain(|s| ids.contains(s.as_str()));
        self.library_thumbs.retain(|k, _| ids.contains(k.as_str()));
        if let Some(p) = library::path()
            && let Err(e) = library::save(&p, &self.library)
        {
            self.toast(format!("Erro ao salvar a biblioteca: {e}"));
        }
    }

    /// "Adicionar à biblioteca": a seleção (com os grupos) vira um item.
    pub fn add_selection_to_library(&mut self) {
        let els = self.selection_with_children();
        if els.is_empty() {
            return;
        }
        library::merge(&mut self.library, vec![LibraryItem::new(els)]);
        self.save_library();
        self.show_library = true;
        self.toast("Adicionado à biblioteca");
    }

    /// Junta os itens de um `.excalidrawlib` à biblioteca.
    pub fn import_library_text(&mut self, text: &str) {
        match library::parse(text) {
            Ok(items) => {
                let total = items.len();
                let n = library::merge(&mut self.library, items);
                self.save_library();
                self.show_library = true;
                self.toast(match (n, total) {
                    (0, 0) => "A biblioteca importada está vazia".to_string(),
                    (0, _) => "Esses itens já estão na biblioteca".to_string(),
                    (1, _) => "1 item importado para a biblioteca".to_string(),
                    _ => format!("{n} itens importados para a biblioteca"),
                });
            }
            Err(e) => self.toast(format!("Não foi possível importar: {e}")),
        }
    }

    pub fn import_library_dialog(&mut self) {
        let fut = rfd::AsyncFileDialog::new()
            .set_title("Importar biblioteca")
            .add_filter("Biblioteca do Excalidraw", &[library::EXT, "json"])
            .pick_file();
        self.start_dialog(DialogKind::ImportLibrary, fut);
    }

    /// Exporta os itens marcados ou, sem marcação, a biblioteca inteira.
    pub fn export_library_dialog(&mut self) {
        let items: Vec<LibraryItem> = if self.library_selected.is_empty() {
            self.library.clone()
        } else {
            self.library
                .iter()
                .filter(|i| self.library_selected.contains(&i.id))
                .cloned()
                .collect()
        };
        self.export_library_items(items);
    }

    fn export_library_items(&mut self, items: Vec<LibraryItem>) {
        if items.is_empty() {
            self.toast("A biblioteca está vazia");
            return;
        }
        let name: String = match items.as_slice() {
            [one] => one
                .name
                .as_deref()
                .map(|n| {
                    n.chars()
                        .map(|c| {
                            if c.is_alphanumeric() || " -_".contains(c) {
                                c
                            } else {
                                '_'
                            }
                        })
                        .collect()
                })
                .unwrap_or_else(|| "biblioteca".into()),
            _ => "biblioteca".into(),
        };
        let fut = rfd::AsyncFileDialog::new()
            .set_title("Exportar biblioteca")
            .add_filter("Biblioteca do Excalidraw", &[library::EXT])
            .set_file_name(format!("{name}.{}", library::EXT))
            .save_file();
        self.start_dialog(DialogKind::ExportLibrary(items), fut);
    }

    /// Guarda o estado atual para o "Desfazer" do painel.
    fn library_snapshot(&mut self, what: impl Into<String>) {
        self.library_undo = Some((self.library.clone(), what.into()));
    }

    /// Remove os itens marcados ou, sem marcação, todos.
    pub fn remove_library_items(&mut self) {
        if self.library_selected.is_empty() {
            self.library_snapshot("Biblioteca limpa");
            self.library.clear();
        } else {
            let sel = std::mem::take(&mut self.library_selected);
            self.library_snapshot(if sel.len() == 1 {
                "1 item removido".to_string()
            } else {
                format!("{} itens removidos", sel.len())
            });
            self.library.retain(|i| !sel.contains(&i.id));
        }
        self.after_library_removal();
    }

    pub fn remove_library_item(&mut self, id: &str) {
        self.library_snapshot("Item removido");
        self.library.retain(|i| i.id != id);
        self.after_library_removal();
    }

    fn after_library_removal(&mut self) {
        let ids: HashSet<&str> = self.library.iter().map(|i| i.id.as_str()).collect();
        if self
            .library_edit
            .as_ref()
            .is_some_and(|e| !ids.contains(e.as_str()))
        {
            self.library_edit = None;
        }
        if self
            .library_rename
            .as_ref()
            .is_some_and(|(r, _)| !ids.contains(r.as_str()))
        {
            self.library_rename = None;
        }
        self.save_library();
    }

    /// Volta a biblioteca ao estado de antes da última remoção ou edição.
    pub fn undo_library(&mut self) {
        if let Some((items, _)) = self.library_undo.take() {
            self.library = items;
            self.library_thumbs.clear();
            self.save_library();
        }
    }

    pub fn start_rename_library_item(&mut self, id: &str) {
        let name = self
            .library
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.name.clone())
            .unwrap_or_default();
        self.library_rename = Some((id.to_string(), name));
    }

    pub fn finish_rename_library_item(&mut self, commit: bool) {
        if let Some((id, name)) = self.library_rename.take()
            && commit
            && library::rename(&mut self.library, &id, &name)
        {
            self.save_library();
        }
    }

    /// Move o item para a posição `to` (0 = início; além do fim = fim).
    pub fn move_library_item(&mut self, id: &str, to: usize) {
        if library::move_item(&mut self.library, id, to) {
            self.save_library();
        }
    }

    /// Arrastar dentro do painel: põe `id` antes de `before`.
    pub fn move_library_item_before(&mut self, id: &str, before: &str) {
        if library::move_before(&mut self.library, id, before) {
            self.save_library();
        }
    }

    /// "Editar no canvas": insere uma cópia do item e espera "Salvar no item".
    pub fn edit_library_item(&mut self, id: &str, at: P) {
        self.finish_rename_library_item(true);
        self.insert_library_item(id, at);
        self.library_edit = Some(id.to_string());
    }

    /// Grava a seleção atual como o novo conteúdo do item em edição.
    pub fn save_library_edit(&mut self) {
        let Some(id) = self.library_edit.clone() else {
            return;
        };
        let els: Vec<Element> = self.selected_elements().into_iter().cloned().collect();
        if els.is_empty() {
            self.toast("Selecione os elementos que devem ficar no item");
            return;
        }
        self.library_snapshot("Item atualizado");
        if library::replace_elements(&mut self.library, &id, els) {
            self.library_thumbs.remove(&id);
            self.save_library();
            self.toast("Item da biblioteca atualizado");
        }
        self.library_edit = None;
    }

    pub fn cancel_library_edit(&mut self) {
        self.library_edit = None;
    }

    /// Exporta só estes itens (menu de um item).
    pub fn export_library_item(&mut self, id: &str) {
        let items: Vec<LibraryItem> = self
            .library
            .iter()
            .filter(|i| i.id == id)
            .cloned()
            .collect();
        self.export_library_items(items);
    }

    /// Insere uma cópia do item centralizada em `at` (mundo) e a seleciona.
    pub fn insert_library_item(&mut self, id: &str, at: P) {
        let Some(els) = self
            .library
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.elements.clone())
        else {
            return;
        };
        let Some(b) = geom::scene_bounds(els.iter()) else {
            return;
        };
        self.finish_editing();
        self.finish_linear();
        let before = self.elements.clone();
        let mut copies = Self::clone_with_new_ids(&els, at - b.center());
        self.adopt_into_frame(&mut copies, at);
        self.selected = copies.iter().map(|e| e.id.clone()).collect();
        self.elements.extend(copies);
        self.tool = Tool::Select;
        self.commit(before);
    }

    /// Centro da área visível do canvas, em coordenadas do mundo.
    pub fn view_center(&self, view: egui::Rect) -> P {
        self.cam.w(view.center())
    }

    /// Executa a ação que esperava a confirmação de alterações não salvas.
    pub fn run_pending(&mut self, p: Pending) {
        match p {
            Pending::New => self.new_scene(),
            Pending::Open => self.open_dialog(),
            Pending::Clear => self.clear_canvas(),
            Pending::ClearLibrary => self.remove_library_items(),
            Pending::Reload => self.reload_from_disk(None),
        }
    }

    /// Consulta o arquivo aberto e recarrega se outro programa o alterou.
    ///
    /// Espera o fim de qualquer interação (arrasto, texto, diálogo) para não
    /// trocar a cena no meio dela.
    fn poll_watch(&mut self, ctx: &egui::Context) {
        if self.watch.is_none() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(WATCH_INTERVAL_MS));
        let busy = !matches!(self.drag, Drag::None)
            || self.editing.is_some()
            || self.confirm.is_some()
            || self.dialog.is_some();
        if busy || self.now - self.last_watch < WATCH_INTERVAL_MS as f64 / 1000.0 {
            return;
        }
        self.last_watch = self.now;
        let Some(text) = self.watch.as_mut().and_then(|w| w.poll()) else {
            return;
        };
        if self.dirty {
            // "Manter o meu" deixa como está; salvar depois sobrescreve
            self.confirm = Some(Pending::Reload);
        } else {
            self.reload_from_disk(Some(text));
        }
    }

    /// Liga ou desliga a edição do desenho aberto pelo `rustdraw mcp`.
    pub fn set_live(&mut self, on: bool) {
        if on == self.live.is_some() {
            return;
        }
        if on {
            let ctx = self.ctx.clone();
            match live::Host::start(move || ctx.request_repaint()) {
                Ok(h) => {
                    self.live = Some(h);
                    self.toast("Edição por IA ligada: o rustdraw mcp agora edita este desenho");
                }
                Err(e) => self.toast(format!("Não foi possível ligar a edição por IA: {e}")),
            }
        } else {
            self.live = None;
            for req in self.live_queue.drain(..) {
                req.respond(Err("a edição por IA foi desligada no editor".into()));
            }
            self.toast("Edição por IA desligada");
        }
        self.autosave_pending = true;
        self.last_change = self.now;
    }

    /// Atende os pedidos do `rustdraw mcp` no desenho aberto. Cada mudança entra
    /// no histórico, então Ctrl+Z desfaz o que a IA fez.
    fn poll_live(&mut self) {
        let Some(host) = &self.live else { return };
        self.live_queue.extend(host.rx.try_iter());
        if self.live_queue.is_empty() {
            return;
        }
        if !matches!(self.drag, Drag::None) || self.editing.is_some() {
            self.ctx
                .request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let source = match &self.file_path {
            Some(p) => p.display().to_string(),
            None => "desenho aberto no RustDraw (ainda sem arquivo)".into(),
        };
        let mut changed = false;
        for req in std::mem::take(&mut self.live_queue) {
            let mut scene = self.scene_file();
            match mcp::run_tool(&req.name, &req.args, &mut scene, &source) {
                Ok((r, c)) => {
                    if c {
                        self.apply_live(scene);
                        changed = true;
                    }
                    req.respond(Ok(r));
                }
                Err(e) => req.respond(Err(e)),
            }
        }
        if changed {
            self.toast(crate::ui::keys("Desenho alterado pela IA (Ctrl+Z desfaz)"));
        }
    }

    fn apply_live(&mut self, scene: SceneFile) {
        // imagens antigas ficam, para que desfazer as traga de volta
        self.files.extend(scene.files_map());
        if let Some(bg) = scene.view_background() {
            self.view_bg = bg;
        }
        let before = std::mem::replace(&mut self.elements, scene.elements);
        let ids: HashSet<String> = self.elements.iter().map(|e| e.id.clone()).collect();
        self.selected.retain(|s| ids.contains(s));
        self.bind_hint = None;
        self.commit(before);
    }

    /// Troca a cena pelo conteúdo do arquivo no disco (lido agora, se `text` for `None`).
    ///
    /// Diferente de abrir: mantém a câmera e a seleção, e entra no histórico,
    /// então Ctrl+Z volta ao desenho de antes da recarga.
    pub fn reload_from_disk(&mut self, text: Option<String>) {
        let Some(path) = self.file_path.clone() else {
            return;
        };
        let Some(text) = text.or_else(|| std::fs::read_to_string(&path).ok()) else {
            self.toast(format!("Não foi possível ler {}", file_name(&path)));
            return;
        };
        let f = match SceneFile::parse(&text) {
            Ok(f) => f,
            // pode ser uma gravação pela metade de um programa que não grava de uma vez
            Err(_) => {
                if let Some(w) = &mut self.watch {
                    w.retry();
                }
                return;
            }
        };
        self.watch = Some(Watch::new(&path, text.as_bytes()));
        self.view_bg = f.view_background().unwrap_or_else(|| "#ffffff".into());
        self.grid = f.grid_enabled();
        if f.elements != self.elements {
            let before = std::mem::replace(&mut self.elements, f.elements);
            self.undo.push(before);
            if self.undo.len() > 300 {
                self.undo.remove(0);
            }
            self.redo.clear();
            let ids: HashSet<String> = self.elements.iter().map(|e| e.id.clone()).collect();
            self.selected.retain(|s| ids.contains(s));
            self.bind_seen.clear();
            self.bind_hint = None;
            self.export_preview = None;
            self.toast(format!("Recarregado: {}", file_name(&path)));
        }
        self.dirty = false;
        self.autosave_pending = true;
        self.last_change = self.now;
    }

    pub fn new_scene(&mut self) {
        if self.dirty && !self.elements.is_empty() && self.confirm.is_none() {
            self.confirm = Some(Pending::New);
            return;
        }
        let bg = self.view_bg.clone();
        self.apply_scene(SceneFile::new(vec![], &bg, self.grid));
        self.file_path = None;
        self.watch = None;
        self.dirty = false;
        self.autosave_pending = true;
    }

    pub fn save(&mut self) {
        self.save_then(None);
    }

    /// Salva e depois executa `then` (se o salvamento der certo).
    pub fn save_then(&mut self, then: Option<Pending>) {
        match self.file_path.clone() {
            Some(p) => {
                self.write_to(&p);
                match then {
                    Some(next) if !self.dirty => self.run_pending(next),
                    Some(next) => self.confirm = Some(next),
                    None => {}
                }
            }
            None => self.save_as_then(then),
        }
    }

    pub fn save_as(&mut self) {
        self.save_as_then(None);
    }

    fn save_as_then(&mut self, then: Option<Pending>) {
        let name = self
            .file_path
            .as_deref()
            .map(file_name)
            .unwrap_or_else(|| "desenho.excalidraw".into());
        let fut = rfd::AsyncFileDialog::new()
            .set_title("Salvar desenho")
            .add_filter("Excalidraw", &["excalidraw"])
            .set_file_name(name)
            .save_file();
        self.start_dialog(DialogKind::Save(then), fut);
    }

    fn write_to(&mut self, p: &std::path::Path) {
        let json = self.scene_file().to_json();
        match std::fs::write(p, &json) {
            Ok(()) => {
                self.watch = Some(Watch::new(p, json.as_bytes()));
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
            obj.insert("objectsSnapModeEnabled".into(), self.snap.into());
            obj.insert("rustdrawMcpLive".into(), self.live.is_some().into());
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

    /// Opções de exportação com o fundo da cena e as imagens usadas por `els`.
    pub fn export_opts_for(&self, els: &[Element]) -> ExportOpts {
        let mut opts = self.export_opts.clone();
        opts.view_background = self.view_bg.clone();
        opts.files = files::subset(&self.files, els);
        opts.frame = None;
        opts
    }

    /// O frame selecionado, quando a seleção é um frame só (com ou sem os filhos).
    pub fn selected_frame(&self) -> Option<&Element> {
        let mut frames = self
            .selected_elements()
            .into_iter()
            .filter(|e| frame::is_frame(e));
        let f = frames.next()?;
        let only = frames.next().is_none()
            && self
                .selected_elements()
                .iter()
                .all(|e| frame::is_frame(e) || e.frame_id.as_deref() == Some(f.id.as_str()));
        only.then_some(f)
    }

    /// Elementos e opções da exportação: com "Somente a seleção" e um frame
    /// selecionado, exporta só o conteúdo dele, recortado pela área do frame.
    pub fn export_job(&self) -> (Vec<Element>, ExportOpts) {
        let frame = self.selected_frame().map(|f| f.id.clone());
        let els = if !self.export_selection_only || self.selected.is_empty() || frame.is_some() {
            self.elements.clone()
        } else {
            self.selection_with_children()
        };
        let mut opts = self.export_opts_for(&els);
        if self.export_selection_only && !self.selected.is_empty() {
            opts.frame = frame;
        }
        (els, opts)
    }

    /// Elementos colados ou inseridos em `at`: o que não veio com o próprio
    /// frame entra no frame onde foi solto (ou fica fora de frames).
    fn adopt_into_frame(&self, copies: &mut [Element], at: P) {
        let pasted: HashSet<String> = copies.iter().map(|e| e.id.clone()).collect();
        let target = frame::frame_at(&self.elements, at, &HashSet::new())
            .map(|i| self.elements[i].id.clone());
        for e in copies.iter_mut().filter(|e| !frame::is_frame(e)) {
            if !e.frame_id.as_ref().is_some_and(|f| pasted.contains(f)) {
                e.frame_id = target.clone();
            }
        }
    }

    /// Seleção mais os filhos dos frames selecionados, na ordem do desenho.
    fn selection_with_children(&self) -> Vec<Element> {
        let ids = frame::with_children(&self.elements, &self.selected);
        self.elements
            .iter()
            .filter(|e| ids.contains(&e.id))
            .cloned()
            .collect()
    }

    /// Como `export_job`, para copiar a seleção (ou tudo) como imagem.
    fn copy_job(&self) -> (Vec<Element>, ExportOpts) {
        let frame = self.selected_frame().map(|f| f.id.clone());
        let els = if self.selected.is_empty() || frame.is_some() {
            self.elements.clone()
        } else {
            self.selection_with_children()
        };
        let mut opts = self.export_opts_for(&els);
        opts.frame = frame;
        (els, opts)
    }

    /// "Exportar frame…" do menu de contexto: abre a exportação só com o frame.
    pub fn export_selected_frame(&mut self) {
        if self.selected_frame().is_some() {
            self.export_selection_only = true;
            self.show_export = true;
            self.export_preview = None;
        }
    }

    pub fn export_file(&mut self, svg: bool) {
        let (els, opts) = self.export_job();
        if export::export_bounds(&els, &opts).is_none() {
            self.toast("Nada para exportar");
            return;
        }
        let base = match &opts.frame {
            Some(id) => self
                .elements
                .iter()
                .find(|e| &e.id == id)
                .map(frame::name)
                .unwrap_or_default(),
            None => self
                .file_path
                .as_deref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "desenho".into()),
        };
        let (ext, label) = if svg { ("svg", "SVG") } else { ("png", "PNG") };
        let fut = rfd::AsyncFileDialog::new()
            .set_title(format!("Exportar {label}"))
            .add_filter(label, &[ext])
            .set_file_name(format!("{base}.{ext}"))
            .save_file();
        self.start_dialog(
            DialogKind::Export {
                svg,
                els,
                opts,
                base,
            },
            fut,
        );
    }

    pub fn copy_svg(&mut self, ctx: &egui::Context) {
        let (els, opts) = self.export_job();
        self.copy_svg_of(ctx, &els, &opts);
    }

    /// Copia a seleção (ou tudo) como SVG — "Copiar como SVG" do menu de contexto.
    pub fn copy_selection_svg(&mut self, ctx: &egui::Context) {
        let (els, opts) = self.copy_job();
        if !els.is_empty() {
            self.copy_svg_of(ctx, &els, &opts);
        }
    }

    fn copy_svg_of(&mut self, ctx: &egui::Context, els: &[Element], opts: &ExportOpts) {
        match export::render_svg(els, opts) {
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
        self.finish_rename();
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
        // apagar um frame apaga o que está dentro dele (como no Excalidraw)
        let gone = frame::with_children(&self.elements, &self.selected);
        self.elements.retain(|e| !gone.contains(&e.id));
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
        let ids: HashMap<&str, String> = els.iter().map(|e| (e.id.as_str(), new_id())).collect();
        els.iter()
            .map(|e| {
                let mut n = e.clone();
                n.id = ids[e.id.as_str()].clone();
                n.seed = rand_seed();
                n.x += offset.x as f64;
                n.y += offset.y as f64;
                n.group_ids = e
                    .group_ids
                    .iter()
                    .map(|g| groups.entry(g.clone()).or_insert_with(new_id).clone())
                    .collect();
                n.bound_elements = None;
                // filhos acompanham o frame copiado; senão, ficam no frame de antes
                if let Some(new) = e.frame_id.as_ref().and_then(|f| ids.get(f.as_str())) {
                    n.frame_id = Some(new.clone());
                }
                if n.extra.contains_key("containerId") {
                    n.extra
                        .insert("containerId".into(), serde_json::Value::Null);
                }
                // ligações só sobrevivem se a forma também foi copiada
                for end in [false, true] {
                    if let Some(mut b) = binding::get(e, end)
                        && let Some(new) = ids.get(b.element_id.as_str())
                    {
                        b.element_id = new.clone();
                        binding::set(&mut n, end, Some(&b));
                    } else if n
                        .extra
                        .contains_key(if end { "endBinding" } else { "startBinding" })
                    {
                        binding::set(&mut n, end, None);
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
        let src = self.selection_with_children();
        let copies = Self::clone_with_new_ids(&src, P::new(10.0, 10.0));
        self.selected = copies.iter().map(|e| e.id.clone()).collect();
        self.elements.extend(copies);
        self.commit(before);
    }

    pub fn copy_selection(&self, ctx: &egui::Context) {
        let sel = self.selection_with_children();
        if !sel.is_empty() {
            ctx.copy_text(clipboard_json(&sel, &self.files));
        }
    }

    pub fn paste(&mut self, text: &str) {
        // "Colar" do menu de contexto cola onde o menu foi aberto
        let at = match self.paste_at.take() {
            Some((p, t)) if self.now - t < 2.0 => p,
            _ => self.pointer_world,
        };
        let before = self.elements.clone();
        if let Ok(f) = SceneFile::parse(text) {
            if f.elements.is_empty() {
                return;
            }
            for (k, v) in f.files_map() {
                self.files.entry(k).or_insert(v);
            }
            let b = geom::scene_bounds(f.elements.iter()).unwrap();
            let off = at - b.center();
            let mut copies = Self::clone_with_new_ids(&f.elements, off);
            self.adopt_into_frame(&mut copies, at);
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
            el.frame_id = frame::frame_at(&self.elements, at, &HashSet::new())
                .map(|i| self.elements[i].id.clone());
            self.selected = [el.id.clone()].into();
            self.elements.push(el);
        } else {
            return;
        }
        self.tool = Tool::Select;
        self.commit(before);
    }

    /// Ferramenta de imagem (tecla 9): escolhe um arquivo e insere no centro da tela.
    pub fn image_dialog(&mut self) {
        self.finish_editing();
        self.finish_linear();
        let fut = rfd::AsyncFileDialog::new()
            .set_title("Inserir imagem")
            .add_filter("Imagens", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file();
        self.start_dialog(DialogKind::Image, fut);
    }

    pub fn insert_image_file(&mut self, p: &std::path::Path, at: Option<P>) {
        match std::fs::read(p) {
            Ok(bytes) => self.insert_image(&bytes, at),
            Err(e) => self.toast(format!("Não foi possível abrir {}: {e}", file_name(p))),
        }
    }

    /// Insere uma imagem (bytes PNG/JPG/GIF/WebP) com o centro em `at`
    /// (padrão: o centro da tela), como o Excalidraw faz ao colar ou arrastar.
    pub fn insert_image(&mut self, bytes: &[u8], at: Option<P>) {
        if self.view_mode {
            return;
        }
        let added = match files::add_bytes(&mut self.files, bytes) {
            Ok(a) => a,
            Err(e) => {
                self.toast(e);
                return;
            }
        };
        let view = if self.view.is_positive() {
            self.view
        } else {
            egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))
        };
        let at = at.unwrap_or_else(|| self.cam.w(view.center()));
        // como o Excalidraw: no máximo metade da altura da tela
        let max_side = ((view.height() * 0.5).max(160.0) / self.cam.zoom) as f64;
        let mut el = files::image_element(&added, at.x as f64, at.y as f64, max_side);
        el.opacity = self.style.opacity;
        el.frame_id = frame::frame_at(&self.elements, at, &HashSet::new())
            .map(|i| self.elements[i].id.clone());
        let before = self.elements.clone();
        self.bad_images.remove(&added.id);
        self.selected = [el.id.clone()].into();
        self.elements.push(el);
        self.tool = Tool::Select;
        self.commit(before);
    }

    /// Imagem colada da área de transferência (print de tela, imagem copiada).
    pub fn paste_image(&mut self, img: &egui::ColorImage) {
        let [w, h] = img.size;
        let raw: Vec<u8> = img
            .pixels
            .iter()
            .flat_map(|c| c.to_srgba_unmultiplied())
            .collect();
        let png =
            image::RgbaImage::from_raw(w as u32, h as u32, raw).and_then(|i| files::encode_png(&i));
        let at = match self.paste_at.take() {
            Some((p, t)) if self.now - t < 2.0 => p,
            _ => self.pointer_world,
        };
        match png {
            Some(png) => self.insert_image(&png, Some(at)),
            None => self.toast("Não foi possível colar a imagem"),
        }
    }

    /// Carrega as texturas das imagens visíveis que ainda não estão na GPU.
    fn load_textures(&mut self, ctx: &egui::Context) {
        let missing: Vec<String> = files::used_ids(&self.elements)
            .into_iter()
            .filter(|id| !self.textures.contains_key(*id) && !self.bad_images.contains(*id))
            .map(String::from)
            .collect();
        for id in missing {
            let Some(img) = files::decode(&self.files, &id) else {
                self.bad_images.insert(id);
                continue;
            };
            // limite de textura das GPUs mais modestas
            const MAX_TEX: u32 = 4096;
            let img = if img.width().max(img.height()) > MAX_TEX {
                image::imageops::resize(
                    &img,
                    (img.width() * MAX_TEX / img.width().max(img.height())).max(1),
                    (img.height() * MAX_TEX / img.width().max(img.height())).max(1),
                    image::imageops::FilterType::Triangle,
                )
            } else {
                img
            };
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            let tex = ctx.load_texture(format!("img-{id}"), color, egui::TextureOptions::LINEAR);
            self.textures.insert(id, tex);
        }
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

    /// "Destravar tudo" (menu de contexto): destrava e seleciona os travados.
    pub fn unlock_all(&mut self) {
        if !self.has_locked() {
            return;
        }
        let before = self.elements.clone();
        self.finish_editing();
        self.tool = Tool::Select;
        self.selected.clear();
        for e in self.elements.iter_mut().filter(|e| e.locked) {
            e.locked = false;
            e.touch();
            self.selected.insert(e.id.clone());
        }
        self.commit(before);
    }

    pub fn has_locked(&self) -> bool {
        self.elements.iter().any(|e| e.locked)
    }

    /// Todos os selecionados estão travados (o menu oferece "Destravar").
    pub fn selection_locked(&self) -> bool {
        let sel = self.selected_elements();
        !sel.is_empty() && sel.iter().all(|e| e.locked)
    }

    /// Agrupar só faz sentido se a seleção ainda não for um único grupo
    /// (mesma regra do Excalidraw).
    pub fn can_group(&self) -> bool {
        let sel = self.selected_elements();
        if sel.len() < 2 {
            return false;
        }
        !sel[0]
            .group_ids
            .iter()
            .any(|g| sel.iter().all(|e| e.group_ids.contains(g)))
    }

    pub fn can_ungroup(&self) -> bool {
        self.selected_elements()
            .iter()
            .any(|e| !e.group_ids.is_empty())
    }

    /// Texto dos elementos de texto selecionados, na ordem do desenho.
    pub fn selected_text(&self) -> String {
        self.elements
            .iter()
            .filter(|e| e.kind == "text" && self.selected.contains(&e.id))
            .map(|e| e.text())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// "Copiar texto": copia só o texto, como texto simples.
    pub fn copy_text(&mut self, ctx: &egui::Context) {
        let t = self.selected_text();
        if !t.is_empty() {
            ctx.copy_text(t);
            self.toast("Texto copiado para a área de transferência");
        }
    }

    pub fn cut_selection(&mut self, ctx: &egui::Context) {
        self.copy_selection(ctx);
        self.delete_selected();
    }

    /// "Colar" do menu: pede a área de transferência ao sistema; o texto chega
    /// como evento de colar no próximo quadro e vai para `at`.
    pub fn paste_at(&mut self, ctx: &egui::Context, at: P) {
        self.paste_at = Some((at, self.now));
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        ctx.request_repaint();
    }

    /// Abre o menu do botão direito. Sobre um elemento fora da seleção,
    /// seleciona-o (com o grupo) antes, como no Excalidraw.
    pub fn open_context_menu(&mut self, p: P, screen: Pos2) {
        self.finish_editing();
        self.finish_linear();
        self.show_menu = false;
        let mut on_element = false;
        if !self.view_mode {
            let in_sel_box = self.selected.len() > 1
                && self
                    .selection_bounds()
                    .is_some_and(|b| b.expand(4.0 / self.cam.zoom).contains(p));
            let label = self
                .frame_label_at(screen)
                .and_then(|id| self.elements.iter().position(|e| e.id == id));
            if in_sel_box {
                on_element = true;
            } else if let Some(i) = label.or_else(|| self.element_at(p)) {
                if !self.selected.contains(&self.elements[i].id) {
                    if self.tool != Tool::Select {
                        self.set_tool(Tool::Select);
                    }
                    self.selected = self.group_of(i);
                }
                on_element = true;
            }
        }
        self.context_menu = Some(ContextMenu {
            pos: screen,
            on_element,
            fresh: true,
        });
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
        let sel = frame::with_children(&self.elements, &self.selected);
        let mid = if horizontal {
            (b.min.x + b.max.x) as f64
        } else {
            (b.min.y + b.max.y) as f64
        };
        self.unbind_detached(&sel);
        for e in self.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
            flip(e, mid, horizontal);
            binding::mirror_fixed(e, horizontal);
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
        let sel = self.selected.clone();
        self.unbind_detached(&sel);
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
                    for k in ["elbowed", "fixedSegments", "startIsSpecial", "endIsSpecial"] {
                        e.extra.remove(k);
                    }
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
        let mut rerouted = Vec::new();
        for e in self
            .elements
            .iter_mut()
            .filter(|e| sel.contains(&e.id) && !frame::is_frame(e))
        {
            if e.kind == "image" {
                e.opacity = src.opacity;
                e.touch();
                continue;
            }
            e.stroke_color = src.stroke_color.clone();
            if e.kind != "text" && !(e.kind == "arrow" && src.kind != "arrow") {
                e.background_color = src.background_color.clone();
            }
            e.fill_style = src.fill_style.clone();
            e.stroke_width = src.stroke_width;
            e.stroke_style = src.stroke_style.clone();
            e.roughness = src.roughness;
            e.opacity = src.opacity;
            if matches!(e.kind.as_str(), "rectangle" | "diamond" | "line") {
                e.roundness = src.is_round().then(
                    || serde_json::json!({"type": if e.kind == "rectangle" { 3 } else { 2 }}),
                );
            }
            if e.kind == "arrow" && src.kind == "arrow" {
                if e.is_elbow() != src.is_elbow() {
                    elbow::set_elbowed(e, src.is_elbow());
                    rerouted.push(e.id.clone());
                }
                e.roundness = src.is_round().then(|| serde_json::json!({"type": 2}));
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
        for id in &rerouted {
            binding::refresh_arrow(&mut self.elements, id);
        }
        self.commit(before);
    }

    /// Shift+Alt+C: copia a seleção (ou tudo) como imagem PNG.
    pub fn copy_png(&mut self, ctx: &egui::Context) {
        let (els, opts) = self.copy_job();
        if els.is_empty() {
            return;
        }
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
        el.frame_id = frame::frame_at(&self.elements, at, &HashSet::new())
            .map(|i| self.elements[i].id.clone());
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

    /// Começa a editar o nome do frame `id`.
    pub fn rename_frame(&mut self, id: &str) {
        self.finish_editing();
        let Some(f) = self
            .elements
            .iter()
            .find(|e| e.id == id && frame::is_frame(e))
        else {
            return;
        };
        self.renaming = Some(FrameRename {
            id: id.to_string(),
            buf: frame::name(f),
            before: self.elements.clone(),
            focus: true,
        });
        self.selected = [id.to_string()].into();
        self.drag = Drag::None;
    }

    /// Grava o nome em edição (vazio volta a ser `null`, que aparece como "Frame").
    pub fn finish_rename(&mut self) {
        let Some(r) = self.renaming.take() else {
            return;
        };
        let name = r.buf.trim().to_string();
        if let Some(f) = self.get_mut(&r.id) {
            let v = if name.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::json!(name)
            };
            if f.extra.get("name") != Some(&v) {
                f.set_field("name", v);
                f.touch();
            }
        }
        self.commit(r.before);
    }

    /// Frame cujo nome (desenhado no último quadro) está em `screen`.
    fn frame_label_at(&self, screen: Pos2) -> Option<String> {
        self.frame_labels
            .iter()
            .rev()
            .find(|(_, r)| r.expand(2.0).contains(screen))
            .map(|(id, _)| id.clone())
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
            while pts.len() > 2 && !el.is_elbow() {
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
                let last = geom::abs_points(el).last().copied();
                if let Some(last) = last {
                    let mods = self.mods;
                    self.bind_end_at(&id, true, last, mods);
                }
            }
        }
        self.bind_hint = None;
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
        (0..self.elements.len()).rev().find(|&i| {
            let e = &self.elements[i];
            geom::hit_test(e, p, tol) && frame::visible_at(&self.elements, e, p)
        })
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

    /// Liga/desliga a grade; como no Excalidraw, desliga o encaixe em objetos.
    pub fn toggle_grid(&mut self) {
        self.grid = !self.grid;
        if self.grid {
            self.snap = false;
        }
    }

    /// Liga/desliga o encaixe em objetos (Alt+S); desliga a grade.
    pub fn toggle_snap(&mut self) {
        self.snap = !self.snap;
        if self.snap {
            self.grid = false;
        }
    }

    /// Se o encaixe em objetos vale agora: Ctrl/Cmd inverte a opção enquanto
    /// estiver pressionado (com a grade ligada, só desliga), como no Excalidraw.
    fn snapping(&self, mods: Modifiers) -> bool {
        if self.snap {
            !mods.command
        } else {
            mods.command && !self.grid
        }
    }

    /// Pontos de encaixe das formas visíveis, menos as de `skip`.
    fn snap_refs(&self, skip: &HashSet<String>) -> Vec<P> {
        let view = Rect::from_two(self.cam.w(self.view.min), self.cam.w(self.view.max));
        snap::reference_points(&self.elements, skip, view)
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
            let local = geom::abs_points(el);
            let pts: Vec<P> = local.iter().map(|p| rotate(*p, c, a)).collect();
            let n = pts.len();
            let long = |i: usize| (pts[i + 1] - pts[i]).len() * z >= MID_MIN_PX;
            let mut v: Vec<(Handle, P)> = Vec::new();
            if el.is_elbow() {
                // em cotovelo: só as pontas e o meio dos segmentos que se movem
                v.push((Handle::Point(0), pts[0]));
                if n > 1 {
                    v.push((Handle::Point(n - 1), pts[n - 1]));
                }
                for i in elbow::movable_segments(n).filter(|&i| long(i)) {
                    v.push((Handle::Mid(i), (pts[i] + pts[i + 1]) * 0.5));
                }
            } else {
                v.extend(pts.iter().enumerate().map(|(i, p)| (Handle::Point(i), *p)));
                let curved = el.is_round() && n > 2;
                for i in (0..n.saturating_sub(1)).filter(|&i| long(i)) {
                    let m = if curved {
                        crate::rough::curve_mid(&local, i)
                    } else {
                        (local[i] + local[i + 1]) * 0.5
                    };
                    v.push((Handle::Mid(i), rotate(m, c, a)));
                }
            }
            return v;
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
        if !frame::is_frame(el) {
            v.push((Handle::Rotate, P::new(mx, y0 - 22.0 / z)));
        }
        v.into_iter().map(|(h, p)| (h, rotate(p, c, a))).collect()
    }

    fn handle_at(&self, screen: Pos2) -> Option<Handle> {
        self.handles()
            .into_iter()
            .find(|(_, p)| (self.cam.s(*p) - screen).length() <= HANDLE_PX + 2.0)
            .map(|(h, _)| h)
    }

    /// Cursor sobre a alça do meio de um segmento.
    fn mid_cursor(&self, i: usize) -> CursorIcon {
        let Some(el) = self.single_selected().filter(|e| e.is_elbow()) else {
            return CursorIcon::PointingHand;
        };
        let pts = geom::abs_points(el);
        match (pts.get(i), pts.get(i + 1)) {
            (Some(a), Some(b)) if (a.y - b.y).abs() < (a.x - b.x).abs() => {
                CursorIcon::ResizeVertical
            }
            (Some(_), Some(_)) => CursorIcon::ResizeHorizontal,
            _ => CursorIcon::Default,
        }
    }

    /// Pontos selecionados, se ainda valem para a seleção atual.
    fn valid_point_sel(&self) -> Option<&BTreeSet<usize>> {
        let (id, set) = self.point_sel.as_ref()?;
        let el = self
            .single_selected()
            .filter(|e| &e.id == id && e.is_linear())?;
        let n = el.points().len();
        (!set.is_empty() && set.iter().all(|&i| i < n)).then_some(set)
    }

    /// Clique num ponto: seleciona (Shift alterna). Devolve os pontos a
    /// arrastar, ou `None` se o clique só tirou o ponto da seleção.
    fn press_point(&mut self, el: &Element, i: usize, shift: bool) -> Option<Vec<usize>> {
        if el.is_elbow() {
            self.point_sel = None;
            return Some(vec![i]);
        }
        let mut set = self.valid_point_sel().cloned().unwrap_or_default();
        if shift {
            if !set.remove(&i) {
                set.insert(i);
            }
        } else if !set.contains(&i) {
            set = [i].into();
        }
        let drag = set.contains(&i).then(|| set.iter().copied().collect());
        self.point_sel = (!set.is_empty()).then(|| (el.id.clone(), set));
        drag
    }

    /// Apaga os pontos selecionados da linha/seta. `false` se não havia nenhum.
    fn delete_points(&mut self) -> bool {
        let Some(set) = self.valid_point_sel().cloned() else {
            self.point_sel = None;
            return false;
        };
        self.point_sel = None;
        let Some(el) = self
            .single_selected()
            .filter(|e| !e.is_elbow() && !e.locked)
            .cloned()
        else {
            return false;
        };
        let pts = geom::abs_points(&el);
        let n = pts.len();
        let keep: Vec<[f64; 2]> = pts
            .iter()
            .enumerate()
            .filter(|(i, _)| !set.contains(i))
            .map(|(_, p)| [p.x as f64, p.y as f64])
            .collect();
        if keep.len() < 2 {
            self.delete_selected();
            return true;
        }
        let before = self.elements.clone();
        if let Some(e) = self.get_mut(&el.id) {
            for (end, i) in [(false, 0), (true, n - 1)] {
                if set.contains(&i) {
                    binding::set(e, end, None);
                }
            }
            e.set_abs_points(&keep);
            e.touch();
        }
        self.commit(before);
        true
    }

    /// Duplo clique com uma linha/seta selecionada: no meio de um segmento em
    /// cotovelo, solta o segmento fixo; numa linha comum, adiciona um ponto ali.
    fn double_click_linear(&mut self, p: P) -> bool {
        let Some(el) = self
            .single_selected()
            .filter(|e| e.is_linear() && !e.locked)
            .cloned()
        else {
            return false;
        };
        let before = match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::Segment { before, .. } | Drag::Points { before, .. } => before,
            Drag::Move { before, .. } => before,
            other => {
                self.drag = other;
                self.elements.clone()
            }
        };
        let handle = self.handle_at(self.cam.s(p));
        if el.is_elbow() {
            let Some(Handle::Mid(i)) = handle else {
                return true;
            };
            if let Some(e) = self.get_mut(&el.id)
                && elbow::release_segment(e, i)
            {
                e.touch();
                binding::refresh_arrow(&mut self.elements, &el.id);
            }
            self.commit(before);
            return true;
        }
        if handle.is_some() || !geom::hit_test(&el, p, 8.0 / self.cam.zoom) {
            self.commit(before);
            return handle.is_some();
        }
        // ponto mais perto no segmento mais perto (no referencial da linha)
        let q = rotate(p, geom::center(&el), -(el.angle as f32));
        let pts = geom::abs_points(&el);
        let Some((i, at)) = pts
            .windows(2)
            .enumerate()
            .map(|(i, w)| {
                let ab = w[1] - w[0];
                let t = ((q - w[0]).dot(ab) / ab.len_sq().max(1e-9)).clamp(0.0, 1.0);
                (i, w[0] + ab * t)
            })
            .min_by(|a, b| (a.1 - q).len().total_cmp(&(b.1 - q).len()))
        else {
            return false;
        };
        let mut new: Vec<[f64; 2]> = pts.iter().map(|p| [p.x as f64, p.y as f64]).collect();
        new.insert(i + 1, [at.x as f64, at.y as f64]);
        if let Some(e) = self.get_mut(&el.id) {
            e.set_abs_points(&new);
            e.touch();
        }
        self.point_sel = Some((el.id.clone(), [i + 1].into()));
        self.commit(before);
        true
    }

    /// Troca o tipo das setas selecionadas (e das próximas): sharp, round ou elbow.
    pub fn set_arrow_type(&mut self, t: &'static str) {
        self.style.arrow_type = t.into();
        let ids: Vec<String> = self
            .elements
            .iter()
            .filter(|e| e.kind == "arrow" && self.selected.contains(&e.id))
            .filter(|e| e.arrow_type() != t)
            .map(|e| e.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        let before = self.elements.clone();
        for id in &ids {
            if let Some(e) = self.get_mut(id) {
                if e.is_elbow() != (t == "elbow") {
                    elbow::set_elbowed(e, t == "elbow");
                }
                e.roundness = (t == "round").then(|| serde_json::json!({"type": 2}));
                e.touch();
            }
        }
        for id in &ids {
            binding::refresh_arrow(&mut self.elements, id);
        }
        self.point_sel = None;
        self.commit(before);
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
        let (
            pos,
            pressed,
            released,
            down,
            mid_pressed,
            mid_down,
            right_pressed,
            scroll,
            zoom_delta,
            mods,
            dbl,
        ) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                i.pointer.button_pressed(PointerButton::Middle),
                i.pointer.middle_down(),
                i.pointer.secondary_pressed(),
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.button_double_clicked(PointerButton::Primary),
            )
        });
        let space = ctx.input(|i| i.key_down(Key::Space)) && !ctx.egui_wants_keyboard_input();
        let over = resp.contains_pointer();
        if over && self.context_menu.is_some() && (zoom_delta != 1.0 || scroll != egui::Vec2::ZERO)
        {
            self.context_menu = None;
        }
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
        self.mods = mods;
        let Some(pos) = pos else { return };
        let wp = self.cam.w(pos);
        self.pointer_world = wp;
        // arrastando um item da biblioteca: o painel cuida do ponteiro
        if self.library_drag.is_some() {
            return;
        }

        // menu de contexto: um clique fora só fecha o menu
        if over && (pressed || mid_pressed) && self.context_menu.take().is_some() {
            return;
        }
        if over && right_pressed && matches!(self.drag, Drag::None | Drag::LinearMulti { .. }) {
            self.open_context_menu(wp, pos);
            return;
        }

        // pan com botão do meio
        if over && mid_pressed {
            self.drag = Drag::Pan { last: pos };
        }

        if over && pressed && self.renaming.is_some() {
            self.finish_rename();
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
            match self.frame_label_at(pos) {
                Some(id) => self.rename_frame(&id),
                None => self.on_double_click(wp),
            }
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
        // com a ferramenta de seta, destaca a forma onde a seta vai começar
        if matches!(self.drag, Drag::None) {
            self.bind_hint = (over && self.tool == Tool::Arrow && !mods.command && !space)
                .then(|| binding::candidate(&self.elements, wp, self.cam.zoom, ""))
                .flatten();
        }

        // cursor
        let cursor = match (&self.drag, self.tool) {
            (Drag::Pan { .. }, _) => CursorIcon::Grabbing,
            (_, Tool::Hand) => CursorIcon::Grab,
            _ if space || self.view_mode => CursorIcon::Grab,
            (Drag::Move { .. }, _) => CursorIcon::Move,
            (_, Tool::Select) if over && self.frame_label_at(pos).is_some() => CursorIcon::Move,
            (_, Tool::Select) if over => match self.handle_at(pos) {
                Some(Handle::N | Handle::S) => CursorIcon::ResizeVertical,
                Some(Handle::E | Handle::W) => CursorIcon::ResizeHorizontal,
                Some(Handle::NW | Handle::SE) => CursorIcon::ResizeNwSe,
                Some(Handle::NE | Handle::SW) => CursorIcon::ResizeNeSw,
                Some(Handle::Rotate) => CursorIcon::Alias,
                Some(Handle::Point(_)) => CursorIcon::PointingHand,
                Some(Handle::Mid(i)) => self.mid_cursor(i),
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
                    self.drag = match h {
                        Handle::Rotate => Drag::Rotate { orig, before },
                        Handle::Point(i) => {
                            let Some(indices) = self.press_point(&orig, i, mods.shift) else {
                                return;
                            };
                            Drag::Points {
                                orig,
                                indices,
                                start: p,
                                before,
                                inserted: false,
                            }
                        }
                        Handle::Mid(i) if orig.is_elbow() => Drag::Segment {
                            orig,
                            index: i,
                            before,
                        },
                        Handle::Mid(i) => {
                            let el = insert_point(&orig, i);
                            if let Some(e) = self.get_mut(&orig.id) {
                                *e = el.clone();
                            }
                            self.point_sel = Some((orig.id.clone(), [i + 1].into()));
                            Drag::Points {
                                orig: el,
                                indices: vec![i + 1],
                                start: p,
                                before,
                                inserted: true,
                            }
                        }
                        _ => Drag::Resize {
                            handle: h,
                            orig,
                            before,
                        },
                    };
                    return;
                }
                self.point_sel = None;
                let in_sel_box = self.selected.len() > 1
                    && self
                        .selection_bounds()
                        .is_some_and(|b| b.expand(4.0 / self.cam.zoom).contains(p));
                // o nome do frame seleciona o frame
                let label = self
                    .frame_label_at(screen)
                    .and_then(|id| self.elements.iter().position(|e| e.id == id));
                let hit = label.or_else(|| self.element_at(p));
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
                    let src = self.selection_with_children();
                    let copies = Self::clone_with_new_ids(&src, P::new(0.0, 0.0));
                    let copied: HashSet<String> = copies.iter().map(|e| e.id.clone()).collect();
                    self.elements.extend(copies);
                    // seleciona as cópias (sem os filhos de frames copiados)
                    self.selected = copied
                        .iter()
                        .filter(|id| {
                            self.elements
                                .iter()
                                .find(|e| &e.id == *id)
                                .and_then(|e| e.frame_id.as_ref())
                                .is_none_or(|f| !copied.contains(f))
                        })
                        .cloned()
                        .collect();
                }
                // os filhos andam com o frame
                let moving = frame::with_children(&self.elements, &self.selected);
                let originals = self
                    .elements
                    .iter()
                    .filter(|e| moving.contains(&e.id) && !e.locked)
                    .map(|e| (e.id.clone(), e.x, e.y))
                    .collect();
                let bounds = Rect::from_points(
                    &self
                        .selected_elements()
                        .into_iter()
                        .filter(|e| !e.locked)
                        .flat_map(|e| {
                            let b = geom::aabb(e);
                            [b.min, b.max]
                        })
                        .collect::<Vec<_>>(),
                );
                self.drag = Drag::Move {
                    start: p,
                    originals,
                    bounds,
                    before,
                    moved: false,
                };
            }
            Tool::Rect | Tool::Diamond | Tool::Ellipse | Tool::Arrow | Tool::Line | Tool::Frame => {
                if let Drag::LinearMulti { id, .. } = &self.drag {
                    // seta em cotovelo: o segundo clique termina
                    if self.elements.iter().any(|e| e.id == *id && e.is_elbow()) {
                        self.finish_linear();
                        return;
                    }
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
                if kind != "frame" {
                    el.frame_id = frame::frame_at(&self.elements, p, &HashSet::new())
                        .map(|i| self.elements[i].id.clone());
                }
                if el.is_linear() {
                    el.set_points(vec![[0.0, 0.0], [0.0, 0.0]]);
                }
                if kind == "arrow"
                    && !mods.command
                    && let Some(c) = binding::candidate(&self.elements, p, self.cam.zoom, "")
                {
                    binding::set(&mut el, false, Some(&Binding::new(&c.id, c.fixed)));
                }
                let id = el.id.clone();
                self.elements.push(el);
                self.selected = [id.clone()].into();
                self.drag = Drag::Create { id, origin, before };
            }
            Tool::Draw => {
                let mut el = Element::new("freedraw", p.x as f64, p.y as f64, &self.style);
                el.set_points(vec![[0.0, 0.0]]);
                el.frame_id = frame::frame_at(&self.elements, p, &HashSet::new())
                    .map(|i| self.elements[i].id.clone());
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
                let marked = frame::with_children(&self.elements, &marked);
                self.drag = Drag::Erase { last: p, marked };
            }
        }
    }

    fn on_double_click(&mut self, p: P) {
        if self.double_click_linear(p) {
            return;
        }
        match self.element_at(p) {
            Some(i) if self.elements[i].kind == "text" => {
                let id = self.elements[i].id.clone();
                self.drag = Drag::None;
                self.edit_text(&id);
            }
            Some(i) if frame::is_frame(&self.elements[i]) => {
                let id = self.elements[i].id.clone();
                self.rename_frame(&id);
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
        let snapping = self.snapping(mods);
        self.snap_guides.clear();
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
                    // a ligação pode ter movido o início; recomeça da origem
                    el.x = origin.x as f64;
                    el.y = origin.y as f64;
                    el.set_points(vec![
                        [0.0, 0.0],
                        [(q.x - origin.x) as f64, (q.y - origin.y) as f64],
                    ]);
                    el.touch();
                    self.bind_end_at(&id, true, q, mods);
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
                    let mut r = Rect::from_two(a, b);
                    if snapping {
                        let refs = self.snap_refs(&HashSet::from([id.clone()]));
                        let tol = snap::SNAP_PX / zoom;
                        // encaixa o canto arrastado (com Alt, os dois cantos se movem)
                        let corner = origin + d;
                        let mut off = snap::offset(&[corner], &refs, tol, (true, true));
                        if mods.shift {
                            // mantém o quadrado: encaixa o eixo mais próximo
                            let (sx, sy) = (d.x.signum(), d.y.signum());
                            let use_x =
                                off.x != 0.0 && (off.y == 0.0 || off.x.abs() <= off.y.abs());
                            let dm = if use_x { off.x * sx } else { off.y * sy };
                            off = P::new(dm * sx, dm * sy);
                        }
                        let d = d + off;
                        r = if mods.alt {
                            Rect::from_two(origin - d, origin + d)
                        } else {
                            Rect::from_two(origin, origin + d)
                        };
                        self.snap_guides = snap::guides(&snap::box_points(r), &refs);
                    }
                    let Some(el) = self.get_mut(&id) else { return };
                    el.x = r.min.x as f64;
                    el.y = r.min.y as f64;
                    el.width = r.width() as f64;
                    el.height = r.height() as f64;
                    el.touch();
                }
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
                    if el.is_elbow() {
                        // em cotovelo, refaz a rota do início até o ponteiro
                        pts = vec![pts[0], pts[n - 1]];
                    }
                    el.set_points(pts);
                    el.touch();
                    let arrow = el.kind == "arrow";
                    if el.is_elbow() {
                        binding::refresh_arrow(&mut self.elements, &id);
                    }
                    self.bind_hint = (arrow && !mods.command)
                        .then(|| binding::candidate(&self.elements, q, zoom, &id))
                        .flatten();
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
                bounds,
                moved,
                ..
            } => {
                let bounds = *bounds;
                let mut d = p - *start;
                let mut axes = (true, true);
                if mods.shift {
                    if d.x.abs() > d.y.abs() {
                        d.y = 0.0;
                        axes.1 = false;
                    } else {
                        d.x = 0.0;
                        axes.0 = false;
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
                let moving: HashSet<String> = originals.iter().map(|o| o.0.clone()).collect();
                if snapping && let Some(b) = bounds {
                    let refs = self.snap_refs(&moving);
                    let pts = |d: P| snap::box_points(Rect::from_two(b.min + d, b.max + d));
                    d = d + snap::offset(&pts(d), &refs, snap::SNAP_PX / zoom, axes);
                    self.snap_guides = snap::guides(&pts(d), &refs);
                }
                self.unbind_detached(&moving);
                for (id, ox, oy) in originals {
                    if let Some(el) = self.get_mut(&id) {
                        el.x = ox + d.x as f64;
                        el.y = oy + d.y as f64;
                        el.touch();
                    }
                }
                self.sync_bindings();
                self.frame_hint = self.drop_frame(&moving, p);
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
                // um frame selecionado leva os filhos junto; não precisam ficar na seleção
                let frames: HashSet<&String> = self
                    .elements
                    .iter()
                    .filter(|e| frame::is_frame(e) && sel.contains(&e.id))
                    .map(|e| &e.id)
                    .collect();
                let children: Vec<String> = self
                    .elements
                    .iter()
                    .filter(|e| e.frame_id.as_ref().is_some_and(|f| frames.contains(f)))
                    .map(|e| e.id.clone())
                    .collect();
                for c in children {
                    sel.remove(&c);
                }
                self.selected = sel;
            }
            Drag::Resize { handle, orig, .. } => {
                let (handle, orig) = (*handle, orig.clone());
                let mut p = p;
                let axes = (
                    matches!(
                        handle,
                        Handle::E | Handle::W | Handle::NE | Handle::NW | Handle::SE | Handle::SW
                    ),
                    matches!(
                        handle,
                        Handle::N | Handle::S | Handle::NE | Handle::NW | Handle::SE | Handle::SW
                    ),
                );
                let refs = (snapping && orig.angle == 0.0 && (axes.0 || axes.1))
                    .then(|| self.snap_refs(&HashSet::from([orig.id.clone()])));
                if let Some(refs) = &refs {
                    p = p + snap::offset(&[p], refs, snap::SNAP_PX / zoom, axes);
                }
                let updated = resize(&orig, handle, p, mods.shift, g);
                if let Some(refs) = &refs {
                    self.snap_guides = snap::guides(&snap::box_points(geom::aabb(&updated)), refs);
                }
                let n = orig.points().len();
                let end_point = match handle {
                    Handle::Point(i) if orig.kind == "arrow" && (i == 0 || i + 1 == n) => {
                        geom::abs_points(&updated).get(i).map(|q| (i != 0, *q))
                    }
                    _ => None,
                };
                if let Some(el) = self.get_mut(&orig.id) {
                    *el = updated;
                    el.touch();
                }
                if let Some((end, q)) = end_point {
                    self.bind_end_at(&orig.id, end, q, mods);
                }
                self.sync_bindings();
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
                self.sync_bindings();
            }
            Drag::Points {
                orig,
                indices,
                start,
                ..
            } => {
                let (orig, indices, start) = (orig.clone(), indices.clone(), *start);
                let c0 = geom::center(&orig);
                let a = orig.angle as f32;
                let local = |w: P| rotate(w, c0, -a);
                let mut pts = geom::abs_points(&orig);
                let n = pts.len();
                if let [i] = indices[..] {
                    let mut q = local(p);
                    if mods.shift && n > 1 {
                        // Shift: ângulo em passos de 15° a partir do vizinho
                        q = snap_angle(pts[if i == 0 { 1 } else { i - 1 }], q);
                    } else if g && a == 0.0 {
                        q = P::new(snap(q.x, true), snap(q.y, true));
                    }
                    pts[i] = q;
                } else {
                    let d = local(p) - local(start);
                    for &i in &indices {
                        pts[i] = pts[i] + d;
                    }
                }
                let mut el = orig.clone();
                el.set_abs_points(
                    &pts.iter()
                        .map(|p| [p.x as f64, p.y as f64])
                        .collect::<Vec<_>>(),
                );
                el.touch();
                let ends: Vec<(bool, P)> = if el.kind == "arrow" {
                    let (c, pts) = (geom::center(&el), geom::abs_points(&el));
                    indices
                        .iter()
                        .filter(|&&i| i == 0 || i + 1 == n)
                        .map(|&i| (i != 0, rotate(pts[i], c, a)))
                        .collect()
                } else {
                    vec![]
                };
                let id = el.id.clone();
                if let Some(e) = self.get_mut(&id) {
                    *e = el;
                }
                // as pontas da seta ligam (ou soltam) onde pararem
                for (end, q) in ends {
                    self.bind_end_at(&id, end, q, mods);
                }
                if self.elements.iter().any(|e| e.id == id && e.is_elbow()) {
                    binding::refresh_arrow(&mut self.elements, &id);
                }
                self.sync_bindings();
            }
            Drag::Segment { orig, index, .. } => {
                let mut el = orig.clone();
                let q = P::new(snap(p.x, g), snap(p.y, g));
                elbow::drag_segment(&mut el, *index, q);
                el.touch();
                let id = el.id.clone();
                if let Some(e) = self.get_mut(&id) {
                    *e = el;
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
                    let gset = frame::with_children(&self.elements, &self.group_of(i));
                    if let Drag::Erase { marked, .. } = &mut self.drag {
                        marked.extend(gset);
                    }
                }
            }
        }
    }

    fn on_release(&mut self, p: P) {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        self.bind_hint = None;
        self.snap_guides.clear();
        let tol = 3.0 / self.cam.zoom as f64;
        match drag {
            Drag::Create { id, before, origin } => {
                let Some(el) = self.elements.iter().find(|e| e.id == id) else {
                    return;
                };
                // linhas: mede o arrasto do ponteiro (a ligação pode ter movido as pontas)
                let tiny = if el.is_linear() {
                    ((p - origin).len() as f64) < tol
                } else {
                    el.width < tol && el.height < tol
                };
                if tiny && el.is_linear() {
                    // modo clique a clique (a ponta final liga ao terminar)
                    if let Some(el) = self.get_mut(&id) {
                        binding::set(el, true, None);
                        el.touch();
                    }
                    self.drag = Drag::LinearMulti { id, before };
                    return;
                }
                if tiny {
                    self.elements.retain(|e| e.id != id);
                    self.selected.clear();
                    return;
                }
                if frame::is_frame(el) {
                    frame::refresh_members(&mut self.elements, &id);
                }
                self.commit(before);
                if !self.tool_locked {
                    self.tool = Tool::Select;
                }
            }
            Drag::FreeDraw { before, .. } => self.commit(before),
            Drag::Move {
                before,
                moved,
                originals,
                ..
            } => {
                self.frame_hint = None;
                if moved {
                    let moving: HashSet<String> = originals.into_iter().map(|o| o.0).collect();
                    let target = self.drop_frame(&moving, p);
                    // o que foi arrastado sozinho (sem o próprio frame) entra no
                    // frame onde foi solto, ou sai do frame
                    let loose: HashSet<String> = self
                        .elements
                        .iter()
                        .filter(|e| {
                            moving.contains(&e.id)
                                && e.frame_id.as_ref().is_none_or(|f| !moving.contains(f))
                        })
                        .map(|e| e.id.clone())
                        .collect();
                    frame::assign(&mut self.elements, &loose, target.as_deref());
                    self.commit(before);
                }
            }
            Drag::Resize { before, orig, .. } => {
                if frame::is_frame(&orig) {
                    frame::refresh_members(&mut self.elements, &orig.id);
                }
                self.commit(before);
            }
            Drag::Points {
                before,
                inserted: true,
                start,
                ..
            } if (p - start).len() * self.cam.zoom < 3.0 => {
                self.elements = before;
                self.point_sel = None;
            }
            Drag::Rotate { before, .. }
            | Drag::Points { before, .. }
            | Drag::Segment { before, .. } => self.commit(before),
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

    /// Frame onde os elementos `moving` entram ao serem soltos em `p` (nenhum
    /// se entre eles houver um frame: frames não ficam dentro de frames).
    fn drop_frame(&self, moving: &HashSet<String>, p: P) -> Option<String> {
        let has_frame = self
            .elements
            .iter()
            .any(|e| frame::is_frame(e) && moving.contains(&e.id));
        if has_frame {
            return None;
        }
        frame::frame_at(&self.elements, p, moving).map(|i| self.elements[i].id.clone())
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
                egui::Event::Cut => self.cut_selection(ctx),
                egui::Event::Paste(t) => self.paste(t),
                egui::Event::PasteImage(img) => self.paste_image(img),
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
        if self.context_menu.is_some()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.context_menu = None;
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
            self.toggle_grid();
        }
        if kc(alt, Key::S) {
            self.toggle_snap();
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
        if (k(n, Key::Delete) || k(n, Key::Backspace)) && !self.delete_points() {
            self.delete_selected();
        }
        if k(n, Key::Escape) {
            if matches!(self.drag, Drag::LinearMulti { .. }) {
                self.finish_linear();
            } else if self.valid_point_sel().is_some() {
                self.point_sel = None;
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
            } else if let Some(el) = self.single_selected().filter(|e| frame::is_frame(e)) {
                let id = el.id.clone();
                self.rename_frame(&id);
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
                let sel = frame::with_children(&self.elements, &self.selected);
                self.unbind_detached(&sel);
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
            (Key::F, Tool::Frame),
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
        // 9: ferramenta de imagem (abre o seletor de arquivo, como no Excalidraw)
        if k(n, Key::Num9) {
            self.image_dialog();
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
        self.view = rect;
        if self.pending_fit {
            self.pending_fit = false;
            self.zoom_to_fit(rect);
        }
        (rect, resp)
    }

    pub fn paint_canvas(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let painter = ui.painter_at(rect);
        self.load_textures(ui.ctx());

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
        self.sync_bindings();

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
        let frame_rects = frame::rects(&self.elements);
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
            // frames são desenhados por cima, com espessura fixa na tela
            if Some(&el.id) == editing_id.as_ref() || frame::is_frame(el) {
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
            // filhos de frame ficam recortados pela área do frame
            let clipped;
            let painter = match el.frame_id.as_deref().and_then(|f| frame_rects.get(f)) {
                Some(r) => {
                    let sr = egui::Rect::from_two_pos(self.cam.s(r.min), self.cam.s(r.max));
                    clipped = painter.with_clip_rect(sr.intersect(rect));
                    &clipped
                }
                None => &painter,
            };
            let pc = PaintCtx {
                painter,
                cam: self.cam,
                dark: self.dark,
                alpha: if marked.contains(&el.id) { 0.25 } else { 1.0 },
                hand_ok: self.hand_ok,
                images: &self.textures,
            };
            render::paint_prims(&pc, &entry.prims);
        }
        if self.cache.len() > self.elements.len() + 64 {
            let keep: HashSet<String> = live.iter().map(|s| s.to_string()).collect();
            self.cache.retain(|k, _| keep.contains(k));
        }
        self.paint_frames(&painter);

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
            let point_sel = self.valid_point_sel().cloned().unwrap_or_default();
            for (h, p) in self.handles() {
                let s = self.cam.s(p);
                match h {
                    Handle::Point(i) if point_sel.contains(&i) => {
                        painter.circle(s, HANDLE_PX / 2.0 + 1.5, SELECTION, sel_stroke);
                    }
                    Handle::Rotate | Handle::Point(_) => {
                        painter.circle(s, HANDLE_PX / 2.0 + 1.0, handle_fill, sel_stroke);
                    }
                    Handle::Mid(_) => {
                        painter.circle(
                            s,
                            HANDLE_PX / 2.0 - 0.5,
                            handle_fill.gamma_multiply(0.7),
                            Stroke::new(1.0, SELECTION.gamma_multiply(0.6)),
                        );
                    }
                    _ => {
                        let r =
                            egui::Rect::from_center_size(s, egui::vec2(HANDLE_PX, HANDLE_PX) * 1.2);
                        painter.rect(r, 2.0, handle_fill, sel_stroke, StrokeKind::Middle);
                    }
                }
            }
        }
        self.paint_bind_hint(&painter);
        self.paint_snap_guides(&painter);
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

    /// Bordas e nomes dos frames (espessura e fonte fixas na tela, como no Excalidraw).
    fn paint_frames(&mut self, painter: &egui::Painter) {
        let mut labels = vec![];
        let renaming = self.renaming.as_ref().map(|r| r.id.as_str());
        for el in self.elements.iter().filter(|e| frame::is_frame(e)) {
            let r = frame::rect(el);
            let sr = egui::Rect::from_two_pos(self.cam.s(r.min), self.cam.s(r.max));
            let stroke_col = crate::model::parse_color(&el.stroke_color)
                .map(|c| render::c32(c, self.dark, el.opacity as f32 / 100.0))
                .unwrap_or(Color32::GRAY);
            let hinted = self.frame_hint.as_deref() == Some(el.id.as_str());
            let radius = (frame::RADIUS * self.cam.zoom).min(sr.width().min(sr.height()) / 2.0);
            painter.rect_stroke(
                sr,
                radius,
                if hinted {
                    Stroke::new(2.0, SELECTION)
                } else {
                    Stroke::new(1.0, stroke_col)
                },
                StrokeKind::Middle,
            );
            if renaming == Some(el.id.as_str()) {
                continue;
            }
            let selected = self.selected.contains(&el.id);
            let col = if selected {
                SELECTION
            } else {
                render::c32(frame::NAME_COLOR, self.dark, 1.0)
            };
            let galley = painter.layout_no_wrap(
                frame::name(el),
                egui::FontId::proportional(frame::NAME_SIZE),
                col,
            );
            let size = galley.size();
            let w = size.x.min(sr.width()).max(1.0);
            let top = sr.min.y - frame::NAME_GAP - size.y;
            let label = egui::Rect::from_min_size(egui::pos2(sr.min.x, top), egui::vec2(w, size.y));
            // nome mais largo que o frame é cortado
            painter
                .with_clip_rect(label.intersect(painter.clip_rect()))
                .galley(label.min, galley, col);
            labels.push((el.id.clone(), label));
        }
        self.frame_labels = labels;
    }

    /// Guias do encaixe em objetos: linhas com um "x" em cada ponto alinhado.
    fn paint_snap_guides(&self, painter: &egui::Painter) {
        let col = if self.dark {
            Color32::from_rgb(0xff, 0x87, 0x87)
        } else {
            Color32::from_rgb(0xff, 0x6b, 0x6b)
        };
        let stroke = Stroke::new(1.0, col);
        for g in &self.snap_guides {
            painter.line_segment([self.cam.s(g.from), self.cam.s(g.to)], stroke);
            for p in &g.points {
                let s = self.cam.s(*p);
                let r = 3.0;
                painter.line_segment([s + egui::vec2(-r, -r), s + egui::vec2(r, r)], stroke);
                painter.line_segment([s + egui::vec2(-r, r), s + egui::vec2(r, -r)], stroke);
            }
        }
    }

    /// Destaca a forma onde a seta vai ligar e os pontos de encaixe dela.
    fn paint_bind_hint(&self, painter: &egui::Painter) {
        let Some(hint) = &self.bind_hint else { return };
        let Some(el) = self.elements.iter().find(|e| e.id == hint.id) else {
            return;
        };
        let Some(outline) = geom::outline(el) else {
            return;
        };
        let c = geom::center(el);
        let a = el.angle as f32;
        let pts: Vec<Pos2> = outline
            .iter()
            .map(|p| self.cam.s(rotate(*p, c, a)))
            .collect();
        painter.add(egui::Shape::closed_line(
            pts,
            Stroke::new(4.0, SELECTION.gamma_multiply(0.35)),
        ));
        let fill = if self.dark {
            Color32::from_gray(30)
        } else {
            Color32::WHITE
        };
        for f in binding::ANCHORS {
            let s = self.cam.s(binding::anchor_world(el, f));
            if hint.fixed == Some(f) {
                painter.circle_filled(s, 5.0, SELECTION);
            } else {
                painter.circle(s, 3.5, fill, Stroke::new(1.5, SELECTION));
            }
        }
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        self.now = ctx.input(|i| i.time);
        self.poll_dialog();
        self.poll_watch(ctx);
        self.poll_live();
        let prefs = (self.dark, self.grid, self.snap, self.view_bg.clone());
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
        // (imagens entram no desenho onde foram soltas; o resto é aberto)
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let at = ctx.input(|i| i.pointer.hover_pos()).map(|p| self.cam.w(p));
        let mut offset = 0.0;
        for f in &dropped {
            let at = at.map(|p| p + P::new(offset, offset));
            let p = f.path().to_path_buf();
            if !files::is_image_path(&p) {
                self.load_path(&p);
                return;
            }
            match f.bytes() {
                Ok(bytes) => self.insert_image(&bytes, at),
                Err(e) => self.toast(format!("Não foi possível abrir {}: {e}", file_name(&p))),
            }
            offset += 20.0;
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
        elbow::mirror_segments(e, horizontal);
    } else if horizontal {
        e.x = mid - e.x - e.width;
    } else {
        e.y = mid - e.y - e.height;
    }
    if e.kind == "image" {
        let mut scale = e
            .extra
            .get("scale")
            .and_then(|v| serde_json::from_value::<[f64; 2]>(v.clone()).ok())
            .unwrap_or([1.0, 1.0]);
        scale[if horizontal { 0 } else { 1 }] *= -1.0;
        e.set_field("scale", serde_json::json!(scale));
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

/// Cópia de `orig` com um ponto novo no meio do segmento `i` (na curva, se curva).
fn insert_point(orig: &Element, i: usize) -> Element {
    let pts = geom::abs_points(orig);
    let mut el = orig.clone();
    if i + 1 >= pts.len() {
        return el;
    }
    let m = if orig.is_round() && pts.len() > 2 {
        crate::rough::curve_mid(&pts, i)
    } else {
        (pts[i] + pts[i + 1]) * 0.5
    };
    let mut rel = orig.points();
    rel.insert(i + 1, [m.x as f64 - orig.x, m.y as f64 - orig.y]);
    el.set_points(rel);
    el.touch();
    el
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
    // imagens mantêm a proporção por padrão; Shift libera (como no Excalidraw)
    let keep_ratio = if orig.kind == "image" {
        !keep_ratio
    } else {
        keep_ratio || text
    };
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
    fn insert_point_in_the_middle() {
        let mut e = Element::new("line", 10.0, 10.0, &Style::default());
        e.roundness = None;
        e.set_points(vec![[0.0, 0.0], [100.0, 0.0]]);
        let r = insert_point(&e, 0);
        assert_eq!(r.points(), vec![[0.0, 0.0], [50.0, 0.0], [100.0, 0.0]]);
        // na curva, o ponto novo fica sobre a curva
        let mut c = Element::new("line", 0.0, 0.0, &Style::default());
        c.set_points(vec![[0.0, 0.0], [100.0, 100.0], [200.0, 0.0]]);
        let r = insert_point(&c, 0);
        let m = r.points()[1];
        let curve = crate::rough::smooth_through(&geom::abs_points(&c));
        let d = geom::dist_to_polyline(P::new(m[0] as f32, m[1] as f32), &curve, false);
        assert!(d < 1.0, "{m:?} a {d} da curva");
    }

    #[test]
    fn arrow_type_of_new_arrows() {
        let mut style = Style::default();
        let a = Element::new("arrow", 0.0, 0.0, &style);
        assert_eq!(a.arrow_type(), "round");
        style.arrow_type = "sharp".into();
        assert_eq!(
            Element::new("arrow", 0.0, 0.0, &style).arrow_type(),
            "sharp"
        );
        style.arrow_type = "elbow".into();
        let e = Element::new("arrow", 0.0, 0.0, &style);
        assert_eq!(e.arrow_type(), "elbow");
        assert_eq!(e.roundness, None);
        assert!(e.extra.contains_key("fixedSegments"));
        // linhas seguem as bordas, não o tipo de seta
        assert!(Element::new("line", 0.0, 0.0, &style).is_round());
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
    fn copies_keep_bindings_only_inside_the_copy() {
        let a = rect(0.0);
        let b = rect(0.0);
        let mut arrow = Element::new("arrow", 0.0, 0.0, &Style::default());
        arrow.set_points(vec![[0.0, 0.0], [50.0, 0.0]]);
        binding::set(&mut arrow, false, Some(&Binding::new(&a.id, None)));
        binding::set(
            &mut arrow,
            true,
            Some(&Binding::new(&b.id, Some([0.0, 0.5]))),
        );
        let copies = App::clone_with_new_ids(&[a.clone(), arrow], P::new(10.0, 10.0));
        let start = binding::get(&copies[1], false).unwrap();
        assert_eq!(
            start.element_id, copies[0].id,
            "ligação remapeada para a cópia"
        );
        assert_eq!(
            binding::get(&copies[1], true),
            None,
            "forma não copiada: solta"
        );
    }

    #[test]
    fn copies_of_a_frame_take_their_children() {
        let s = Style::default();
        let f = Element::new("frame", 0.0, 0.0, &s);
        let mut inside = Element::new("rectangle", 10.0, 10.0, &s);
        inside.frame_id = Some(f.id.clone());
        let copies = App::clone_with_new_ids(&[inside.clone(), f.clone()], P::new(10.0, 10.0));
        assert_eq!(copies[0].frame_id.as_ref(), Some(&copies[1].id));
        // copiar só o filho mantém o frame de origem
        let alone = App::clone_with_new_ids(&[inside], P::new(10.0, 10.0));
        assert_eq!(alone[0].frame_id.as_ref(), Some(&f.id));
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
