//! Ícones da interface e do app, com traço de mão livre.
//!
//! Os desenhos vêm do design dos ícones do RustDraw: caminhos SVG numa grade
//! de 24 px (ferramentas e ações, traço 1,75) e de 100 px (ícone do app, a
//! porca sextavada). Eles são pintados como vetores no egui, então ficam
//! nítidos em qualquer escala, e rasterizados com o tiny-skia para o ícone da
//! janela e do pacote do macOS.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

/// Espessura do traço na grade de 24 px.
const STROKE: f32 = 1.75;

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Hand,
    Select,
    Rect,
    Diamond,
    Ellipse,
    Arrow,
    Line,
    Draw,
    Text,
    Image,
    Eraser,
    Frame,
    Laser,
    Note,
    Lock(bool),
    Menu,
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    Open,
    Save,
    ExportPng,
    ExportSvg,
    Moon,
    Grid,
    SendToBack,
    SendBackward,
    BringForward,
    BringToFront,
    Duplicate,
    Delete,
    Library,
    Help,
    SaveAs,
    NewFile,
    ExportFrame,
    Cut,
    Copy,
    Paste,
    CopyStyles,
    PasteStyles,
    Snap,
    Zen,
    ViewMode,
    Stats,
    Group,
    Ungroup,
    FlipH,
    FlipV,
    SelectAll,
    AddToLibrary,
    Mcp,
}

impl Icon {
    /// Caminho do traço e caminho preenchido (só subcaminhos convexos).
    fn paths(self) -> (&'static str, &'static str) {
        match self {
            Icon::Lock(true) => (
                "M6 10.8 H18.2 M18 10.3 V20.1 M18.4 19.8 H5.7 M6.1 20.3 V10.4 \
                 M8.6 10.6 V7.6 C8.6 5.7 10.1 4.2 12 4.2 C13.9 4.2 15.4 5.7 15.4 7.6 V10.6 \
                 M12 14.4 V16.2",
                "",
            ),
            // a mesma trava com a alça solta, levantada
            Icon::Lock(false) => (
                "M6 10.8 H18.2 M18 10.3 V20.1 M18.4 19.8 H5.7 M6.1 20.3 V10.4 \
                 M8.6 10.6 V5.8 C8.6 3.9 10.1 2.4 12 2.4 C13.9 2.4 15.4 3.9 15.4 5.8 V7.2 \
                 M12 14.4 V16.2",
                "",
            ),
            Icon::Hand => (
                "M8 13.2 V6 C8 5.2 8.7 4.5 9.5 4.5 C10.3 4.5 11 5.2 11 6 V11.6 \
                 M11 11 V4.4 C11 3.6 11.7 2.9 12.5 2.9 C13.3 2.9 14 3.6 14 4.4 V11 \
                 M14 11 V5.6 C14 4.8 14.7 4.1 15.5 4.1 C16.3 4.1 17 4.8 17 5.6 V12 \
                 M17 9.6 C17 8.8 17.7 8.1 18.5 8.1 C19.3 8.1 20 8.8 20 9.6 V14.2 \
                 C20 18 16.9 21.1 13.1 21.1 H11.9 C9.9 21.1 8.1 20.1 7 18.5 L4.3 14.5 \
                 C3.8 13.8 4 12.8 4.7 12.4 C5.4 11.9 6.3 12.1 6.8 12.7 L8 14.2",
                "",
            ),
            Icon::Select => (
                "M5.1 3.7 L19 10.2 L12.6 12.5 L10 19.1 L4.8 4.5 M12.9 12.9 L17.8 17.8",
                "",
            ),
            Icon::Rect => (
                "M3.2 6.1 H20.7 M20 5.3 V18.5 M20.8 17.9 H3.5 M4.1 18.7 V5.5",
                "",
            ),
            Icon::Diamond => (
                "M11.6 2.9 L21.1 12.2 M21.2 11.5 L12.4 21 M12.9 20.9 L2.9 12.4 M2.8 12.9 L12.4 2.8",
                "",
            ),
            Icon::Ellipse => (
                "M12.6 4.2 C17.4 4 20.6 7.4 20.5 12 C20.4 16.6 16.8 19.8 12 19.8 \
                 C7.2 19.8 3.6 16.5 3.6 12 C3.6 7.3 7.4 4.1 12.2 4 C13.6 4 14.8 4.3 15.9 4.9",
                "",
            ),
            Icon::Arrow => ("M4.4 19.4 L19.4 4.6 M11.6 4.1 L19.5 4.4 L19.8 12.2", ""),
            Icon::Line => (
                "M5.3 18.6 L18.8 5.3",
                "M2.8 19.6 a1.6 1.6 0 1 0 3.2 0 a1.6 1.6 0 1 0 -3.2 0 \
                 M18.1 4.4 a1.6 1.6 0 1 0 3.2 0 a1.6 1.6 0 1 0 -3.2 0",
            ),
            Icon::Draw => (
                "M15.2 4.2 L19.8 8.8 L8.9 19.7 L3.9 20.1 L4.3 15.1 L15.4 4 M12.9 6.6 L17.4 11.1",
                "",
            ),
            Icon::Text => ("M5 6.6 V4.7 H19.3 V6.8 M12.1 4.9 V19.4 M9.3 19.3 H14.9", ""),
            Icon::Image => (
                "M3.4 5.1 H20.6 M20.1 4.5 V19.3 M20.7 18.9 H3.3 M3.9 19.5 V4.7 \
                 M4.2 16.7 L9 11.6 L14.1 16.5 M12.6 15 L15.3 12.4 L20 16.7",
                "M14.1 8.6 a1.5 1.5 0 1 0 3 0 a1.5 1.5 0 1 0 -3 0",
            ),
            Icon::Eraser => (
                "M8.7 19.6 L3.9 14.8 L13.8 4.9 L19.7 10.8 L10.9 19.6 M8.6 19.6 H20.4 \
                 M8.9 9.8 L14.8 15.7",
                "",
            ),
            // cerquilha com as bordas passando dos cantos, como no Excalidraw
            Icon::Frame => (
                "M7.8 3.3 V20.5 M16.3 3.6 V20.8 M3.3 7.7 H20.6 M3.6 16.4 H20.9",
                "",
            ),
            // caneta com raios saindo da ponta
            Icon::Laser => (
                "M9.4 12.6 L17.6 4.4 C18.2 3.8 19.2 3.8 19.8 4.4 C20.4 5 20.4 6 19.8 6.6 \
                 L11.6 14.8 L9.4 12.6 M3.4 16.5 H5.2 M4.3 12.5 L5.8 13.5 M7.5 20.6 V18.9 \
                 M4.5 19.7 L5.9 18.3",
                "M6.3 16.4 a1.3 1.3 0 1 0 2.6 0 a1.3 1.3 0 1 0 -2.6 0",
            ),
            // folha com a ponta dobrada (preenchida), uma linha à mão e uma reta
            Icon::Note => (
                "M3.6 4.1 H20.3 M19.9 3.7 V14.2 M20.2 13.9 L13.9 20.2 M14.3 19.9 H4 M4.1 20.3 V3.7 \
                 M7.4 9.4 C8.4 8.2 9.6 8.2 10.6 9.4 C11.6 10.6 12.8 10.6 13.8 9.4 C14.6 8.4 15.6 8.4 16.6 9.2 \
                 M7.6 13.4 H11.6",
                "M14.3 14.3 L19.6 14.3 L14.3 19.6 z",
            ),
            Icon::Menu => ("M4 6.6 H20.2 M3.8 12 H20 M4 17.4 H20.2", ""),
            Icon::Undo => (
                "M8.6 4.6 L4.2 9 L8.8 13.3 M4.6 9 H14.6 C17.6 9 20 11.4 20 14.4 \
                 C20 17.4 17.6 19.6 14.6 19.6 H11",
                "",
            ),
            Icon::Redo => (
                "M15.4 4.6 L19.8 9 L15.2 13.3 M19.4 9 H9.4 C6.4 9 4 11.4 4 14.4 \
                 C4 17.4 6.4 19.6 9.4 19.6 H13",
                "",
            ),
            Icon::ZoomIn => (
                "M10.6 3.8 C14.4 3.8 17.4 6.8 17.4 10.6 C17.4 14.4 14.4 17.4 10.6 17.4 \
                 C6.8 17.4 3.8 14.4 3.8 10.6 C3.8 6.8 6.8 3.8 11.4 3.9 M15.6 15.6 L20.4 20.4 \
                 M7.8 10.6 H13.4 M10.6 7.8 V13.4",
                "",
            ),
            Icon::ZoomOut => (
                "M10.6 3.8 C14.4 3.8 17.4 6.8 17.4 10.6 C17.4 14.4 14.4 17.4 10.6 17.4 \
                 C6.8 17.4 3.8 14.4 3.8 10.6 C3.8 6.8 6.8 3.8 11.4 3.9 M15.6 15.6 L20.4 20.4 \
                 M7.8 10.6 H13.4",
                "",
            ),
            Icon::Open => (
                "M3.4 6 C3.4 5.2 4 4.6 4.8 4.6 H9.4 L11.4 6.9 H19.2 C20 6.9 20.6 7.5 20.6 8.3 \
                 V18 C20.6 18.8 20 19.4 19.2 19.4 H4.8 C4 19.4 3.4 18.8 3.4 18 V5.4 \
                 M3.6 10.4 H20.4",
                "",
            ),
            Icon::Save => (
                "M3.9 4.4 H16.3 L20.1 8.2 V19.7 H4 V3.9 M7.6 4.6 V9 H15 V4.8 M7.4 19.4 V14 \
                 H16.6 V19.4",
                "",
            ),
            Icon::ExportPng => (
                "M10.6 5.4 H3.8 V20.2 H18.6 V13.4 M3.9 17.6 L7.8 13.8 L11.6 17.4 M14 4 H20 V10 \
                 M19.8 4.2 L12.6 11.4",
                "M7 9.6 a1.2 1.2 0 1 0 2.4 0 a1.2 1.2 0 1 0 -2.4 0",
            ),
            Icon::ExportSvg => (
                "M4.6 18.6 C6.2 11 11.6 6.6 19.2 5.6 M4.6 18.6 L10.8 7.8",
                "M3 17 h3.2 v3.2 h-3.2 z M17.6 4 h3.2 v3.2 h-3.2 z \
                 M9.5 7.8 a1.3 1.3 0 1 0 2.6 0 a1.3 1.3 0 1 0 -2.6 0",
            ),
            Icon::Moon => (
                "M19.6 14.6 C18.4 15.2 17 15.6 15.6 15.6 C11.4 15.6 8.2 12.4 8.2 8.2 \
                 C8.2 6.6 8.6 5.2 9.4 4 C6 5.2 3.6 8.4 3.6 12.2 C3.6 16.8 7.4 20.6 12 20.6 \
                 C15.4 20.6 18.3 18.4 19.6 14.6",
                "",
            ),
            Icon::Grid => (
                "M3.6 4 H20.4 M20 3.6 V20.4 M20.4 20 H3.6 M4 20.4 V3.6 M9.4 4.2 V19.8 \
                 M14.6 4.2 V19.8 M4.2 9.4 H19.8 M4.2 14.6 H19.8",
                "",
            ),
            // seta até a barra: até o fundo / até o topo
            Icon::SendToBack => (
                "M12.1 3.8 V15.4 M7.7 11.5 L12 15.9 L16.3 11.4 M4.1 20 H19.9",
                "",
            ),
            Icon::BringToFront => (
                "M11.9 20.2 V8.6 M7.7 12.5 L12 8.1 L16.3 12.6 M4.1 4 H19.9",
                "",
            ),
            // o "L" de trás em dois retângulos: o preenchimento só aceita
            // subcaminhos convexos
            Icon::SendBackward => (
                "M8.4 8.4 H19.8 V19.8 H8.4 Z",
                "M3.6 3.6 h12.6 v4 h-12.6 z M3.6 7.6 h4 v8.6 h-4 z",
            ),
            Icon::BringForward => ("M4.2 15.6 V4.2 H15.6 V8.2", "M8.4 8.4 h11.4 v11.4 h-11.4 z"),
            Icon::Duplicate => (
                "M8.2 8.4 H19.9 M19.6 8 V19.8 M20 19.6 H8.4 M8.6 20 V8.2 M15.6 7.8 V4.2 H4.2 \
                 V15.6 H7.8",
                "",
            ),
            Icon::Delete => (
                "M3.8 6.6 H20.3 M9.4 6.4 V4.2 H14.6 V6.4 M6 6.8 L7.1 19.8 H16.9 L18 6.8 \
                 M10.2 10.4 V16.4 M13.8 10.4 V16.4",
                "",
            ),
            Icon::Library => (
                "M5 4 V20 M9 4 V20 M4.4 4.3 H9.6 M4.4 19.7 H9.6 M12.4 5.5 L16.4 4.4 L20.6 19.1 \
                 L16.6 20.2 L12.3 5.2",
                "",
            ),
            Icon::Help => (
                "M12.6 3.6 C17.2 3.8 20.6 7.4 20.4 12.2 C20.2 16.8 16.6 20.4 12 20.4 \
                 C7.4 20.4 3.6 16.8 3.6 12 C3.6 7.4 7.2 3.6 12 3.6 C13.2 3.6 14.4 3.8 15.4 4.3 \
                 M9.6 9.6 C9.6 8.2 10.7 7.2 12 7.2 C13.3 7.2 14.4 8.2 14.4 9.4 \
                 C14.4 11.4 12 11.6 12 13.6",
                "M10.9 16.6 a1.1 1.1 0 1 0 2.2 0 a1.1 1.1 0 1 0 -2.2 0",
            ),
            // disquete com um lápis no canto
            Icon::SaveAs => (
                "M3.9 4.4 H14.3 L17.6 7.7 V10.6 M4 4 V18.6 H10.4 M7.2 4.6 V8.2 H13.2 V4.8 \
                 M18.6 12.3 L20.9 14.6 L14.6 20.9 L12 21.2 L12.3 18.6 L18.8 12.1",
                "",
            ),
            Icon::NewFile => (
                "M13.6 3.6 H5.6 V20.4 H18.4 V8.4 L13.4 3.4 M13.6 3.8 V8.4 H18.2 M12 11.6 V17.4 \
                 M9.1 14.5 H14.9",
                "",
            ),
            // a cerquilha do frame com a seta de exportar
            Icon::ExportFrame => (
                "M6.4 8.6 V20.8 M13.2 8.8 V20.6 M3.2 11.6 H16.4 M3.4 17.8 H16.2 \
                 M14.6 3.8 H20.4 V9.6 M20.2 4 L15.2 9",
                "",
            ),
            Icon::Cut => (
                "M4.2 17.4 a2.8 2.8 0 1 0 5.6 0 a2.8 2.8 0 1 0 -5.6 0 \
                 M14.2 17.4 a2.8 2.8 0 1 0 5.6 0 a2.8 2.8 0 1 0 -5.6 0 \
                 M8.8 15.2 L17.4 3.6 M15.2 15.2 L6.6 3.6",
                "",
            ),
            Icon::Copy => (
                "M9 8.2 H18.6 C19.4 8.2 20 8.8 20 9.6 V19.2 C20 20 19.4 20.6 18.6 20.6 H9 \
                 C8.2 20.6 7.6 20 7.6 19.2 V9.6 C7.6 8.8 8.2 8.2 9 8.2 M16.2 5.2 V4.8 \
                 C16.2 4 15.6 3.4 14.8 3.4 H5.2 C4.4 3.4 3.8 4 3.8 4.8 V14.4 \
                 C3.8 15.2 4.4 15.8 5.2 15.8 H5.4",
                "",
            ),
            Icon::Paste => (
                "M8.6 5 H6.2 C5.4 5 4.8 5.6 4.8 6.4 V19.6 C4.8 20.4 5.4 21 6.2 21 H17.8 \
                 C18.6 21 19.2 20.4 19.2 19.6 V6.4 C19.2 5.6 18.6 5 17.8 5 H15.4 \
                 M9 3.2 H15 V6.8 H9 Z M8.6 11.4 H15.4 M8.6 15.4 H13",
                "",
            ),
            // conta-gotas pega o estilo; o rolo de pintura aplica
            Icon::CopyStyles => (
                "M14.6 9.4 L5.6 18.4 L5.2 20.8 L7.6 20.4 L16.6 11.4 M13.2 8 L18 12.8 \
                 M15.4 10.2 L18.6 7 C19.6 6 19.6 4.4 18.6 3.4 C17.6 2.4 16 2.4 15 3.4 L11.8 6.6",
                "",
            ),
            Icon::PasteStyles => (
                "M5 4.4 H17.4 V9.2 H5 Z M17.4 6.8 H20 V12.2 H12 V15 M10.8 15 H13.2 V20.6 H10.8 Z",
                "",
            ),
            // ímã
            Icon::Snap => (
                "M5.4 4.4 V11.6 C5.4 15.4 8.4 18.4 12.2 18.4 C16 18.4 19 15.4 19 11.6 V4.4 \
                 M5.2 4.2 H9.6 V11.4 C9.6 12.8 10.8 14 12.2 14 C13.6 14 14.8 12.8 14.8 11.4 \
                 V4.2 H19.2 M5.4 7.8 H9.6 M14.8 7.8 H19 M10.4 21 H14",
                "",
            ),
            // círculo aberto (ensō) com um ponto no meio
            Icon::Zen => (
                "M15.8 4.6 C19.4 6.2 21 10.2 19.8 14 C18.6 17.8 14.8 20.2 10.8 19.6 \
                 C6.8 19 3.8 15.6 3.8 11.6 C3.8 7.8 6.6 4.6 10.2 4",
                "M10.6 12 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0",
            ),
            Icon::ViewMode => (
                "M2.8 12.2 C5 7.8 8.4 5.6 12 5.6 C15.6 5.6 19 7.8 21.2 12 \
                 C19 16.2 15.6 18.4 12 18.4 C8.4 18.4 5 16.2 2.9 11.6 \
                 M9.2 12 a2.8 2.8 0 1 0 5.6 0 a2.8 2.8 0 1 0 -5.6 0",
                "",
            ),
            Icon::Stats => (
                "M5.4 19.6 V13 H8.6 V19.6 M10.4 19.6 V5.6 H13.6 V19.6 M15.4 19.6 V9.6 H18.6 V19.6 \
                 M3.4 19.8 H20.8",
                "",
            ),
            // caixa de seleção com as alças nos cantos
            Icon::Group => (
                "M6.4 4 H17.6 M20 6.4 V17.6 M17.6 20 H6.4 M4 17.6 V6.4 \
                 M7.6 7.6 H12.4 V12.4 H7.6 Z M11.6 14.2 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0",
                "M2.6 2.6 h2.8 v2.8 h-2.8 z M18.6 2.6 h2.8 v2.8 h-2.8 z \
                 M18.6 18.6 h2.8 v2.8 h-2.8 z M2.6 18.6 h2.8 v2.8 h-2.8 z",
            ),
            Icon::Ungroup => (
                "M4 9.2 V4 H9.2 M20 14.8 V20 H14.8 M6.6 6.6 H11.8 V11.8 H6.6 Z \
                 M12.4 15 a2.6 2.6 0 1 0 5.2 0 a2.6 2.6 0 1 0 -5.2 0",
                "",
            ),
            // triângulo e o reflexo dele, preenchido, do outro lado do eixo
            Icon::FlipH => (
                "M12 3.4 V5.4 M12 8 V10 M12 12.6 V14.6 M12 17.2 V19.2 M9.4 6.4 L3.4 17.6 H9.4 Z",
                "M14.6 6.4 L20.6 17.6 L14.6 17.6 z",
            ),
            Icon::FlipV => (
                "M3.4 12 H5.4 M8 12 H10 M12.6 12 H14.6 M17.2 12 H19.2 M6.4 9.4 L17.6 3.4 V9.4 Z",
                "M6.4 14.6 L17.6 20.6 L17.6 14.6 z",
            ),
            Icon::SelectAll => (
                "M4 6.6 V4 H6.6 M17.4 4 H20 V6.6 M20 17.4 V20 H17.4 M6.6 20 H4 V17.4 \
                 M10.2 4 H13.8 M20 10.2 V13.8 M13.8 20 H10.2 M4 13.8 V10.2 \
                 M9.4 8.8 L16 11.9 L13 13 L11.8 16.1 Z",
                "",
            ),
            // a biblioteca com um "+" no lugar do livro inclinado
            Icon::AddToLibrary => (
                "M4.6 4 V20 M8.6 4 V20 M4 4.3 H9.2 M4 19.7 H9.2 M12.2 5.4 L15.6 4.5 L16.9 9 \
                 M17.4 12.6 V20.6 M13.4 16.6 H21.4",
                "",
            ),
            // tomada: o assistente se liga ao editor
            Icon::Mcp => (
                "M9 3.4 V7.4 M15 3.4 V7.4 M6.4 7.4 H17.6 V11 C17.6 14.1 15.1 16.6 12 16.6 \
                 C8.9 16.6 6.4 14.1 6.4 11 Z M12 16.6 V20.8",
                "",
            ),
        }
    }
}

/// Pinta o ícone ocupando o quadrado `r` (a grade de 24 px esticada nele).
pub fn paint(p: &Painter, r: Rect, icon: Icon, col: Color32) {
    let (stroke, fill) = icon.paths();
    let k = r.width().min(r.height()) / 24.0;
    let origin = r.center() - egui::vec2(12.0, 12.0) * k;
    let map = |q: [f32; 2]| origin + egui::vec2(q[0], q[1]) * k;
    let w = STROKE * k;
    for sub in parse(stroke) {
        let pts: Vec<Pos2> = sub.flatten().into_iter().map(map).collect();
        if sub.closed {
            p.add(Shape::closed_line(pts, Stroke::new(w, col)));
        } else {
            p.add(Shape::line(pts, Stroke::new(w, col)));
        }
    }
    for sub in parse(fill) {
        let pts: Vec<Pos2> = sub.flatten().into_iter().map(map).collect();
        p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
    }
}

// --- Caminhos SVG ---------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Seg {
    Line([f32; 2]),
    Cubic([f32; 2], [f32; 2], [f32; 2]),
}

#[derive(Debug)]
struct Sub {
    start: [f32; 2],
    segs: Vec<Seg>,
    closed: bool,
}

impl Sub {
    fn flatten(&self) -> Vec<[f32; 2]> {
        let mut out = vec![self.start];
        let mut at = self.start;
        for s in &self.segs {
            match *s {
                Seg::Line(b) => {
                    out.push(b);
                    at = b;
                }
                Seg::Cubic(c1, c2, b) => {
                    const N: usize = 10;
                    for i in 1..=N {
                        let t = i as f32 / N as f32;
                        let u = 1.0 - t;
                        let f = |j: usize| {
                            u * u * u * at[j]
                                + 3.0 * u * u * t * c1[j]
                                + 3.0 * u * t * t * c2[j]
                                + t * t * t * b[j]
                        };
                        out.push([f(0), f(1)]);
                    }
                    at = b;
                }
            }
        }
        if self.closed && out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        out
    }
}

/// Lê um atributo `d` de SVG: M L H V C A Z, maiúsculos ou minúsculos.
/// Os arcos viram cúbicas.
fn parse(d: &str) -> Vec<Sub> {
    let mut toks = Tokens {
        s: d.as_bytes(),
        i: 0,
    };
    let mut subs: Vec<Sub> = vec![];
    let mut cur = [0.0f32; 2];
    let mut cmd = b'M';
    while let Some(c) = toks.command() {
        if c.is_none() && cmd.eq_ignore_ascii_case(&b'Z') {
            break;
        }
        cmd = c.unwrap_or(match cmd {
            b'M' => b'L',
            b'm' => b'l',
            other => other,
        });
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { cur } else { [0.0, 0.0] };
        let pt = |toks: &mut Tokens| -> Option<[f32; 2]> {
            Some([base[0] + toks.num()?, base[1] + toks.num()?])
        };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let Some(p) = pt(&mut toks) else { break };
                subs.push(Sub {
                    start: p,
                    segs: vec![],
                    closed: false,
                });
                cur = p;
            }
            b'Z' => {
                if let Some(s) = subs.last_mut() {
                    s.closed = true;
                    cur = s.start;
                }
            }
            up => {
                let seg = match up {
                    b'L' => pt(&mut toks).map(Seg::Line),
                    b'H' => toks.num().map(|x| Seg::Line([base[0] + x, cur[1]])),
                    b'V' => toks.num().map(|y| Seg::Line([cur[0], base[1] + y])),
                    b'C' => (|| Some(Seg::Cubic(pt(&mut toks)?, pt(&mut toks)?, pt(&mut toks)?)))(),
                    b'A' => {
                        let Some(a) = (|| {
                            Some((
                                toks.num()?,
                                toks.num()?,
                                toks.num()?,
                                toks.num()? != 0.0,
                                toks.num()? != 0.0,
                                pt(&mut toks)?,
                            ))
                        })() else {
                            break;
                        };
                        let (rx, ry, rot, large, sweep, to) = a;
                        let segs = arc(cur, rx, ry, rot, large, sweep, to);
                        if let Some(s) = subs.last_mut() {
                            s.segs.extend(segs);
                        }
                        cur = to;
                        continue;
                    }
                    _ => break,
                };
                let Some(seg) = seg else { break };
                cur = match seg {
                    Seg::Line(p) | Seg::Cubic(_, _, p) => p,
                };
                if let Some(s) = subs.last_mut() {
                    s.segs.push(seg);
                }
            }
        }
    }
    subs
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tokens<'_> {
    fn skip(&mut self) {
        while self
            .s
            .get(self.i)
            .is_some_and(|c| c.is_ascii_whitespace() || *c == b',')
        {
            self.i += 1;
        }
    }

    /// `None` no fim; `Some(None)` quando vem um número (repete o comando).
    fn command(&mut self) -> Option<Option<u8>> {
        self.skip();
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() {
            self.i += 1;
            Some(Some(c))
        } else {
            Some(None)
        }
    }

    fn num(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        let mut dot = false;
        while let Some(&c) = self.s.get(self.i) {
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' && !dot {
                dot = true;
                self.i += 1;
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()
    }
}

/// Arco elíptico do SVG em cúbicas de no máximo 90° (SVG 1.1, F.6.5).
fn arc(
    from: [f32; 2],
    rx: f32,
    ry: f32,
    rot_deg: f32,
    large: bool,
    sweep: bool,
    to: [f32; 2],
) -> Vec<Seg> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || from == to {
        return vec![Seg::Line(to)];
    }
    let (sin, cos) = rot_deg.to_radians().sin_cos();
    let dx = (from[0] - to[0]) / 2.0;
    let dy = (from[1] - to[1]) / 2.0;
    let x1 = cos * dx + sin * dy;
    let y1 = -sin * dx + cos * dy;
    let l = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if l > 1.0 {
        rx *= l.sqrt();
        ry *= l.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = (num / den).max(0.0).sqrt();
    if large == sweep {
        k = -k;
    }
    let cx1 = k * rx * y1 / ry;
    let cy1 = -k * ry * x1 / rx;
    let cx = cos * cx1 - sin * cy1 + (from[0] + to[0]) / 2.0;
    let cy = sin * cx1 + cos * cy1 + (from[1] + to[1]) / 2.0;
    let angle = |ux: f32, uy: f32| uy.atan2(ux);
    let t1 = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry) - t1;
    let tau = std::f32::consts::TAU;
    if sweep && dt < 0.0 {
        dt += tau;
    } else if !sweep && dt > 0.0 {
        dt -= tau;
    }
    let n = (dt.abs() / (tau / 4.0)).ceil().max(1.0) as usize;
    let step = dt / n as f32;
    let alpha = 4.0 / 3.0 * (step / 4.0).tan();
    let point = |t: f32| {
        let (s, c) = t.sin_cos();
        let (x, y) = (rx * c, ry * s);
        [cos * x - sin * y + cx, sin * x + cos * y + cy]
    };
    let deriv = |t: f32| {
        let (s, c) = t.sin_cos();
        let (x, y) = (-rx * s, ry * c);
        [cos * x - sin * y, sin * x + cos * y]
    };
    (0..n)
        .map(|i| {
            let (a, b) = (t1 + step * i as f32, t1 + step * (i + 1) as f32);
            let (pa, pb) = (point(a), point(b));
            let (da, db) = (deriv(a), deriv(b));
            let end = if i + 1 == n { to } else { pb };
            Seg::Cubic(
                [pa[0] + alpha * da[0], pa[1] + alpha * da[1]],
                [pb[0] - alpha * db[0], pb[1] - alpha * db[1]],
                end,
            )
        })
        .collect()
}

// --- Ícone do app ---------------------------------------------------------

/// Cores da variação Óxido: azulejo, traço e hachura.
const TILE: [u8; 3] = [0xB5, 0x48, 0x2A];
const INK: [u8; 3] = [0xFC, 0xF6, 0xEE];
const HATCH: [u8; 3] = [0xD9, 0x7C, 0x58];

/// Grade de 100 px.
const TILE_PATH: &str = "M23 0 H77 A23 23 0 0 1 100 23 V77 A23 23 0 0 1 77 100 H23 \
                         A23 23 0 0 1 0 77 V23 A23 23 0 0 1 23 0 Z";
const HATCH_PATH: &str = "M25.1 42.3 L41.9 25.9 M25.4 50.5 L52.4 23.2 M24.9 58.6 L57.5 25.7 \
                          M26.8 65.5 L62.6 28.7 M32.1 68.5 L68.1 31.8 M37.5 70.5 L73.5 34.6 \
                          M41.7 73.5 L74.5 41.6 M46.8 76.9 L74.8 49.2 M58.3 73.3 L74.2 57";
const HOLE_PATH: &str = "M37.5 50 A12.5 12.5 0 1 0 62.5 50 A12.5 12.5 0 1 0 37.5 50 Z";
const NUT_PATH: &str = "M47.6 16.7 L80.1 35.3 M77.9 31.2 L77.4 68.9 M80.4 64.6 L47.2 83.4 \
                        M52.8 83.6 L19.8 64.4 M22.1 69 L22.6 31 M19.6 35.6 L52.5 16.4 \
                        M52.4 38.9 C58.6 39.4 61.4 44.4 61.1 50.3 C60.8 56.5 56.1 61.2 49.8 61 \
                        C43.8 60.8 38.9 56.2 39.1 50 C39.3 43.9 43.9 39.1 50.3 38.9 \
                        C52.8 38.8 54.9 39.4 56.9 40.7";
/// Abaixo de 48 px o traço é simplificado: sem hachura, linhas fechadas.
const NUT_SMALL: &str = "M50 19 L77 34.5 L77 65.5 L50 81 L23 65.5 L23 34.5 Z \
                         M61 50 A11 11 0 1 1 39 50 A11 11 0 1 1 61 50";
const NUT_TINY: &str = "M50 17 L78.5 33.5 L78.5 66.5 L50 83 L21.5 66.5 L21.5 33.5 Z \
                        M60 50 A10 10 0 1 1 40 50 A10 10 0 1 1 60 50";

fn skia_path(d: &str) -> Option<tiny_skia::Path> {
    let mut pb = tiny_skia::PathBuilder::new();
    for sub in parse(d) {
        pb.move_to(sub.start[0], sub.start[1]);
        for s in &sub.segs {
            match *s {
                Seg::Line(b) => pb.line_to(b[0], b[1]),
                Seg::Cubic(c1, c2, b) => pb.cubic_to(c1[0], c1[1], c2[0], c2[1], b[0], b[1]),
            }
        }
        if sub.closed {
            pb.close();
        }
    }
    pb.finish()
}

/// Ícone do app em `size`×`size`, com `margin` (fração do lado) livre em
/// volta do azulejo (o macOS espera uns 10%). RGBA sem alfa pré-multiplicado.
pub fn app_icon_rgba(size: u32, margin: f32) -> Option<Vec<u8>> {
    use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Pixmap, Stroke, Transform};
    let mut pm = Pixmap::new(size, size)?;
    let tile = size as f32 * (1.0 - 2.0 * margin);
    let off = size as f32 * margin;
    let tf = Transform::from_scale(tile / 100.0, tile / 100.0).post_translate(off, off);
    let paint = |c: [u8; 3]| {
        let mut p = Paint::default();
        p.set_color_rgba8(c[0], c[1], c[2], 255);
        p.anti_alias = true;
        p
    };
    let stroke = |width: f32| Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pm.fill_path(
        &skia_path(TILE_PATH)?,
        &paint(TILE),
        FillRule::Winding,
        tf,
        None,
    );
    let (nut, width) = if tile >= 48.0 {
        pm.stroke_path(
            &skia_path(HATCH_PATH)?,
            &paint(HATCH),
            &stroke(2.2),
            tf,
            None,
        );
        pm.fill_path(
            &skia_path(HOLE_PATH)?,
            &paint(TILE),
            FillRule::Winding,
            tf,
            None,
        );
        (NUT_PATH, 3.6)
    } else if tile >= 20.0 {
        (NUT_SMALL, 7.0)
    } else {
        (NUT_TINY, 9.5)
    };
    pm.stroke_path(&skia_path(nut)?, &paint(INK), &stroke(width), tf, None);
    Some(
        pm.pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect(),
    )
}

/// Ícone da janela. No macOS ele também vai para o Dock, então leva a margem
/// dos ícones do sistema.
pub fn window_icon() -> egui::IconData {
    let size = 256;
    let margin = if cfg!(target_os = "macos") { 0.1 } else { 0.0 };
    egui::IconData {
        rgba: app_icon_rgba(size, margin).unwrap_or_default(),
        width: size,
        height: size,
    }
}

/// O ícone do app em PNG (comando `rustdraw icon`).
pub fn app_icon_png(size: u32, margin: f32) -> Result<Vec<u8>, String> {
    let rgba = app_icon_rgba(size, margin).ok_or("tamanho inválido")?;
    let img = image::RgbaImage::from_raw(size, size, rgba).ok_or("tamanho inválido")?;
    crate::files::encode_png(&img).ok_or_else(|| "falha ao gerar o PNG".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_absolute_and_relative_commands() {
        let subs = parse("M1 2 H5 V7 L1 2 Z m1 1 h2 v2 h-2 z");
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].start, [1.0, 2.0]);
        assert_eq!(
            subs[0].segs,
            vec![
                Seg::Line([5.0, 2.0]),
                Seg::Line([5.0, 7.0]),
                Seg::Line([1.0, 2.0])
            ]
        );
        assert!(subs[0].closed);
        // depois do Z o ponto atual volta ao início do subcaminho
        assert_eq!(subs[1].start, [2.0, 3.0]);
        assert_eq!(subs[1].segs.last(), Some(&Seg::Line([2.0, 5.0])));
        assert!(subs[1].closed);
    }

    #[test]
    fn implicit_lineto_after_move_and_negative_numbers() {
        let subs = parse("M0 0 1-1 2,0");
        assert_eq!(
            subs[0].segs,
            vec![Seg::Line([1.0, -1.0]), Seg::Line([2.0, 0.0])]
        );
    }

    #[test]
    fn full_circle_from_two_arcs_stays_on_the_circle() {
        let subs = parse("M2 5 a3 3 0 1 0 6 0 a3 3 0 1 0 -6 0");
        let pts = subs[0].flatten();
        assert!(pts.len() > 8);
        for p in pts {
            let r = ((p[0] - 5.0).powi(2) + (p[1] - 5.0).powi(2)).sqrt();
            assert!((r - 3.0).abs() < 0.05, "raio {r}");
        }
    }

    #[test]
    fn every_icon_parses_inside_the_grid() {
        let all = [
            Icon::Hand,
            Icon::Select,
            Icon::Rect,
            Icon::Diamond,
            Icon::Ellipse,
            Icon::Arrow,
            Icon::Line,
            Icon::Draw,
            Icon::Text,
            Icon::Image,
            Icon::Eraser,
            Icon::Frame,
            Icon::Laser,
            Icon::Note,
            Icon::Lock(true),
            Icon::Lock(false),
            Icon::Menu,
            Icon::Undo,
            Icon::Redo,
            Icon::ZoomIn,
            Icon::ZoomOut,
            Icon::Open,
            Icon::Save,
            Icon::ExportPng,
            Icon::ExportSvg,
            Icon::Moon,
            Icon::Grid,
            Icon::SendToBack,
            Icon::SendBackward,
            Icon::BringForward,
            Icon::BringToFront,
            Icon::Duplicate,
            Icon::Delete,
            Icon::Library,
            Icon::Help,
            Icon::SaveAs,
            Icon::NewFile,
            Icon::ExportFrame,
            Icon::Cut,
            Icon::Copy,
            Icon::Paste,
            Icon::CopyStyles,
            Icon::PasteStyles,
            Icon::Snap,
            Icon::Zen,
            Icon::ViewMode,
            Icon::Stats,
            Icon::Group,
            Icon::Ungroup,
            Icon::FlipH,
            Icon::FlipV,
            Icon::SelectAll,
            Icon::AddToLibrary,
            Icon::Mcp,
        ];
        for icon in all {
            let (stroke, fill) = icon.paths();
            let subs = parse(stroke);
            assert!(!subs.is_empty());
            for sub in subs.iter().chain(&parse(fill)) {
                assert!(!sub.segs.is_empty());
                for p in sub.flatten() {
                    assert!((1.0..=23.0).contains(&p[0]) && (1.0..=23.0).contains(&p[1]));
                }
            }
        }
    }

    /// `paint` preenche com `convex_polygon`: um subcaminho côncavo sairia torto.
    #[test]
    fn fill_subpaths_are_convex() {
        for icon in [
            Icon::Line,
            Icon::Image,
            Icon::ExportPng,
            Icon::ExportSvg,
            Icon::SendBackward,
            Icon::BringForward,
            Icon::Help,
            Icon::Zen,
            Icon::Group,
            Icon::FlipH,
            Icon::FlipV,
        ] {
            for sub in parse(icon.paths().1) {
                let pts = sub.flatten();
                let n = pts.len();
                let mut sign = 0.0f32;
                for i in 0..n {
                    let (a, b, c) = (pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
                    let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
                    if cross.abs() < 1e-4 {
                        continue;
                    }
                    assert!(sign == 0.0 || sign == cross.signum(), "{:?}", sub.start);
                    sign = cross.signum();
                }
            }
        }
    }

    #[test]
    fn app_icon_has_the_tile_color_and_round_corners() {
        for size in [16, 32, 64, 256] {
            let px = app_icon_rgba(size, 0.0).unwrap();
            assert_eq!(px.len(), (size * size * 4) as usize);
            // canto transparente (azulejo arredondado)
            assert_eq!(px[3], 0);
            // perto da borda, no meio do lado esquerdo: cor do azulejo
            let i = ((size / 2 * size + size / 16) * 4) as usize;
            assert_eq!(&px[i..i + 4], &[TILE[0], TILE[1], TILE[2], 255]);
        }
        // com margem, a borda fica livre
        let px = app_icon_rgba(128, 0.1).unwrap();
        let i = ((64 * 128 + 4) * 4) as usize;
        assert_eq!(px[i + 3], 0);
    }

    #[test]
    fn app_icon_png_is_a_png() {
        let png = app_icon_png(32, 0.0).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
