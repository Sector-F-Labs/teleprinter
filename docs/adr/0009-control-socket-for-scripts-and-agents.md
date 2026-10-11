# 0009. A control socket lets programs in a pane open, move and close panes

Date: 2026-10-12

## Status

Proposed

## Context

The user wants an agent such as Claude Code, running in one pane, to be able to show them
something in another: asked to explain a change, it opens a pane beside itself with nvim on the
changed code and steps through it while it talks. That needs a way for a program to drive the
layout from outside the UI. Today only the mouse and menus can split, move or close panes.

kitty (`kitten @ launch`, `KITTY_LISTEN_ON`) and wezterm (`wezterm cli split-pane`,
`WEZTERM_PANE`) solve the same problem the same way: a socket per terminal process, a CLI in the
same binary that talks to it, and env vars in every shell naming the socket and the caller's own
pane. The env vars matter most. With them, a program knows which window it's in and which pane is
its own, so it can open something next to itself without having to find the window first.

What runs in the new pane doesn't concern the terminal. nvim already has its own socket
(`nvim --listen`, driven with `nvim --server … --remote-send`), and the agent can drive it
directly, e.g. a quickfix list of `file:line: explanation` entries stepped through with `:cnext`.
We considered building nvim support into Teleprinter and ruled it out. An MCP server isn't needed
either: an agent with a shell can run the CLI, and a skill can tell it when
`$TELEPRINTER_SOCKET` is set.

[ADR-0008](0008-drag-moves-panes-like-a-tiling-window-manager.md)'s tree already offers split,
insert, move, swap and remove on stable `u64` pane ids, so the socket adds a second way to make
those calls. It doesn't need a new model.

## Decision

- **Each Teleprinter process listens on a Unix socket** at
  `$XDG_RUNTIME_DIR/teleprinter/<pid>.sock`, mode 0600, removed on exit. A listener thread parses
  requests and sends them over a channel, then requests a repaint. `App::update` drains the
  channel and calls the same split, move and close the UI uses, as `session.rs` does for pty
  events.
- **Every shell gets `TELEPRINTER_SOCKET` and `TELEPRINTER_PANE`** (its pane's id).
- **The protocol is newline-delimited JSON**, one request and one reply per line. To start with:
  - `split`: pane, direction, optional command (argv), cwd, `focus`; replies with the new pane's id.
  - `list`: each pane's id, title, cwd, rect, whether it's collapsed and whether it's active.
  - `close`, `focus`.

  `move` and `collapse` come later.
- **A pane opened over the socket doesn't take the keyboard** unless `focus` is true. The user is
  usually typing to whoever opened it. Panes can run their own command, not only the login shell
  or the global `TELEPRINTER_COMMAND`, and close when it exits, as panes already do.
- **A pane opened over the socket gets the caller's environment**, laid over the window's own
  (less `PWD`, `OLDPWD`, `SHLVL` and `_`). Teleprinter started from the desktop has the session's
  bare `PATH`; the caller's shell has read its rc files. A program run straight from the socket,
  not through a shell, would otherwise miss everything those set (`~/.cargo/bin`, `EDITOR`, …),
  and an nvim opened beside an agent behaved differently from one the user opens. kitty offers
  this as `launch --copy-env`; here `msg split` always sends it, in the request's `env`.
- **The CLI is the same binary**: `teleprinter msg split --right -- nvim …` (`--focus` to give it the keyboard). It uses
  `$TELEPRINTER_SOCKET` unless given `--socket`.
- **There is no `send-text`** (typing into another pane) for now.

## Consequences

- Scripts and agents can lay out panes: the nvim tour, a log tail beside a build, a test runner
  that opens its failures.
- Any process running as the user can drive the terminal, run commands in new panes and read pane
  titles and directories. That is no more than such a process can already do, but it is one more
  thing an agent that's been prompt-injected can reach. Leaving out `send-text` keeps the user's
  own shells out of reach. If we add it, it should be limited to panes the caller opened.
- The protocol is an interface others will script against. Changing it later breaks them, so
  requests and replies should carry no more than they need.
- Variables the caller unset but the window has still reach the new pane: the caller's
  environment is added, it doesn't replace the window's.
- A program that's been moved out of the pane tree (e.g. into tmux, or over ssh) inherits stale
  env vars, and the CLI should fail clearly when the socket or pane is gone.
- Panes opened by programs and panes opened by hand look the same. There's no way yet to clean up
  "everything this agent opened" except closing them one by one.
