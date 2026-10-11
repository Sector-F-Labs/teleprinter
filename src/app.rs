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
use crate::tiles::{Dir, Divider, Node, PaneId, Side};
use crate::view::{self, Requests, ViewState};

/// The skin used until the user picks one: stock egui, following the desktop.
const HOME_SKIN: &str = "Default";

const MIN_FONT: f32 = 8.0;
const MAX_FONT: f32 = 32.0;

/// Space between tiles, and around them; dividers are dragged in it.
const GAP: f32 = 4.0;

/// Seconds a dragged pane must stay over another before dropping swaps them.
const SWAP_AFTER: f64 = 0.6;

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
    /// A new pane in the empty space at this point (the background's menu).
    NewAt(egui::Pos2),
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
    /// The pane the dragged one is over, and since when (egui time), for swapping.
    hovering: Option<(PaneId, f64)>,
    /// How tall a collapsed pane is: its title bar, measured from the panes shown.
    title_height: f32,
    /// Where each pane was last laid out, for placing a new one in empty space.
    tiles: Vec<(PaneId, Rect)>,
    /// Where the background was right-clicked, for its menu.
    menu_at: Option<egui::Pos2>,
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
            hovering: None,
            tiles: Vec::new(),
            menu_at: None,
            title_height: 34.0,
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
        // `TELEPRINTER_COLLAPSED=2,3` starts those panes collapsed (for screenshots).
        for id in std::env::var("TELEPRINTER_COLLAPSED").unwrap_or_default().split(',') {
            if let Ok(id) = id.trim().parse() {
                set_collapsed(&cc.egui_ctx, id, true);
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

    /// A new pane in the empty space at `at`: below the pane above it in that
    /// column (the space folded panes leave), or below the active pane.
    fn new_at(&mut self, ctx: &egui::Context, at: egui::Pos2) {
        let target = self.pane_above(at);
        let cwd = self.panes.get(&target).and_then(|p| p.session.as_ref().ok()).and_then(Session::cwd);
        let id = self.spawn(ctx, cwd.or_else(dirs::home_dir));
        self.tree.insert(target, Side::Bottom, id);
        self.activate(id);
    }

    /// The pane nearest above `at` in its column, else the active one.
    fn pane_above(&self, at: egui::Pos2) -> PaneId {
        self.tiles
            .iter()
            .filter(|(_, r)| r.x_range().contains(at.x) && r.bottom() <= at.y)
            .max_by(|a, b| a.1.bottom().total_cmp(&b.1.bottom()))
            .map_or(self.active, |(id, _)| *id)
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
            Some(Ok(session)) => session.label(),
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
        // Not set when the window is collapsed and its contents aren't shown.
        let mut content_top = None;
        // egui sizes a collapsed window's title bar to its text, so the text is sized to
        // fill the tile: its width less the arrow, the close button and the gaps around them.
        let title: egui::Atoms<'static> = if collapsed(ctx, id) {
            use egui::AtomExt;
            let button = ctx.fonts_mut(|f| f.row_height(&egui::TextStyle::Heading.resolve(&style)));
            let gaps = 4.0 * style.spacing.item_spacing.x;
            let width = rect.width() - margin.sum().x - 2.0 * button - gaps;
            // Centred like an open window's title: equal spacers either side of the text.
            let font = egui::TextStyle::Heading.resolve(&style);
            let text = ctx.fonts_mut(|f| f.layout_no_wrap(title.clone(), font, egui::Color32::WHITE).size().x);
            let side = ((width - text) / 2.0 - style.spacing.item_spacing.x).max(0.0);
            let spacer = |n: u8| egui::Atom::custom(Id::new(("title-spacer", id, n)), egui::vec2(side, 0.0));
            egui::Atoms::new((spacer(0), title.atom_size(egui::vec2(text, button)), spacer(1)))
        } else {
            egui::Atoms::new(title)
        };
        let shown = egui::Window::new(title)
            .id(pane_id(id))
            .open(&mut open)
            .collapsible(true)
            .resizable(false)
            .movable(false)
            .constrain(false)
            .frame(frame)
            .fixed_rect(rect)
            .show(ctx, |ui| {
                content_top = Some(ui.max_rect().top());
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
        // The title bar, less the collapse arrow at its left end and the close button at its
        // right: a collapsed window is all title bar.
        let bottom = match content_top {
            Some(top) => {
                self.title_height = top - margin.top - window.top() + margin.bottom;
                top - margin.top
            }
            None => {
                // Collapsed: the window is all title bar, so this is the height to give it.
                self.title_height = window.height();
                window.bottom()
            }
        };
        let height = bottom - window.top();
        let strip = Rect::from_min_max(
            egui::pos2(window.left() + height, window.top()),
            egui::pos2(window.right() - height - margin.right, bottom),
        );
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
        if response.double_clicked() {
            set_collapsed(ctx, id, !collapsed(ctx, id));
        } else if response.clicked() {
            self.activate(id);
        }
        if response.drag_started() && self.panes.len() > 1 {
            self.dragging = Some(id);
        }
    }

    /// While a pane is dragged: show where it would go, and put it there on
    /// release. Over another pane it joins that pane's row or column on the
    /// nearest side, as in a tiling window manager; held still over one for
    /// `SWAP_AFTER`, it swaps places with it instead (ADR-0008).
    fn drag_to_move(&mut self, ctx: &egui::Context, tiles: &[(PaneId, Rect)]) {
        let Some(from) = self.dragging else { return };
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let (pos, released, now) = ctx.input(|i| (i.pointer.interact_pos(), !i.pointer.primary_down(), i.time));
        let target = pos.and_then(|p| tiles.iter().find(|(id, rect)| *id != from && rect.contains(p)));
        // How long the pointer has been over this target.
        self.hovering = match (target, self.hovering) {
            (Some((id, _)), Some((was, since))) if *id == was => Some((was, since)),
            (Some((id, _)), _) => Some((*id, now)),
            (None, _) => None,
        };
        let swap = self.hovering.is_some_and(|(_, since)| now - since >= SWAP_AFTER);
        if let Some((_, since)) = self.hovering
            && !swap
        {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(SWAP_AFTER - (now - since)));
        }

        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("drop-preview")));
        let style = ctx.global_style();
        let accent = style.visuals.selection.bg_fill;
        if let Some((_, rect)) = tiles.iter().find(|(id, _)| *id == from) {
            painter.rect_stroke(*rect, 4.0, egui::Stroke::new(1.5, accent), egui::StrokeKind::Inside);
        }
        let drop = target.zip(pos).map(|((id, rect), p)| (*id, *rect, Side::nearest(*rect, p)));
        if let Some((_, rect, side)) = drop {
            let (area, label) = if swap { (rect, "Swap") } else { (side.half(rect), "Move here") };
            painter.rect_filled(area, 4.0, accent.gamma_multiply(0.35));
            painter.rect_stroke(area, 4.0, egui::Stroke::new(2.0, accent), egui::StrokeKind::Inside);
            painter.text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::TextStyle::Heading.resolve(&style),
                style.visuals.strong_text_color(),
            );
        }
        if released {
            match drop {
                Some((to, _, _)) if swap => self.tree.swap(from, to),
                Some((to, _, side)) => {
                    self.tree.move_pane(from, to, side);
                }
                None => {}
            }
            self.dragging = None;
            self.hovering = None;
            self.activate(from);
        }
    }

    /// The handle in the gap between two sides of a split, for resizing it.
    fn divider(&mut self, ctx: &egui::Context, divider: &Divider) {
        let id = Id::new(("divider", &divider.path, divider.index));
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
                self.tree.set_ratio(&divider.path, divider.index, ratio);
            }
        });
    }

    /// Every pane in its tile, the dividers between them, and dragging to swap.
    fn show_tiles(&mut self, ctx: &egui::Context, desktop: Rect, palette: &Palette, actions: &mut Vec<Action>) {
        let folded = |id| collapsed(ctx, id);
        let (tiles, dividers) = self.tree.layout(desktop, GAP, &folded, self.title_height);
        self.tiles = tiles.clone();
        // The keyboard goes to a pane that's open.
        if folded(self.active)
            && let Some(open) = self.tree.panes().into_iter().find(|id| !folded(*id))
        {
            self.activate(open);
        }
        for (id, rect) in &tiles {
            if let Some((strip, layer)) = self.show_pane(ctx, *id, *rect, palette, actions) {
                self.drag_handle(ctx, *id, strip, layer);
            }
        }
        for divider in &dividers {
            self.divider(ctx, divider);
        }
        self.drag_to_move(ctx, &tiles);
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
            .show(ui, |ui| {
                // The background between and below the tiles: right-click for a new pane.
                let rect = ui.max_rect();
                let response = ui.interact(rect, Id::new("desktop"), egui::Sense::click());
                if response.secondary_clicked() {
                    self.menu_at = response.interact_pointer_pos();
                }
                response.context_menu(|ui| {
                    if ui.button("New Terminal").clicked() {
                        if let Some(at) = self.menu_at {
                            actions.push(Action::NewAt(at));
                        }
                        ui.close();
                    }
                });
                rect
            })
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
                Action::NewAt(at) => self.new_at(&ctx, at),
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

/// Whether a pane's window is collapsed, as its arrow left it: egui keeps
/// that state, and the layout follows it.
fn collapsed(ctx: &egui::Context, id: PaneId) -> bool {
    egui::collapsing_header::CollapsingState::load(ctx, pane_id(id).with("collapsing")).is_some_and(|s| !s.is_open())
}

fn set_collapsed(ctx: &egui::Context, id: PaneId, folded: bool) {
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ctx, pane_id(id).with("collapsing"), true);
    state.set_open(!folded);
    state.store(ctx);
}

fn pane_id(id: PaneId) -> Id {
    Id::new(("pane", id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, RawInput, pos2, vec2};

    /// An app with panes 1 | 2 side by side and no shells behind them.
    fn two_panes() -> App {
        panes_split(Dir::Right)
    }

    fn panes_split(dir: Dir) -> App {
        let (skins, _) = skin::load_all(skin::BUILT_IN, &[]);
        let mut tree = Node::Leaf(1);
        tree.split(1, dir, 2);
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
            hovering: None,
            tiles: Vec::new(),
            menu_at: None,
            title_height: 34.0,
            command: None,
            requests: Requests::default(),
            screenshot: None,
        }
    }

    const DESKTOP: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 800.0, y: 500.0 } };

    thread_local! {
        static TIME: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    }

    /// One frame, 50 ms after the last.
    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<Event>) {
        let time = TIME.with(|t| {
            t.set(t.get() + 0.05);
            t.get()
        });
        let input = RawInput { screen_rect: Some(DESKTOP), events, time: Some(time), ..Default::default() };
        let _ = ctx.run_ui(input, |ui| {
            let palette = Palette::new(&app.skins[0], ui.visuals()).0;
            let mut actions = Vec::new();
            app.show_tiles(ui.ctx(), DESKTOP, &palette, &mut actions);
        });
    }

    fn click(ctx: &egui::Context, app: &mut App, at: Pos2) {
        frame(ctx, app, vec![Event::PointerMoved(at)]);
        frame(ctx, app, vec![button(at, true)]);
        frame(ctx, app, vec![button(at, false)]);
    }

    /// Where each pane's window is, as egui last placed it.
    fn window_rect(ctx: &egui::Context, id: PaneId) -> Rect {
        ctx.memory(|m| m.area_rect(pane_id(id))).unwrap()
    }

    #[test]
    fn the_arrow_collapses_a_pane_and_its_neighbour_takes_the_space() {
        let ctx = egui::Context::default();
        let mut app = panes_split(Dir::Down);
        frame(&ctx, &mut app, vec![]);
        let before = window_rect(&ctx, 2);
        // The collapse arrow sits at the left end of pane 1's title bar.
        let title = window_rect(&ctx, 1);
        click(&ctx, &mut app, pos2(title.left() + 18.0, title.top() + 17.0));
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]);
        }
        assert!(collapsed(&ctx, 1));
        let (folded, open) = (window_rect(&ctx, 1), window_rect(&ctx, 2));
        assert!(folded.height() < 60.0, "{folded:?}");
        assert!(folded.bottom() <= open.top(), "no overlap: {folded:?} {open:?}");
        // Test panes have no shell, so their windows are only as tall as their error
        // message; the open one moving up to the folded one's title bar shows the space moved.
        assert!(open.top() < before.top() - 150.0, "{open:?} was {before:?}");
        assert_eq!(app.active, 2, "the keyboard moves to the open pane");
    }

    #[test]
    fn a_new_pane_goes_below_the_pane_above_the_click() {
        let ctx = egui::Context::default();
        // Panes 1 | 2 side by side, both folded: empty space below each title bar.
        let mut app = two_panes();
        set_collapsed(&ctx, 1, true);
        set_collapsed(&ctx, 2, true);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.pane_above(pos2(100.0, 300.0)), 1);
        assert_eq!(app.pane_above(pos2(700.0, 300.0)), 2);
        app.active = 2;
        assert_eq!(app.pane_above(pos2(100.0, 1.0)), 2, "nothing above: the active pane");
    }

    #[test]
    fn double_clicking_a_title_bar_folds_and_unfolds() {
        let ctx = egui::Context::default();
        let mut app = panes_split(Dir::Down);
        frame(&ctx, &mut app, vec![]);
        let title = window_rect(&ctx, 1);
        let at = pos2(title.center().x, title.top() + 15.0);
        click(&ctx, &mut app, at);
        click(&ctx, &mut app, at);
        assert!(collapsed(&ctx, 1));
        for _ in 0..20 {
            frame(&ctx, &mut app, vec![]); // a second apart, so it's a new double-click
        }
        click(&ctx, &mut app, at);
        click(&ctx, &mut app, at);
        assert!(!collapsed(&ctx, 1));
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// Drags from `from` to `to` in small steps, as a mouse would.
    fn drag(ctx: &egui::Context, app: &mut App, from: Pos2, to: Pos2) {
        drag_and_hold(ctx, app, from, to, 0);
    }

    /// Drags, then holds still at `to` for `hold` frames (50 ms each) before letting go.
    fn drag_and_hold(ctx: &egui::Context, app: &mut App, from: Pos2, to: Pos2, hold: usize) {
        frame(ctx, app, vec![Event::PointerMoved(from)]);
        frame(ctx, app, vec![button(from, true)]);
        for step in 1..=10 {
            let p = from + (to - from) * (step as f32 / 10.0);
            frame(ctx, app, vec![Event::PointerMoved(p)]);
        }
        for _ in 0..hold {
            frame(ctx, app, vec![]);
        }
        frame(ctx, app, vec![button(to, false)]);
        frame(ctx, app, vec![]);
    }

    #[test]
    fn dropping_a_pane_on_another_moves_it_to_that_side() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        // Pane 1's title bar, then near the bottom of pane 2.
        drag(&ctx, &mut app, pos2(100.0, 15.0), pos2(600.0, 480.0));
        assert_eq!(app.tree, Node::Split { dir: Dir::Down, children: vec![(Node::Leaf(2), 1.0), (Node::Leaf(1), 1.0)] });
        assert_eq!(app.dragging, None);
        assert_eq!(app.active, 1, "the dragged pane stays active");
    }

    #[test]
    fn holding_still_over_a_pane_swaps_instead() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        drag_and_hold(&ctx, &mut app, pos2(100.0, 15.0), pos2(600.0, 480.0), 15);
        assert_eq!(app.tree.panes(), [2, 1]);
        let Node::Split { dir, .. } = &app.tree else { panic!() };
        assert_eq!(*dir, Dir::Right, "same shape, places traded");
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
        let gap = app.tree.layout(DESKTOP, GAP, &|_| false, 34.0).1[0].rect.center();
        drag(&ctx, &mut app, gap, gap + vec2(-200.0, 0.0));
        let tiles = app.tree.layout(DESKTOP, GAP, &|_| false, 34.0).0;
        assert!((tiles[0].1.width() - 200.0).abs() < 1.0, "{:?}", tiles[0].1);
    }
}
