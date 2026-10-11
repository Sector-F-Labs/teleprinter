# Teleprinter

A terminal emulator in Rust on egui and [Patina](https://github.com/Sector-F-Labs/patina), with
`alacritty_terminal` doing the emulation (ADR-0001). Private repo; origin is on urd.

Significant decisions are ADRs in `docs/adr/` (see its README, and the `adr` skill for adding
one). Read the relevant ones before changing fonts, colours or key handling.

## Commands

```bash
make test          # unit tests (key mapping, palette, cell hit-testing)
make check         # tests + clippy -D warnings
make run           # run it (your login shell)
make screenshot    # docs/screenshot-*.png, running scripts/demo.sh in a throwaway config dir
```

Never touch the user's config: test runs set `XDG_CONFIG_HOME`/`XDG_DATA_HOME` to scratch dirs
(as `make screenshot` does). `TELEPRINTER_COMMAND` runs a command through `sh -c` instead of the
login shell, `TELEPRINTER_SKIN` picks a skin for one run, `TELEPRINTER_SCREENSHOT` saves a PNG
and quits. Check visual changes by reading the screenshot back.

## Layout

- `src/session.rs`: the pty and alacritty_terminal's event loop; input, paste, resize, events
  (title, clipboard store, colour queries, child exit). Each event requests a repaint.
- `src/view.rs`: the widget: cell metrics, painting, key → bytes (`key_bytes`), mouse selection,
  wheel scrolling.
- `src/palette.rs`: ANSI palette from the skin's `[app.teleprinter]`, else from the visuals
  (ADR-0003).
- `src/fonts.rs`: bundled MesloLGS NF (ADR-0002).
- `src/app.rs`: window, Patina title bar and menus, skins, screenshot mode.
- `src/config.rs`: `~/.config/teleprinter/config.toml` (skin, system title bar, font size).

## Gotchas

- egui turns Ctrl+C/X/V into Copy/Cut/Paste events with no Key event; `view::handle_keys` checks
  Shift to tell clipboard from `^C` (ADR-0004).
- egui drops widget focus when the window loses focus; the widget takes it back whenever nothing
  else holds it.
- Alt+letter arrives as a Key and then a Text event; the Key sends ESC+letter and the Text is
  swallowed.
- Only ASCII cells are joined into text runs. Other glyphs (Nerd icons, CJK) are placed per cell
  so columns stay aligned.
