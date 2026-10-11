# 0001. Build Teleprinter on Patina, with alacritty_terminal for the emulation

Date: 2026-10-11

## Status

Accepted

## Context

The user wanted a terminal that belongs to the Patina family: same frame, same skins, following
the desktop's light/dark setting live, which most terminals do badly. A terminal has two halves:
the emulation (parsing VT/xterm escape sequences into a grid with scrollback, modes and
selection) and the front end (drawing the grid, keyboard and mouse input, the pty).

The emulation is the part that takes years to get right, and it already exists as a library:
`alacritty_terminal`, Alacritty's core published as a crate and used by Zed's terminal. WezTerm's
`termwiz` was the other candidate; it's less used outside WezTerm. Building on `egui_term` (a
ready-made egui widget over `alacritty_terminal`) was considered and passed over, so that
drawing, input and skin integration stay ours.

## Decision

We will build Teleprinter with egui/eframe on [Patina](https://github.com/Sector-F-Labs/patina)
(`patina-ui`), with `alacritty_terminal` doing the emulation and its own pty and event loop
running the shell:

- Patina's own window frame with word menus (File, Edit, View, Skins) on the left and nothing on
  the right, since a terminal's output belongs to the shell; a "System Title Bar" setting as the
  escape hatch. The shell's OSC title becomes the window title.
- The grid painted with egui's painter, one text shape per run of same-style ASCII cells, and
  every other glyph placed at its own cell so icons and wide characters can't push columns out of
  line.
- Redraws driven by the pty: alacritty_terminal's events ask egui for a repaint, so an idle
  terminal does no work.

## Consequences

- Teleprinter looks like the rest of the family and takes the same skins.
- Escape-sequence correctness comes from alacritty_terminal; ours is the input mapping and drawing.
- egui's text layout wasn't made for a fast-scrolling grid. If large output stutters, rows need
  caching or a renderer of our own.
- Not in the first version: mouse reporting to programs (vim's `mouse=a`, tmux), IME, tabs,
  splits, images (sixel/kitty), and fonts for scripts Meslo lacks (CJK shows as boxes).
- It moves to a new egui version only together with Patina.
