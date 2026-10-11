//! The control socket (ADR-0009): programs in a pane open, list, focus and
//! close panes by sending a line of JSON to `$TELEPRINTER_SOCKET`, as kitty
//! and wezterm let them. A thread per connection parses each line and
//! hands it to the app, which answers on its next frame; `teleprinter msg`
//! is the client.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tiles::{PaneId, Side};

/// How long a connection waits for the app to answer, e.g. while it's hidden
/// and not drawing.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// One request, `{"cmd": "split", …}`.
#[derive(Deserialize, Serialize, Debug, PartialEq)]
#[serde(tag = "cmd", rename_all = "lowercase", deny_unknown_fields)]
pub enum Request {
    /// A new pane beside `pane` (the caller's, else the active one), running
    /// `command` (else the login shell) in `cwd` (else `pane`'s directory).
    /// It takes the keyboard only with `focus`.
    Split {
        pane: Option<PaneId>,
        #[serde(default = "right")]
        side: Where,
        #[serde(default)]
        command: Vec<String>,
        cwd: Option<PathBuf>,
        #[serde(default)]
        focus: bool,
    },
    List,
    Close { pane: PaneId },
    Focus { pane: PaneId },
}

fn right() -> Where {
    Where::Right
}

/// A side, as it's spelt on the wire.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Where {
    Left,
    Right,
    Up,
    Down,
}

impl From<Where> for Side {
    fn from(w: Where) -> Side {
        match w {
            Where::Left => Side::Left,
            Where::Right => Side::Right,
            Where::Up => Side::Top,
            Where::Down => Side::Bottom,
        }
    }
}

/// A request and where its answer goes: `{"ok": true, …}` or
/// `{"ok": false, "error": …}`.
pub type Call = (Request, Sender<Value>);

pub fn ok(fields: Value) -> Value {
    let mut reply = serde_json::json!({ "ok": true });
    if let (Value::Object(reply), Value::Object(fields)) = (&mut reply, fields) {
        reply.extend(fields);
    }
    reply
}

pub fn error(message: impl std::fmt::Display) -> Value {
    serde_json::json!({ "ok": false, "error": message.to_string() })
}

/// The listening socket; removed when dropped.
pub struct Server {
    pub path: PathBuf,
    pub calls: Receiver<Call>,
}

impl Server {
    /// Listens on `$XDG_RUNTIME_DIR/teleprinter/<pid>.sock`, readable only by
    /// the user. Each request asks egui for a repaint, so an idle window
    /// answers at once.
    pub fn start(ctx: &egui::Context) -> std::io::Result<Self> {
        let dir = socket_dir();
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        let path = dir.join(format!("{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path); // left by an earlier process with this pid
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let (tx, calls) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, ctx) = (tx.clone(), ctx.clone());
                std::thread::spawn(move || serve(stream, &tx, &ctx));
            }
        });
        Ok(Self { path, calls })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn socket_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("teleprinter"),
        // SAFETY: getuid can't fail.
        None => std::env::temp_dir().join(format!("teleprinter-{}", unsafe { libc::getuid() })),
    }
}

/// Answers each line of one connection until it closes.
fn serve(stream: UnixStream, calls: &Sender<Call>, ctx: &egui::Context) {
    let Ok(mut out) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Err(e) => error(format!("bad request: {e}")),
            Ok(request) => {
                let (tx, rx) = mpsc::channel();
                if calls.send((request, tx)).is_err() {
                    return; // the window has closed
                }
                ctx.request_repaint();
                rx.recv_timeout(ANSWER_WITHIN).unwrap_or_else(|_| error("the window didn't answer"))
            }
        };
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}

/// `teleprinter msg …`: sends one request to `$TELEPRINTER_SOCKET` (or
/// `--socket`) and prints the reply. Returns the exit code.
pub fn client(args: &[String]) -> i32 {
    let (socket, request) = match parse(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("teleprinter msg: {message}\n\n{USAGE}");
            return 2;
        }
    };
    let Some(socket) = socket.or_else(|| std::env::var_os("TELEPRINTER_SOCKET").map(PathBuf::from)) else {
        eprintln!("teleprinter msg: not in a Teleprinter pane ($TELEPRINTER_SOCKET isn't set); pass --socket");
        return 1;
    };
    match send(&socket, &request) {
        Ok(reply) => {
            println!("{reply}");
            if reply["ok"] == Value::Bool(true) { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("teleprinter msg: {}: {e}", socket.display());
            1
        }
    }
}

fn send(socket: &Path, request: &Request) -> std::io::Result<Value> {
    let mut stream = UnixStream::connect(socket)?;
    writeln!(stream, "{}", serde_json::to_string(request)?)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

pub const USAGE: &str = "\
usage: teleprinter msg [--socket PATH] <command>

  split [--left|--right|--up|--down] [--pane ID] [--cwd DIR] [--focus] [-- PROGRAM ARGS…]
        a new pane beside ID (default: this pane), running PROGRAM (default: your shell);
        prints its id. It takes the keyboard only with --focus.
  list  the panes: id, title, cwd, rect, collapsed, active
  close [ID]   (default: this pane)
  focus ID";

/// The caller's own pane, from `$TELEPRINTER_PANE`.
fn this_pane() -> Option<PaneId> {
    std::env::var("TELEPRINTER_PANE").ok()?.parse().ok()
}

fn parse(args: &[String]) -> Result<(Option<PathBuf>, Request), String> {
    let mut args = args.iter().map(String::as_str).peekable();
    let mut socket = None;
    if args.peek() == Some(&"--socket") {
        args.next();
        socket = Some(PathBuf::from(args.next().ok_or("--socket needs a path")?));
    }
    let id = |arg: Option<&str>| -> Result<PaneId, String> {
        let arg = arg.ok_or("a pane id is needed")?;
        arg.parse().map_err(|_| format!("not a pane id: {arg}"))
    };
    let request = match args.next().ok_or("which command?")? {
        "split" => {
            let (mut pane, mut side, mut cwd, mut focus, mut command) = (this_pane(), Where::Right, None, false, Vec::new());
            while let Some(arg) = args.next() {
                match arg {
                    "--left" => side = Where::Left,
                    "--right" => side = Where::Right,
                    "--up" => side = Where::Up,
                    "--down" => side = Where::Down,
                    "--pane" => pane = Some(id(args.next())?),
                    // Relative to the caller, not to the window.
                    "--cwd" => cwd = Some(std::path::absolute(args.next().ok_or("--cwd needs a directory")?).map_err(|e| e.to_string())?),
                    "--focus" => focus = true,
                    "--" => command = args.by_ref().map(String::from).collect(),
                    other => return Err(format!("unknown option: {other}")),
                }
            }
            Request::Split { pane, side, command, cwd, focus }
        }
        "list" => Request::List,
        "close" => Request::Close { pane: match args.next() {
            Some(arg) => id(Some(arg))?,
            None => this_pane().ok_or("which pane? ($TELEPRINTER_PANE isn't set)")?,
        } },
        "focus" => Request::Focus { pane: id(args.next())? },
        other => return Err(format!("unknown command: {other}")),
    };
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}"));
    }
    Ok((socket, request))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn split_takes_a_side_a_pane_and_a_command() {
        let (_, request) = parse(&args("split --down --pane 3 --focus -- nvim +42 src/view.rs")).unwrap();
        assert_eq!(
            request,
            Request::Split {
                pane: Some(3),
                side: Where::Down,
                command: args("nvim +42 src/view.rs"),
                cwd: None,
                focus: true
            }
        );
    }

    #[test]
    fn wire_requests_fill_in_defaults() {
        let request: Request = serde_json::from_str(r#"{"cmd":"split"}"#).unwrap();
        assert_eq!(request, Request::Split { pane: None, side: Where::Right, command: vec![], cwd: None, focus: false });
        assert_eq!(serde_json::from_str::<Request>(r#"{"cmd":"close","pane":2}"#).unwrap(), Request::Close { pane: 2 });
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"close"}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"split","sdie":"left"}"#).is_err());
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert!(parse(&args("split --sideways")).is_err());
        assert!(parse(&args("focus")).is_err());
        assert!(parse(&args("focus two")).is_err());
        assert!(parse(&args("list extra")).is_err());
        assert_eq!(parse(&args("--socket /x list")).unwrap(), (Some(PathBuf::from("/x")), Request::List));
    }

    #[test]
    fn replies_carry_ok() {
        assert_eq!(ok(serde_json::json!({ "pane": 4 })), serde_json::json!({ "ok": true, "pane": 4 }));
        assert_eq!(error("no"), serde_json::json!({ "ok": false, "error": "no" }));
    }
}
