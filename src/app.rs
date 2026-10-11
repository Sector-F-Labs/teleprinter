//! The window: Patina's frame and title bar around the tiled panes, each a
//! terminal session in a pinned egui window (ADR-0005).

use std::collections::HashMap;
use std::path::PathBuf;

use alacritty_terminal::event::WindowSize;
use egui::{Id, Rect};
use patina::frame::{self, TitleBar};
use patina::skin::{self, Skin};

use crate::config::Config;
use crate::palette::Palette;
use crate::session::Session;
use crate::tiles::{Dir, Divider, Node, PaneId};
use crate::view::{self, Requests, ViewState};

/// The skin used until the user picks one: stock egui, following the desktop.
const HOME_SKIN: &str = "Default";

const MIN_FONT: f32 = 8.0;
const MAX_FONT: f32 = 32.0;

/// Space between tiles, and around them; dividers are dragged in it.
const GAP: f32 = 4.0;

/// What the menus ask for, handled after they're drawn.
enum Action {
    Quit,
    SetSkin(String),
    SystemTitleBar(bool),
    FontSize(f32),
    Copy,
    Paste,
    /// Split the active pane.
    Split(Dir),
    Close(PaneId),
}

struct Pane {
    session: Result<Session, String>,
    view: ViewState,
}

pub struct App {
    config: Config,
    custom_frame: bool,
    skins: Vec<Skin>,
    skin_errors: Vec<String>,
    panes: HashMap<PaneId, Pane>,
    tree: Node,
    /// The pane that has the keyboard, and that splits and menu actions act on.
    active: PaneId,
    next_id: PaneId,
    /// The pane whose title bar is being dragged onto another, to swap them.
    dragging: Option<PaneId>,
    /// `TELEPRINTER_COMMAND`, run in every pane instead of the login shell.
    command: Option<(String, Vec<String>)>,
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
        let mut app = Self {
            custom_frame: !config.system_title_bar,
            config,
            skins,
            skin_errors,
            panes: HashMap::new(),
            tree: Node::Leaf(1),
            active: 1,
            next_id: 1,
            dragging: None,
            command,
            requests: Requests { focus: true, ..Default::default() },
            screenshot: std::env::var_os("TELEPRINTER_SCREENSHOT").map(|p| (PathBuf::from(p), false)),
        };
        let first = app.spawn(&cc.egui_ctx, dirs::home_dir());
        app.tree = Node::Leaf(first);
        app.active = first;
        // `TELEPRINTER_SPLITS=right,down` splits the active pane in turn at start
        // (for screenshots).
        for split in std::env::var("TELEPRINTER_SPLITS").unwrap_or_default().split(',') {
            match split.trim() {
                "right" => app.split(&cc.egui_ctx, Dir::Right),
                "down" => app.split(&cc.egui_ctx, Dir::Down),
                _ => {}
            }
        }
        app.skin().apply(&cc.egui_ctx);
        app
    }

    /// Starts a shell in a new pane, not yet placed in the tree.
    fn spawn(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        // A first guess at the size; the widget resizes it on its first frame.
        let size = WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 };
        let session = Session::spawn(ctx, self.command.clone(), cwd, size).map_err(|e| e.to_string());
        self.panes.insert(id, Pane { session, view: ViewState::default() });
        id
    }

    /// Splits the active pane; the new one starts in the same directory and
    /// takes the keyboard.
    fn split(&mut self, ctx: &egui::Context, dir: Dir) {
        let cwd = self.panes.get(&self.active).and_then(|p| p.session.as_ref().ok()).and_then(Session::cwd);
        let id = self.spawn(ctx, cwd.or_else(dirs::home_dir));
        self.tree.split(self.active, dir, id);
        self.activate(id);
    }

    /// Closes a pane; closing the last one closes the window.
    fn close(&mut self, ctx: &egui::Context, id: PaneId) {
        if !self.tree.remove(id) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.panes.remove(&id);
        if self.active == id {
            let next = self.tree.panes()[0];
            self.activate(next);
        }
    }

    fn activate(&mut self, id: PaneId) {
        self.active = id;
        self.requests.focus = true;
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

    fn title(&self, id: PaneId) -> String {
        match self.panes.get(&id).map(|p| &p.session) {
            Some(Ok(session)) => session.title.clone().unwrap_or_else(|| "Shell".into()),
            _ => "Shell".into(),
        }
    }

    /// Left side of the title bar: word menus (Patina's ADR-0006).
    fn menus(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>, problems: &[String]) {
        ui.menu_button("File", |ui| {
            if ui.button("Split Right").clicked() {
                actions.push(Action::Split(Dir::Right));
                ui.close();
            }
            if ui.button("Split Down").clicked() {
                actions.push(Action::Split(Dir::Down));
                ui.close();
            }
            if ui.button("Close Pane").clicked() {
                actions.push(Action::Close(self.active));
                ui.close();
            }
            ui.separator();
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

    /// One pane: its session in an egui window pinned to `rect`, the same
    /// window whichever tile it's in. Returns the window's title strip, for
    /// dragging.
    fn show_pane(
        &mut self,
        ctx: &egui::Context,
        id: PaneId,
        rect: Rect,
        palette: &Palette,
        actions: &mut Vec<Action>,
    ) -> Option<(Rect, egui::LayerId)> {
        let title = self.title(id);
        let active = id == self.active;
        let style = ctx.global_style();
        let several = self.panes.len() > 1;
        // The active pane's border is the selection colour, once there's more than one.
        let stroke = if active && several {
            egui::Stroke::new(1.5, style.visuals.selection.bg_fill)
        } else {
            style.visuals.window_stroke
        };
        let frame = egui::Frame::window(&style).shadow(egui::Shadow::NONE).stroke(stroke).fill(palette.background);
        let margin = frame.total_margin();
        let mut open = true;
        let font_size = self.config.font_size;
        let mut idle = Requests::default();
        let requests = if active { &mut self.requests } else { &mut idle };
        let pane = self.panes.get_mut(&id)?;
        let mut content_top = rect.top();
        let shown = egui::Window::new(title)
            .id(Id::new(("pane", id)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .constrain(false)
            .frame(frame)
            .fixed_rect(rect)
            .show(ctx, |ui| {
                content_top = ui.max_rect().top();
                let Ok(session) = &mut pane.session else {
                    let error = pane.session.as_ref().err().cloned().unwrap_or_default();
                    ui.colored_label(ui.visuals().error_fg_color, format!("Couldn't start the shell: {error}"));
                    return false;
                };
                let response = view::show(ui, session, &mut pane.view, palette, font_size, requests, active);
                let mut copy = false;
                let mut paste = false;
                response.context_menu(|ui| {
                    if ui.button("Split Right").clicked() {
                        actions.push(Action::Split(Dir::Right));
                        ui.close();
                    }
                    if ui.button("Split Down").clicked() {
                        actions.push(Action::Split(Dir::Down));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Copy").clicked() {
                        copy = true;
                        ui.close();
                    }
                    if ui.button("Paste").clicked() {
                        paste = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Close").clicked() {
                        actions.push(Action::Close(id));
                        ui.close();
                    }
                });
                if copy {
                    actions.push(Action::Copy);
                }
                if paste {
                    actions.push(Action::Paste);
                }
                // A click or right-click in a pane makes it the active one.
                response.has_focus() || response.clicked() || response.secondary_clicked()
            })?;
        if !open {
            actions.push(Action::Close(id));
        }
        if shown.inner == Some(true) && !active {
            self.active = id;
        }
        let window = shown.response.rect;
        // The title bar, less the close button at its right end.
        let bottom = content_top - margin.top;
        let height = bottom - window.top();
        let strip = Rect::from_min_max(window.min, egui::pos2(window.right() - height - margin.right, bottom));
        Some((strip, shown.response.layer_id))
    }

    /// A tile's title bar, as a handle: drag it onto another tile to swap them.
    fn drag_handle(&mut self, ctx: &egui::Context, id: PaneId, strip: Rect, layer: egui::LayerId) {
        // The Ui and the handle need ids of their own; sharing one, the handle never got dragged.
        let ui = egui::Ui::new(ctx.clone(), Id::new(("pane-handle-ui", id)), egui::UiBuilder::new().layer_id(layer).max_rect(strip));
        let response = ui.interact(strip, Id::new(("pane-handle", id)), egui::Sense::click_and_drag());
        if self.panes.len() > 1 {
            response.clone().on_hover_cursor(egui::CursorIcon::Grab);
        }
        if response.clicked() {
            self.activate(id);
        }
        if response.drag_started() && self.panes.len() > 1 {
            self.dragging = Some(id);
        }
    }

    /// While a tile is dragged: highlight the tile under the pointer, and
    /// swap the two when the button is released over it.
    fn drag_to_swap(&mut self, ctx: &egui::Context, tiles: &[(PaneId, Rect)]) {
        let Some(from) = self.dragging else { return };
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let (pos, released) = ctx.input(|i| (i.pointer.interact_pos(), !i.pointer.primary_down()));
        let target = pos.and_then(|p| tiles.iter().find(|(id, rect)| *id != from && rect.contains(p)));
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("swap-target")));
        let selection = ctx.global_style().visuals.selection;
        if let Some((_, rect)) = tiles.iter().find(|(id, _)| *id == from) {
            painter.rect_stroke(*rect, 4.0, egui::Stroke::new(1.5, selection.bg_fill), egui::StrokeKind::Inside);
        }
        if let Some((_, rect)) = target {
            painter.rect_filled(*rect, 4.0, selection.bg_fill.gamma_multiply(0.35));
            painter.rect_stroke(*rect, 4.0, egui::Stroke::new(2.0, selection.bg_fill), egui::StrokeKind::Inside);
        }
        if released {
            if let Some((to, _)) = target {
                self.tree.swap(from, *to);
            }
            self.dragging = None;
            self.activate(from);
        }
    }

    /// The handle in the gap between two sides of a split, for resizing it.
    fn divider(&mut self, ctx: &egui::Context, divider: &Divider) {
        let id = Id::new(("divider", &divider.path));
        egui::Area::new(id).fixed_pos(divider.rect.min).constrain(false).show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(divider.rect.size(), egui::Sense::drag());
            let cursor = match divider.dir {
                Dir::Right => egui::CursorIcon::ResizeHorizontal,
                Dir::Down => egui::CursorIcon::ResizeVertical,
            };
            let response = response.on_hover_cursor(cursor);
            if response.hovered() || response.dragged() {
                let stroke = ui.visuals().widgets.hovered.fg_stroke;
                let c = rect.center();
                match divider.dir {
                    Dir::Right => ui.painter().vline(c.x, c.y - 20.0..=c.y + 20.0, stroke),
                    Dir::Down => ui.painter().hline(c.x - 20.0..=c.x + 20.0, c.y, stroke),
                };
            }
            if response.dragged()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let area = divider.area;
                let ratio = match divider.dir {
                    Dir::Right => (pos.x - area.left()) / (area.width() - GAP),
                    Dir::Down => (pos.y - area.top()) / (area.height() - GAP),
                };
                self.tree.set_ratio(&divider.path, ratio);
            }
        });
    }

    /// Every pane in its tile, the dividers between them, and dragging to swap.
    fn show_tiles(&mut self, ctx: &egui::Context, desktop: Rect, palette: &Palette, actions: &mut Vec<Action>) {
        let (tiles, dividers) = self.tree.layout(desktop, GAP);
        for (id, rect) in &tiles {
            if let Some((strip, layer)) = self.show_pane(ctx, *id, *rect, palette, actions) {
                self.drag_handle(ctx, *id, strip, layer);
            }
        }
        for divider in &dividers {
            self.divider(ctx, divider);
        }
        self.drag_to_swap(ctx, &tiles);
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

        let mut exited = Vec::new();
        for (id, pane) in &mut self.panes {
            if let Ok(session) = &mut pane.session {
                session.pump(&ctx, &palette);
                if session.exited && self.screenshot.is_none() {
                    exited.push(*id);
                }
            }
        }
        for id in exited {
            self.close(&ctx, id);
        }

        let title = match self.panes.len() {
            0 | 1 => self.title(self.active),
            n => format!("{} ({n} panes)", self.title(self.active)),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));

        let mut actions = Vec::new();
        egui::Panel::top("header").frame(frame::header_frame(ui.style(), &style, radius)).show(ui, |ui| {
            TitleBar::new(&title, &style).custom(self.custom_frame).menu_bar(true).show(
                ui,
                |ui| self.menus(ui, &mut actions, &problems),
                // A terminal's output is the shell's; nothing goes on the right.
                |_ui| {},
            );
        });

        // The tiles sit on the window's own panel colour, in the gaps between them.
        let desktop = egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(GAP).corner_radius(egui::CornerRadius {
                sw: radius,
                se: radius,
                ..Default::default()
            }))
            .show(ui, |ui| ui.max_rect())
            .inner;
        self.show_tiles(&ctx, desktop, &palette, &mut actions);

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
                Action::Split(dir) => self.split(&ctx, dir),
                Action::Close(id) => self.close(&ctx, id),
            }
            self.requests.focus = true;
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, RawInput, pos2, vec2};

    /// An app with panes 1 | 2 side by side and no shells behind them.
    fn two_panes() -> App {
        let (skins, _) = skin::load_all(skin::BUILT_IN, &[]);
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        let pane = || Pane { session: Err("no shell in tests".into()), view: ViewState::default() };
        App {
            config: Config::default(),
            custom_frame: true,
            skins,
            skin_errors: Vec::new(),
            panes: HashMap::from([(1, pane()), (2, pane())]),
            tree,
            active: 1,
            next_id: 3,
            dragging: None,
            command: None,
            requests: Requests::default(),
            screenshot: None,
        }
    }

    const DESKTOP: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 800.0, y: 500.0 } };

    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<Event>) {
        let input = RawInput { screen_rect: Some(DESKTOP), events, ..Default::default() };
        let _ = ctx.run_ui(input, |ui| {
            let palette = Palette::new(&app.skins[0], ui.visuals()).0;
            let mut actions = Vec::new();
            app.show_tiles(ui.ctx(), DESKTOP, &palette, &mut actions);
        });
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// Drags from `from` to `to` in small steps, as a mouse would.
    fn drag(ctx: &egui::Context, app: &mut App, from: Pos2, to: Pos2) {
        frame(ctx, app, vec![Event::PointerMoved(from)]);
        frame(ctx, app, vec![button(from, true)]);
        for step in 1..=10 {
            let p = from + (to - from) * (step as f32 / 10.0);
            frame(ctx, app, vec![Event::PointerMoved(p)]);
        }
        frame(ctx, app, vec![button(to, false)]);
        frame(ctx, app, vec![]);
    }

    #[test]
    fn dragging_a_title_bar_onto_another_tile_swaps_them() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        // Pane 1's title bar, then the middle of pane 2.
        drag(&ctx, &mut app, pos2(100.0, 15.0), pos2(600.0, 250.0));
        assert_eq!(app.tree.panes(), [2, 1]);
        assert_eq!(app.dragging, None);
        assert_eq!(app.active, 1, "the dragged pane stays active");
    }

    #[test]
    fn dropping_a_tile_on_itself_changes_nothing() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        drag(&ctx, &mut app, pos2(100.0, 15.0), pos2(200.0, 250.0));
        assert_eq!(app.tree.panes(), [1, 2]);
    }

    #[test]
    fn dragging_the_divider_resizes_the_split() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        let gap = app.tree.layout(DESKTOP, GAP).1[0].rect.center();
        drag(&ctx, &mut app, gap, gap + vec2(-200.0, 0.0));
        let tiles = app.tree.layout(DESKTOP, GAP).0;
        assert!((tiles[0].1.width() - 200.0).abs() < 1.0, "{:?}", tiles[0].1);
    }
}
