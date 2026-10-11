# <img src="packaging/linux/teleprinter.svg" width="48" alt=""> Teleprinter

A terminal emulator in the [Patina](https://github.com/Sector-F-Labs/patina) family: Patina's
window frame and skins, following the desktop's light/dark setting, with
[`alacritty_terminal`](https://crates.io/crates/alacritty_terminal) doing the emulation and
MesloLGS NF as the font.

![Teleprinter in the Notebook skin](docs/screenshot-notebook.png)

Right-click a terminal to **Split Right** or **Split Down**. Panes tile the window, each in a
window of its own titled with what's running in it and where, as tmux names its windows
(`vim — ~/Src/teleprinter`, back to `zsh — ~/Src/teleprinter` when vim exits). Drag a pane's title bar onto another to move it to that side, as
in a tiling window manager (hold still over a pane to swap the two instead), and drag the gap
between panes to resize. Collapse a pane with the arrow on its title bar (or
double-click the title): in a stack, its neighbours take the space, so a column can hold many
folded panes whose titles still show what's running.

![Three panes in the Dark Orange skin](docs/screenshot-split-dark-orange.png)

![Two folded panes in a stack, Notebook skin](docs/screenshot-collapsed-notebook.png)

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
| Drag a pane's title bar onto another | Move it to that side of the other pane |
| …and hold still before letting go | Swap the two panes |
| Right-click the background | New Terminal in that space |
| Drag the gap between panes | Resize them |
| Title bar arrow, or double-click the title | Collapse / expand the pane |
| Mouse wheel | Scroll history (arrow keys in full-screen programs like `less`) |

Ctrl+C, Ctrl+V and the rest go to the shell as usual.

## Build

```sh
make            # list the targets
make run        # run it
make check      # tests and clippy
make screenshot # regenerate docs/screenshot-*.png
make install    # per user, into ~/.local (PREFIX=… to change), with launcher and icon
make uninstall  # remove it again
make icons      # re-render assets/teleprinter-icon-128.png from packaging/linux/teleprinter.svg
```

## Status

A first prototype. Not yet: keyboard shortcuts for splitting and moving between panes, saving the
layout, mouse reporting to programs (vim `mouse=a`, tmux), input methods,
tabs, choosing a font, fonts for CJK and other scripts Meslo lacks.

## Licence

BSD-3-Clause. The bundled MesloLGS NF fonts are Apache 2.0; see `assets/fonts/`.
