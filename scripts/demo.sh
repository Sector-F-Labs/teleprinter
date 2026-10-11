#!/bin/sh
# What `make screenshot` shows: the palette, text styles, box drawing and a few Nerd Font icons.
printf '\033]0;teleprinter demo\007'
printf '\033[1mTeleprinter\033[0m — Patina terminal on alacritty_terminal\n\n'
for i in 0 1 2 3 4 5 6 7; do printf '\033[4%sm   \033[0m' "$i"; done; printf '\n'
for i in 0 1 2 3 4 5 6 7; do printf '\033[10%sm   \033[0m' "$i"; done; printf '\n\n'
printf '\033[31mred\033[0m \033[32mgreen\033[0m \033[33myellow\033[0m \033[34mblue\033[0m \033[35mmagenta\033[0m \033[36mcyan\033[0m\n'
printf '\033[1mbold\033[0m \033[3mitalic\033[0m \033[1;3mbold italic\033[0m \033[4munderline\033[0m \033[9mstrike\033[0m \033[2mdim\033[0m \033[7minverse\033[0m\n\n'
i=16; while [ $i -lt 52 ]; do printf '\033[48;5;%sm  ' "$i"; i=$((i+1)); done; printf '\033[0m\n'
i=232; while [ $i -lt 256 ]; do printf '\033[48;5;%sm   ' "$i"; i=$((i+1)); done; printf '\033[0m\n\n'
printf '┌──────────┬──────────┐\n│ box      │ drawing  │\n└──────────┴──────────┘\n\n'
printf '\356\234\250 git  \356\230\225 rust  \357\201\273 folder  \357\214\233 freebsd  — こんにちは\n\n'
printf '\033[32m~/Src\033[0m $ '
sleep 30
