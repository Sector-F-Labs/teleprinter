//! Inline images: the iTerm2 protocol (`OSC 1337 ; File=… : base64 BEL`, what
//! `imgcat` sends) and the kitty graphics protocol (`APC G … ; base64 ST`, what
//! yazi, chafa, `kitten icat` and timg send). alacritty_terminal ignores both
//! and its event loop has no hook, so the pty is wrapped: `ImagePty` filters
//! what the loop reads, takes the image out of the stream and leaves cells in
//! its place. Each cell holds a private-use character that names its row
//! (`U+10EE00 + row`) and has the image's id as its foreground colour, so the
//! image scrolls, wraps and is overwritten with the text around it. The view
//! paints those cells as slices of the image's texture.
//!
//! Kitty's queries are answered from the filter (it writes to the pty), which
//! is how programs find out the terminal can do graphics.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use base64::Engine;
use egui::{ColorImage, TextureHandle, TextureOptions};
use polling::{Event, PollMode, Poller};

const OSC: &[u8] = b"\x1b]1337;";
const APC: &[u8] = b"\x1b_G";
/// Base of the row-carrying placeholder characters.
pub const PLACEHOLDER: u32 = 0x10EE00;
/// Largest sequence taken (base64 included); bigger ones are dropped.
const MAX_SEQUENCE: usize = 48 << 20;
/// Most images kept for a later kitty `a=p`.
const MAX_STORED: usize = 64;

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

/// An image's pixels, as the program sent them.
enum Source {
    /// PNG, JPEG, GIF or WebP.
    Encoded(Vec<u8>),
    Rgba { width: u32, height: u32, data: Vec<u8> },
}

impl Source {
    fn size(&self) -> Option<(u32, u32)> {
        match self {
            Source::Rgba { width, height, .. } => Some((*width, *height)),
            Source::Encoded(bytes) => {
                image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
            }
        }
    }
}

/// What the pty thread tells the UI, in stream order.
enum ImageEvent {
    Add { id: u32, source: Arc<Source>, cols: u32, rows: u32, kitty: Option<u32> },
    /// Images up to `cutoff` (the last id made) are gone, all or just kitty id `kitty`.
    Delete { cutoff: u32, kitty: Option<u32> },
}

/// What the pty thread and the UI share: the pane's size, for sizing images,
/// and the events waiting for the UI.
#[derive(Default)]
pub struct Shared {
    columns: AtomicU32,
    cell_w: AtomicU32,
    cell_h: AtomicU32,
    last_id: AtomicU32,
    events: Mutex<Vec<ImageEvent>>,
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
    source: Option<Arc<Source>>,
    cols: u32,
    rows: u32,
    kitty: Option<u32>,
    texture: Option<TextureHandle>,
}

impl Images {
    pub fn take(&mut self, shared: &Shared) {
        for event in shared.events.lock().unwrap().drain(..) {
            match event {
                ImageEvent::Add { id, source, cols, rows, kitty } => {
                    self.entries.insert(id, Entry { source: Some(source), cols, rows, kitty, texture: None });
                }
                ImageEvent::Delete { cutoff, kitty } => {
                    self.entries.retain(|&id, e| id > cutoff || kitty.is_some_and(|k| e.kitty != Some(k)));
                }
            }
        }
    }

    /// The texture of image `id` and its size in cells. `None` for an image that
    /// was deleted, which leaves its cells blank.
    pub fn texture(&mut self, ctx: &egui::Context, id: u32) -> Option<(TextureHandle, u32, u32)> {
        let e = self.entries.get_mut(&id)?;
        if e.texture.is_none() {
            let color = match &**e.source.as_ref()? {
                Source::Encoded(bytes) => {
                    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
                    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
                }
                Source::Rgba { width, height, data } => {
                    ColorImage::from_rgba_unmultiplied([*width as usize, *height as usize], data)
                }
            };
            e.texture = Some(ctx.load_texture(format!("inline-image-{id}"), color, TextureOptions::LINEAR));
            e.source = None;
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

/// The control data of a kitty command: `a=T,f=100,m=1`.
type Keys = HashMap<u8, String>;

fn parse_keys(control: &[u8]) -> Keys {
    String::from_utf8_lossy(control)
        .split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k.len() == 1).then(|| (k.as_bytes()[0], v.to_string()))
        })
        .collect()
}

fn num(keys: &Keys, key: u8) -> Option<u32> {
    keys.get(&key)?.parse().ok()
}

/// Reads the pty, passing everything but image sequences through.
pub struct Filter {
    file: File,
    shared: Arc<Shared>,
    /// Read but not yet scanned: a sequence that may still be arriving.
    raw: Vec<u8>,
    out: VecDeque<u8>,
    /// An iTerm2 multipart image (what `imgcat` sends): its parameters and the base64 so far.
    multipart: Option<(Vec<u8>, Vec<u8>)>,
    /// A kitty transfer in chunks: the first chunk's keys and the base64 so far.
    chunked: Option<(Keys, Vec<u8>)>,
    /// Images sent with an id, for a later `a=p`.
    stored: HashMap<u32, Arc<Source>>,
}

impl Filter {
    fn new(file: File, shared: Arc<Shared>) -> Self {
        Self {
            file,
            shared,
            raw: Vec::new(),
            out: VecDeque::new(),
            multipart: None,
            chunked: None,
            stored: HashMap::new(),
        }
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
            let prefix = if rest.starts_with(OSC) {
                Some(OSC)
            } else if rest.starts_with(APC) {
                Some(APC)
            } else {
                None
            };
            match prefix {
                Some(prefix) => match terminator(rest, prefix) {
                    Some((payload_end, end)) => {
                        let body = &rest[prefix.len()..payload_end];
                        let replacement = if prefix == OSC { self.osc(body) } else { Some(self.apc(body)) };
                        match replacement {
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
                },
                None if !eof && (OSC.starts_with(rest) || APC.starts_with(rest)) => break,
                None => {
                    self.out.push_back(0x1b);
                    pos += 1;
                }
            }
        }
        self.raw = raw[pos..].to_vec();
    }

    /// What an OSC 1337 body becomes: `None` if it isn't about images.
    fn osc(&mut self, body: &[u8]) -> Option<Vec<u8>> {
        if let Some(rest) = body.strip_prefix(b"File=") {
            return Some(self.iterm_image(rest));
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
            return Some(self.iterm_image(&whole));
        }
        None
    }

    fn iterm_image(&self, params_and_data: &[u8]) -> Vec<u8> {
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
        let Ok(bytes) = base64::engine::general_purpose::STANDARD
            .decode(data.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect::<Vec<_>>())
        else {
            return Vec::new();
        };
        let source = Arc::new(Source::Encoded(bytes));
        self.place(source, None, &width, &height, keep_aspect, false)
    }

    /// A kitty graphics command: what it leaves in the grid, and any reply to the program.
    fn apc(&mut self, body: &[u8]) -> Vec<u8> {
        let (control, payload) = match body.iter().position(|&b| b == b';') {
            Some(i) => (&body[..i], &body[i + 1..]),
            None => (body, &[][..]),
        };
        let keys = parse_keys(control);
        let more = keys.get(&b'm').is_some_and(|m| m == "1");
        if let Some((_, data)) = &mut self.chunked {
            if data.len() + payload.len() > MAX_SEQUENCE {
                self.chunked = None;
                return Vec::new();
            }
            data.extend_from_slice(payload);
            if more {
                return Vec::new();
            }
            let (first, data) = self.chunked.take().unwrap();
            return self.command(first, data);
        }
        if more {
            self.chunked = Some((keys, payload.to_vec()));
            return Vec::new();
        }
        self.command(keys, payload.to_vec())
    }

    fn command(&mut self, keys: Keys, data: Vec<u8>) -> Vec<u8> {
        let action = keys.get(&b'a').and_then(|a| a.chars().next()).unwrap_or('t');
        let id = num(&keys, b'i').or(num(&keys, b'I'));
        match action {
            'q' => {
                let result = self.source(&keys, &data).map(|_| ());
                self.reply(&keys, result);
                Vec::new()
            }
            't' | 'T' => match self.source(&keys, &data) {
                Ok(source) => {
                    let source = Arc::new(source);
                    if let Some(id) = id {
                        if self.stored.len() >= MAX_STORED {
                            self.stored.clear();
                        }
                        self.stored.insert(id, source.clone());
                    }
                    self.reply(&keys, Ok(()));
                    if action == 'T' { self.display(source, &keys, id) } else { Vec::new() }
                }
                Err(e) => {
                    self.reply(&keys, Err(e));
                    Vec::new()
                }
            },
            'p' => match id.and_then(|id| self.stored.get(&id).cloned()) {
                Some(source) => {
                    self.reply(&keys, Ok(()));
                    self.display(source, &keys, id)
                }
                None => {
                    self.reply(&keys, Err("ENOENT:no such image".into()));
                    Vec::new()
                }
            },
            'd' => {
                let what = keys.get(&b'd').map(String::as_str).unwrap_or("a");
                let cutoff = self.shared.last_id.load(Ordering::Relaxed);
                let kitty = match what {
                    "a" | "A" => None,
                    "i" | "I" => match id {
                        Some(id) => Some(id),
                        None => return Vec::new(),
                    },
                    _ => return Vec::new(),
                };
                if what.chars().all(|c| c.is_ascii_uppercase()) {
                    match kitty {
                        Some(k) => drop(self.stored.remove(&k)),
                        None => self.stored.clear(),
                    }
                }
                self.shared.events.lock().unwrap().push(ImageEvent::Delete { cutoff, kitty });
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The image a transmit command carries.
    fn source(&self, keys: &Keys, data: &[u8]) -> Result<Source, String> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let decode = |d: &[u8]| {
            b64.decode(d.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect::<Vec<_>>())
                .map_err(|_| "EINVAL:bad base64".to_string())
        };
        let medium = keys.get(&b't').map(String::as_str).unwrap_or("d");
        let mut raw = match medium {
            "d" => decode(data)?,
            "f" | "t" => {
                let path = String::from_utf8(decode(data)?).map_err(|_| "EINVAL:bad path".to_string())?;
                let bytes = std::fs::read(&path).map_err(|e| format!("ENOENT:{e}"))?;
                // A temporary file is the terminal's to delete, if it's marked as one.
                if medium == "t" && path.contains("tty-graphics-protocol") {
                    let _ = std::fs::remove_file(&path);
                }
                bytes
            }
            _ => return Err("EINVAL:only direct and file transmission are supported".into()),
        };
        if keys.get(&b'o').is_some_and(|o| o == "z") {
            let mut inflated = Vec::new();
            flate2::read::ZlibDecoder::new(&raw[..])
                .take(MAX_SEQUENCE as u64)
                .read_to_end(&mut inflated)
                .map_err(|_| "EINVAL:bad compressed data".to_string())?;
            raw = inflated;
        }
        match num(keys, b'f').unwrap_or(32) {
            100 => {
                let source = Source::Encoded(raw);
                source.size().ok_or_else(|| "EBADPNG:can't read the image".to_string())?;
                Ok(source)
            }
            f @ (24 | 32) => {
                let (Some(width), Some(height)) = (num(keys, b's'), num(keys, b'v')) else {
                    return Err("EINVAL:raw pixels need s and v".into());
                };
                let channels = if f == 24 { 3 } else { 4 };
                if width == 0 || height == 0 || raw.len() != width as usize * height as usize * channels {
                    return Err("EINVAL:wrong amount of pixel data".into());
                }
                let data = if f == 24 {
                    raw.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
                } else {
                    raw
                };
                Ok(Source::Rgba { width, height, data })
            }
            _ => Err("EINVAL:unsupported format".into()),
        }
    }

    /// Tells the program how a command went: only when it gave an id, and `q` allows.
    fn reply(&self, keys: &Keys, result: Result<(), String>) {
        let Some(id) = num(keys, b'i').or(num(keys, b'I')) else { return };
        let quiet = num(keys, b'q').unwrap_or(0);
        let message = match result {
            Ok(()) if quiet == 0 => "OK".to_string(),
            Err(e) if quiet < 2 => e,
            _ => return,
        };
        let _ = (&self.file).write_all(format!("\x1b_Gi={id};{message}\x1b\\").as_bytes());
    }

    fn display(&self, source: Arc<Source>, keys: &Keys, kitty: Option<u32>) -> Vec<u8> {
        let (c, r) = (keys.get(&b'c'), keys.get(&b'r'));
        // Both given means exactly that many cells; one means the other follows the aspect.
        let keep_aspect = !(c.is_some() && r.is_some());
        let keep_cursor = num(keys, b'C') == Some(1);
        self.place(source, kitty, c.map(String::as_str).unwrap_or(""), r.map(String::as_str).unwrap_or(""), keep_aspect, keep_cursor)
    }

    /// The bytes that leave room for the image in the grid: rows of placeholder
    /// cells, each but the last followed by a line feed back at the start column.
    /// The cursor ends after the last row, as in iTerm2 and kitty, or where it
    /// was when `keep_cursor`.
    fn place(&self, source: Arc<Source>, kitty: Option<u32>, width: &str, height: &str, keep_aspect: bool, keep_cursor: bool) -> Vec<u8> {
        let Some((px_w, px_h)) = source.size() else { return Vec::new() };
        let columns = self.shared.columns.load(Ordering::Relaxed).max(1);
        let cell_w = self.shared.cell_w.load(Ordering::Relaxed).max(1) as f32;
        let cell_h = self.shared.cell_h.load(Ordering::Relaxed).max(1) as f32;
        let (cols, rows) = fit(px_w as f32, px_h as f32, cell_w, cell_h, columns, width, height, keep_aspect);
        let id = (self.shared.last_id.fetch_add(1, Ordering::Relaxed) + 1) & 0xFF_FFFF;
        self.shared.events.lock().unwrap().push(ImageEvent::Add { id, source, cols, rows, kitty });

        let [r, g, b] = id_color(id);
        // ESC 7 saves the colour with the cursor, so ESC 8 puts both back.
        let mut out = if keep_cursor { b"\x1b7".to_vec() } else { Vec::new() };
        out.extend(format!("\x1b[38;2;{r};{g};{b}m").into_bytes());
        for row in 0..rows {
            let ch = char::from_u32(PLACEHOLDER + row.min(255)).unwrap();
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            for _ in 0..cols {
                out.extend(s.as_bytes());
            }
            if row + 1 < rows {
                // Down a row without scrolling when the cursor is to be kept.
                let down: &[u8] = if keep_cursor { b"\x1b[B" } else { b"\n" };
                out.extend(format!("\x1b[{cols}D").into_bytes());
                out.extend(down);
            }
        }
        out.extend(if keep_cursor { &b"\x1b8"[..] } else { &b"\x1b[39m"[..] });
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
/// OSC ends with BEL or ESC \; APC only with ESC \.
fn terminator(seq: &[u8], prefix: &[u8]) -> Option<(usize, usize)> {
    (prefix.len()..seq.len()).find_map(|i| match seq[i] {
        0x07 if prefix == OSC => Some((i, i + 1)),
        0x1b if seq.get(i + 1) == Some(&b'\\') => Some((i, i + 2)),
        _ => None,
    })
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
    use std::os::unix::net::UnixStream;

    const SIZE: WindowSize = WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 };

    fn filter(shared: Arc<Shared>) -> Filter {
        Filter::new(File::open("/dev/null").unwrap(), shared)
    }

    /// A filter whose pty is one end of a socket pair: the other end hears the replies.
    fn talking(shared: Arc<Shared>) -> (Filter, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        b.set_nonblocking(true).unwrap();
        (Filter::new(File::from(std::os::fd::OwnedFd::from(a)), shared), b)
    }

    fn png() -> String {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgba8(16, 32)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn out(f: &Filter) -> String {
        String::from_utf8(f.out.iter().copied().collect()).unwrap()
    }

    fn events(shared: &Shared) -> usize {
        shared.events.lock().unwrap().len()
    }

    #[test]
    fn an_image_becomes_placeholder_rows_and_text_passes() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        f.raw.extend_from_slice(format!("a\x1b[1mb\x1b]1337;File=inline=1:{}\x07c", png()).as_bytes());
        f.scan(false);
        let out = out(&f);
        assert!(out.starts_with("a\x1b[1mb\x1b[38;2;0;0;1m"));
        assert!(out.ends_with("\x1b[39mc"));
        // 16x32 px at 8x16 cells is 2 columns and 2 rows.
        assert_eq!(out.matches('\u{10EE00}').count(), 2);
        assert_eq!(out.matches('\u{10EE01}').count(), 2);
        assert_eq!(events(&shared), 1);
    }

    #[test]
    fn a_sequence_split_across_reads_waits() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared);
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
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        let data = png();
        let (a, b) = data.split_at(data.len() / 2);
        let text = format!(
            "\x1b]1337;MultipartFile=inline=1;size=9\x07\x1b]1337;FilePart={a}\x07\x1b]1337;FilePart={b}\x07\x1b]1337;FileEnd\x07x"
        );
        f.raw.extend_from_slice(text.as_bytes());
        f.scan(false);
        let out = out(&f);
        assert_eq!(out.matches('\u{10EE00}').count(), 2);
        assert!(out.ends_with("\x1b[39mx"));
        assert_eq!(events(&shared), 1);
    }

    #[test]
    fn other_escapes_pass_through() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared);
        f.raw.extend_from_slice(b"\x1b[31mred\x1b]0;title\x07\x1b_Xother\x1b\\");
        f.scan(false);
        assert_eq!(out(&f), "\x1b[31mred\x1b]0;title\x07\x1b_Xother\x1b\\");
    }

    #[test]
    fn sizes_fit_the_line() {
        assert_eq!(fit(160.0, 160.0, 8.0, 16.0, 10, "", "", true), (10, 5));
        assert_eq!(fit(80.0, 80.0, 8.0, 16.0, 80, "4", "", true), (4, 2));
    }

    #[test]
    fn a_kitty_query_is_answered() {
        let (mut f, mut peer) = talking(Shared::new(SIZE));
        f.raw.extend_from_slice(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c");
        f.scan(false);
        // Nothing is drawn, and the device attributes request still reaches the terminal.
        assert_eq!(out(&f), "\x1b[c");
        let mut reply = String::new();
        peer.read_to_string(&mut reply).ok();
        assert_eq!(reply, "\x1b_Gi=31;OK\x1b\\");
    }

    #[test]
    fn a_kitty_png_in_chunks_is_displayed() {
        let shared = Shared::new(SIZE);
        let (mut f, mut peer) = talking(shared.clone());
        let data = png();
        let (a, b) = data.split_at(data.len() / 2);
        f.raw.extend_from_slice(format!("\x1b_Ga=T,f=100,i=7,m=1;{a}\x1b\\\x1b_Gm=0;{b}\x1b\\x").as_bytes());
        f.scan(false);
        let out = out(&f);
        assert_eq!(out.matches('\u{10EE00}').count(), 2);
        assert!(out.ends_with("\x1b[39mx"));
        assert_eq!(events(&shared), 1);
        let mut reply = String::new();
        peer.read_to_string(&mut reply).ok();
        assert_eq!(reply, "\x1b_Gi=7;OK\x1b\\");
    }

    #[test]
    fn q_2_keeps_quiet_and_c_1_keeps_the_cursor() {
        let shared = Shared::new(SIZE);
        let (mut f, mut peer) = talking(shared);
        f.raw.extend_from_slice(format!("\x1b_Ga=T,f=100,i=7,q=2,C=1,c=3,r=2;{}\x1b\\", png()).as_bytes());
        f.scan(false);
        let out = out(&f);
        assert!(out.starts_with("\x1b7\x1b[38;2;0;0;1m"));
        assert!(out.contains("\x1b[3D\x1b[B"));
        assert!(out.ends_with("\x1b8"));
        let mut reply = String::new();
        peer.read_to_string(&mut reply).ok();
        assert_eq!(reply, "");
    }

    #[test]
    fn raw_pixels_and_errors() {
        let (mut f, mut peer) = talking(Shared::new(SIZE));
        let rgb = base64::engine::general_purpose::STANDARD.encode([0u8; 2 * 2 * 3]);
        f.raw.extend_from_slice(format!("\x1b_Ga=T,f=24,s=2,v=2,i=1;{rgb}\x1b\\").as_bytes());
        f.raw.extend_from_slice(format!("\x1b_Ga=T,f=24,s=3,v=3,i=2;{rgb}\x1b\\").as_bytes());
        f.scan(false);
        assert_eq!(out(&f).matches('\u{10EE00}').count(), 1);
        let mut reply = String::new();
        peer.read_to_string(&mut reply).ok();
        assert!(reply.starts_with("\x1b_Gi=1;OK\x1b\\\x1b_Gi=2;EINVAL"));
    }

    #[test]
    fn deleting_removes_the_images_up_to_then() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        f.raw.extend_from_slice(format!("\x1b_Ga=T,f=100,i=1;{p}\x1b\\\x1b_Ga=d,d=A\x1b\\\x1b_Ga=T,f=100,i=1;{p}\x1b\\", p = png()).as_bytes());
        f.scan(false);
        let mut images = Images::default();
        images.take(&shared);
        // The first is gone; the one drawn after the delete stays.
        assert_eq!(images.entries.keys().copied().collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn a_kitty_put_shows_a_stored_image_again() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        f.raw.extend_from_slice(format!("\x1b_Ga=t,f=100,i=5;{}\x1b\\\x1b_Ga=p,i=5\x1b\\", png()).as_bytes());
        f.scan(false);
        assert_eq!(events(&shared), 1);
        assert_eq!(out(&f).matches('\u{10EE00}').count(), 2);
    }
}
