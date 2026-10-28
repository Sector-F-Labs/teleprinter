# 0016. Braille, scan lines and sextants drawn in the cell

Date: 2026-10-28

## Status

Accepted

## Context

MesloLGS NF has no Braille patterns (U+2800-28FF), no scan lines (U+23BA-23BD, which
alacritty_terminal produces for the VT100 line-drawing set) and no Symbols for Legacy Computing.
egui's fallback fonts have none of them either, so each drew as the small `◻` replacement square
(issue 2). btop, bottom and plotting libraries use Braille; newer TUIs use sextants.

## Decision

We will draw these from the cell's geometry in `src/boxdraw.rs`, as ADR-0015 does for box drawing:
Braille as eight round dots on a 2x4 grid, the scan lines as light strokes at the top, a quarter,
three quarters and the bottom of the cell, and the sextants U+1FB00-1FB3B as a 2x3 grid of
blocks. We will not bundle another fallback font for them.

## Consequences

- They fit the cell exactly, join up across cells and never show as tofu.
- They ignore the font's weight and the bold/italic flags.
- Other missing symbols (U+2424, U+240B, U+2B24, the rest of the legacy-computing block, the
  Powerline gaps at U+E0A4-E0AF, U+E0C9, U+E0CB, U+E0D3) still fall back to `◻`.
