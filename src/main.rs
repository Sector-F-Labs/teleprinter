//! Teleprinter: a terminal emulator, built on egui and Patina, with
//! alacritty_terminal doing the emulation.

mod app;
mod config;
mod fonts;
mod palette;
mod screenshot;
mod session;
mod view;

fn main() -> eframe::Result {
    let config = config::Config::load();
    let custom_frame = !config.system_title_bar;
    let options = eframe::NativeOptions {
        viewport: patina::frame::viewport(
            egui::ViewportBuilder::default()
                .with_title("Teleprinter")
                .with_app_id("com.sectorflabs.Teleprinter")
                .with_inner_size([820.0, 520.0])
                .with_min_inner_size([320.0, 200.0]),
            custom_frame,
        ),
        ..Default::default()
    };
    eframe::run_native("teleprinter", options, Box::new(|cc| Ok(Box::new(app::App::new(cc, config)))))
}
