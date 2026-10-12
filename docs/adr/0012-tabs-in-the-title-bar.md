# 0012. Tabs in the title bar, in place of the window title

Date: 2026-10-12

## Status

Proposed

## Context

Every pane's window shows its own title ([ADR-0006](0006-pane-titles-from-the-foreground-process.md)),
so the window title centred in Patina's title bar only repeated the active pane's. The user asked for
that space, between the menus and the window buttons, to hold tabs instead. Collapsing panes
([ADR-0007](0007-collapse-panes-to-their-title-bars.md)) was chosen earlier as the way to keep many
panes in one layout without tabs; tabs are for separate layouts, not more panes in one.

## Decision

- **A tab is a layout of its own**: a tile tree and the pane that has the keyboard in it. Only the
  shown tab's panes are laid out and drawn; the others' shells keep running. Pane ids stay unique
  across tabs, and collapse state stays keyed by pane.
- **The title bar has no title.** After the menus come the tabs, each named for its active pane and
  sharing the bar's width (up to 200 px each, truncated), then `+`. They're drawn as tabs, not
  buttons: the bar's full height, padding included, and square whatever the skin's rounding. The
  shown tab is filled in the panel colour, so it joins the panes below (in the extreme background
  colour on a bar that's already the panel colour). Pane titles are in the body size, as the tabs
  and menus are, not egui's heading size. The window title still goes to the desktop for the
  taskbar.
- **Opening and closing**: `+`, File > New Tab or Ctrl+Shift+T opens a tab with one pane in the
  active pane's directory. A tab closes when its last pane does, or from File > Close Tab, its
  right-click menu, the cross a tab shows while hovered, or a middle click; the next tab takes its place. The last tab's last pane closes
  the window, as before. Ctrl+PageUp/PageDown cycle tabs. These keys are taken before the panes see
  them.
- **The control socket spans tabs**: `split` goes into the target pane's tab, shown or not; `focus`
  shows the pane's tab; `list` lists every tab's panes with a 1-based `tab`, and a `rect` only for
  the shown tab's.

## Consequences

- One title per pane, and the title bar's width does something.
- Ctrl+Shift+T and Ctrl+PageUp/PageDown no longer reach programs (they rarely use them; GNOME
  Terminal binds the same keys).
- With the system title bar on, the desktop shows the title and the tabs sit in the toolbar.
- Tabs can't yet be reordered, renamed, or have panes dragged between them; the socket has no
  request to open a tab.
