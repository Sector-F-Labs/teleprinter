# 0011. Report clicks and drags to programs that ask for the mouse; Shift selects

Date: 2026-10-12

## Status

Accepted

## Context

[ADR-0010](0010-report-the-wheel-to-programs-that-ask-for-the-mouse.md) reported only the wheel
and kept clicks and drags for Teleprinter's selection. Neovim (`mouse=a`, its default) turns on
mouse tracking and expects clicks: to move the cursor, pick a split, drag a divider, select in
visual mode. In Teleprinter a click did nothing in it.

Other terminals settle the clash between the program's mouse and the terminal's selection the same
way: while a program tracks the mouse, every button goes to it, and Shift gives the mouse back to
the terminal. ADR-0010 already uses Shift that way for the wheel.

## Decision

We will report the left, middle and right buttons to a program that has turned on mouse tracking:
a press on the terminal, its release wherever the pointer is, and, under `?1002`/`?1003`, motion
while the button is held, once per cell. Under `?1003` motion with no button is reported too, while
the pointer is over the terminal. Encodings and modifiers are as in ADR-0010; a legacy release is
button 3, an SGR one ends in `m`.

Holding Shift keeps the mouse for Teleprinter: Shift+drag selects, Shift+double-click selects a
word, Shift+right-click opens the pane menu. Without tracking, nothing changes. A click still
makes its pane the active one either way.

## Consequences

- Neovim, htop, tmux and the like take clicks and drags as they do in other terminals.
- Selecting text and the pane menu need Shift in those programs. Neovim's own visual selection
  (copied with `"+y`) is the other way to copy.
- If the release happens where Teleprinter doesn't see it (outside the window), it's reported on
  the next frame, at the last cell reported, so the program isn't left dragging.
- Back and forward buttons aren't reported; few programs use them.
