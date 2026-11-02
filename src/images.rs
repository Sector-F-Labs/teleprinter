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

use std::collections::{BTreeMap, HashMap};
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
/// What a pane's images may take as pixels (RGBA) before the oldest are dropped.
const TEXTURE_BUDGET: usize = 256 << 20;
/// Most images, and bytes of them, kept for a later kitty `a=p`.
const MAX_STORED: usize = 64;
const STORED_BUDGET: usize = 64 << 20;

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
    /// What it takes in memory as sent.
    fn len(&self) -> usize {
        match self {
            Source::Encoded(bytes) => bytes.len(),
            Source::Rgba { data, .. } => data.len(),
        }
    }

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
    Add { id: u32, source: Arc<Source>, cols: u32, rows: u32, kitty: Option<u32>, bytes: usize },
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
pub struct Images {
    /// By id, so the oldest is first.
    entries: BTreeMap<u32, Entry>,
    budget: usize,
}

impl Default for Images {
    fn default() -> Self {
        Self::with_budget(TEXTURE_BUDGET)
    }
}

struct Entry {
    /// Its pixels as RGBA, what it costs once it's a texture.
    bytes: usize,
    source: Option<Arc<Source>>,
    cols: u32,
    rows: u32,
    kitty: Option<u32>,
    texture: Option<TextureHandle>,
    /// The pixels couldn't be decoded: not tried again.
    failed: bool,
}

impl Images {
    fn with_budget(budget: usize) -> Self {
        Self { entries: BTreeMap::new(), budget }
    }

    /// Drops the oldest images until the rest fit the budget. The newest stays whatever its size.
    fn trim(&mut self) {
        let mut total: usize = self.entries.values().map(|e| e.bytes).sum();
        while total > self.budget && self.entries.len() > 1 {
            let (_, oldest) = self.entries.pop_first().unwrap();
            total -= oldest.bytes;
        }
    }

    pub fn take(&mut self, shared: &Shared) {
        for event in shared.events.lock().unwrap().drain(..) {
            match event {
                ImageEvent::Add { id, source, cols, rows, kitty, bytes } => {
                    self.entries.insert(id, Entry { bytes, source: Some(source), cols, rows, kitty, texture: None, failed: false });
                    self.trim();
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
        if e.failed {
            return None;
        }
        if e.texture.is_none() {
            let color = e.source.take().and_then(|source| match &*source {
                Source::Encoded(bytes) => {
                    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
                    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
                }
                Source::Rgba { width, height, data } => {
                    Some(ColorImage::from_rgba_unmultiplied([*width as usize, *height as usize], data))
                }
            });
            let Some(color) = color else {
                e.failed = true;
                return None;
            };
            e.texture = Some(ctx.load_texture(format!("inline-image-{id}"), color, TextureOptions::LINEAR));
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
        Ok(Self { inner, reader: Filter::new(file, shared.clone())?, shared })
    }
}

impl EventedReadWrite for ImagePty {
    type Reader = Filter;
    type Writer = File;

    unsafe fn register(&mut self, p: &Arc<Poller>, e: Event, m: PollMode) -> io::Result<()> {
        unsafe {
            self.inner.register(p, e, m)?;
            // The wake pipe reports under the pty's own key, so the loop reads the pty for it.
            p.add_with_mode(&self.reader.wake.read, Event::readable(e.key), PollMode::Level)
        }
    }
    fn reregister(&mut self, p: &Arc<Poller>, e: Event, m: PollMode) -> io::Result<()> {
        self.inner.reregister(p, e, m)
    }
    fn deregister(&mut self, p: &Arc<Poller>) -> io::Result<()> {
        let _ = p.delete(&self.reader.wake.read);
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

/// A pipe that makes the event loop's poller report the pty readable while output is
/// waiting in the filter. The kernel's buffer may be empty by then, and alacritty's
/// loop would sleep on it with the rest of the output unread.
struct Wake {
    read: File,
    write: File,
    signaled: bool,
}

impl Wake {
    fn new() -> io::Result<Self> {
        use std::os::fd::FromRawFd;
        let mut fds = [0i32; 2];
        // SAFETY: pipe2 fills `fds` with two new descriptors, which the Files then own.
        unsafe {
            if libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { read: File::from_raw_fd(fds[0]), write: File::from_raw_fd(fds[1]), signaled: false })
        }
    }

    fn set(&mut self, on: bool) {
        if on && !self.signaled {
            let _ = (&self.write).write(&[1]);
            self.signaled = true;
        } else if !on && self.signaled {
            let mut byte = [0u8; 16];
            let _ = (&self.read).read(&mut byte);
            self.signaled = false;
        }
    }
}

/// Reads the pty, passing everything but image sequences through.
pub struct Filter {
    file: File,
    shared: Arc<Shared>,
    wake: Wake,
    /// The start of a sequence that may still be arriving, from its ESC on.
    pending: Vec<u8>,
    /// How much of `pending` was already searched for its end.
    searched: usize,
    /// Filtered output, and how much of it the event loop has taken.
    out: Vec<u8>,
    out_pos: usize,
    /// An iTerm2 multipart image (what `imgcat` sends): its parameters and the base64 so far.
    multipart: Option<(Vec<u8>, Vec<u8>)>,
    /// A kitty transfer in chunks: the first chunk's keys and the base64 so far.
    chunked: Option<(Keys, Vec<u8>)>,
    /// Images sent with an id, for a later `a=p`, oldest first, and what they add up to.
    stored: Vec<(u32, Arc<Source>)>,
    stored_bytes: usize,
}

impl Filter {
    fn new(file: File, shared: Arc<Shared>) -> io::Result<Self> {
        Ok(Self {
            file,
            shared,
            wake: Wake::new()?,
            pending: Vec::new(),
            searched: 0,
            out: Vec::new(),
            out_pos: 0,
            multipart: None,
            chunked: None,
            stored: Vec::new(),
            stored_bytes: 0,
        })
    }

    /// Scans `bytes`, after the held-back start of a sequence if there is one, and
    /// puts what's complete on `out`. `eof` flushes what's held.
    fn feed(&mut self, bytes: &[u8], eof: bool) {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let pos = self.process(&data, eof);
        // Nothing consumed: the whole of it is one unfinished sequence, kept as it is.
        self.pending = if pos == 0 { data } else { data[pos..].to_vec() };
        self.searched = self.pending.len();
    }

    /// Returns how much of `data` was dealt with; the rest is a sequence still arriving.
    fn process(&mut self, data: &[u8], eof: bool) -> usize {
        let mut pos = 0;
        while pos < data.len() {
            let Some(esc) = data[pos..].iter().position(|&b| b == 0x1b).map(|i| pos + i) else {
                self.out.extend_from_slice(&data[pos..]);
                return data.len();
            };
            self.out.extend_from_slice(&data[pos..esc]);
            pos = esc;
            let rest = &data[esc..];
            let prefix = if rest.starts_with(OSC) {
                Some(OSC)
            } else if rest.starts_with(APC) {
                Some(APC)
            } else {
                None
            };
            match prefix {
                Some(prefix) => {
                    // Only the held-back sequence, at the start of the data, was searched before.
                    let from = if esc == 0 { self.searched } else { 0 };
                    match terminator(rest, prefix, from) {
                        Some((payload_end, end)) => {
                            let body = &rest[prefix.len()..payload_end];
                            let replacement = if prefix == OSC { self.osc(body) } else { Some(self.apc(body)) };
                            match replacement {
                                Some(replacement) => self.out.extend(replacement),
                                // Some other OSC 1337: the terminal's to ignore.
                                None => self.out.extend_from_slice(&rest[..end]),
                            }
                            pos = esc + end;
                        }
                        None if rest.len() > MAX_SEQUENCE => {
                            // Too big to be one: let it through as text.
                            self.out.push(0x1b);
                            pos += 1;
                        }
                        None if eof => {
                            self.out.extend_from_slice(rest);
                            return data.len();
                        }
                        None => return pos,
                    }
                }
                None if !eof && (OSC.starts_with(rest) || APC.starts_with(rest)) => return pos,
                None => {
                    self.out.push(0x1b);
                    pos += 1;
                }
            }
        }
        pos
    }

    /// Hands the event loop what's waiting in `out`, as much as fits.
    fn take_out(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.out.len() - self.out_pos);
        buf[..n].copy_from_slice(&self.out[self.out_pos..self.out_pos + n]);
        self.out_pos += n;
        n
    }

    fn fill(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.out_pos < self.out.len() {
            return Ok(self.take_out(buf));
        }
        self.out.clear();
        self.out_pos = 0;
        loop {
            let n = self.file.read(buf)?;
            if n == 0 {
                self.feed(&[], true);
                return Ok(self.take_out(buf));
            }
            // Plain output goes straight through, untouched.
            if self.pending.is_empty() && !buf[..n].contains(&0x1b) {
                return Ok(n);
            }
            let input = buf[..n].to_vec();
            self.feed(&input, false);
            if self.out_pos < self.out.len() {
                return Ok(self.take_out(buf));
            }
        }
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
                        self.store(id, source.clone());
                    }
                    self.reply(&keys, Ok(()));
                    if action == 'T' { self.display(source, &keys, id) } else { Vec::new() }
                }
                Err(e) => {
                    self.reply(&keys, Err(e));
                    Vec::new()
                }
            },
            'p' => match id.and_then(|id| self.stored.iter().find(|(k, _)| *k == id).map(|(_, s)| s.clone())) {
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
                    self.stored.retain(|(k, _)| kitty.is_some_and(|id| *k != id));
                    self.stored_bytes = self.stored.iter().map(|(_, s)| s.len()).sum();
                }
                self.shared.events.lock().unwrap().push(ImageEvent::Delete { cutoff, kitty });
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Keeps an image for a later `a=p`, within `STORED_BUDGET`: the oldest go first.
    fn store(&mut self, id: u32, source: Arc<Source>) {
        self.stored.retain(|(k, _)| *k != id);
        self.stored.push((id, source));
        self.stored_bytes = self.stored.iter().map(|(_, s)| s.len()).sum();
        while self.stored.len() > 1 && (self.stored.len() > MAX_STORED || self.stored_bytes > STORED_BUDGET) {
            let (_, oldest) = self.stored.remove(0);
            self.stored_bytes -= oldest.len();
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
                read_image_file(std::path::Path::new(&path), medium == "t")?
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
        // A small file can claim a huge picture; its texture would be bigger than we keep.
        if px_w as usize * px_h as usize * 4 > TEXTURE_BUDGET {
            return Vec::new();
        }
        let columns = self.shared.columns.load(Ordering::Relaxed).max(1);
        let cell_w = self.shared.cell_w.load(Ordering::Relaxed).max(1) as f32;
        let cell_h = self.shared.cell_h.load(Ordering::Relaxed).max(1) as f32;
        let (cols, rows) = fit(px_w as f32, px_h as f32, cell_w, cell_h, columns, width, height, keep_aspect);
        let id = (self.shared.last_id.fetch_add(1, Ordering::Relaxed) + 1) & 0xFF_FFFF;
        self.shared.events.lock().unwrap().push(ImageEvent::Add { id, source, cols, rows, kitty, bytes: px_w as usize * px_h as usize * 4 });

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

/// The contents of a file a program named, for kitty's `t=f` and `t=t`. Only a regular
/// file outside /proc, /sys and /dev, of a size we'd take: anything else (a device that
/// never ends, a pipe that never answers) would hang or fill the pty thread. A temporary
/// file (`t=t`) is the terminal's to delete, but only in the temp directory and when
/// named for the protocol.
fn read_image_file(path: &std::path::Path, temporary: bool) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let real = path.canonicalize().map_err(|_| "ENOENT:no such file".to_string())?;
    if ["/proc", "/sys", "/dev"].iter().any(|dir| real.starts_with(dir)) {
        return Err("EPERM:not a place to read images from".into());
    }
    // Non-blocking, so opening a pipe doesn't wait for a writer.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&real)
        .map_err(|_| "ENOENT:can't open the file".to_string())?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err("EBADF:not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SEQUENCE as u64 + 1).read_to_end(&mut bytes).map_err(|_| "EIO:can't read the file".to_string())?;
    if bytes.len() > MAX_SEQUENCE {
        return Err("EFBIG:file too big".into());
    }
    let named_for_it = real.to_string_lossy().contains("tty-graphics-protocol");
    if temporary && named_for_it && real.starts_with(std::env::temp_dir()) {
        let _ = std::fs::remove_file(&real);
    }
    Ok(bytes)
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
/// OSC ends with BEL or ESC \; APC only with ESC \. `from` is how far an earlier try got.
fn terminator(seq: &[u8], prefix: &[u8], from: usize) -> Option<(usize, usize)> {
    // One back: the ESC of an ST may have been the last byte then.
    (prefix.len().max(from.saturating_sub(1))..seq.len()).find_map(|i| match seq[i] {
        0x07 if prefix == OSC => Some((i, i + 1)),
        0x1b if seq.get(i + 1) == Some(&b'\\') => Some((i, i + 2)),
        _ => None,
    })
}

impl Read for Filter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let result = self.fill(buf);
        // Output that didn't fit waits for the next call; see `Wake`.
        let waiting = self.out_pos < self.out.len();
        self.wake.set(waiting);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    const SIZE: WindowSize = WindowSize { num_cols: 80, num_lines: 24, cell_width: 8, cell_height: 16 };

    fn filter(shared: Arc<Shared>) -> Filter {
        Filter::new(File::open("/dev/null").unwrap(), shared).unwrap()
    }

    /// A filter whose pty is one end of a socket pair: the other end hears the replies.
    fn talking(shared: Arc<Shared>) -> (Filter, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        b.set_nonblocking(true).unwrap();
        (Filter::new(File::from(std::os::fd::OwnedFd::from(a)), shared).unwrap(), b)
    }

    fn png() -> String {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgba8(16, 32)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn out(f: &Filter) -> String {
        String::from_utf8(f.out[f.out_pos..].to_vec()).unwrap()
    }

    fn events(shared: &Shared) -> usize {
        shared.events.lock().unwrap().len()
    }

    #[test]
    fn an_image_becomes_placeholder_rows_and_text_passes() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        f.feed(format!("a\x1b[1mb\x1b]1337;File=inline=1:{}\x07c", png()).as_bytes(), false);
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
        f.feed(a.as_bytes(), false);
        assert!(f.out.is_empty());
        f.feed(b.as_bytes(), false);
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
        f.feed(text.as_bytes(), false);
        let out = out(&f);
        assert_eq!(out.matches('\u{10EE00}').count(), 2);
        assert!(out.ends_with("\x1b[39mx"));
        assert_eq!(events(&shared), 1);
    }

    #[test]
    fn other_escapes_pass_through() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared);
        f.feed(b"\x1b[31mred\x1b]0;title\x07\x1b_Xother\x1b\\", false);
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
        f.feed(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c", false);
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
        f.feed(format!("\x1b_Ga=T,f=100,i=7,m=1;{a}\x1b\\\x1b_Gm=0;{b}\x1b\\x").as_bytes(), false);
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
        f.feed(format!("\x1b_Ga=T,f=100,i=7,q=2,C=1,c=3,r=2;{}\x1b\\", png()).as_bytes(), false);
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
        f.feed(format!("\x1b_Ga=T,f=24,s=2,v=2,i=1;{rgb}\x1b\\").as_bytes(), false);
        f.feed(format!("\x1b_Ga=T,f=24,s=3,v=3,i=2;{rgb}\x1b\\").as_bytes(), false);
        assert_eq!(out(&f).matches('\u{10EE00}').count(), 1);
        let mut reply = String::new();
        peer.read_to_string(&mut reply).ok();
        assert!(reply.starts_with("\x1b_Gi=1;OK\x1b\\\x1b_Gi=2;EINVAL"));
    }

    #[test]
    fn deleting_removes_the_images_up_to_then() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        f.feed(format!("\x1b_Ga=T,f=100,i=1;{p}\x1b\\\x1b_Ga=d,d=A\x1b\\\x1b_Ga=T,f=100,i=1;{p}\x1b\\", p = png()).as_bytes(), false);
        let mut images = Images::default();
        images.take(&shared);
        // The first is gone; the one drawn after the delete stays.
        assert_eq!(images.entries.keys().copied().collect::<Vec<_>>(), vec![2]);
    }

    /// Four 2048-byte images (16x32 px RGBA) shown to an `Images` with `budget`.
    fn four_images(budget: usize) -> Vec<u32> {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        for _ in 0..4 {
            f.feed(format!("\x1b]1337;File=inline=1:{}\x07", png()).as_bytes(), false);
        }
        let mut images = Images::with_budget(budget);
        images.take(&shared);
        images.entries.keys().copied().collect()
    }

    #[test]
    fn the_oldest_images_go_when_over_budget() {
        assert_eq!(four_images(5000), vec![3, 4]);
        // One bigger than the whole budget is still shown.
        assert_eq!(four_images(10), vec![4]);
        assert_eq!(four_images(1 << 20), vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_kitty_put_shows_a_stored_image_again() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        f.feed(format!("\x1b_Ga=t,f=100,i=5;{}\x1b\\\x1b_Ga=p,i=5\x1b\\", png()).as_bytes(), false);
        assert_eq!(events(&shared), 1);
        assert_eq!(out(&f).matches('\u{10EE00}').count(), 2);
    }

    #[test]
    fn only_regular_files_are_read() {
        let dir = std::env::temp_dir().join(format!("tp-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.png");
        std::fs::write(&file, b"data").unwrap();
        assert_eq!(read_image_file(&file, false).unwrap(), b"data");
        // Not deleted: not marked as the protocol's temporary file.
        assert!(file.exists());
        assert!(read_image_file(std::path::Path::new("/dev/zero"), false).is_err());
        assert!(read_image_file(std::path::Path::new("/proc/self/environ"), false).is_err());
        assert!(read_image_file(&dir, false).is_err());
        let fifo = dir.join("pipe");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(read_image_file(&fifo, false).is_err());
        let marked = dir.join("tty-graphics-protocol-x");
        std::fs::write(&marked, b"data").unwrap();
        assert!(read_image_file(&marked, true).is_ok());
        assert!(!marked.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plain_output_passes_straight_through() {
        let (mut f, peer) = talking(Shared::new(SIZE));
        (&peer).write_all(b"hello\r\nworld").unwrap();
        let mut buf = [0u8; 64];
        let n = f.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello\r\nworld");
        assert!(!f.wake.signaled);
    }

    #[test]
    fn output_that_didnt_fit_wakes_the_loop_until_it_is_read() {
        let (mut f, peer) = talking(Shared::new(SIZE));
        (&peer).write_all(format!("\x1b]1337;File=inline=1:{}\x07", png()).as_bytes()).unwrap();
        let mut small = [0u8; 10];
        let mut got = Vec::new();
        let n = f.read(&mut small).unwrap();
        got.extend_from_slice(&small[..n]);
        // The rest of the placeholder cells wait, and the poller is told.
        assert!(f.wake.signaled);
        while f.wake.signaled {
            let n = f.read(&mut small).unwrap();
            got.extend_from_slice(&small[..n]);
        }
        assert_eq!(String::from_utf8(got).unwrap().matches('\u{10EE01}').count(), 2);
    }

    #[test]
    fn a_big_sequence_in_small_pieces_is_found_without_rescanning() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        let seq = format!("\x1b]1337;File=inline=1:{}\x1b\\", png());
        // Split the ST itself between two pieces, and feed a byte at a time before that.
        let bytes = seq.as_bytes();
        for chunk in bytes[..bytes.len() - 1].chunks(7) {
            f.feed(chunk, false);
            assert!(out(&f).is_empty());
        }
        f.feed(&bytes[bytes.len() - 1..], false);
        assert_eq!(out(&f).matches('\u{10EE00}').count(), 2);
        assert!(f.pending.is_empty());
    }

    #[test]
    fn an_image_that_wont_decode_is_given_up_on() {
        let shared = Shared::new(SIZE);
        let mut f = filter(shared.clone());
        let good = base64::engine::general_purpose::STANDARD.decode(png()).unwrap();
        // A real header, cut off before the end of the pixels.
        let bad = base64::engine::general_purpose::STANDARD.encode(&good[..good.len() - 15]);
        f.feed(format!("\x1b]1337;File=inline=1:{bad}\x07").as_bytes(), false);
        let mut images = Images::default();
        images.take(&shared);
        let ctx = egui::Context::default();
        assert!(images.texture(&ctx, 1).is_none());
        assert!(images.entries[&1].failed);
        assert!(images.entries[&1].source.is_none());
        assert!(images.texture(&ctx, 1).is_none());
    }

    #[test]
    fn stored_images_are_kept_within_a_budget() {
        let mut f = filter(Shared::new(SIZE));
        let big = || Arc::new(Source::Rgba { width: 1, height: 1, data: vec![0u8; 40 << 20] });
        f.store(1, big());
        f.store(2, big());
        assert_eq!(f.stored.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec![2]);
        for id in 10..100 {
            f.store(id, Arc::new(Source::Rgba { width: 1, height: 1, data: vec![0; 4] }));
        }
        assert_eq!(f.stored.len(), MAX_STORED);
        assert_eq!(f.stored_bytes, f.stored.iter().map(|(_, s)| s.len()).sum::<usize>());
    }
}
