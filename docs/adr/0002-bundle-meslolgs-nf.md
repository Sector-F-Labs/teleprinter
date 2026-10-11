# 0002. Bundle MesloLGS NF as the terminal font

Date: 2026-10-11

## Status

Accepted

## Context

A terminal needs a monospace font with box drawing, and the user's prompt needs Nerd Font icons.
Patina's rule is stock egui fonts unless an app truly needs its own, and egui's built-in
monospace has neither. The user's preferred font is MesloLGS NF (the powerlevel10k build of
Meslo LG with the Nerd Font glyphs patched in), from their dotfiles. UbuntuMono Nerd Font and
JetBrains Mono Nerd Font were the alternatives if Meslo couldn't be shipped.

The licence was checked before bundling: the files are byte-identical to
`romkatv/powerlevel10k-media`, whose `MesloLGS NF License.txt` is Apache 2.0 (André Berg). Meslo
derives from Apple's Menlo, itself from Bitstream Vera and DejaVu, whose licences allow bundling
with software.

## Decision

We will bundle MesloLGS NF's regular, bold, italic and bold-italic faces in the binary
(`include_bytes!`), with egui's own fonts behind them as fallback, and keep the licence in
`assets/fonts/`. The font lives in Teleprinter, not Patina, until a second app wants it.

## Consequences

- The terminal looks the same everywhere and needs no installed fonts.
- The binary carries about 10 MB of fonts.
- A user can't yet choose another font; a config setting with Meslo as the fallback is the
  likely next step.
- Scripts Meslo and egui's fonts lack (CJK) show as boxes until there's a system-font fallback.
