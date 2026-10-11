# Teleprinter

A terminal emulator in the [Patina](https://github.com/Sector-F-Labs/patina) family: Patina's
window frame and skins, following the desktop's light/dark setting, with
[`alacritty_terminal`](https://crates.io/crates/alacritty_terminal) doing the emulation and
MesloLGS NF as the font.

![Teleprinter in the Notebook skin](docs/screenshot-notebook.png)

Right-click a terminal to **Split Right** or **Split Down**. Panes tile the window, each in a
window of its own titled with what's running in it and where, as tmux names its windows
(`vim — ~/Src/teleprinter`, back to `zsh — ~/Src/teleprinter` when vim exits). Drag a pane's title bar onto another to swap them, and
drag the gap between panes to resize.

![Three panes in the Dark Orange skin](docs/screenshot-split-dark-orange.png)

Each skin colours the terminal too. Skins can set their own palette under `[app.teleprinter]`
(`foreground`, `background`, `cursor`, `selection`, `black` … `bright_white`); anything left out
comes from the skin's own colours and a light or dark Tango palette.

## Shortcuts

| Keys | Does |
|------|------|
| Ctrl+Shift+C | Copy the selection |
| Ctrl+Shift+V | Paste |
| Shift+Page Up / Down | Scroll back through history |
| Mouse drag / double / triple click | Select text / a word / lines |
| Right-click → Split Right / Split Down | Split the pane (also in File) |
| Drag a pane's title bar onto another | Swap the two panes |
| Drag the gap between panes | Resize them |
| Mouse wheel | Scroll history (arrow keys in full-screen programs like `less`) |

Ctrl+C, Ctrl+V and the rest go to the shell as usual.

## Build

```sh
make            # list the targets
make run        # run it
make check      # tests and clippy
make screenshot # regenerate docs/screenshot-*.png
```

## Status

A first prototype. Not yet: keyboard shortcuts for splitting and moving between panes, saving the
layout, mouse reporting to programs (vim `mouse=a`, tmux), input methods,
tabs, choosing a font, fonts for CJK and other scripts Meslo lacks.

## Licence

BSD-3-Clause. The bundled MesloLGS NF fonts are Apache 2.0; see `assets/fonts/`.
