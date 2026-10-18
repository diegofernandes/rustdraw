//! RustDraw — editor de diagramas no estilo Excalidraw, nativo e offline.
//!
//! Uso:
//!   rustdraw [arquivo.excalidraw]
//!   rustdraw export <entrada.excalidraw> <saida.png|saida.svg> [--scale N] [--dark] [--no-bg]
//!   rustdraw mcp [arquivo.excalidraw]

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod binding;
mod edit;
mod export;
mod files;
mod fonts;
mod geom;
mod mcp;
mod model;
mod render;
mod rough;
mod ui;

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
         rustdraw export <entrada.excalidraw> <saida.png|saida.svg> [--scale N] [--dark] [--no-bg]\n  \
         rustdraw mcp [arquivo.excalidraw]   (servidor MCP por stdio)",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2)
}

fn cli_export(args: &[String]) -> Result<(), String> {
    let mut pos = Vec::new();
    let mut opts = export::ExportOpts::default();
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
    opts.files = scene.files_map();
    if output.to_lowercase().ends_with(".svg") {
        std::fs::write(output, export::render_svg(&scene.elements, &opts)?)
            .map_err(|e| e.to_string())?;
    } else {
        std::fs::write(output, export::render_png(&scene.elements, &opts)?)
            .map_err(|e| e.to_string())?;
    }
    println!("{output} ({} elementos)", scene.elements.len());
    Ok(())
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "RustDraw",
        options,
        Box::new(move |cc| {
            Ok(Box::new(RustDraw {
                app: app::App::new(cc, open),
                dark_applied: None,
            }))
        }),
    )
}
