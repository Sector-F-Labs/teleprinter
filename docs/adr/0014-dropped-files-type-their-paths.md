# 0014. Dropped files type their paths, with our own Wayland drop target

Date: 2026-10-25

## Status

Accepted

## Context

Dragging an image from the file manager into Claude Code works in other terminals and did nothing
here. Those terminals type the dropped files' paths, shell-quoted, as a paste; Claude Code sees an
image path in a paste and attaches the image. egui reports dropped files, but only where winit
does: winit 0.30 (eframe 0.35's) handles drops on X11, macOS and Windows and not on Wayland, the
user's session.

Running under XWayland would get winit's X11 drops, at the cost of blurry fractional scaling and
the rest of Wayland. Waiting for a winit with Wayland drops means waiting for eframe to move to it.

## Decision

We will type dropped files' paths into the pane under the drop (else the active pane, which takes
the keyboard), each single-quoted if it has anything but plain path characters, separated and
followed by a space, sent as a paste so bracketed paste applies (`src/drop.rs`). egui's dropped
files cover X11 and the other platforms. On Wayland we open a `wl_data_device` of our own on
winit's connection (`Backend::from_foreign_display`), on its own event queue and thread, accept
`text/uri-list` offers, read the list on drop and turn its `file:` URIs into paths.

## Consequences

- Drops work like GNOME Terminal's, and Claude Code attaches dropped images.
- We depend on `wayland-client`/`wayland-backend` directly and own some unsafe: the display pointer
  borrowed from winit must outlive the thread, which it does as long as the window.
- Only `file:` URIs are typed; dropped text or web links do nothing yet.
- When eframe's winit handles Wayland drops, delete the Wayland half and keep the typing.
