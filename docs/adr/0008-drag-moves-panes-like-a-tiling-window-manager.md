# 0008. Dragging moves a pane like a tiling window manager; holding still swaps

Date: 2026-10-11

## Status

Accepted

## Context

[ADR-0005](0005-tiled-panes-as-pinned-windows.md) made dropping one pane on another swap them, so
the layout's shape never changed. The user wanted it to behave like a tiling window manager:
with two panes on the left and two on the right, dragging one from the left to the right should
leave one on the left and three on the right, with a swap only when you hover over a pane for a
while. A binary split tree can't give three even panes in a column without nesting splits at
uneven ratios, which is why tiling window managers such as i3 and Sway use rows and columns of
any number of children.

The user also asked for a way to open a terminal in the bare background that folded panes leave
([ADR-0007](0007-collapse-panes-to-their-title-bars.md)).

## Decision

- **The layout is a tree of rows and columns with any number of children,** each with a weight
  (its share). Splitting a pane halves it. A split left with one child becomes that child, and a
  split inside one running the same way merges into it.
- **Dropping a pane on another moves it** to that pane's nearest side (left, right, top or
  bottom): it leaves its place, and joins the target's row or column if that runs the same way,
  where the children then share the space evenly; otherwise the target becomes a split of the two.
  While dragging, the half of the target it would take is highlighted ("Move here").
- **Holding still over a pane for 0.6 s turns the drop into a swap**: the whole target is
  highlighted ("Swap") and the two trade places, keeping the shape. Moving to another pane starts
  the wait again.
- **Dividers sit between neighbours in a row or column** and move only those two.
- **Right-clicking the background** (between and below the tiles) offers New Terminal: the new
  pane goes below the pane above the click in that column, in that pane's directory, so it fills
  the space folded panes left.

## Consequences

- Panes can be rearranged into any shape by dragging, and a row or column of three or more stays
  even.
- A drop always changes the layout unless you wait for the swap; a quick drag in the wrong place
  needs another drag to undo.
- Even sharing on a move resets any sizes set by hand in the target's row or column.
- There's still no keyboard way to move panes.
