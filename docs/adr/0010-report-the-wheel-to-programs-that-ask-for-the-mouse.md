# 0010. Report the wheel to programs that ask for the mouse; Shift keeps it local

Date: 2026-10-12

## Status

Accepted, partly superseded by [0011](0011-report-clicks-and-drags-shift-selects.md) (clicks and drags)

## Context

Claude Code can't be scrolled in Teleprinter. It runs full screen: it switches to the alternate
screen (`?1049h`) and turns on mouse tracking (`?1000h`, `?1002h`, `?1003h`) with SGR encoding
(`?1006h`), then scrolls its own transcript when the terminal reports wheel events. Teleprinter
ignores mouse modes. On the alternate screen it turns the wheel into ↑/↓ (alternate scroll, as
for `less`), which Claude Code takes as prompt history; on the main screen it scrolls the
scrollback. Neither reaches the program as a wheel.

The alternate screen has no scrollback of its own, so scrolling locally there shows nothing new.
Every common terminal (xterm, Alacritty, GNOME Terminal, kitty) reports the wheel as mouse buttons
4 and 5 once a program turns on mouse tracking, and lets Shift bypass the reporting.

Reporting clicks and drags too was considered: the same modes ask for them, and vim, htop and tmux
use them. It was left for later because the left button is Teleprinter's selection today, and
reporting it means deciding when a drag selects and when it goes to the program.

## Decision

We will report the wheel as mouse events whenever the program has turned on any mouse mode
(`MOUSE_REPORT_CLICK`, `MOUSE_DRAG` or `MOUSE_MOTION`). Each line of wheel movement becomes one
button-64 (up) or button-65 (down) press at the cell under the pointer, with Shift, Alt and Ctrl
added in as xterm does. The encoding follows the program's choice: SGR (`ESC [ < b ; x ; y M`)
under `?1006`, otherwise the legacy `ESC [ M` bytes, whose coordinates stop at column and row 223.

Shift+wheel is never reported: it scrolls Teleprinter's scrollback, as before. Without a mouse
mode, nothing changes: alternate scroll on the alternate screen, scrollback on the main screen.
Clicks and drags stay Teleprinter's selection.

## Consequences

- Claude Code, and other full-screen programs that track the mouse, scroll with the wheel as they
  do in other terminals.
- A program that turns on mouse tracking but ignores wheel events can't be wheel-scrolled; Shift
  is the way out, as in other terminals.
- Programs that want clicks (vim with `mouse=a`, htop, tmux) still don't get them. Revisit when one
  of those matters: report buttons too, with Shift+drag to select, as Alacritty does.
- The `?1005` UTF-8 coordinate encoding isn't supported; programs that ask for it get the legacy
  bytes. Few programs use it now that SGR exists.
