//! The window: Patina's frame and title bar around the tiled panes, each a
//! terminal session in a pinned egui window (ADR-0005). Tabs in the title
//! bar each hold a layout of panes (ADR-0012).

use std::collections::HashMap;
use std::path::PathBuf;

use alacritty_terminal::event::WindowSize;
use egui::{Id, Rect};
use patina::frame::{self, FrameStyle, TitleBar};
use patina::skin::{self, Skin};
use patina::tabs::{self, TabAction, TabStrip};

use crate::config::Config;
use crate::control::{self, Request};
use crate::palette::Palette;
use crate::session::Session;
use crate::tiles::{Dir, Divider, Node, PaneId, Side};
use crate::view::{self, Requests, ViewState};

/// The skin used until the user picks one: stock egui, following the desktop.
const HOME_SKIN: &str = "Default";
/// The app's name for its config folder, and so its skins folder.
const APP: &str = "teleprinter";

const MIN_FONT: f32 = 8.0;
const MAX_FONT: f32 = 32.0;

/// The gap between tiles, and around them, that View > Gaps Between Panes
/// turns on; `gap` in the config sets any other (none by default).
const GAP_ON: f32 = 4.0;
const MAX_GAP: f32 = 32.0;
const MAX_PANE_TITLE: f32 = 64.0;

/// How wide a divider's handle is at least, over the panes' edges when the
/// gap is narrower.
const GRAB: f32 = 8.0;

/// Seconds a dragged pane must stay over another before dropping swaps them.
const SWAP_AFTER: f64 = 0.6;

/// What the menus ask for, handled after they're drawn.
enum Action {
    Quit,
    SetSkin(String),
    SetLightDark(String),
    ReloadSkins,
    CopySkins,
    OpenSkinsFolder,
    SystemTitleBar(bool),
    FontSize(f32),
    Gap(f32),
    Copy,
    Paste,
    /// Split the active pane.
    Split(Dir),
    Close(PaneId),
    /// A new pane in the empty space at this point (the background's menu).
    NewAt(egui::Pos2),
    NewTab,
    ShowTab(usize),
    CloseTab(usize),
}

/// A tab: a layout of panes of its own, shown in place of the others.
struct Tab {
    tree: Node,
    /// The pane that has the keyboard while the tab is shown.
    active: PaneId,
}

impl Tab {
    fn new(pane: PaneId) -> Self {
        Self { tree: Node::Leaf(pane), active: pane }
    }
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
    /// The latest message for the status bar.
    status: Option<Status>,
    /// Every tab's panes.
    panes: HashMap<PaneId, Pane>,
    /// Never empty: closing the last tab closes the window.
    tabs: Vec<Tab>,
    /// The tab shown; its active pane has the keyboard, and splits and menu actions act on it.
    tab: usize,
    next_id: PaneId,
    /// The pane whose title bar is being dragged onto another, to swap them.
    dragging: Option<PaneId>,
    /// The pane the dragged one is over, and since when (egui time), for swapping.
    hovering: Option<(PaneId, f64)>,
    /// How tall a collapsed pane is: its title bar, measured from the panes shown.
    title_height: f32,
    /// Where each pane of the shown tab was last laid out, for placing a new one in empty space.
    tiles: Vec<(PaneId, Rect)>,
    /// Where the background was right-clicked, for its menu.
    menu_at: Option<egui::Pos2>,
    /// `TELEPRINTER_COMMAND`, run in every pane instead of the login shell.
    command: Option<(String, Vec<String>)>,
    requests: Requests,
    /// `TELEPRINTER_SCREENSHOT`: where to save one, and whether it was asked for.
    screenshot: Option<(PathBuf, bool)>,
    /// Where programs in the panes ask for panes (ADR-0009); `None` if it couldn't start.
    control: Option<control::Server>,
    /// The window title last sent: sending one requests a repaint, so only on a change.
    window_title: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, mut config: Config) -> Self {
        patina::appearance::watch(&cc.egui_ctx);
        crate::fonts::install(&cc.egui_ctx);
        // Disable egui's default Ctrl+Q (Cmd+Q on Mac) quit shortcut
        cc.egui_ctx.options_mut(|o| o.quit_shortcuts.clear());
        let (skins, skin_errors) = load_skins();
        if let Ok(name) = std::env::var("TELEPRINTER_SKIN") {
            config.skin = Some(name); // for this run only: never saved
        }
        // `TELEPRINTER_COMMAND` runs that through `sh -c` instead of the login shell
        // (for screenshots and tests).
        let command = std::env::var("TELEPRINTER_COMMAND").ok().map(|c| ("/bin/sh".to_string(), vec!["-c".into(), c]));
        let control = control::Server::start(&cc.egui_ctx)
            .inspect_err(|e| eprintln!("teleprinter: no control socket: {e}"))
            .ok();
        let mut app = Self {
            custom_frame: !config.system_title_bar,
            config,
            skins,
            skin_errors,
            status: None,
            panes: HashMap::new(),
            tabs: Vec::new(),
            tab: 0,
            next_id: 1,
            dragging: None,
            hovering: None,
            tiles: Vec::new(),
            menu_at: None,
            title_height: 34.0,
            command,
            requests: Requests { focus: true, ..Default::default() },
            screenshot: std::env::var_os("TELEPRINTER_SCREENSHOT").map(|p| (PathBuf::from(p), false)),
            control,
            window_title: String::new(),
        };
        let first = app.spawn(&cc.egui_ctx, None, dirs::home_dir(), HashMap::new());
        app.tabs.push(Tab::new(first));
        // `TELEPRINTER_SPLITS=right,down` splits the active pane in turn at start
        // (for screenshots).
        for split in std::env::var("TELEPRINTER_SPLITS").unwrap_or_default().split(',') {
            match split.trim() {
                "right" => app.split(&cc.egui_ctx, Dir::Right),
                "down" => app.split(&cc.egui_ctx, Dir::Down),
                _ => {}
            }
        }
        // `TELEPRINTER_TABS=3` opens that many tabs, showing the first (for screenshots).
        let tabs = std::env::var("TELEPRINTER_TABS").ok().and_then(|n| n.trim().parse().ok()).unwrap_or(1);
        for _ in 1..tabs {
            app.new_tab(&cc.egui_ctx);
        }
        app.show_tab(0);
        // `TELEPRINTER_COLLAPSED=2,3` starts those panes collapsed (for screenshots).
        for id in std::env::var("TELEPRINTER_COLLAPSED").unwrap_or_default().split(',') {
            if let Ok(id) = id.trim().parse() {
                set_collapsed(&cc.egui_ctx, id, true);
            }
        }
        app.apply_skin(&cc.egui_ctx);
        app
    }

    /// Starts `command` (else `TELEPRINTER_COMMAND`, else the login shell) in
    /// a new pane, not yet placed in the tree, with `env` added to the
    /// window's environment. It's told its pane and the control socket, to
    /// ask for more.
    fn spawn(&mut self, ctx: &egui::Context, command: Option<(String, Vec<String>)>, cwd: Option<PathBuf>, mut env: HashMap<String, String>) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        // A first guess at the size; the widget resizes it on its first frame.
        let size = WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 };
        env.insert("TELEPRINTER_PANE".into(), id.to_string());
        if let Some(control) = &self.control {
            env.insert("TELEPRINTER_SOCKET".into(), control.path.display().to_string());
        }
        let command = command.or_else(|| self.command.clone());
        let session = Session::spawn(ctx, command, cwd, env, size).map_err(|e| e.to_string());
        self.panes.insert(id, Pane { session, view: ViewState::default() });
        id
    }

    /// A pane's shell's directory, else the home directory.
    fn cwd(&self, id: PaneId) -> Option<PathBuf> {
        self.panes.get(&id).and_then(|p| p.session.as_ref().ok()).and_then(Session::cwd).or_else(dirs::home_dir)
    }

    /// Splits the active pane; the new one starts in the same directory and
    /// takes the keyboard.
    fn split(&mut self, ctx: &egui::Context, dir: Dir) {
        let active = self.active();
        let id = self.spawn(ctx, None, self.cwd(active), HashMap::new());
        self.shown_mut().tree.split(active, dir, id);
        self.activate(id);
    }

    /// A tab with one pane, in the active pane's directory, shown.
    fn new_tab(&mut self, ctx: &egui::Context) {
        let id = self.spawn(ctx, None, self.cwd(self.active()), HashMap::new());
        self.tabs.push(Tab::new(id));
        self.show_tab(self.tabs.len() - 1);
    }

    fn show_tab(&mut self, tab: usize) {
        self.tab = tab.min(self.tabs.len() - 1);
        self.dragging = None;
        self.hovering = None;
        self.requests.focus = true;
    }

    /// Closes every pane in a tab, and so the tab.
    fn close_tab(&mut self, ctx: &egui::Context, tab: usize) {
        for id in self.tabs.get(tab).map(|t| t.tree.panes()).unwrap_or_default() {
            self.close(ctx, id);
        }
    }

    fn shown(&self) -> &Tab {
        &self.tabs[self.tab]
    }

    fn shown_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.tab]
    }

    /// The pane with the keyboard.
    fn active(&self) -> PaneId {
        self.shown().active
    }

    /// The tab a pane is in.
    fn tab_of(&self, id: PaneId) -> Option<usize> {
        self.tabs.iter().position(|t| t.tree.panes().contains(&id))
    }

    /// A new pane in the empty space at `at`: below the pane above it in that
    /// column (the space folded panes leave), or below the active pane.
    fn new_at(&mut self, ctx: &egui::Context, at: egui::Pos2) {
        let target = self.pane_above(at);
        let id = self.spawn(ctx, None, self.cwd(target), HashMap::new());
        self.shown_mut().tree.insert(target, Side::Bottom, id);
        self.activate(id);
    }

    /// Answers what programs sent to the control socket since the last frame.
    fn answer_calls(&mut self, ctx: &egui::Context) {
        let Some(control) = &self.control else { return };
        let calls: Vec<_> = control.calls.try_iter().collect();
        for (request, reply) in calls {
            let _ = reply.send(self.answer(ctx, request));
        }
    }

    fn answer(&mut self, ctx: &egui::Context, request: Request) -> serde_json::Value {
        let known = |app: &Self, id| app.tab_of(id).is_some();
        match request {
            Request::Split { pane, side, command, cwd, focus, env } => {
                let target = pane.unwrap_or(self.active());
                let Some(tab) = self.tab_of(target) else {
                    return control::error(format!("no pane {target}"));
                };
                let cwd = cwd.or_else(|| self.cwd(target));
                let command = command.split_first().map(|(program, args)| (program.clone(), args.to_vec()));
                let id = self.spawn(ctx, command, cwd, env);
                if let Some(Err(e)) = self.panes.get(&id).map(|p| &p.session) {
                    let e = e.clone();
                    self.panes.remove(&id);
                    return control::error(format!("couldn't start it: {e}"));
                }
                // In the target's tab, shown or not.
                self.tabs[tab].tree.split_side(target, side.into(), id);
                if focus {
                    self.activate(id);
                }
                control::ok(serde_json::json!({ "pane": id }))
            }
            Request::List => {
                let panes: Vec<_> = (self.tabs.iter().enumerate())
                    .flat_map(|(t, tab)| tab.tree.panes().into_iter().map(move |id| (t, id)))
                    .map(|(t, id)| {
                        let session = self.panes.get(&id).and_then(|p| p.session.as_ref().ok());
                        // Only the shown tab's panes are laid out.
                        let rect = (t == self.tab)
                            .then(|| self.tiles.iter().find(|(p, _)| *p == id))
                            .flatten()
                            .map(|(_, r)| [r.left(), r.top(), r.width(), r.height()]);
                        serde_json::json!({
                            "pane": id,
                            "tab": t + 1,
                            "title": self.title(id),
                            "cwd": session.and_then(Session::cwd),
                            "rect": rect,
                            "collapsed": collapsed(ctx, id),
                            "active": t == self.tab && id == self.active(),
                        })
                    })
                    .collect();
                control::ok(serde_json::json!({ "panes": panes }))
            }
            Request::Close { pane } if known(self, pane) => {
                self.close(ctx, pane);
                control::ok(serde_json::json!({}))
            }
            Request::Focus { pane } if known(self, pane) => {
                set_collapsed(ctx, pane, false);
                self.activate(pane);
                control::ok(serde_json::json!({}))
            }
            Request::Close { pane } | Request::Focus { pane } => control::error(format!("no pane {pane}")),
        }
    }

    /// The pane nearest above `at` in its column, else the active one.
    fn pane_above(&self, at: egui::Pos2) -> PaneId {
        self.tiles
            .iter()
            .filter(|(_, r)| r.x_range().contains(at.x) && r.bottom() <= at.y)
            .max_by(|a, b| a.1.bottom().total_cmp(&b.1.bottom()))
            .map_or(self.active(), |(id, _)| *id)
    }

    /// Closes a pane; closing a tab's last one closes the tab, and the last
    /// tab's the window.
    fn close(&mut self, ctx: &egui::Context, id: PaneId) {
        let Some(t) = self.tab_of(id) else { return };
        let tab = &mut self.tabs[t];
        if tab.tree.remove(id) {
            if tab.active == id {
                tab.active = tab.tree.panes()[0];
            }
        } else if self.tabs.len() == 1 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        } else {
            // The next tab takes its place, or the one before it at the end.
            self.tabs.remove(t);
            self.show_tab(if t < self.tab { self.tab - 1 } else { self.tab });
        }
        self.panes.remove(&id);
        self.requests.focus = true;
    }

    /// Gives a pane the keyboard, showing its tab.
    fn activate(&mut self, id: PaneId) {
        if let Some(t) = self.tab_of(id) {
            self.tab = t;
            self.tabs[t].active = id;
        }
        self.requests.focus = true;
    }

    /// The chosen skin, falling back to `HOME_SKIN` (a saved name may no
    /// longer exist), then Default.
    fn skin(&self) -> &Skin {
        let find = |name: &str| self.skins.iter().find(|s| s.name == name);
        find(self.config.skin.as_deref().unwrap_or(HOME_SKIN)).or_else(|| find(HOME_SKIN)).unwrap_or(&self.skins[0])
    }

    fn apply_skin(&self, ctx: &egui::Context) {
        let skin = self.skin();
        skin.apply(ctx);
        if skin.follows_desktop() {
            ctx.set_theme(self.config.theme());
        }
    }

    /// The space between panes and around them, as the config sets it.
    fn gap(&self) -> f32 {
        self.config.gap.clamp(0.0, MAX_GAP)
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
    fn menus(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
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
                actions.push(Action::Close(self.active()));
                ui.close();
            }
            ui.separator();
            if ui.add(egui::Button::new("New Tab").shortcut_text("Ctrl+Shift+T")).clicked() {
                actions.push(Action::NewTab);
                ui.close();
            }
            if ui.button("Close Tab").clicked() {
                actions.push(Action::CloseTab(self.tab));
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
            let mut gaps = self.gap() > 0.0;
            if ui.checkbox(&mut gaps, "Gaps Between Panes").changed() {
                actions.push(Action::Gap(if gaps { GAP_ON } else { 0.0 }));
            }
            let mut system = !self.custom_frame;
            if ui.checkbox(&mut system, "System Title Bar").changed() {
                actions.push(Action::SystemTitleBar(system));
            }
        });
        // Patina's skin menu (its ADR-0017): only controls; what they did goes to the status bar.
        ui.menu_button("Skins", |ui| {
            // Size the menu to its widest entry instead of wrapping entries to its first width.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let current = self.skin();
            for skin in &self.skins {
                let label = match &skin.author {
                    Some(author) if skin.path.is_some() => format!("{} — {author}", skin.name),
                    _ => skin.name.clone(),
                };
                let resp = ui.radio(skin.name == current.name, label);
                let resp = match &skin.path {
                    Some(path) => resp.on_hover_text(path.display().to_string()),
                    None => resp,
                };
                if resp.clicked() {
                    actions.push(Action::SetSkin(skin.name.clone()));
                }
            }
            ui.separator();
            ui.add_enabled_ui(current.follows_desktop(), |ui| {
                for (value, label) in [("system", "Follow desktop"), ("light", "Light"), ("dark", "Dark")] {
                    // An unset or unknown setting follows the desktop.
                    let on = match value {
                        "system" => !matches!(self.config.light_dark.as_str(), "light" | "dark"),
                        _ => self.config.light_dark == value,
                    };
                    if ui.radio(on, label).clicked() {
                        actions.push(Action::SetLightDark(value.into()));
                    }
                }
            });
            ui.separator();
            if ui.button("Reload skins").clicked() {
                actions.push(Action::ReloadSkins);
            }
            if ui
                .button("Copy built-in skins to folder")
                .on_hover_text("Puts editable copies in the skins folder; never overwrites existing files")
                .clicked()
            {
                actions.push(Action::CopySkins);
            }
            let folder = skin::app_dir(APP).map(|d| d.display().to_string()).unwrap_or_default();
            if ui.button("Open skins folder").on_hover_text(folder).clicked() {
                actions.push(Action::OpenSkinsFolder);
            }
        });
    }

    /// Rescans the built-in and user skins; the config keeps the one in use by name.
    fn reload_skins(&mut self, ctx: &egui::Context) {
        (self.skins, self.skin_errors) = load_skins();
        self.apply_skin(ctx);
    }

    /// The version on the left, then the latest message, coloured by its level. Skins that failed
    /// to load, and colours the skin gets wrong (`problems`, which have fallbacks, so they warn),
    /// stay there, ahead of newer messages, until they're fixed.
    fn status_bar(&self, ui: &mut egui::Ui, problems: &[String]) {
        ui.horizontal(|ui| {
            // Just the version: the app's name is already in the title bar and the window list,
            // so repeating it here adds nothing.
            ui.label(egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).weak().small());
            let plural = |n: usize| if n == 1 { "" } else { "s" };
            let lasting = if !self.skin_errors.is_empty() {
                let n = self.skin_errors.len();
                Some((Status::error(format!("Couldn't load {n} skin{}", plural(n))), self.skin_errors.join("\n")))
            } else if !problems.is_empty() {
                let n = problems.len();
                Some((Status::warn(format!("{n} problem{} in this skin's colours", plural(n))), problems.join("\n")))
            } else {
                None
            };
            let shown = lasting.as_ref().map(|(s, _)| s).or(self.status.as_ref());
            if let Some(status) = shown {
                ui.separator();
                let color = match status.level {
                    Level::Error => ui.visuals().error_fg_color,
                    Level::Warn => ui.visuals().warn_fg_color,
                    Level::Info => ui.visuals().weak_text_color(),
                };
                let resp = ui.label(egui::RichText::new(&status.text).color(color).small());
                if let Some((_, details)) = &lasting {
                    resp.on_hover_text(details);
                }
            }
        });
    }

    /// The tabs, after the menus, each named for its active pane, then +
    /// for a new one, spanning the bar's height (`span`, padding and all).
    fn tab_strip(&self, ui: &mut egui::Ui, span: egui::Rangef, style: &FrameStyle, actions: &mut Vec<Action>) {
        let titles: Vec<_> = self.tabs.iter().map(|tab| self.title(tab.active)).collect();
        if let Some(action) = TabStrip::new(&titles, self.tab, style).span(span).show(ui) {
            actions.push(action.into());
        }
    }

    /// One pane: its session in an egui window pinned to `rect`, the same
    /// window whichever tile it's in. Returns the window's title strip, for
    /// dragging.
    fn show_pane(
        &mut self,
        ctx: &egui::Context,
        id: PaneId,
        rect: Rect,
        corners: egui::CornerRadius,
        palette: &Palette,
        actions: &mut Vec<Action>,
    ) -> Option<(Rect, egui::LayerId)> {
        let title = self.title(id);
        let active = id == self.active();
        let style = ctx.global_style();
        let several = self.shown().tree.panes().len() > 1;
        // The active pane's border is the selection colour, once there's more than one.
        let stroke = if active && several {
            egui::Stroke::new(1.5, style.visuals.selection.bg_fill)
        } else {
            style.visuals.window_stroke
        };
        let mut frame = egui::Frame::window(&style).shadow(egui::Shadow::NONE).stroke(stroke).fill(palette.background).corner_radius(corners);
        // egui's title bar is a heading's row, which its buttons are sized to, inside the frame's
        // margin; the margin above and below makes up the configured height. The content gets
        // the same margin, as egui gives it.
        let row = ctx.fonts_mut(|f| f.row_height(&egui::TextStyle::Heading.resolve(&style)));
        let spare = (self.config.pane_title_height.min(MAX_PANE_TITLE) - row).max(0.0);
        frame.inner_margin.top = (spare / 2.0).floor() as i8;
        frame.inner_margin.bottom = (spare - (spare / 2.0).floor()).round() as i8;
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
        // In the body size, as the rest of the bar's and menus' text is, not egui's heading size;
        // the title bar keeps the heading's height, which egui sizes its buttons by.
        let text_style = egui::TextStyle::Body;
        let label = egui::RichText::new(title.clone()).text_style(text_style.clone());
        let title: egui::Atoms<'static> = if collapsed(ctx, id) {
            use egui::AtomExt;
            let button = ctx.fonts_mut(|f| f.row_height(&egui::TextStyle::Heading.resolve(&style)));
            let gaps = 4.0 * style.spacing.item_spacing.x;
            let width = rect.width() - margin.sum().x - 2.0 * button - gaps;
            // Centred like an open window's title: equal spacers either side of the text.
            let font = text_style.resolve(&style);
            let text = ctx.fonts_mut(|f| f.layout_no_wrap(title, font, egui::Color32::WHITE).size().x);
            let side = ((width - text) / 2.0 - style.spacing.item_spacing.x).max(0.0);
            let spacer = |n: u8| egui::Atom::custom(Id::new(("title-spacer", id, n)), egui::vec2(side, 0.0));
            egui::Atoms::new((spacer(0), label.atom_size(egui::vec2(text, button)), spacer(1)))
        } else {
            egui::Atoms::new(label)
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
                    // Fill the tile anyway, as a terminal would.
                    ui.allocate_space(ui.available_size());
                    return false;
                };
                let response = view::show(ui, session, &mut pane.view, palette, font_size, requests, active);
                let mut copy = false;
                let mut paste = false;
                // A program tracking the mouse gets the right-click; Shift+right-click is the menu.
                // Only the opening click is held back, so a menu already open stays open.
                let program_click = response.secondary_clicked() && view::takes_right_click(ui, session);
                if !program_click {
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
                }
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
            self.shown_mut().active = id;
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

    /// A tile's title bar, as a handle: drag it onto another tile to swap them,
    /// right-click it for the split menu.
    fn drag_handle(&mut self, ctx: &egui::Context, id: PaneId, strip: Rect, layer: egui::LayerId, actions: &mut Vec<Action>) {
        // The Ui and the handle need ids of their own; sharing one, the handle never got dragged.
        let ui = egui::Ui::new(ctx.clone(), Id::new(("pane-handle-ui", id)), egui::UiBuilder::new().layer_id(layer).max_rect(strip));
        let response = ui.interact(strip, Id::new(("pane-handle", id)), egui::Sense::click_and_drag());
        let several = self.shown().tree.panes().len() > 1;
        if several {
            response.clone().on_hover_cursor(egui::CursorIcon::Grab);
        }
        if response.double_clicked() {
            set_collapsed(ctx, id, !collapsed(ctx, id));
        } else if response.clicked() || response.secondary_clicked() {
            self.activate(id);
        }
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
            if ui.button("Close").clicked() {
                actions.push(Action::Close(id));
                ui.close();
            }
        });
        if response.drag_started() && several {
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
                Some((to, _, _)) if swap => self.shown_mut().tree.swap(from, to),
                Some((to, _, side)) => {
                    self.shown_mut().tree.move_pane(from, to, side);
                }
                None => {}
            }
            self.dragging = None;
            self.hovering = None;
            self.activate(from);
        }
    }

    /// The handle in the gap between two sides of a split, for resizing it:
    /// at least `GRAB` wide, over the panes' edges (their frames' margins)
    /// where the gap is narrower, and above the panes for that.
    /// `spacing` is the layout's, negative where panes overlap.
    fn divider(&mut self, ctx: &egui::Context, divider: &Divider, spacing: f32) {
        let id = Id::new(("divider", &divider.path, divider.index));
        // Across the split, the divider is `spacing` wide: none, or less, where panes overlap.
        let (c, size) = (divider.rect.center(), divider.rect.size());
        let handle = match divider.dir {
            Dir::Right => Rect::from_center_size(c, egui::vec2(size.x.abs().max(GRAB), size.y)),
            Dir::Down => Rect::from_center_size(c, egui::vec2(size.x, size.y.abs().max(GRAB))),
        };
        let gap = spacing;
        egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(handle.min).constrain(false).show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(handle.size(), egui::Sense::drag());
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
                    Dir::Right => (pos.x - area.left()) / (area.width() - gap),
                    Dir::Down => (pos.y - area.top()) / (area.height() - gap),
                };
                self.shown_mut().tree.set_ratio(&divider.path, divider.index, ratio);
            }
        });
    }

    /// Every pane in its tile, the dividers between them, and dragging to swap.
    fn show_tiles(&mut self, ctx: &egui::Context, desktop: Rect, palette: &Palette, actions: &mut Vec<Action>) {
        let folded = |id| collapsed(ctx, id);
        let (area, spacing) = self.tile_area(ctx, desktop);
        let (tiles, dividers) = self.shown().tree.layout(area, spacing, &folded, self.title_height);
        self.tiles = tiles.clone();
        // The keyboard goes to a pane that's open.
        if folded(self.active())
            && let Some(open) = self.shown().tree.panes().into_iter().find(|id| !folded(*id))
        {
            self.activate(open);
        }
        // Touching, panes are square: the status bar below holds the window's rounded bottom
        // corners.
        let corners = if self.gap() > 0.0 {
            egui::Frame::window(&ctx.global_style()).corner_radius
        } else {
            egui::CornerRadius::ZERO
        };
        for (id, rect) in &tiles {
            if let Some((strip, layer)) = self.show_pane(ctx, *id, *rect, corners, palette, actions) {
                self.drag_handle(ctx, *id, strip, layer, actions);
            }
        }
        // Overlapping, the active pane is drawn last, so its accent border is whole.
        if self.gap() == 0.0 {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, pane_id(self.active())));
        }
        for divider in &dividers {
            self.divider(ctx, divider, spacing);
        }
        self.drag_to_move(ctx, &tiles);
    }

    /// Where the tiles go in `desktop`, and the spacing between them. With
    /// no gap, neighbours overlap by a border's width and the outer panes
    /// reach a border past the desktop, so every edge is one line: shared
    /// between two panes, or with the window's edge or the title bar's.
    fn tile_area(&self, ctx: &egui::Context, desktop: Rect) -> (Rect, f32) {
        let gap = self.gap();
        if gap > 0.0 {
            return (desktop, gap);
        }
        let border = ctx.global_style().visuals.window_stroke.width;
        (desktop.expand(border), -border)
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

        self.answer_calls(&ctx);
        let mut exited = Vec::new();
        let several = self.panes.len() > 1;
        for (id, pane) in &mut self.panes {
            if let Ok(session) = &mut pane.session {
                session.pump(&ctx, &palette);
                // Screenshots keep the last pane when its command ends, to have something to show.
                if session.exited && (self.screenshot.is_none() || several) {
                    exited.push(*id);
                }
            }
        }
        for id in exited {
            self.close(&ctx, id);
        }

        // For the taskbar: the title bar has the tabs instead.
        let title = match self.shown().tree.panes().len() {
            0 | 1 => self.title(self.active()),
            n => format!("{} ({n} panes)", self.title(self.active())),
        };
        if title != self.window_title {
            self.window_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        let mut actions = Vec::new();
        // Taken before the panes see them: these keys don't reach the shell.
        actions.extend(tabs::keys(&ctx, self.tabs.len(), self.tab).map(Action::from));
        actions.extend(tab_number(&ctx, self.tabs.len()).map(Action::ShowTab));
        let header = frame::header_frame(ui.style(), &style, radius);
        egui::Panel::top("header").frame(header).show(ui, |ui| {
            // The bar's full height, for the tabs to fill.
            let span = (ui.max_rect() + header.inner_margin).y_range();
            // No title: each pane has its own, and the space is the tabs'.
            TitleBar::new("", &style).custom(self.custom_frame).menu_bar(true).show(
                ui,
                |ui| {
                    self.menus(ui, &mut actions);
                    self.tab_strip(ui, span, &style, &mut actions);
                },
                // A terminal's output is the shell's; nothing goes on the right.
                |_ui| {},
            );
        });

        // The tiles sit on the window's own panel colour, in the gaps between them.
        // Patina's status bar (its ADR-0016), holding both bottom corners.
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::side_top_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(10, 4))
                    .corner_radius(egui::CornerRadius { sw: radius, se: radius, ..Default::default() }),
            )
            .show(ui, |ui| self.status_bar(ui, &problems));

        let desktop = egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(self.gap()))
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
                    self.apply_skin(&ctx);
                    self.save_config();
                }
                Action::SetLightDark(value) => {
                    self.config.light_dark = value;
                    self.apply_skin(&ctx);
                    self.save_config();
                }
                Action::ReloadSkins => self.reload_skins(&ctx),
                Action::CopySkins => {
                    let copied = skin::app_dir(APP)
                        .ok_or_else(|| "no config folder".to_string())
                        .and_then(|dir| skin::export_built_ins(&dir, skin::BUILT_IN));
                    self.status = Some(match copied {
                        Ok(export) => {
                            // The copies share the built-ins' names, so after a reload they're
                            // the versions in use.
                            self.reload_skins(&ctx);
                            Status::info(export_summary(&export.copied, &export.skipped))
                        }
                        Err(e) => Status::error(format!("Couldn't copy skins: {e}")),
                    });
                }
                Action::OpenSkinsFolder => {
                    if let Some(dir) = skin::app_dir(APP) {
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = patina::open::path(&dir);
                    }
                }
                Action::SystemTitleBar(on) => {
                    self.custom_frame = !on;
                    self.config.system_title_bar = on;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(on));
                    self.save_config();
                }
                Action::Gap(gap) => {
                    self.config.gap = gap;
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
                Action::NewTab => self.new_tab(&ctx),
                Action::ShowTab(tab) => self.show_tab(tab),
                Action::CloseTab(tab) => self.close_tab(&ctx, tab),
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

impl From<TabAction> for Action {
    fn from(action: TabAction) -> Self {
        match action {
            TabAction::New => Action::NewTab,
            TabAction::Show(tab) => Action::ShowTab(tab),
            TabAction::Close(tab) => Action::CloseTab(tab),
        }
    }
}

/// Alt+1 to Alt+9 show that tab, as in GNOME Terminal. Taken, with the digit
/// egui sends after it as text, only when there is such a tab, so readline's
/// Alt+digit still works past the last one.
fn tab_number(ctx: &egui::Context, count: usize) -> Option<usize> {
    const DIGITS: [egui::Key; 9] = {
        use egui::Key::*;
        [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9]
    };
    ctx.input_mut(|i| {
        let n = DIGITS.iter().take(count).position(|&key| i.consume_key(egui::Modifiers::ALT, key))?;
        let digit = (n + 1).to_string();
        i.events.retain(|e| !matches!(e, egui::Event::Text(t) if *t == digit));
        Some(n)
    })
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

/// Patina's built-in skins and Teleprinter's own folder.
fn load_skins() -> (Vec<Skin>, Vec<String>) {
    skin::load_all(skin::BUILT_IN, &skin::app_dir(APP).into_iter().collect::<Vec<_>>())
}

/// A message for the status bar.
struct Status {
    level: Level,
    text: String,
}

enum Level {
    Error,
    Warn,
    Info,
}

impl Status {
    fn error(text: impl Into<String>) -> Self {
        Status { level: Level::Error, text: text.into() }
    }

    fn warn(text: impl Into<String>) -> Self {
        Status { level: Level::Warn, text: text.into() }
    }

    fn info(text: impl Into<String>) -> Self {
        Status { level: Level::Info, text: text.into() }
    }
}

/// What Copy built-in skins did, for the status bar.
fn export_summary(copied: &[String], skipped: &[String]) -> String {
    let mut parts = Vec::new();
    if !copied.is_empty() {
        parts.push(format!("Copied {}", copied.join(", ")));
    }
    if !skipped.is_empty() {
        parts.push(format!("kept your existing {}", skipped.join(", ")));
    }
    parts.join("; ")
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
            status: None,
            panes: HashMap::from([(1, pane()), (2, pane())]),
            tabs: vec![Tab { tree, active: 1 }],
            tab: 0,
            next_id: 3,
            dragging: None,
            hovering: None,
            tiles: Vec::new(),
            menu_at: None,
            title_height: 34.0,
            command: None,
            requests: Requests::default(),
            screenshot: None,
            control: None,
            window_title: String::new(),
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
        assert_eq!(app.active(), 2, "the keyboard moves to the open pane");
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
        app.tabs[0].active = 2;
        assert_eq!(app.pane_above(pos2(100.0, 1.0)), 2, "nothing above: the active pane");
    }

    /// Runs frames until egui has settled the windows into their tiles.
    fn settle(ctx: &egui::Context, app: &mut App) {
        for _ in 0..5 {
            frame(ctx, app, vec![]);
        }
    }

    #[test]
    fn closing_a_pane_lets_its_neighbour_fill_the_space() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        settle(&ctx, &mut app);
        assert!(window_rect(&ctx, 1).width() < 450.0, "{:?}", window_rect(&ctx, 1));
        app.close(&ctx, 2);
        settle(&ctx, &mut app);
        assert!(window_rect(&ctx, 1).width() > 780.0, "{:?}", window_rect(&ctx, 1));
    }

    #[test]
    fn moving_a_pane_away_lets_the_rest_fill_its_column() {
        let ctx = egui::Context::default();
        // 1 | (2 over 3); move 3 below 1: each column has two, evenly.
        let mut app = two_panes();
        app.tabs[0].tree.split(2, Dir::Down, 3);
        app.panes.insert(3, Pane { session: Err("no shell in tests".into()), view: ViewState::default() });
        settle(&ctx, &mut app);
        assert!(window_rect(&ctx, 1).height() > 480.0);
        app.tabs[0].tree.move_pane(3, 1, Side::Bottom);
        settle(&ctx, &mut app);
        assert!(window_rect(&ctx, 2).height() > 480.0, "2 fills the right: {:?}", window_rect(&ctx, 2));
        assert!(window_rect(&ctx, 1).height() < 260.0, "1 shares the left: {:?}", window_rect(&ctx, 1));
    }

    #[test]
    fn unfolding_a_pane_takes_its_space_back() {
        let ctx = egui::Context::default();
        let mut app = panes_split(Dir::Down);
        settle(&ctx, &mut app);
        set_collapsed(&ctx, 1, true);
        settle(&ctx, &mut app);
        assert!(window_rect(&ctx, 2).height() > 400.0, "{:?}", window_rect(&ctx, 2));
        set_collapsed(&ctx, 1, false);
        settle(&ctx, &mut app);
        assert!(window_rect(&ctx, 2).height() < 260.0, "{:?}", window_rect(&ctx, 2));
        assert!(window_rect(&ctx, 1).height() > 200.0, "{:?}", window_rect(&ctx, 1));
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
        assert_eq!(app.tabs[0].tree, Node::Split { dir: Dir::Down, children: vec![(Node::Leaf(2), 1.0), (Node::Leaf(1), 1.0)] });
        assert_eq!(app.dragging, None);
        assert_eq!(app.active(), 1, "the dragged pane stays active");
    }

    #[test]
    fn holding_still_over_a_pane_swaps_instead() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        drag_and_hold(&ctx, &mut app, pos2(100.0, 15.0), pos2(600.0, 480.0), 15);
        assert_eq!(app.tabs[0].tree.panes(), [2, 1]);
        let Node::Split { dir, .. } = &app.tabs[0].tree else { panic!() };
        assert_eq!(*dir, Dir::Right, "same shape, places traded");
    }

    #[test]
    fn dropping_a_tile_on_itself_changes_nothing() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        drag(&ctx, &mut app, pos2(100.0, 15.0), pos2(200.0, 250.0));
        assert_eq!(app.tabs[0].tree.panes(), [1, 2]);
    }

    #[test]
    fn dragging_the_divider_resizes_the_split() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        let (area, spacing) = app.tile_area(&ctx, DESKTOP);
        let gap = app.tabs[0].tree.layout(area, spacing, &|_| false, 34.0).1[0].rect.center();
        drag(&ctx, &mut app, gap, gap + vec2(-200.0, 0.0));
        let tiles = app.tabs[0].tree.layout(area, spacing, &|_| false, 34.0).0;
        assert!((tiles[0].1.width() - 200.0).abs() < 1.0, "{:?}", tiles[0].1);
    }

    #[test]
    fn right_clicking_a_title_bar_activates_the_pane_and_opens_its_menu() {
        let ctx = egui::Context::default();
        let mut app = two_panes();
        frame(&ctx, &mut app, vec![]);
        let title = window_rect(&ctx, 2);
        let at = pos2(title.center().x, title.top() + 15.0);
        frame(&ctx, &mut app, vec![Event::PointerMoved(at)]);
        let right = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Default::default() };
        frame(&ctx, &mut app, vec![right(true)]);
        frame(&ctx, &mut app, vec![right(false)]);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.active(), 2);
        assert!(egui::Popup::is_any_open(&ctx), "the split menu is open");
    }

    /// Tabs: panes 1 | 2, and pane 3 alone, shown.
    fn two_tabs() -> App {
        let mut app = two_panes();
        app.panes.insert(3, Pane { session: Err("no shell in tests".into()), view: ViewState::default() });
        app.tabs.push(Tab::new(3));
        app.tab = 1;
        app
    }

    #[test]
    fn only_the_shown_tab_is_laid_out() {
        let ctx = egui::Context::default();
        let mut app = two_tabs();
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.tiles.iter().map(|t| t.0).collect::<Vec<_>>(), [3]);
        app.show_tab(0);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.tiles.iter().map(|t| t.0).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn closing_a_tabs_last_pane_closes_the_tab() {
        let ctx = egui::Context::default();
        let mut app = two_tabs();
        app.close(&ctx, 3);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!((app.tab, app.active()), (0, 1), "the tab before it is shown");
        assert!(!app.panes.contains_key(&3));
    }

    #[test]
    fn closing_a_pane_in_a_hidden_tab_leaves_the_shown_one() {
        let ctx = egui::Context::default();
        let mut app = two_tabs();
        app.close(&ctx, 1);
        assert_eq!(app.tabs[0].tree, Node::Leaf(2));
        assert_eq!(app.tabs[0].active, 2);
        assert_eq!((app.tab, app.active()), (1, 3));
    }

    #[test]
    fn focusing_a_pane_over_the_socket_shows_its_tab() {
        let ctx = egui::Context::default();
        let mut app = two_tabs();
        app.answer(&ctx, Request::Focus { pane: 2 });
        assert_eq!((app.tab, app.active()), (0, 2));
        let list = app.answer(&ctx, Request::List);
        let tabs: Vec<_> = list["panes"].as_array().unwrap().iter().map(|p| p["tab"].as_u64().unwrap()).collect();
        assert_eq!(tabs, [1, 1, 2]);
    }

    #[test]
    fn alt_and_a_number_shows_that_tab_and_the_digit_never_reaches_the_shell() {
        let alt = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
        let ctx = egui::Context::default();
        let taken = |events: Vec<Event>| {
            let mut out = (None, vec![]);
            let _ = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
                out = (tab_number(ui.ctx(), 2), ui.input(|i| i.events.clone()));
            });
            out
        };
        let (tab, left) = taken(vec![alt(egui::Key::Num2), Event::Text("2".into())]);
        assert_eq!(tab, Some(1));
        assert!(left.is_empty(), "{left:?}");
        // No third tab: Alt+3 goes to the shell, for readline.
        let (tab, left) = taken(vec![alt(egui::Key::Num3), Event::Text("3".into())]);
        assert_eq!((tab, left.len()), (None, 2));
    }

    /// One frame of just the title bar, as `ui` draws it; the actions it asked for.
    fn header_frame(ctx: &egui::Context, app: &App, events: Vec<Event>) -> Vec<Action> {
        let time = TIME.with(|t| {
            t.set(t.get() + 0.05);
            t.get()
        });
        let input = RawInput { screen_rect: Some(DESKTOP), events, time: Some(time), ..Default::default() };
        let mut actions = Vec::new();
        let _ = ctx.run_ui(input, |ui| {
            let style = app.skins[0].frame_style(ui.visuals());
            let header = frame::header_frame(ui.style(), &style, 0);
            egui::Panel::top("header").frame(header).show(ui, |ui| {
                let span = (ui.max_rect() + header.inner_margin).y_range();
                TitleBar::new("", &style).menu_bar(true).show(ui, |ui| app.tab_strip(ui, span, &style, &mut actions), |_| {});
            });
        });
        actions
    }

    /// Tab `i`'s id in Patina's strip, and its cross's.
    fn tab_ids(i: usize) -> (Id, Id) {
        TabStrip::new(&[""], 0, &FrameStyle::from_visuals(&egui::Visuals::dark())).tab_id(i)
    }

    #[test]
    fn a_tab_takes_clicks_from_the_top_of_the_bar() {
        let ctx = egui::Context::default();
        let app = two_tabs();
        header_frame(&ctx, &app, vec![]);
        let first = ctx.read_response(tab_ids(0).0).unwrap().rect;
        assert_eq!(first.top(), 0.0, "the tab starts at the window's top edge");
        let at = pos2(first.center().x, 2.0);
        header_frame(&ctx, &app, vec![Event::PointerMoved(at)]);
        header_frame(&ctx, &app, vec![button(at, true)]);
        let actions = header_frame(&ctx, &app, vec![button(at, false)]);
        assert!(matches!(actions[..], [Action::ShowTab(0)]));
    }

    #[test]
    fn a_hovered_tab_closes_from_its_cross() {
        let ctx = egui::Context::default();
        let app = two_tabs();
        header_frame(&ctx, &app, vec![]);
        let tab = ctx.read_response(tab_ids(0).0).unwrap().rect;
        header_frame(&ctx, &app, vec![Event::PointerMoved(tab.center())]);
        let cross = ctx.read_response(tab_ids(0).1).expect("hovered, the tab has a cross").rect;
        let at = cross.center();
        header_frame(&ctx, &app, vec![Event::PointerMoved(at)]);
        header_frame(&ctx, &app, vec![button(at, true)]);
        let actions = header_frame(&ctx, &app, vec![button(at, false)]);
        assert!(matches!(actions[..], [Action::CloseTab(0)]));
    }
}
