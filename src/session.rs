//! One shell on a pty, run by alacritty_terminal (ADR-0001). Its event loop
//! reads the pty on its own thread and updates the `Term`; this side writes
//! input, resizes, and turns the loop's events into window changes. Each
//! event asks egui for a repaint, so the window redraws when the shell
//! writes and is otherwise idle.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

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
    /// The shell's process, for finding its working directory.
    pid: u32,
    /// The pty, to ask which process group is in the foreground.
    tty: Option<std::fs::File>,
    /// What runs in the foreground, for the pane's title (as tmux names windows).
    pub foreground: Option<Foreground>,
    last_probe: Option<Instant>,
    /// A second look due after output, for programs that start without writing.
    probe_again: Option<Instant>,
}

/// The program in the foreground of a pane, and its directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Foreground {
    pub name: String,
    pub cwd: Option<std::path::PathBuf>,
}

/// How often output may trigger another look at the foreground process.
const PROBE_INTERVAL: Duration = Duration::from_millis(200);

impl Session {
    /// Starts `command` (the user's login shell when `None`) in `cwd`, with
    /// `env` added to its environment.
    pub fn spawn(
        ctx: &egui::Context,
        command: Option<(String, Vec<String>)>,
        cwd: Option<std::path::PathBuf>,
        mut env: HashMap<String, String>,
        window_size: WindowSize,
    ) -> std::io::Result<Self> {
        tty::setup_env();
        env.insert("TERM_PROGRAM".into(), "teleprinter".into());
        let options = tty::Options {
            shell: command.map(|(program, args)| tty::Shell::new(program, args)),
            working_directory: cwd,
            drain_on_exit: true,
            env,
        };
        let pty = tty::new(&options, window_size, 0)?;
        let pid = pty.child().id();
        let tty = pty.file().try_clone().ok();
        let (tx, events) = mpsc::channel();
        let listener = Listener { events: tx, ctx: ctx.clone() };
        let size = GridSize { columns: window_size.num_cols as usize, lines: window_size.num_lines as usize };
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Self { term, sender, events, window_size, title: None, exited: false, pid, tty, foreground: None, last_probe: None, probe_again: None })
    }

    /// The shell's current directory, where a split from this pane starts.
    /// Read from /proc, so only on Linux; elsewhere `None` (home is used).
    pub fn cwd(&self) -> Option<std::path::PathBuf> {
        std::fs::read_link(format!("/proc/{}/cwd", self.pid)).ok()
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
        let mut output = false;
        while let Ok(event) = self.events.try_recv() {
            output |= matches!(event, Event::Wakeup);
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
        // There's no event for a new foreground program, so output is the cue to look:
        // a program nearly always writes (vim drawing, the prompt coming back). One that
        // starts silently, like `sleep`, only follows the echo of Enter, so every burst of
        // output also gets a second look once things settle.
        let now = Instant::now();
        let ready = self.last_probe.is_none_or(|at| now - at >= PROBE_INTERVAL);
        if output {
            self.probe_again = Some(now + PROBE_INTERVAL);
        }
        if (output && ready) || self.last_probe.is_none() || self.probe_again.is_some_and(|at| now >= at) {
            self.last_probe = Some(now);
            if !output {
                self.probe_again = None;
            }
            self.foreground = self.probe();
        }
        if let Some(at) = self.probe_again {
            ctx.request_repaint_after(at.saturating_duration_since(now));
        }
    }

    /// The pty's foreground process group leader: its name and directory.
    /// Read from /proc, so on Linux only; elsewhere `None`, and the title
    /// falls back to the one the shell sets.
    fn probe(&self) -> Option<Foreground> {
        use std::os::fd::AsRawFd;
        let fd = self.tty.as_ref()?.as_raw_fd();
        // SAFETY: tcgetpgrp only reads the terminal's state; fd stays open for the call.
        let group = unsafe { libc::tcgetpgrp(fd) };
        if group <= 0 {
            return None;
        }
        let name = std::fs::read_to_string(format!("/proc/{group}/comm")).ok()?.trim().to_string();
        let cwd = std::fs::read_link(format!("/proc/{group}/cwd")).ok();
        Some(Foreground { name, cwd })
    }

    /// The pane's title: the foreground program and where it is, as
    /// `vim — ~/Src/teleprinter`; the shell's own title where that can't be
    /// read; else "Shell".
    pub fn label(&self) -> String {
        match (&self.foreground, &self.title) {
            (Some(fg), _) => title(fg, dirs::home_dir().as_deref()),
            (None, Some(title)) => title.clone(),
            (None, None) => "Shell".into(),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

/// `name — dir`, with the home directory as `~`.
pub fn title(fg: &Foreground, home: Option<&std::path::Path>) -> String {
    let Some(cwd) = &fg.cwd else { return fg.name.clone() };
    let dir = match home.and_then(|h| cwd.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => cwd.display().to_string(),
    };
    format!("{} — {dir}", fg.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn fg(name: &str, cwd: Option<&str>) -> Foreground {
        Foreground { name: name.into(), cwd: cwd.map(PathBuf::from) }
    }

    #[test]
    fn titles_name_the_program_and_its_directory() {
        let home = Some(Path::new("/home/me"));
        assert_eq!(title(&fg("vim", Some("/home/me/src/x")), home), "vim — ~/src/x");
        assert_eq!(title(&fg("zsh", Some("/home/me")), home), "zsh — ~");
        assert_eq!(title(&fg("htop", Some("/etc")), home), "htop — /etc");
        assert_eq!(title(&fg("ssh", None), home), "ssh");
    }
}
