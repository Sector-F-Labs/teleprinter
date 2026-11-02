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
  scanner. Output with no escape in it is read straight into the event loop's buffer; otherwise
  it's copied once to be scanned. `make bench` (the real event loop on `cat` of 100 MB files):
  plain text 72 MB/s on master and on this branch, text with an escape every few bytes 64 and
  62, and the same with a thread contending for the terminal lock 61/61 and 53/52.
- Output left over when the event loop's buffer is full would sit unseen, as the pty has nothing
  more to say: a pipe in the same poller wakes the loop until it has been read.
- An image is the cells it left: text written over some of them cuts it, and selecting across it
  copies the placeholder characters. Cells are limited to 256 rows an image.
- Kitty's cropping, shared-memory transfer and unicode placeholders are not handled. Delete
  removes images in the scrollback too, where kitty only removes visible ones.
- Kitty's file transfer (`t=f`, `t=t`) reads only regular files, not under /proc, /sys or /dev, of
  at most 48 MB. A temporary file is deleted only in the temp directory and when named for the
  protocol. Images that claim more than 256 MB of pixels are refused.
- Images kept for a later kitty `a=p` are capped at 64 MB and 64 images, oldest first.
- A pane keeps at most 256 MB of image pixels; past that the oldest images are dropped and their
  cells go blank, even if still on screen. Images are not freed when they scroll out of the history.
- Inside tmux, `imgcat` doesn't work: it needs tmux's passthrough wrapper (DCS `tmux;`).
- Revisit if sixel is wanted, or if alacritty_terminal gains a hook for the pty bytes.
