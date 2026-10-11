# 0004. Ctrl+Shift+C/V for the clipboard; Ctrl+C stays an interrupt

Date: 2026-10-11

## Status

Proposed

## Context

egui turns Ctrl+C, Ctrl+X and Ctrl+V into Copy, Cut and Paste events, with no key event, but in
a terminal those keys belong to the shell: Ctrl+C interrupts, Ctrl+V is a literal next in vim and
the shell. GNOME Terminal, Konsole and others use Ctrl+Shift+C/V for the clipboard.

## Decision

We will follow that convention: Ctrl+Shift+C copies the selection and Ctrl+Shift+V pastes;
Ctrl+C, Ctrl+X and Ctrl+V without Shift send `^C`, `^X` and `^V`. The Edit menu and the
right-click menu offer Copy and Paste too. Pastes are bracketed when the program asked for
bracketed paste, so a shell doesn't run pasted lines as they arrive.

## Consequences

- Behaves like the terminals the user already knows on Linux.
- Copying needs Shift, unlike every other Patina app; the Edit menu shows the shortcuts.
