# 0013. Colour emoji from a bundled Noto Color Emoji, painted as images

Date: 2026-10-18

## Status

Accepted

## Context

MesloLGS NF (ADR-0002) has no emoji, so emoji fell back to egui's own fonts: NotoEmoji-Regular
and emoji-icon-font, both monochrome. Other terminals (Ptyxis, WezTerm) ask fontconfig and get
Noto Color Emoji in colour. Giving egui a colour font wouldn't help: epaint 0.35 draws every glyph
from its outline into a coverage-only atlas tinted with the text colour, and never reads COLR,
CBDT or sbix.

So emoji have to be painted as images, outside egui's text. The sets considered were Twemoji
(`egui-twemoji` wraps it, but targets egui 0.33; its images are CC-BY, so need a credit), Noto
Color Emoji (OFL font, Apache 2.0 images, and what the user sees in their other terminals) and
OpenMoji (CC BY-SA). For Noto, the options were bundling the repo's ~3,700 PNGs, keyed by file
name, or bundling the font and reading it: its cmap and GSUB ligatures resolve sequences, and
its CBDT table holds a 136 px PNG per glyph. The COLRv1 build is smaller but would need a vector
rasteriser.

## Decision

We will bundle Noto Color Emoji's CBDT font (`assets/fonts/NotoColorEmoji.ttf`, OFL) and paint
emoji from it ourselves (`src/emoji.rs`, with `ttf-parser`). A cell is looked up when it's wide,
a regional indicator, or carries VS16 or a keycap mark, so plain digits and `#` stay text. Since
alacritty_terminal puts each part of a ZWJ sequence, skin tone or flag in its own cell, the view
joins neighbouring cells into one sequence before the lookup. The glyph's PNG becomes a texture
(made once per glyph, with mipmaps) drawn in at most two cells; anything the font doesn't know
falls back to egui's text as before.

## Consequences

- Emoji look as they do in GTK terminals, without depending on installed fonts.
- The binary grows by about 11 MB, to roughly twice its fonts' previous size.
- A ZWJ family takes six cells to the shell but is drawn in the first two, leaving a gap, as
  wcwidth-based terminals do. Narrow emoji with VS16 (❤️) are drawn in one cell, so small.
- Emoji ignore the foreground colour, inverse and dim.
- Revisit if egui gains colour glyphs (the CBDT parsing could go), or if a user wants the system
  emoji font: fontconfig's file could be read in place of the bundled one.
