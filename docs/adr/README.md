# Architecture Decision Records

This directory holds ADRs for Teleprinter — short records of significant architecture decisions,
written at the time they're made, following Michael Nygard's format as popularized by Martin
Fowler: <https://martinfowler.com/bliki/ArchitectureDecisionRecord.html>.

## When to write one

Write an ADR when a decision is:

- Hard, or expensive, to reverse once other code depends on it.
- Not obvious from reading the code — a "why", not a "what".
- The kind of thing that'll get re-litigated later without a record ("didn't we already decide
  this?").

Small, reversible choices don't need one. Rule of thumb: if you'd want to explain the choice to a
new contributor in a paragraph beyond what the code itself shows, it's ADR-sized.

An ADR is a point-in-time record of *a decision as it was made*, including the options considered
and rejected. It does not get rewritten later — a decision that reverses or replaces an earlier one
gets its own new ADR that supersedes the old one, which stays in place with its status updated.

## Convention

- One file per decision: `NNNN-short-kebab-title.md`, numbered sequentially, zero-padded to 4
  digits. Numbers are never reused, even for a decision that gets reversed.
- Start from [`0000-template.md`](0000-template.md) — copy it, don't write from scratch.
- `Status` is one of: `Proposed`, `Accepted`, `Rejected`, `Deprecated`, or `Superseded by
  ADR-NNNN` (linked to the superseding record).
- Every new ADR gets a row in the index below.
- Keep records short — a paragraph or two per section is normal. An ADR documents a decision and
  its reasoning, not a design document.

Teleprinter is a [Patina](https://github.com/Sector-F-Labs/patina) app. Decisions about the shared
frame, skins and components are recorded in Patina's own ADRs; record here only what this app
decides.

If you're working with Claude Code in this repo, `.claude/skills/adr/SKILL.md` automates the
mechanics of adding one.

## Index

| # | Title | Status |
|---|-------|--------|
| [0001](0001-build-on-patina-with-alacritty-terminal.md) | Build Teleprinter on Patina, with alacritty_terminal for the emulation | Accepted |
| [0002](0002-bundle-meslolgs-nf.md) | Bundle MesloLGS NF as the terminal font | Accepted |
| [0003](0003-terminal-colours-come-from-the-skin.md) | Terminal colours come from the skin | Proposed |
| [0004](0004-ctrl-shift-for-the-clipboard.md) | Ctrl+Shift+C/V for the clipboard; Ctrl+C stays an interrupt | Proposed |

## Decision Relationship

```mermaid
flowchart TD
    A[0001: Patina + alacritty_terminal] --> B[0002: bundled MesloLGS NF]
    A --> C[0003: colours from the skin]
    A --> D[0004: Ctrl+Shift clipboard]
```

## Revisit Triggers

- egui's text layout can't keep up with fast output (`cat` of a large file stutters): consider
  caching rows or a glyph atlas of our own ([0001](0001-build-on-patina-with-alacritty-terminal.md)).
- A second Patina app wants a monospace font: move Meslo into Patina
  ([0002](0002-bundle-meslolgs-nf.md)).
- A second app wants an ANSI palette (e.g. a log viewer): move the palette keys into Patina's shared
  skin format ([0003](0003-terminal-colours-come-from-the-skin.md)).
