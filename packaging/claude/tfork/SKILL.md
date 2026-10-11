---
name: tfork
description: Teleprinter fork. Fork this Claude Code conversation into a new Teleprinter pane below you, as a separate interactive session that starts with this conversation's history. Use when the user types /tfork, or asks to fork, branch or split off this conversation into a pane. Needs $TELEPRINTER_SOCKET (running inside Teleprinter).
---

# /tfork: fork this conversation into a pane

`claude --resume <session-id> --fork-session` starts a new session with a copy of this one's
history; running it through `teleprinter msg split` (see the `teleprinter-panes` skill) puts it in
a pane below you. The fork and this session go their own ways from there.

Arguments: $ARGUMENTS

- `--down` (default), `--up`, `--left` or `--right` place the pane. Forks go below, so the
  right side stays free for the panes that show things (nvim tours, man pages, logs).
- Anything else is the fork's first prompt.

## Do it

1. If `$TELEPRINTER_SOCKET` is unset, say you're not running inside Teleprinter and stop.
2. Run, from the session's **primary working directory** (sessions are stored per project
   directory, so `--resume` only finds this one from there):

   ```bash
   DIR=--down    # from the arguments
   PROMPT=''     # the rest of the arguments, if any
   teleprinter msg split "$DIR" --cwd "<primary working directory>" -- \
     env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_SESSION_ID \
         -u CLAUDE_CODE_CHILD_SESSION -u CLAUDE_CODE_SESSION_ATTENDED -u CLAUDE_PID \
         -u CLAUDE_CODE_MESSAGING_SOCKET -u CLAUDE_CODE_MESSAGING_TOKEN \
     claude --resume "$CLAUDE_CODE_SESSION_ID" --fork-session ${PROMPT:+"$PROMPT"}
   ```

   A pane opened over the socket gets your environment, so the `env -u` drops the variables that
   tie a shell to *this* Claude session; without it the fork thinks it's running inside you.

3. Reply in one line: where the fork is and its pane id. Don't pass `--focus`; the user
   clicks the pane when they want it.

If `CLAUDE_CODE_SESSION_ID` is empty, this session's transcript is the newest
`~/.claude/projects/<project dir with / as ->/*.jsonl`; its file name is the id. Say that you
guessed.
