//! The window: Patina's frame and title bar around one terminal session.

use std::path::PathBuf;

use alacritty_terminal::event::WindowSize;
use patina::frame::{self, TitleBar};
use patina::skin::{self, Skin};

use crate::config::Config;
use crate::palette::Palette;
use crate::session::Session;
use crate::view::{self, Requests, ViewState};

/// The skin used until the user picks one: stock egui, following the desktop.
const HOME_SKIN: &str = "Default";

const MIN_FONT: f32 = 8.0;
const MAX_FONT: f32 = 32.0;

/// What the header asks for, handled after it's drawn.
enum Action {
    Quit,
    SetSkin(String),
    SystemTitleBar(bool),
    FontSize(f32),
    Copy,
    Paste,
}

pub struct App {
    config: Config,
    custom_frame: bool,
    skins: Vec<Skin>,
    skin_errors: Vec<String>,
    session: Result<Session, String>,
    view: ViewState,
    requests: Requests,
    /// `TELEPRINTER_SCREENSHOT`: where to save one, and whether it was asked for.
    screenshot: Option<(PathBuf, bool)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, mut config: Config) -> Self {
        patina::appearance::watch(&cc.egui_ctx);
        crate::fonts::install(&cc.egui_ctx);
        let dirs: Vec<_> = skin::shared_dir().into_iter().collect();
        let (skins, skin_errors) = skin::load_all(skin::BUILT_IN, &dirs);
        if let Ok(name) = std::env::var("TELEPRINTER_SKIN") {
            config.skin = Some(name); // for this run only: never saved
        }
        // `TELEPRINTER_COMMAND` runs that through `sh -c` instead of the login shell
        // (for screenshots and tests).
        let command = std::env::var("TELEPRINTER_COMMAND").ok().map(|c| ("/bin/sh".to_string(), vec!["-c".into(), c]));
        // A first guess at the size; the widget resizes it on the first frame.
        let size = WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 };
        let session = Session::spawn(&cc.egui_ctx, command, dirs::home_dir(), size).map_err(|e| e.to_string());
        let app = Self {
            custom_frame: !config.system_title_bar,
            config,
            skins,
            skin_errors,
            session,
            view: ViewState::default(),
            requests: Requests { focus: true, ..Default::default() },
            screenshot: std::env::var_os("TELEPRINTER_SCREENSHOT").map(|p| (PathBuf::from(p), false)),
        };
        app.skin().apply(&cc.egui_ctx);
        app
    }

    /// The chosen skin, falling back to `HOME_SKIN` (a saved name may no
    /// longer exist), then Default.
    fn skin(&self) -> &Skin {
        let find = |name: &str| self.skins.iter().find(|s| s.name == name);
        find(self.config.skin.as_deref().unwrap_or(HOME_SKIN)).or_else(|| find(HOME_SKIN)).unwrap_or(&self.skins[0])
    }

    fn save_config(&self) {
        if self.screenshot.is_none() && std::env::var_os("TELEPRINTER_SKIN").is_none() {
            self.config.save();
        }
    }

    /// Left side of the title bar: word menus (Patina's ADR-0006).
    fn menus(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>, problems: &[String]) {
        ui.menu_button("File", |ui| {
            if ui.button("Quit").clicked() {
                actions.push(Action::Quit);
                ui.close();
            }
        });
        ui.menu_button("Edit", |ui| {
            if ui.add(egui::Button::new("Copy").shortcut_text("Ctrl+Shift+C")).clicked() {
                actions.push(Action::Copy);
                ui.close();
            }
            if ui.add(egui::Button::new("Paste").shortcut_text("Ctrl+Shift+V")).clicked() {
                actions.push(Action::Paste);
                ui.close();
            }
        });
        ui.menu_button("View", |ui| {
            let size = self.config.font_size;
            if ui.add_enabled(size < MAX_FONT, egui::Button::new("Larger Text")).clicked() {
                actions.push(Action::FontSize(size + 1.0));
            }
            if ui.add_enabled(size > MIN_FONT, egui::Button::new("Smaller Text")).clicked() {
                actions.push(Action::FontSize(size - 1.0));
            }
            if ui.button("Normal Size").clicked() {
                actions.push(Action::FontSize(Config::default().font_size));
            }
            ui.separator();
            let mut system = !self.custom_frame;
            if ui.checkbox(&mut system, "System Title Bar").changed() {
                actions.push(Action::SystemTitleBar(system));
            }
        });
        ui.menu_button("Skins", |ui| {
            let current = self.skin().name.clone();
            for skin in &self.skins {
                if ui.radio(skin.name == current, &skin.name).clicked() {
                    actions.push(Action::SetSkin(skin.name.clone()));
                }
            }
            for error in self.skin_errors.iter().chain(problems) {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        });
    }

    fn take_screenshot(&mut self, ctx: &egui::Context) {
        let Some((path, requested)) = &mut self.screenshot else { return };
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            match crate::screenshot::save(&image, path) {
                Ok(()) => eprintln!("saved {}", path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if !*requested && ctx.input(|i| i.time) > 1.5 {
            *requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Every frame: Default's colours follow the desktop's light/dark setting.
        let style = self.skin().frame_style(ui.visuals());
        let (palette, problems) = Palette::new(self.skin(), ui.visuals());
        let radius = if self.custom_frame { frame::corner_radius(&ctx, style.radius) } else { 0 };

        let title = match &mut self.session {
            Ok(session) => {
                session.pump(&ctx, &palette);
                if session.exited && self.screenshot.is_none() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                session.title.clone().unwrap_or_else(|| "Teleprinter".into())
            }
            Err(_) => "Teleprinter".into(),
        };
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Title(title.clone()));

        let mut actions = Vec::new();
        egui::Panel::top("header").frame(frame::header_frame(ui.style(), &style, radius)).show(ui, |ui| {
            TitleBar::new(&title, &style).custom(self.custom_frame).menu_bar(true).show(
                ui,
                |ui| self.menus(ui, &mut actions, &problems),
                // A terminal's output is the shell's; nothing goes on the right.
                |_ui| {},
            );
        });
        for action in actions {
            match action {
                Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Action::SetSkin(name) => {
                    self.config.skin = Some(name);
                    self.skin().apply(&ctx);
                    self.save_config();
                }
                Action::SystemTitleBar(on) => {
                    self.custom_frame = !on;
                    self.config.system_title_bar = on;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(on));
                    self.save_config();
                }
                Action::FontSize(size) => {
                    self.config.font_size = size.clamp(MIN_FONT, MAX_FONT);
                    self.save_config();
                }
                Action::Copy => self.requests.copy = true,
                Action::Paste => {
                    self.requests.paste = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                }
            }
            self.requests.focus = true;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.background).inner_margin(6).corner_radius(egui::CornerRadius {
                sw: radius,
                se: radius,
                ..Default::default()
            }))
            .show(ui, |ui| match &mut self.session {
                Ok(session) => {
                    let response =
                        view::show(ui, session, &mut self.view, &palette, self.config.font_size, &mut self.requests);
                    response.context_menu(|ui| {
                        if ui.button("Copy").clicked() {
                            self.requests.copy = true;
                            self.requests.focus = true;
                            ui.close();
                        }
                        if ui.button("Paste").clicked() {
                            self.requests.paste = true;
                            self.requests.focus = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                            ui.close();
                        }
                    });
                }
                Err(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, format!("Couldn't start the shell: {error}"));
                }
            });

        if self.custom_frame {
            frame::edges(&ctx, radius, style.border);
        }
        self.take_screenshot(&ctx);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        frame::clear_color(visuals, self.custom_frame)
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        frame::release_grab(ctx, raw_input);
    }
}
