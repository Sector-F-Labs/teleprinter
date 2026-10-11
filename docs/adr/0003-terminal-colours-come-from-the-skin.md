# 0003. Terminal colours come from the skin

Date: 2026-10-11

## Status

Proposed

## Context

A terminal has its own palette: 16 ANSI colours plus foreground, background, cursor and
selection. Patina's rule is that every colour an app paints comes from the skin or the visuals,
and that the app follows the desktop's light/dark setting. Most terminals keep the palette
separate from the toolkit's theme, which is why they clash when the desktop switches.

## Decision

We will take the palette from the skin's `[app.teleprinter]` section (`foreground`,
`background`, `cursor`, `selection`, `black` … `bright_white`), with every key optional.
Anything a skin doesn't set comes from the visuals: background from `extreme_bg_color`, text and
cursor from the strong text colour, selection from the selection colour. The ANSI colours default
to GNOME Terminal's Tango palette, in a light or dark variant picked by the visuals. The palette
is rebuilt every frame, so a light/dark switch reaches the terminal at once. Colours a program
sets with OSC 4/10/11 override the palette for that session.

## Consequences

- Every skin gets a terminal that matches it, without having to write a palette.
- Skins can't yet swap palettes independently of the rest of the look (e.g. Solarized in Dark
  Orange); that would need named palettes.
- If another app needs ANSI colours, the keys should move into Patina's shared skin format.
