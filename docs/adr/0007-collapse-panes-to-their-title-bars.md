# 0007. Collapse panes to their title bars, and give the space to their neighbours

Date: 2026-10-11

## Status

Accepted

## Context

With panes tiled ([ADR-0005](0005-tiled-panes-as-pinned-windows.md)), every pane takes space, so
many panes means small panes, or tabs. The user asked for egui's standard collapse arrow on the
pane windows instead, so a column can hold many folded panes and the open one gets the room,
without tabs. Titles that follow the foreground program
([ADR-0006](0006-pane-titles-from-the-foreground-process.md)) make a folded pane still say what's
running in it.

## Decision

- **Pane windows are collapsible with egui's own arrow**, and egui's collapse state is the truth:
  the layout reads it every frame (`CollapsingState` under the window's id). Double-clicking a
  title bar folds or unfolds too, through our drag handle, which leaves the arrow and the close
  button uncovered.
- **In a stack (a Down split), a folded side is only as tall as its title bars**, and the other
  side takes the rest. With both sides folded, the title bars stack at the top. A stack with a
  folded side has no divider, since there's nothing to resize.
- **Side by side (a Right split), a folded pane keeps its width** and shows its title bar at the
  top of its column; the space below is left empty. Folding is for stacks.
- **The collapsed height is measured,** from the collapsed windows themselves, so it follows the
  skin's fonts and margins.
- **egui sizes a collapsed title bar to its text,** so a folded pane's title is padded with spacer
  atoms either side, measured to fill the tile and keep the text centred like an open window's.
- If the active pane is folded, the keyboard moves to the first open pane.
- `TELEPRINTER_COLLAPSED=2,4` starts those panes folded, for screenshots.

## Consequences

- Many panes fit in a column: fold the ones you aren't using and read their titles.
- Folded panes' shells keep running; their output and titles keep updating.
- Folding in a side-by-side split saves nothing. Giving its width away too would need vertical
  title bars, or a different shape for folded panes.
- The full-width folded title bar depends on how egui lays out window titles (the arrow, two
  grows, the text, the close button). An egui upgrade that changes that layout would show here.
- Folded state isn't saved, like the layout itself.
