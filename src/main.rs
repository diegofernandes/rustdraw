//! RustDraw — editor de diagramas no estilo Excalidraw, nativo e offline.
//!
//! Uso:
//!   rustdraw [arquivo.excalidraw]
//!   rustdraw export <entrada.excalidraw> <saida.png|saida.svg|saida.tex> [--scale N] [--dark] [--no-bg] [--frame NOME]
//!   rustdraw mcp [arquivo.excalidraw]
//!   rustdraw icon <saida.png> [--size N] [--margin F]

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod binding;
mod edit;
mod elbow;
mod export;
mod files;
mod fonts;
mod frame;
mod geom;
mod icons;
mod label;
mod laser;
mod library;
mod live;
#[cfg(target_os = "macos")]
mod macos;
mod mcp;
mod model;
mod note;
mod render;
mod rough;
mod snap;
mod tikz;
mod ui;
mod watch;

use std::path::PathBuf;

struct RustDraw {
    app: app::App,
    dark_applied: Option<bool>,
}

impl eframe::App for RustDraw {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let app = &mut self.app;
        if self.dark_applied != Some(app.dark) {
            ctx.set_visuals(if app.dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            self.dark_applied = Some(app.dark);
        }
        app.tick(&ctx);
        let (rect, resp) = app.begin_canvas(ui);
        app.handle_keys(&ctx, rect);
        app.handle_pointer(&ctx, &resp);
        app.paint_canvas(ui, rect);
        ui::overlays(app, &ctx, rect);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.flush_autosave();
    }
}

fn usage() -> ! {
    eprintln!(
        "RustDraw {}\n\n\
         Uso:\n  rustdraw [arquivo.excalidraw]\n  \
         rustdraw export <entrada.excalidraw> <saida.png|saida.svg|saida.tex> [--scale N] [--dark] [--no-bg] [--frame NOME]\n  \
         rustdraw mcp [arquivo.excalidraw]   (servidor MCP por stdio)\n  \
         rustdraw icon <saida.png> [--size N] [--margin F]   (ícone do app)",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2)
}

fn cli_export(args: &[String]) -> Result<(), String> {
    let mut pos = Vec::new();
    let mut opts = export::ExportOpts::default();
    let mut frame = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--scale" => {
                opts.scale = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or("--scale precisa de um número")?
            }
            "--dark" => opts.dark = true,
            "--no-bg" => opts.background = false,
            "--frame" => frame = Some(it.next().ok_or("--frame precisa do nome do frame")?.clone()),
            _ => pos.push(a.clone()),
        }
    }
    let [input, output] = pos.as_slice() else {
        usage()
    };
    let text = std::fs::read_to_string(input).map_err(|e| format!("{input}: {e}"))?;
    let scene = model::SceneFile::parse(&text)?;
    if let Some(bg) = scene.view_background() {
        opts.view_background = bg;
    }
    if let Some(name) = frame {
        opts.frame = Some(
            frame::find(&scene.elements, &name).ok_or(format!("frame não encontrado: {name}"))?,
        );
    }
    opts.files = scene.files_map();
    let format = export::Format::from_path(output).unwrap_or(export::Format::Png);
    export::write_file(std::path::Path::new(output), format, &scene.elements, &opts)?;
    println!("{output} ({} elementos)", scene.elements.len());
    Ok(())
}

/// Grava o ícone do app em PNG; o pacote do macOS é montado com ele.
fn cli_icon(args: &[String]) -> Result<(), String> {
    let (mut size, mut margin, mut out) = (512u32, 0.0f32, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                size = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|s| (16..=2048).contains(s))
                    .ok_or("--size precisa de um número entre 16 e 2048")?
            }
            "--margin" => {
                margin = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|m| (0.0..0.4).contains(m))
                    .ok_or("--margin precisa de uma fração entre 0 e 0.4")?
            }
            _ if out.is_none() => out = Some(a.clone()),
            _ => usage(),
        }
    }
    let Some(out) = out else { usage() };
    std::fs::write(&out, icons::app_icon_png(size, margin)?).map_err(|e| format!("{out}: {e}"))
}

/// No Windows o executável é um app gráfico (abre sem janela de console), e o
/// que os subcomandos escrevem sumiria. Com isto a saída aparece no console de
/// onde ele foi chamado (cmd, PowerShell). Uma saída já redirecionada, como o
/// pipe do `rustdraw mcp`, fica como está.
#[cfg(windows)]
fn attach_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(std_handle: u32) -> *mut std::ffi::c_void;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    // SAFETY: funções do kernel32 sem ponteiros de entrada; falhar (sem
    // console no processo pai) só deixa tudo como estava
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(not(windows))]
fn attach_console() {}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|a| a.starts_with('-') || matches!(a.as_str(), "export" | "icon" | "mcp"))
    {
        attach_console();
    }
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => usage(),
        Some("-V" | "--version") => {
            println!("rustdraw {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("export") => {
            if let Err(e) = cli_export(&args[1..]) {
                eprintln!("erro: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some("icon") => {
            if let Err(e) = cli_icon(&args[1..]) {
                eprintln!("erro: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some("mcp") => {
            if let Err(e) = mcp::run(args.get(1).map(PathBuf::from)) {
                eprintln!("erro: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        _ => {}
    }
    let open = args.first().map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RustDraw")
            .with_app_id("rustdraw")
            .with_icon(icons::window_icon())
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    macos::install();
    eframe::run_native(
        "RustDraw",
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            macos::set_context(&cc.egui_ctx);
            Ok(Box::new(RustDraw {
                app: app::App::new(cc, open),
                dark_applied: None,
            }))
        }),
    )
}
