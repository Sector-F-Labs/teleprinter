//! MesloLGS NF, bundled (ADR-0002): the terminal grid's regular, bold,
//! italic and bold-italic faces, with egui's own fonts behind them as
//! fallback for anything Meslo lacks. Colour emoji are painted apart from
//! text, by `emoji` (ADR-0013).

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontId};

const FACES: [(&str, &[u8]); 4] = [
    ("meslo", include_bytes!("../assets/fonts/MesloLGS-NF-Regular.ttf")),
    ("meslo-bold", include_bytes!("../assets/fonts/MesloLGS-NF-Bold.ttf")),
    ("meslo-italic", include_bytes!("../assets/fonts/MesloLGS-NF-Italic.ttf")),
    ("meslo-bold-italic", include_bytes!("../assets/fonts/MesloLGS-NF-BoldItalic.ttf")),
];

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, bytes) in FACES {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
        let mut list = vec![name.to_string()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), list);
    }
    // egui's own monospace text (menus never use it, code labels do) gets Meslo too.
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "meslo".into());
    ctx.set_fonts(fonts);
}

/// The face for a cell's bold/italic flags.
pub fn font(size: f32, bold: bool, italic: bool) -> FontId {
    let name = match (bold, italic) {
        (false, false) => "meslo",
        (true, false) => "meslo-bold",
        (false, true) => "meslo-italic",
        (true, true) => "meslo-bold-italic",
    };
    FontId::new(size, FontFamily::Name(name.into()))
}
