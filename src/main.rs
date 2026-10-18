//! Teleprinter: a terminal emulator, built on egui and Patina, with
//! alacritty_terminal doing the emulation.

mod app;
mod config;
mod control;
mod emoji;
mod fonts;
mod palette;
mod screenshot;
mod session;
mod tiles;
mod view;

/// The window icon, rendered from `packaging/linux/teleprinter.svg` by `make
/// icons`. On Wayland, GNOME shows the installed launcher's icon instead.
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../assets/teleprinter-icon-128.png");

fn window_icon() -> egui::IconData {
    let icon = image::load_from_memory_with_format(WINDOW_ICON_PNG, image::ImageFormat::Png)
        .expect("embedded window icon should decode as PNG")
        .into_rgba8();
    let (width, height) = icon.dimensions();
    egui::IconData {
        rgba: icon.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "msg") {
        std::process::exit(control::client(&args[1..]));
    }
    let config = config::Config::load();
    let custom_frame = !config.system_title_bar;
    let options = eframe::NativeOptions {
        viewport: patina::frame::viewport(
            egui::ViewportBuilder::default()
                .with_title("Teleprinter")
                .with_app_id("com.sectorflabs.Teleprinter")
                .with_icon(window_icon())
                .with_inner_size([1280.0, 800.0])
                .with_min_inner_size([320.0, 200.0]),
            custom_frame,
        ),
        ..Default::default()
    };
    eframe::run_native(
        "teleprinter",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, config)))),
    )
}
