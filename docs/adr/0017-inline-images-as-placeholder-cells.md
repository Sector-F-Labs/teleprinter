# 0017. Inline images as placeholder cells, from a filtered pty

Date: 2026-11-02

## Status

Proposed

## Context

Programs show images in the terminal with escape sequences: iTerm2's `OSC 1337 File=` (`imgcat`)
and the kitty graphics protocol (`APC G`: yazi, chafa, `kitten icat`, timg). alacritty_terminal
parses neither and drops them, and its `EventLoop` reads the pty on its own thread with no hook
for the bytes, so an image can't be taken out of the stream from our side of the event channel.
Yazi and chafa decide what to use by asking the terminal, so an unrecognised terminal got
chafa's coarse blocks, or "failed to spawn chafa" where it isn't installed.

Options weighed:

- Copy alacritty_terminal's event loop and parser to add graphics: a fork to carry through every
  upgrade (ADR-0001 chose the crate to avoid that).
- Draw images as overlays at screen positions the filter works out itself: it doesn't know the
  cursor, and the image would not scroll, wrap or be overwritten with the text.
- Wrap the pty, take the sequences out of the byte stream, and leave cells in the grid in their
  place for the view to paint.

## Decision

We will wrap the pty in `ImagePty` (`src/images.rs`), whose reader filters what the event loop
reads. An image becomes rows of private-use characters (`U+10EE00` plus the row), written with the
image's id as the foreground colour, and the view paints each such cell as its slice of the
image's texture (made on first paint). The grid then scrolls, wraps, erases and overwrites images
like text. The iTerm2 protocol (single and multipart, as `imgcat` sends) and kitty's transmit,
display, put, query and delete are supported; a kitty delete drops the images made so far, which
blanks their cells. The filter answers kitty's queries by writing to the pty, which is how
programs find that graphics work. Sixel is not supported.

## Consequences

- yazi, chafa (`-f kitty`) and `imgcat` show real images; yazi needs no `TERM_PROGRAM` trick.
- No change to alacritty_terminal, but every byte the shell writes now passes through our
  scanner. Big `cat` output takes about as long as before.
- An image is the cells it left: text written over some of them cuts it, and selecting across it
  copies the placeholder characters. Cells are limited to 256 rows an image.
- Kitty's cropping, shared-memory transfer and unicode placeholders are not handled. Delete
  removes images in the scrollback too, where kitty only removes visible ones.
- Textures are freed only on delete, so a long session full of images grows.
- Inside tmux, `imgcat` doesn't work: it needs tmux's passthrough wrapper (DCS `tmux;`).
- Revisit if sixel is wanted, or if alacritty_terminal gains a hook for the pty bytes.
