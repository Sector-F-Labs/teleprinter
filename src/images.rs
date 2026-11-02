//! Inline images (the iTerm2 protocol, `OSC 1337 ; File=… : base64 BEL`, what
//! `imgcat` sends). alacritty_terminal ignores the sequence and its event loop
//! has no hook, so the pty is wrapped: `ImagePty` filters what the loop reads,
//! takes the image out of the stream and leaves cells in its place. Each cell
//! holds a private-use character that names its row (`U+10EE00 + row`) and has
//! the image's id as its foreground colour, so the image scrolls, wraps and is
//! overwritten with the text around it. The view paints those cells as slices
//! of the image's texture.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use base64::Engine;
use egui::{ColorImage, TextureHandle, TextureOptions};
use polling::{Event, PollMode, Poller};

const PREFIX: &[u8] = b"\x1b]1337;";
/// Base of the row-carrying placeholder characters.
pub const PLACEHOLDER: u32 = 0x10EE00;
/// Largest sequence taken (base64 included); bigger ones are dropped.
const MAX_SEQUENCE: usize = 48 << 20;

/// The id an image's cells carry, as a foreground colour.
pub fn id_color(id: u32) -> [u8; 3] {
    [(id >> 16) as u8, (id >> 8) as u8, id as u8]
}

pub fn id_of(rgb: [u8; 3]) -> u32 {
    (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32
}

/// The row of an image a placeholder cell shows.
pub fn row_of(c: char) -> Option<u32> {
    let c = c as u32;
    (PLACEHOLDER..PLACEHOLDER + 256).contains(&c).then(|| c - PLACEHOLDER)
}

struct Incoming {
    id: u32,
    bytes: Vec<u8>,
    cols: u32,
    rows: u32,
}

/// What the pty thread and the UI share: the pane's size, for sizing images,
/// and images waiting for a texture.
#[derive(Default)]
pub struct Shared {
    columns: AtomicU32,
    cell_w: AtomicU32,
    cell_h: AtomicU32,
    next_id: AtomicU32,
    incoming: Mutex<Vec<Incoming>>,
}

impl Shared {
    pub fn new(size: WindowSize) -> Arc<Self> {
        let shared = Arc::new(Self::default());
        shared.set_size(size);
        shared
    }

    fn set_size(&self, size: WindowSize) {
        self.columns.store(size.num_cols as u32, Ordering::Relaxed);
        self.cell_w.store(size.cell_width as u32, Ordering::Relaxed);
        self.cell_h.store(size.cell_height as u32, Ordering::Relaxed);
    }
}

/// Images of one pane, with their textures made on first paint.
#[derive(Default)]
pub struct Images {
    entries: HashMap<u32, Entry>,
}

struct Entry {
    bytes: Vec<u8>,
    cols: u32,
    rows: u32,
    texture: Option<TextureHandle>,
}

impl Images {
    pub fn take(&mut self, shared: &Shared) {
        for i in shared.incoming.lock().unwrap().drain(..) {
            self.entries.insert(i.id, Entry { bytes: i.bytes, cols: i.cols, rows: i.rows, texture: None });
        }
    }

    /// The texture of image `id` and its size in cells.
    pub fn texture(&mut self, ctx: &egui::Context, id: u32) -> Option<(TextureHandle, u32, u32)> {
        let e = self.entries.get_mut(&id)?;
        if e.texture.is_none() {
            let img = image::load_from_memory(&e.bytes).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            e.texture = Some(ctx.load_texture(format!("inline-image-{id}"), color, TextureOptions::LINEAR));
            e.bytes = Vec::new();
        }
        Some((e.texture.clone()?, e.cols, e.rows))
    }
}

/// The pty with its output filtered for images.
pub struct ImagePty {
    inner: Pty,
    reader: Filter,
    shared: Arc<Shared>,
}

impl ImagePty {
    pub fn new(inner: Pty, shared: Arc<Shared>) -> io::Result<Self> {
        let file = inner.file().try_clone()?;
        Ok(Self { inner, reader: Filter::new(file, shared.clone()), shared })
    }
}

impl EventedReadWrite for ImagePty {
    type Reader = Filter;
    type Writer = File;

    unsafe fn register(&mut self, p: &Arc<Poller>, e: Event, m: PollMode) -> io::Result<()> {
        unsafe { self.inner.register(p, e, m) }
    }
    fn reregister(&mut self, p: &Arc<Poller>, e: Event, m: PollMode) -> io::Result<()> {
        self.inner.reregister(p, e, m)
    }
    fn deregister(&mut self, p: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(p)
    }
    fn reader(&mut self) -> &mut Filter {
        &mut self.reader
    }
    fn writer(&mut self) -> &mut File {
        self.inner.writer()
    }
}

impl EventedPty for ImagePty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl OnResize for ImagePty {
    fn on_resize(&mut self, size: WindowSize) {
        self.shared.set_size(size);
        self.inner.on_resize(size);
    }
}

/// Reads the pty, passing everything but image sequences through.
pub struct Filter {
    file: File,
    shared: Arc<Shared>,
    /// Read but not yet scanned: a sequence that may still be arriving.
    raw: Vec<u8>,
    out: VecDeque<u8>,
    /// A multipart image (what `imgcat` sends): its parameters and the base64 so far.
    multipart: Option<(Vec<u8>, Vec<u8>)>,
}

impl Filter {
    fn new(file: File, shared: Arc<Shared>) -> Self {
        Self { file, shared, raw: Vec::new(), out: VecDeque::new(), multipart: None }
    }

    /// Moves what's complete in `raw` to `out`; `eof` flushes the rest.
    fn scan(&mut self, eof: bool) {
        let raw = std::mem::take(&mut self.raw);
        let mut pos = 0;
        while pos < raw.len() {
            let Some(esc) = raw[pos..].iter().position(|&b| b == 0x1b).map(|i| pos + i) else {
                self.out.extend(&raw[pos..]);
                pos = raw.len();
                break;
            };
            self.out.extend(&raw[pos..esc]);
            pos = esc;
            let rest = &raw[esc..];
            if rest.starts_with(PREFIX) {
                match terminator(rest) {
                    Some((payload_end, end)) => {
                        match self.sequence(&rest[PREFIX.len()..payload_end]) {
                            Some(replacement) => self.out.extend(replacement),
                            // Some other OSC 1337: the terminal's to ignore.
                            None => self.out.extend(&rest[..end]),
                        }
                        pos = esc + end;
                    }
                    None if rest.len() > MAX_SEQUENCE => {
                        // Too big to be one: let it through as text.
                        self.out.push_back(0x1b);
                        pos += 1;
                    }
                    None if eof => {
                        self.out.extend(rest);
                        pos = raw.len();
                    }
                    None => break,
                }
            } else if PREFIX.starts_with(rest) && !eof {
                break;
            } else {
                self.out.push_back(0x1b);
                pos += 1;
            }
        }
        self.raw = raw[pos..].to_vec();
    }

    /// What an OSC 1337 body becomes: `None` if it isn't about images.
    fn sequence(&mut self, body: &[u8]) -> Option<Vec<u8>> {
        if let Some(rest) = body.strip_prefix(b"File=") {
            return Some(self.image(rest));
        }
        if let Some(params) = body.strip_prefix(b"MultipartFile=") {
            self.multipart = Some((params.to_vec(), Vec::new()));
            return Some(Vec::new());
        }
        if let Some(part) = body.strip_prefix(b"FilePart=") {
            if let Some((_, data)) = &mut self.multipart {
                if data.len() + part.len() > MAX_SEQUENCE {
                    self.multipart = None;
                } else {
                    data.extend_from_slice(part);
                }
            }
            return Some(Vec::new());
        }
        if body == b"FileEnd" {
            let (mut whole, data) = self.multipart.take()?;
            whole.push(b':');
            whole.extend(data);
            return Some(self.image(&whole));
        }
        None
    }

    /// The bytes that leave room for the image in the grid: rows of
    /// placeholder cells, each followed by a line feed back at the start column.
    fn image(&self, params_and_data: &[u8]) -> Vec<u8> {
        let Some(colon) = params_and_data.iter().position(|&b| b == b':') else { return Vec::new() };
        let (params, data) = (&params_and_data[..colon], &params_and_data[colon + 1..]);
        let params = String::from_utf8_lossy(params);
        let mut inline = false;
        let (mut width, mut height) = (String::new(), String::new());
        let mut keep_aspect = true;
        for p in params.split(';') {
            match p.split_once('=') {
                Some(("inline", v)) => inline = v == "1",
                Some(("width", v)) => width = v.to_string(),
                Some(("height", v)) => height = v.to_string(),
                Some(("preserveAspectRatio", v)) => keep_aspect = v != "0",
                _ => {}
            }
        }
        if !inline {
            return Vec::new();
        }
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect::<Vec<_>>()) else {
            return Vec::new();
        };
        let Some((px_w, px_h)) = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
        else {
            return Vec::new();
        };
        let columns = self.shared.columns.load(Ordering::Relaxed).max(1);
        let cell_w = self.shared.cell_w.load(Ordering::Relaxed).max(1) as f32;
        let cell_h = self.shared.cell_h.load(Ordering::Relaxed).max(1) as f32;
        let (cols, rows) = fit(px_w as f32, px_h as f32, cell_w, cell_h, columns, &width, &height, keep_aspect);
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed) % 0xFF_FFFE + 1;
        self.shared.incoming.lock().unwrap().push(Incoming { id, bytes, cols, rows });

        let [r, g, b] = id_color(id);
        let mut out = format!("\x1b[38;2;{r};{g};{b}m").into_bytes();
        for row in 0..rows {
            let ch = char::from_u32(PLACEHOLDER + row.min(255)).unwrap();
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            for _ in 0..cols {
                out.extend(s.as_bytes());
            }
            // The cursor stays after the image's last row, as in iTerm2.
            if row + 1 < rows {
                out.extend(format!("\x1b[{cols}D\n").into_bytes());
            }
        }
        out.extend(b"\x1b[39m");
        out
    }
}

/// A size parameter: `N` cells, `Npx`, `N%` of `total` cells, or auto.
fn dimension(spec: &str, cell_px: f32, total_cells: f32) -> Option<f32> {
    let spec = spec.trim();
    if let Some(px) = spec.strip_suffix("px") {
        px.parse::<f32>().ok().map(|px| px / cell_px)
    } else if let Some(pct) = spec.strip_suffix('%') {
        pct.parse::<f32>().ok().map(|p| p / 100.0 * total_cells)
    } else {
        spec.parse::<f32>().ok()
    }
}

/// The image's size in cells.
#[allow(clippy::too_many_arguments)]
fn fit(px_w: f32, px_h: f32, cell_w: f32, cell_h: f32, columns: u32, width: &str, height: &str, keep_aspect: bool) -> (u32, u32) {
    let max_cols = columns as f32;
    let w = dimension(width, cell_w, max_cols);
    let h = dimension(height, cell_h, 1000.0);
    let (mut cols, mut rows) = match (w, h) {
        (Some(w), Some(h)) if !keep_aspect => (w, h),
        (Some(w), Some(h)) => {
            let scale = (w * cell_w / px_w).min(h * cell_h / px_h);
            (px_w * scale / cell_w, px_h * scale / cell_h)
        }
        (Some(w), None) => (w, px_h * (w * cell_w / px_w) / cell_h),
        (None, Some(h)) => (px_w * (h * cell_h / px_h) / cell_w, h),
        // Natural size, shrunk to fit the line.
        (None, None) => (px_w / cell_w, px_h / cell_h),
    };
    if cols > max_cols {
        rows *= max_cols / cols;
        cols = max_cols;
    }
    ((cols.ceil() as u32).clamp(1, columns), (rows.ceil() as u32).clamp(1, 256))
}

/// Where the payload ends and the whole sequence ends, if all of it is here.
fn terminator(seq: &[u8]) -> Option<(usize, usize)> {
    seq.iter().enumerate().find_map(|(i, &b)| match b {
        0x07 => Some((i, i + 1)),
        0x1b if seq.get(i + 1) == Some(&b'\\') => Some((i, i + 2)),
        _ => None,
    })
    .filter(|&(end, _)| end > PREFIX.len() - 1)
}

impl Read for Filter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut tmp = [0u8; 65536];
        while self.out.is_empty() {
            match self.file.read(&mut tmp) {
                Ok(0) => {
                    self.scan(true);
                    break;
                }
                Ok(n) => {
                    self.raw.extend_from_slice(&tmp[..n]);
                    self.scan(false);
                }
                Err(e) => return Err(e),
            }
        }
        let n = buf.len().min(self.out.len());
        for (slot, byte) in buf.iter_mut().zip(self.out.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(shared: Arc<Shared>) -> Filter {
        let file = File::open("/dev/null").unwrap();
        Filter::new(file, shared)
    }

    fn png() -> String {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgba8(16, 32)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn an_image_becomes_placeholder_rows_and_text_passes() {
        let shared = Shared::new(WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 });
        let mut f = filter(shared.clone());
        f.raw.extend_from_slice(format!("a\x1b[1mb\x1b]1337;File=inline=1:{}\x07c", png()).as_bytes());
        f.scan(false);
        let out = String::from_utf8(f.out.iter().copied().collect()).unwrap();
        assert!(out.starts_with("a\x1b[1mb\x1b[38;2;0;0;1m"));
        assert!(out.ends_with("\x1b[39mc"));
        // 16x32 px at 8x16 cells is 2 columns and 2 rows.
        assert_eq!(out.matches('\u{10EE00}').count(), 2);
        assert_eq!(out.matches('\u{10EE01}').count(), 2);
        assert_eq!(shared.incoming.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_sequence_split_across_reads_waits() {
        let shared = Shared::new(WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 });
        let mut f = filter(shared.clone());
        let seq = format!("\x1b]1337;File=inline=1:{}\x07", png());
        let (a, b) = seq.split_at(20);
        f.raw.extend_from_slice(a.as_bytes());
        f.scan(false);
        assert!(f.out.is_empty());
        f.raw.extend_from_slice(b.as_bytes());
        f.scan(false);
        assert!(!f.out.is_empty());
    }

    #[test]
    fn a_multipart_image_is_put_together() {
        let shared = Shared::new(WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 });
        let mut f = filter(shared.clone());
        let data = png();
        let (a, b) = data.split_at(data.len() / 2);
        let text = format!(
            "\x1b]1337;MultipartFile=inline=1;size=9\x07\x1b]1337;FilePart={a}\x07\x1b]1337;FilePart={b}\x07\x1b]1337;FileEnd\x07x"
        );
        f.raw.extend_from_slice(text.as_bytes());
        f.scan(false);
        let out = String::from_utf8(f.out.iter().copied().collect()).unwrap();
        assert_eq!(out.matches('\u{10EE00}').count(), 2);
        assert!(out.ends_with("\x1b[39mx"));
        assert_eq!(shared.incoming.lock().unwrap().len(), 1);
    }

    #[test]
    fn other_escapes_pass_through() {
        let shared = Shared::new(WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 });
        let mut f = filter(shared);
        f.raw.extend_from_slice(b"\x1b[31mred\x1b]0;title\x07");
        f.scan(false);
        assert_eq!(f.out.iter().copied().collect::<Vec<_>>(), b"\x1b[31mred\x1b]0;title\x07");
    }

    #[test]
    fn sizes_fit_the_line() {
        assert_eq!(fit(160.0, 160.0, 8.0, 16.0, 10, "", "", true), (10, 5));
        assert_eq!(fit(80.0, 80.0, 8.0, 16.0, 80, "4", "", true), (4, 2));
    }
}
