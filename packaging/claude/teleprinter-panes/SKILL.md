---
name: teleprinter-panes
description: Show the user code in a new Teleprinter pane beside you, with nvim taken to the exact lines and highlighted, and step through a guided tour of them. Use when $TELEPRINTER_SOCKET is set (you're running inside Teleprinter) and the user asks you to explain, walk through, show or review code or a change you made ("explain what you did", "show me where", "walk me through the diff"). Also for opening a pane to tail a log or watch a command beside you. Don't use it when $TELEPRINTER_SOCKET is unset.
---

# Showing code in a Teleprinter pane

Teleprinter lets programs in a pane open panes beside themselves (its ADR-0009). Every shell it
runs has `TELEPRINTER_SOCKET` (that window's control socket) and `TELEPRINTER_PANE` (your own pane's
id), so `teleprinter msg …` always acts on the window you're running in.

Check first: `[ -n "$TELEPRINTER_SOCKET" ]`. If it's unset, explain in text as usual. Don't hunt
for sockets in `$XDG_RUNTIME_DIR/teleprinter/`: they belong to other windows.

## The commands

```bash
teleprinter msg split [--left|--right|--up|--down] [--pane ID] [--cwd DIR] [--focus] [-- PROGRAM ARGS…]
teleprinter msg list            # panes: id, title, cwd, rect, collapsed, active
teleprinter msg close [ID]
teleprinter msg focus ID
```

Each prints one line of JSON: `{"ok":true,"pane":4}` or `{"ok":false,"error":"…"}`, exiting
non-zero on failure. `split` opens beside your pane (default `--right`) and runs PROGRAM in place
of a shell; the pane closes when PROGRAM exits. Its directory defaults to your pane's.

**Never pass `--focus` unless the user asks.** They're typing to you; a pane that takes the
keyboard sends their next message into nvim.

## A code tour in nvim

Use this to explain a change or a piece of code: one quickfix entry per place, which you then step
through while explaining each one in chat.

1. **Write the stops** as a quickfix file, `path:line:col: note`, one per line, in the order you'll
   explain them. Use absolute paths. Keep each note to one short line; the real explanation goes in
   your reply.

   ```bash
   TOUR="$XDG_RUNTIME_DIR/claude-tour-$TELEPRINTER_PANE"
   cat > "$TOUR.qf" <<'EOF'
   /home/me/src/app/src/server.rs:96:5: binds the socket, mode 0600
   /home/me/src/app/src/app.rs:150:5: drains requests once per frame
   EOF
   ```

2. **Open nvim on it beside you**, listening on a socket so you can drive it:

   ```bash
   rm -f "$TOUR.sock"
   teleprinter msg split -- nvim --listen "$TOUR.sock" -q "$TOUR.qf"
   for i in $(seq 30); do [ -S "$TOUR.sock" ] && break; sleep 0.1; done
   ```

   Note the pane id from the reply. nvim opens at the first stop.

3. **Move between stops** as your explanation reaches each one. Start every send with
   `<C-\><C-N>` so it works whatever mode the user left nvim in:

   ```bash
   N() { nvim --server "$TOUR.sock" --remote-send "<C-\\><C-N>$1"; }
   N ':cc 2<CR>zz'              # stop 2, centred
   N ':120<CR>zzV133G'          # highlight lines 120–133 (visual line selection)
   N ':e /abs/path/file.rs<CR>:42<CR>zz'   # anywhere, outside the tour
   ```

   To read nvim's state rather than change it, use `nvim --server "$TOUR.sock" --remote-expr
   'line(".")'`.

4. **Tell the user** there's a pane on the right with the tour, and that they can move through it
   themselves with `:cnext`/`:cprev` (or `]q`/`[q`) and `:copen` for the list.

5. **When you're done**, leave the pane open; the user closes it. If they ask you to close it, or
   you're opening a new tour, close the old one first:
   `nvim --server "$TOUR.sock" --remote-send '<C-\><C-N>:qa!<CR>'` (or `teleprinter msg close ID`),
   then `rm -f "$TOUR.sock" "$TOUR.qf"`.

## Gotchas

- **Socket paths are limited to 108 bytes.** With a longer `--listen` path, nvim exits at once and
  the pane vanishes silently. Keep sockets in `$XDG_RUNTIME_DIR` with short names, never in deep
  scratch directories.
- **Line numbers go stale** if you edit a file after writing the tour. Write the tour after your
  edits, from the files as they are (`grep -n`), not from memory of the diff.
- **One tour at a time.** If `$TOUR.sock` already exists and answers, reuse it (`:cfile $TOUR.qf`
  reloads the list) instead of opening a second pane.
- **For a diff rather than locations**, a pane running `git diff` through a pager works too:
  `teleprinter msg split -- sh -c 'git diff | less -R'`.
- **`teleprinter: command not found`, or `msg` unknown:** the installed build predates the control
  socket. Say so and explain in text instead.
