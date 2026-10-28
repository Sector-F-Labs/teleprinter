# 0015. Box drawing and block glyphs drawn in the cell

Date: 2026-10-28

## Status

Accepted

## Context

MesloLGS NF's line glyphs (U+2500-257F) are 3359 units tall in a cell of 2584 (1.64 em in
1.26 em), and the blocks and Powerline separators overshoot by about 50 units. We paint every
cell as text after all the backgrounds, so tmux's pane borders poked into the rows above and
below, over junctions and the status line (issue 1). Clipping the glyphs to their cell would stop
the spill but leave gaps and uneven joins, as they were designed for a taller cell.

## Decision

We will draw box drawing, block elements (U+2580-259F) and the two solid Powerline triangles
(U+E0B0, U+E0B2) from rectangles, lines and triangles inside the cell (`src/boxdraw.rs`), in the
cell's foreground colour. Strokes are an eighth of the cell wide, at least a point; heavy is
twice that, and double lines are two light strokes a light stroke apart. Dashed lines are drawn
solid and arcs as square corners. Shade blocks are the foreground at 25, 50 and 75% opacity. Other
Powerline glyphs stay with the font.

## Consequences

- Borders and blocks meet across rows and columns with no seams and never leave their cell.
- They no longer follow the font's weight or the bold/italic flags.
- Dashes and arcs look plainer than the font's; draw them if anyone minds.
- The other Powerline glyphs (outlines, rounded ends, flames) can still overshoot a little.
