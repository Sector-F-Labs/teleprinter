//! The terminal widget: paints the grid and sends keyboard, paste and
//! scroll input to the session. Painted with egui's own painter from the
//! active palette, so skins reach every cell (ADR-0003).

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::{Dimensions, Indexed, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{CursorShape, NamedColor};
use egui::{Color32, Event, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use ttf_parser::GlyphId;

use crate::emoji;
use crate::fonts;
use crate::palette::{self, Palette};
use crate::session::Session;

/// Requests from the window's menus, handled on the next frame the widget draws.
#[derive(Default)]
pub struct Requests {
    pub copy: bool,
    pub paste: bool,
    pub focus: bool,
}

/// Per-widget state kept between frames.
#[derive(Default)]
pub struct ViewState {
    /// Wheel movement not yet worth a whole line.
    scroll_rest: f32,
    /// An Alt+letter already sent as ESC+letter, whose Text event must be dropped.
    swallow_text: bool,
    /// The button pressed on a program that tracks the mouse, until it's released.
    mouse_down: Option<u8>,
    /// The cell last reported to it, so motion is sent once per cell.
    mouse_cell: Option<(usize, usize)>,
}

pub fn show(
    ui: &mut egui::Ui,
    session: &mut Session,
    state: &mut ViewState,
    palette: &Palette,
    font_size: f32,
    requests: &mut Requests,
    active: bool,
) -> egui::Response {
    let rect = ui.available_rect_before_wrap();
    let ppp = ui.ctx().pixels_per_point();
    let regular = fonts::font(font_size, false, false);
    let (cell_w, cell_h) = ui.ctx().fonts_mut(|f| (f.glyph_width(&regular, 'M'), f.row_height(&regular)));
    // The font's own advance across, so box-drawing glyphs meet; whole pixels down.
    let cell = vec2(cell_w, (cell_h * ppp).round() / ppp);
    let columns = ((rect.width() / cell.x).floor() as u16).max(2);
    let lines = ((rect.height() / cell.y).floor() as u16).max(1);
    session.resize(WindowSize {
        num_cols: columns,
        num_lines: lines,
        cell_width: (cell.x * ppp) as u16,
        cell_height: (cell.y * ppp) as u16,
    });

    // Claim the whole space, so a window around the terminal is as big as its tile.
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let id = response.id;
    // Nothing else (a menu, a dialog) wants the keyboard: the active pane's shell has it.
    // egui drops focus when the window loses it, so this also takes it back on returning.
    let unclaimed = active && ui.memory(|m| m.focused().is_none());
    if response.clicked() || response.drag_started() || response.secondary_clicked() || requests.focus || unclaimed {
        response.request_focus();
        requests.focus = false;
    }
    if response.has_focus() {
        // Tab, arrows and Escape belong to the shell, not egui's focus navigation.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
            )
        });
        handle_keys(ui, session, state, requests);
    }
    if requests.copy {
        requests.copy = false;
        copy_selection(ui.ctx(), session);
    }
    handle_mouse(ui, session, state, &response, rect.min, cell);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }

    paint(ui, session, palette, font_size, rect.min, cell, response.has_focus());
    response
}

fn copy_selection(ctx: &egui::Context, session: &Session) {
    if let Some(text) = session.term.lock().selection_to_string() {
        ctx.copy_text(text);
    }
}

fn handle_keys(ui: &egui::Ui, session: &mut Session, state: &mut ViewState, requests: &mut Requests) {
    let (events, modifiers) = ui.input(|i| (i.events.clone(), i.modifiers));
    let mode = *session.term.lock().mode();
    for event in events {
        match event {
            Event::Text(text) => {
                if std::mem::take(&mut state.swallow_text) {
                    continue;
                }
                session.write(text.into_bytes());
            }
            Event::Key { key, pressed: true, modifiers, .. } => {
                if modifiers.shift && !modifiers.ctrl && matches!(key, Key::PageUp | Key::PageDown) {
                    let mut term = session.term.lock();
                    term.scroll_display(if key == Key::PageUp { Scroll::PageUp } else { Scroll::PageDown });
                } else if let Some(bytes) = key_bytes(key, modifiers, mode) {
                    // Alt+letter arrives as a Key and then as Text; the Key sent ESC+letter.
                    state.swallow_text = modifiers.alt && !modifiers.ctrl && key_char(key).is_some();
                    session.write(bytes);
                }
            }
            // egui turns Ctrl+C/X/V into these. With Shift they're the clipboard, without
            // they're the control characters the shell expects (interrupt, ^X, ^V).
            Event::Copy => {
                if modifiers.shift {
                    copy_selection(ui.ctx(), session);
                } else {
                    session.write(&b"\x03"[..]);
                }
            }
            Event::Cut if !modifiers.shift => session.write(&b"\x18"[..]),
            Event::Paste(text) => {
                if modifiers.shift || std::mem::take(&mut requests.paste) {
                    session.paste(&text);
                } else {
                    session.write(&b"\x16"[..]);
                }
            }
            _ => {}
        }
    }
}

/// The letter or symbol a key types, for Ctrl and Alt combinations.
fn key_char(key: Key) -> Option<char> {
    let name = key.symbol_or_name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_graphic() => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

/// The bytes a key sends, as xterm does, or `None` for keys that only type
/// text (that comes as a Text event).
pub fn key_bytes(key: Key, m: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    // xterm's modifier parameter: 1 + Shift + 2·Alt + 4·Ctrl.
    let param = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8;
    let csi = |end: char| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{end}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{end}").into_bytes()
        } else {
            format!("\x1b[{end}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 { format!("\x1b[{n};{param}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    let ss3 = |end: char| -> Vec<u8> {
        if param > 1 { format!("\x1b[1;{param}{end}").into_bytes() } else { format!("\x1bO{end}").into_bytes() }
    };
    let alt = |bytes: &[u8]| -> Vec<u8> {
        let mut out = if m.alt { vec![0x1b] } else { Vec::new() };
        out.extend_from_slice(bytes);
        out
    };
    Some(match key {
        Key::ArrowUp => csi('A'),
        Key::ArrowDown => csi('B'),
        Key::ArrowRight => csi('C'),
        Key::ArrowLeft => csi('D'),
        Key::Home => csi('H'),
        Key::End => csi('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => ss3('P'),
        Key::F2 => ss3('Q'),
        Key::F3 => ss3('R'),
        Key::F4 => ss3('S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Enter => alt(b"\r"),
        Key::Backspace if m.ctrl => alt(b"\x08"),
        Key::Backspace => alt(b"\x7f"),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => alt(b"\t"),
        Key::Escape => b"\x1b".to_vec(),
        Key::Space if m.ctrl => alt(b"\0"),
        _ => {
            let c = key_char(key)?;
            if m.ctrl {
                let byte = match c {
                    'a'..='z' => c as u8 - b'a' + 1,
                    '[' => 0x1b,
                    '\\' => 0x1c,
                    ']' => 0x1d,
                    '6' => 0x1e,
                    '-' | '/' => 0x1f,
                    '2' | '@' => 0,
                    _ => return None,
                };
                alt(&[byte])
            } else if m.alt {
                // Shifted symbols come through Text; Alt+letter is ESC then the letter.
                let c = if m.shift { c.to_ascii_uppercase() } else { c };
                alt(c.to_string().as_bytes())
            } else {
                return None;
            }
        }
    })
}

/// The cell under a position, and which half of it.
fn cell_at(pos: Pos2, origin: Pos2, cell: Vec2, size: (usize, usize), display_offset: usize) -> (Point, Side) {
    let (columns, lines) = size;
    let x = ((pos.x - origin.x) / cell.x).max(0.0);
    let column = (x as usize).min(columns - 1);
    let line = (((pos.y - origin.y) / cell.y).max(0.0) as usize).min(lines - 1);
    let side = if x.fract() < 0.5 { Side::Left } else { Side::Right };
    (Point::new(Line(line as i32 - display_offset as i32), Column(column)), side)
}

fn handle_mouse(
    ui: &egui::Ui,
    session: &mut Session,
    state: &mut ViewState,
    response: &egui::Response,
    origin: Pos2,
    cell: Vec2,
) {
    let mut term = session.term.lock();
    let size = (term.columns(), term.screen_lines());
    let offset = term.grid().display_offset();
    let reporting = reports_mouse(*term.mode(), ui.input(|i| i.modifiers));

    if reporting {
        drop(term);
        report_buttons(ui, session, state, response, origin, cell, size);
        term = session.term.lock();
    } else if let Some(code) = state.mouse_down.take() {
        // Shift went down mid-press, or the program stopped tracking: finish what it saw start.
        let mode = *term.mode();
        drop(term);
        let at = state.mouse_cell.unwrap_or_default();
        session.write(mouse_report(code, false, at, Modifiers::NONE, mode));
        term = session.term.lock();
    }

    if let Some(pos) = response.interact_pointer_pos().filter(|_| !reporting) {
        let (point, side) = cell_at(pos, origin, cell, size, offset);
        let clicks = ui.input(|i| {
            if i.pointer.button_triple_clicked(egui::PointerButton::Primary) {
                3
            } else if i.pointer.button_double_clicked(egui::PointerButton::Primary) {
                2
            } else {
                1
            }
        });
        if clicks > 1 {
            let ty = if clicks == 3 { SelectionType::Lines } else { SelectionType::Semantic };
            term.selection = Some(Selection::new(ty, point, side));
        } else if response.drag_started_by(egui::PointerButton::Primary) {
            term.selection = Some(Selection::new(SelectionType::Simple, point, side));
        } else if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(selection) = term.selection.as_mut() {
                selection.update(point, side);
            }
        } else if response.clicked() {
            term.selection = None;
        }
    }

    if response.hovered() {
        let delta = ui.input(|i| {
            i.events.iter().fold(0.0, |sum, e| match e {
                Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, .. } => sum + delta.y,
                Event::MouseWheel { unit: egui::MouseWheelUnit::Page, delta, .. } => sum + delta.y * size.1 as f32,
                Event::MouseWheel { delta, .. } => sum + delta.y / cell.y,
                _ => sum,
            })
        });
        state.scroll_rest += delta;
        let lines = state.scroll_rest.trunc() as i32;
        state.scroll_rest -= lines as f32;
        if lines != 0 {
            let mode = *term.mode();
            let (modifiers, pointer) = ui.input(|i| (i.modifiers, i.pointer.hover_pos()));
            if reporting {
                // The program tracks the mouse (Claude Code, full screen): it scrolls itself (ADR-0010).
                drop(term);
                let at = screen_cell(pointer.unwrap_or(origin), origin, cell, size);
                let report = mouse_report(if lines > 0 { 64 } else { 65 }, true, at, modifiers, mode);
                session.write(report.repeat(lines.unsigned_abs() as usize));
            } else if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
                // Full-screen programs (less, vim) get arrow keys instead, as in other terminals.
                drop(term);
                let key = if lines > 0 { Key::ArrowUp } else { Key::ArrowDown };
                let bytes = key_bytes(key, Modifiers::NONE, mode).unwrap_or_default();
                session.write(bytes.repeat(lines.unsigned_abs() as usize));
            } else {
                term.scroll_display(Scroll::Delta(lines));
            }
        }
    }
}

/// Whether the mouse goes to the program: it asked for it, and Shift isn't keeping it for
/// Teleprinter's selection and menu (ADR-0010, ADR-0011).
fn reports_mouse(mode: TermMode, modifiers: Modifiers) -> bool {
    mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift
}

/// Whether a right-click in this pane is the program's rather than the pane menu's.
pub fn takes_right_click(ui: &egui::Ui, session: &Session) -> bool {
    reports_mouse(*session.term.lock().mode(), ui.input(|i| i.modifiers))
}

/// The cell of the screen under `pos`, 0-based, whatever the scrollback shows.
fn screen_cell(pos: Pos2, origin: Pos2, cell: Vec2, size: (usize, usize)) -> (usize, usize) {
    let (point, _) = cell_at(pos, origin, cell, size, 0);
    (point.column.0, point.line.0 as usize)
}

/// Presses, releases and drags for a program that tracks the mouse (ADR-0011). A press counts
/// only on the terminal; its release and drags are reported wherever the pointer goes.
fn report_buttons(
    ui: &egui::Ui,
    session: &mut Session,
    state: &mut ViewState,
    response: &egui::Response,
    origin: Pos2,
    cell: Vec2,
    size: (usize, usize),
) {
    let mode = *session.term.lock().mode();
    let (events, modifiers, any_down) = ui.input(|i| (i.events.clone(), i.modifiers, i.pointer.any_down()));
    let mut out = Vec::new();
    for event in events {
        match event {
            Event::PointerButton { pos, button, pressed, modifiers, .. } => {
                let code = match button {
                    egui::PointerButton::Primary => 0,
                    egui::PointerButton::Middle => 1,
                    egui::PointerButton::Secondary => 2,
                    _ => continue,
                };
                let at = screen_cell(pos, origin, cell, size);
                if pressed && state.mouse_down.is_none() && response.hovered() && response.rect.contains(pos) {
                    state.mouse_down = Some(code);
                    state.mouse_cell = Some(at);
                    out.extend(mouse_report(code, true, at, modifiers, mode));
                } else if !pressed && state.mouse_down == Some(code) {
                    state.mouse_down = None;
                    out.extend(mouse_report(code, false, at, modifiers, mode));
                }
            }
            Event::PointerMoved(pos) => {
                let at = screen_cell(pos, origin, cell, size);
                if state.mouse_cell == Some(at) {
                    continue;
                }
                // Motion is the button's code plus 32; 3 is no button, for `?1003`.
                let code = match state.mouse_down {
                    Some(code) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => code + 32,
                    None if mode.contains(TermMode::MOUSE_MOTION) && response.rect.contains(pos) => 3 + 32,
                    _ => continue,
                };
                state.mouse_cell = Some(at);
                out.extend(mouse_report(code, true, at, modifiers, mode));
            }
            _ => {}
        }
    }
    // Released where egui didn't see it (outside the window): don't leave the program dragging.
    if let Some(code) = state.mouse_down.filter(|_| !any_down) {
        state.mouse_down = None;
        out.extend(mouse_report(code, false, state.mouse_cell.unwrap_or_default(), modifiers, mode));
    }
    if !out.is_empty() {
        session.write(out);
    }
}

/// A mouse report, xterm's way: `button` is 0–2 for left, middle and right, 64/65 for the wheel,
/// plus 32 for motion; Shift adds 4, Alt 8, Ctrl 16. At a 0-based cell of the screen. SGR under
/// `?1006` (`m` for a release), else the legacy bytes, where any release is button 3.
fn mouse_report(button: u8, pressed: bool, (column, line): (usize, usize), modifiers: Modifiers, mode: TermMode) -> Vec<u8> {
    let held = if modifiers.shift { 4 } else { 0 } + if modifiers.alt { 8 } else { 0 } + if modifiers.ctrl { 16 } else { 0 };
    let (x, y) = (column + 1, line + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        format!("\x1b[<{};{x};{y}{end}", button + held).into_bytes()
    } else {
        // Each value is one byte offset by 32, so columns and rows stop at 223.
        let byte = |v: usize| (32 + v.min(223)) as u8;
        let button = if pressed { button } else { 3 } + held;
        vec![0x1b, b'[', b'M', byte(button as usize), byte(x), byte(y)]
    }
}

fn paint(ui: &egui::Ui, session: &Session, palette: &Palette, font_size: f32, origin: Pos2, cell: Vec2, focused: bool) {
    let painter = ui.painter();
    let term = session.term.lock();
    let content = term.renderable_content();
    let colors = content.colors;
    let offset = content.display_offset as i32;
    let selection = content.selection;
    let cursor = content.cursor;
    let at = |point: Point| origin + vec2(point.column.0 as f32 * cell.x, (point.line.0 + offset) as f32 * cell.y);

    // A run of cells on one line drawn as one piece of text: same colour and face.
    struct Run {
        start: Pos2,
        text: String,
        color: Color32,
        bold: bool,
        italic: bool,
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut emojis: Vec<(GlyphId, Rect)> = Vec::new();
    let mut last: Option<Point> = None;
    // Cells before this index are the tail of an emoji sequence already placed.
    let mut emoji_until = 0;
    let cells: Vec<Indexed<&Cell>> = content.display_iter.collect();

    for (i, indexed) in cells.iter().enumerate() {
        let point = indexed.point;
        let c = &indexed.cell;
        if c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let mut fg = palette.resolve(c.fg, colors);
        let mut bg = palette.resolve(c.bg, colors);
        if c.flags.contains(Flags::DIM) {
            fg = palette::dim(fg);
        }
        if c.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if selection.is_some_and(|s| s.contains(point)) {
            bg = palette.selection;
        }
        let width = if c.flags.contains(Flags::WIDE_CHAR) { 2.0 } else { 1.0 };
        let pos = at(point);
        let cell_rect = Rect::from_min_size(pos, vec2(cell.x * width, cell.y));
        if bg != palette.background {
            painter.rect_filled(cell_rect, 0.0, bg);
        }
        let is_cursor = focused && cursor.shape == CursorShape::Block && point == cursor.point;
        if is_cursor {
            painter.rect_filled(cell_rect, 0.0, palette.cursor);
            fg = palette.named(NamedColor::Background, colors);
        }
        if c.flags.contains(Flags::HIDDEN) {
            continue;
        }
        if c.flags.intersects(Flags::ALL_UNDERLINES) {
            let y = cell_rect.bottom() - 1.0;
            painter.hline(cell_rect.x_range(), y, Stroke::new(1.0, fg));
        }
        if c.flags.contains(Flags::STRIKEOUT) {
            painter.hline(cell_rect.x_range(), cell_rect.center().y, Stroke::new(1.0, fg));
        }
        if (c.c == ' ' && c.zerowidth().is_none()) || i < emoji_until {
            last = None;
            continue;
        }
        if emoji::wants(c.c, width > 1.0, c.zerowidth().unwrap_or_default())
            && let Some((glyph, end, span)) = emoji_at(&cells, i)
        {
            // Two cells at most: a ZWJ family is six cells to the shell, but one picture.
            emojis.push((glyph, Rect::from_min_size(pos, vec2(cell.x * span.min(2.0), cell.y))));
            emoji_until = end;
            last = None;
            continue;
        }
        let bold = c.flags.contains(Flags::BOLD);
        let italic = c.flags.contains(Flags::ITALIC);
        // Only plain ASCII joins a run: other glyphs (icons, CJK, emoji) may be wider
        // or narrower than a cell, so each is placed at its own cell.
        let ascii = c.c.is_ascii() && c.zerowidth().is_none() && !is_cursor;
        let follows = last.is_some_and(|l| l.line == point.line && l.column.0 + 1 == point.column.0);
        match runs.last_mut() {
            Some(run) if ascii && follows && run.color == fg && run.bold == bold && run.italic == italic => {
                run.text.push(c.c)
            }
            _ => {
                let mut text = c.c.to_string();
                text.extend(c.zerowidth().into_iter().flatten());
                runs.push(Run { start: pos, text, color: fg, bold, italic });
            }
        }
        last = if ascii { Some(point) } else { None };
    }

    for run in runs {
        painter.text(run.start, egui::Align2::LEFT_TOP, run.text, fonts::font(font_size, run.bold, run.italic), run.color);
    }
    for (glyph, rect) in emojis {
        emoji::paint(painter, glyph, rect);
    }

    // Other cursor shapes, and the hollow block of an unfocused window.
    let rect = Rect::from_min_size(at(cursor.point), cell);
    let stroke = Stroke::new(1.0, palette.cursor);
    match cursor.shape {
        CursorShape::Hidden => {}
        CursorShape::Block | CursorShape::HollowBlock if !focused || cursor.shape == CursorShape::HollowBlock => {
            {
                painter.rect_stroke(rect.shrink(0.5), 0.0, stroke, egui::StrokeKind::Inside);
            }
        }
        CursorShape::Block | CursorShape::HollowBlock => {}
        CursorShape::Beam => {
            painter.rect_filled(Rect::from_min_size(rect.min, vec2(2.0, cell.y)), 0.0, palette.cursor);
        }
        CursorShape::Underline => {
            painter.rect_filled(Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.max), 0.0, palette.cursor);
        }
    }
}

/// The emoji starting at `cells[i]`, with the cells it runs over: alacritty puts each part of
/// a ZWJ sequence, skin tone or flag in its own cell. The glyph, the index after its last
/// cell, and its width in cells. Falls back to the first cell alone if the whole isn't known.
fn emoji_at(cells: &[Indexed<&Cell>], i: usize) -> Option<(GlyphId, usize, f32)> {
    let text_of = |c: &Cell| std::iter::once(c.c).chain(c.zerowidth().into_iter().flatten().copied()).collect::<String>();
    let width_of = |c: &Cell| if c.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
    let first = &cells[i];
    let mut text = text_of(first.cell);
    let (mut end, mut span) = (i + 1, width_of(first.cell));
    let mut j = end;
    while let Some(next) = cells.get(j) {
        if next.cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            j += 1;
            continue;
        }
        let adjacent = next.point.line == first.point.line && next.point.column.0 == first.point.column.0 + span;
        if !adjacent || !emoji::continues(&text, next.cell.c) {
            break;
        }
        text.push_str(&text_of(next.cell));
        span += width_of(next.cell);
        j += 1;
        end = j;
    }
    match emoji::glyph(&text) {
        Some(glyph) => Some((glyph, end, span as f32)),
        None if end > i + 1 => emoji::glyph(&text_of(first.cell)).map(|g| (g, i + 1, width_of(first.cell) as f32)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(key: Key, m: Modifiers) -> Option<String> {
        key_bytes(key, m, TermMode::empty()).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn control_letters_are_control_bytes() {
        assert_eq!(bytes(Key::D, Modifiers::CTRL).as_deref(), Some("\x04"));
        assert_eq!(bytes(Key::OpenBracket, Modifiers::CTRL).as_deref(), Some("\x1b"));
        assert_eq!(bytes(Key::A, Modifiers::NONE), None, "plain letters come as Text");
    }

    #[test]
    fn mouse_reports_follow_the_programs_encoding() {
        let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(mouse_report(64, true, (0, 0), Modifiers::NONE, sgr), b"\x1b[<64;1;1M");
        assert_eq!(mouse_report(65, true, (9, 4), Modifiers::CTRL, sgr), b"\x1b[<81;10;5M");
        assert_eq!(mouse_report(0, false, (9, 4), Modifiers::NONE, sgr), b"\x1b[<0;10;5m", "SGR releases keep the button");
        assert_eq!(mouse_report(32, true, (1, 1), Modifiers::NONE, sgr), b"\x1b[<32;2;2M", "left drag");
        let legacy = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(mouse_report(64, true, (2, 3), Modifiers::ALT, legacy), b"\x1b[M\x68\x23\x24");
        assert_eq!(mouse_report(65, true, (400, 0), Modifiers::NONE, legacy), b"\x1b[M\x61\xff\x21", "past 223 stays at 223");
        assert_eq!(mouse_report(2, false, (0, 0), Modifiers::NONE, legacy), b"\x1b[M\x23\x21\x21", "legacy releases are button 3");
    }

    #[test]
    fn shift_keeps_the_mouse_for_teleprinter() {
        let tracking = TermMode::MOUSE_DRAG;
        assert!(reports_mouse(tracking, Modifiers::NONE));
        assert!(!reports_mouse(tracking, Modifiers::SHIFT));
        assert!(!reports_mouse(TermMode::empty(), Modifiers::NONE), "no tracking: selection as ever");
    }

    #[test]
    fn alt_letters_are_escape_prefixed() {
        assert_eq!(bytes(Key::B, Modifiers::ALT).as_deref(), Some("\x1bb"));
        assert_eq!(bytes(Key::Backspace, Modifiers::ALT).as_deref(), Some("\x1b\x7f"));
    }

    #[test]
    fn arrows_follow_cursor_mode_and_modifiers() {
        assert_eq!(bytes(Key::ArrowUp, Modifiers::NONE).as_deref(), Some("\x1b[A"));
        assert_eq!(bytes(Key::ArrowLeft, Modifiers::CTRL).as_deref(), Some("\x1b[1;5D"));
        let app = key_bytes(Key::ArrowUp, Modifiers::NONE, TermMode::APP_CURSOR).unwrap();
        assert_eq!(app, b"\x1bOA");
    }

    #[test]
    fn function_and_editing_keys() {
        assert_eq!(bytes(Key::F1, Modifiers::NONE).as_deref(), Some("\x1bOP"));
        assert_eq!(bytes(Key::F5, Modifiers::NONE).as_deref(), Some("\x1b[15~"));
        assert_eq!(bytes(Key::Delete, Modifiers::SHIFT).as_deref(), Some("\x1b[3;2~"));
        assert_eq!(bytes(Key::Tab, Modifiers::SHIFT).as_deref(), Some("\x1b[Z"));
    }

    #[test]
    fn a_position_maps_to_its_cell_and_half() {
        let (p, side) = cell_at(pos2(25.0, 41.0), Pos2::ZERO, vec2(10.0, 20.0), (80, 24), 0);
        assert_eq!((p.line, p.column, side), (Line(2), Column(2), Side::Right));
        let (p, _) = cell_at(pos2(5.0, 5.0), Pos2::ZERO, vec2(10.0, 20.0), (80, 24), 3);
        assert_eq!(p.line, Line(-3), "scrolled back three lines");
        let (p, _) = cell_at(pos2(5000.0, 5000.0), Pos2::ZERO, vec2(10.0, 20.0), (80, 24), 0);
        assert_eq!((p.line, p.column), (Line(23), Column(79)), "clamped to the grid");
    }
}
