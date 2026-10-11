//! One shell on a pty, run by alacritty_terminal (ADR-0001). Its event loop
//! reads the pty on its own thread and updates the `Term`; this side writes
//! input, resizes, and turns the loop's events into window changes. Each
//! event asks egui for a repaint, so the window redraws when the shell
//! writes and is otherwise idle.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;

use crate::palette::Palette;

#[derive(Clone)]
pub struct Listener {
    events: Sender<Event>,
    ctx: egui::Context,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.events.send(event);
        self.ctx.request_repaint();
    }
}

/// The grid size in cells, for `Term::new` and `Term::resize`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GridSize {
    pub columns: usize,
    pub lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

pub struct Session {
    pub term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    events: Receiver<Event>,
    window_size: WindowSize,
    /// Set by the shell with OSC 0/2; `None` until then.
    pub title: Option<String>,
    pub exited: bool,
}

impl Session {
    /// Starts `command` (the user's login shell when `None`) in `cwd`.
    pub fn spawn(
        ctx: &egui::Context,
        command: Option<(String, Vec<String>)>,
        cwd: Option<std::path::PathBuf>,
        window_size: WindowSize,
    ) -> std::io::Result<Self> {
        tty::setup_env();
        let options = tty::Options {
            shell: command.map(|(program, args)| tty::Shell::new(program, args)),
            working_directory: cwd,
            drain_on_exit: true,
            env: HashMap::from([("TERM_PROGRAM".into(), "teleprinter".into())]),
        };
        let pty = tty::new(&options, window_size, 0)?;
        let (tx, events) = mpsc::channel();
        let listener = Listener { events: tx, ctx: ctx.clone() };
        let size = GridSize { columns: window_size.num_cols as usize, lines: window_size.num_lines as usize };
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Self { term, sender, events, window_size, title: None, exited: false })
    }

    /// Sends bytes to the shell, scrolling back down to it first.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.term.lock().scroll_display(Scroll::Bottom);
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    /// Pastes text, bracketed when the program asked for that, so a shell
    /// doesn't run pasted lines as they arrive.
    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        if bracketed {
            // A pasted end marker would end the paste early and run what follows.
            let text = text.replace("\x1b[201~", "");
            self.write(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.write(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
    }

    pub fn resize(&mut self, size: WindowSize) {
        let old = self.window_size;
        if (old.num_cols, old.num_lines, old.cell_width, old.cell_height)
            == (size.num_cols, size.num_lines, size.cell_width, size.cell_height)
        {
            return;
        }
        self.window_size = size;
        self.term.lock().resize(GridSize { columns: size.num_cols as usize, lines: size.num_lines as usize });
        let _ = self.sender.send(Msg::Resize(size));
    }

    /// Handles what the event loop sent since the last frame.
    pub fn pump(&mut self, ctx: &egui::Context, palette: &Palette) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Title(title) => self.title = Some(title),
                Event::ResetTitle => self.title = None,
                Event::PtyWrite(text) => self.write(text.into_bytes()),
                Event::ClipboardStore(_, text) => ctx.copy_text(text),
                Event::ColorRequest(index, format) => {
                    let rgb = palette.query(index, self.term.lock().colors());
                    self.write(format(rgb).into_bytes());
                }
                Event::TextAreaSizeRequest(format) => self.write(format(self.window_size).into_bytes()),
                Event::ChildExit(_) | Event::Exit => self.exited = true,
                // Reading the clipboard for a program (OSC 52 load) is off by default in
                // alacritty_terminal, and egui can't read it synchronously anyway.
                Event::ClipboardLoad(..) => {}
                Event::Bell | Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}
