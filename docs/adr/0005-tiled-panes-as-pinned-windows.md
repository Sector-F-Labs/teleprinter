# 0005. Split into tiled panes, each a pinned window, swapped by dragging

Date: 2026-10-11

## Status

Accepted

## Context

The user wanted multiplexing in the terminal itself: right-click Split Right and Split Down, tiled
the way UnAmp tiles (UnAmp's ADR-0023: real `egui::Window`s pinned in place, not panels made to
look like windows), and panes that can be dragged to trade places while the layout stays tiled.
tmux-style panes drawn as bare rectangles, and free-floating windows, were both ruled out by that.

## Decision

- **The layout is a binary tree of splits** (`src/tiles.rs`). Split Right or Down replaces the
  pane's leaf with a split holding it and a new pane, half and half. Closing a pane (its close
  button, the menus, or its shell exiting) gives its space to its sibling; closing the last one
  closes the window.
- **Every pane is an `egui::Window` pinned to its tile** with `fixed_rect`: not movable or
  resizable, no shadow, title from the shell (OSC 0/2), a close button. One pane or many, it's the
  same window, so there's one way panes are drawn.
- **Dragging a pane's title bar onto another swaps the two leaves.** The tree keeps its shape, so
  the panes trade tiles and everything stays tiled. Since pinned windows can't be moved, the title
  bar (less the close button) gets a drag handle of our own on the window's layer; the tile under
  the pointer is highlighted while dragging.
- **Dividers:** the gap between the two sides of a split is a drag handle that sets the split's
  ratio, kept between 10% and 90%.
- **The active pane** has the keyboard, a border in the selection colour, and is what the menus
  and the window title act on. A click, right-click or drag makes a pane active; a new split
  becomes active.
- **A split starts in the active shell's directory**, read from `/proc/<pid>/cwd`; where there's
  no /proc (FreeBSD without linprocfs), in the home directory.
- Split Right, Split Down and Close Pane are also in the File menu.

## Consequences

- Panes look like UnAmp's tiles, and every skin reaches them.
- Rearranging is drag-and-drop between existing tiles; changing the shape (moving a pane into a
  different split) means closing and splitting again.
- The layout isn't saved: a new window starts with one pane.
- With one pane, its title bar repeats the window's title. Hiding it while there's only one pane
  would save a row, at the cost of two ways of drawing a pane.
- No keyboard shortcuts for splitting or moving between panes yet.
