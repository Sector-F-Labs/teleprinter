# 0006. Pane titles follow the foreground program, as tmux names windows

Date: 2026-10-11

## Status

Accepted

## Context

Pane titles came from the title a shell sets with OSC 0/2, so a shell that sets none showed
"Shell" in every pane, and nothing said which pane had vim and which had a build running. The user
asked for titles that change with what's in the pane, as tmux does: its automatic window names are
the foreground program (`vim`, `htop`, back to `zsh`).

There's no event for a new foreground program. tmux re-checks on a timer (`status-interval`); the
user prefers event-driven updates over polling.

## Decision

- **A pane's title is `program — directory`**, e.g. `vim — ~/Src/teleprinter`, with the home
  directory shortened to `~`. The program is the pty's foreground process group
  (`tcgetpgrp` on a duplicate of the pty's fd), and its name and directory come from
  `/proc/<pid>/comm` and `/proc/<pid>/cwd`.
- **Output is the cue to look again,** at most every 200 ms: a new program nearly always writes
  something (vim drawing, the prompt coming back). Each burst of output also gets one more look
  200 ms later, so a program that starts silently right after Enter's echo (`sleep`, a quiet build)
  is still caught. An idle pane does no work.
- **Where /proc can't be read** (FreeBSD without linprocfs, macOS) the title falls back to the
  shell's OSC title, then "Shell".
- The window title is the active pane's title.

## Consequences

- Panes say what's running and where, whatever the shell's configuration.
- A program that starts more than 200 ms after the last output and never writes keeps the old
  title until something does.
- An OSC title a program sets (vim's file name, a remote shell's `user@host`) is no longer shown
  on Linux; `ssh` shows as `ssh — <local dir>`. Showing the OSC title when the foreground isn't
  the shell is the obvious next step if that's missed.
- FreeBSD support needs its own lookup (`sysctl kern.proc.pid`) to get the same titles.
