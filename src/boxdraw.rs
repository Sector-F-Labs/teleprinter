//! Box drawing, block elements, Braille, scan lines, sextants and the Powerline triangles, drawn from rectangles inside the
//! cell instead of from the font (ADR-0015). MesloLGS's line glyphs are 1.64 em tall in a
//! 1.26 em cell, so as text they spill into the rows above and below.

use egui::{Pos2, Vec2, pos2};

/// A piece of a glyph in cell-local points, origin at the cell's top left.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    /// A filled rectangle, `[x0, y0, x1, y1]`; `alpha` is below 1 for the shade blocks.
    Rect { rect: [f32; 4], alpha: f32 },
    Line { from: Pos2, to: Pos2, width: f32 },
    Triangle([Pos2; 3]),
    Circle { center: Pos2, radius: f32 },
}

/// Whether `c` is drawn here rather than by the font.
pub fn handles(c: char) -> bool {
    matches!(
        c,
        '\u{2500}'..='\u{259F}' | '\u{2800}'..='\u{28FF}' | '\u{23BA}'..='\u{23BD}' | '\u{1FB00}'..='\u{1FB3B}' | '\u{E0B0}' | '\u{E0B2}'
    )
}

/// The pieces of `c` in a cell of the given size, or none if it isn't one of ours.
pub fn pieces(c: char, cell: Vec2) -> Vec<Piece> {
    match c {
        '\u{2500}'..='\u{257F}' => lines(c, cell),
        '\u{2580}'..='\u{259F}' => blocks(c, cell),
        '\u{2800}'..='\u{28FF}' => braille(c, cell),
        '\u{23BA}'..='\u{23BD}' => scan_line(c, cell),
        '\u{1FB00}'..='\u{1FB3B}' => sextants(c, cell),
        '\u{E0B0}' => vec![Piece::Triangle([pos2(0.0, 0.0), pos2(cell.x, cell.y / 2.0), pos2(0.0, cell.y)])],
        '\u{E0B2}' => vec![Piece::Triangle([pos2(cell.x, 0.0), pos2(0.0, cell.y / 2.0), pos2(cell.x, cell.y)])],
        _ => Vec::new(),
    }
}

/// Weights of the arms (left, right, up, down) of U+2500..=U+257F:
/// 0 none, 1 light, 2 heavy, 3 double. Dashed lines are drawn solid, arcs as corners.
#[rustfmt::skip]
const ARMS: [(u8, u8, u8, u8); 0x80] = [
    (1, 1, 0, 0), (2, 2, 0, 0), (0, 0, 1, 1), (0, 0, 2, 2), // 2500
    (1, 1, 0, 0), (2, 2, 0, 0), (0, 0, 1, 1), (0, 0, 2, 2),
    (1, 1, 0, 0), (2, 2, 0, 0), (0, 0, 1, 1), (0, 0, 2, 2),
    (0, 1, 0, 1), (0, 2, 0, 1), (0, 1, 0, 2), (0, 2, 0, 2), // 250C
    (1, 0, 0, 1), (2, 0, 0, 1), (1, 0, 0, 2), (2, 0, 0, 2),
    (0, 1, 1, 0), (0, 2, 1, 0), (0, 1, 2, 0), (0, 2, 2, 0),
    (1, 0, 1, 0), (2, 0, 1, 0), (1, 0, 2, 0), (2, 0, 2, 0),
    (0, 1, 1, 1), (0, 2, 1, 1), (0, 1, 2, 1), (0, 1, 1, 2), // 251C
    (0, 1, 2, 2), (0, 2, 2, 1), (0, 2, 1, 2), (0, 2, 2, 2),
    (1, 0, 1, 1), (2, 0, 1, 1), (1, 0, 2, 1), (1, 0, 1, 2), // 2524
    (1, 0, 2, 2), (2, 0, 2, 1), (2, 0, 1, 2), (2, 0, 2, 2),
    (1, 1, 0, 1), (2, 1, 0, 1), (1, 2, 0, 1), (2, 2, 0, 1), // 252C
    (1, 1, 0, 2), (2, 1, 0, 2), (1, 2, 0, 2), (2, 2, 0, 2),
    (1, 1, 1, 0), (2, 1, 1, 0), (1, 2, 1, 0), (2, 2, 1, 0), // 2534
    (1, 1, 2, 0), (2, 1, 2, 0), (1, 2, 2, 0), (2, 2, 2, 0),
    (1, 1, 1, 1), (2, 1, 1, 1), (1, 2, 1, 1), (2, 2, 1, 1), // 253C
    (1, 1, 2, 1), (1, 1, 1, 2), (1, 1, 2, 2), (2, 1, 2, 1),
    (1, 2, 2, 1), (2, 1, 1, 2), (1, 2, 1, 2), (2, 2, 2, 1),
    (2, 2, 1, 2), (2, 1, 2, 2), (1, 2, 2, 2), (2, 2, 2, 2),
    (1, 1, 0, 0), (2, 2, 0, 0), (0, 0, 1, 1), (0, 0, 2, 2), // 254C
    (3, 3, 0, 0), (0, 0, 3, 3), (0, 3, 0, 1), (0, 1, 0, 3), // 2550
    (0, 3, 0, 3), (3, 0, 0, 1), (1, 0, 0, 3), (3, 0, 0, 3),
    (0, 3, 1, 0), (0, 1, 3, 0), (0, 3, 3, 0), (3, 0, 1, 0),
    (1, 0, 3, 0), (3, 0, 3, 0), (0, 3, 1, 1), (0, 1, 3, 3),
    (0, 3, 3, 3), (3, 0, 1, 1), (1, 0, 3, 3), (3, 0, 3, 3), // 2560
    (3, 3, 0, 1), (1, 1, 0, 3), (3, 3, 0, 3), (3, 3, 1, 0),
    (1, 1, 3, 0), (3, 3, 3, 0), (3, 3, 1, 1), (1, 1, 3, 3),
    (3, 3, 3, 3), (0, 1, 0, 1), (1, 0, 0, 1), (1, 0, 1, 0), // 256C
    (0, 1, 1, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), // 2570, diagonals are lines
    (1, 0, 0, 0), (0, 0, 1, 0), (0, 1, 0, 0), (0, 0, 0, 1), // 2574
    (2, 0, 0, 0), (0, 0, 2, 0), (0, 2, 0, 0), (0, 0, 0, 2),
    (1, 2, 0, 0), (0, 0, 1, 2), (2, 1, 0, 0), (0, 0, 2, 1),
];

/// Light stroke width: an eighth of the cell, at least a point.
fn light(cell: Vec2) -> f32 {
    (cell.x / 8.0).round().max(1.0)
}

fn lines(c: char, cell: Vec2) -> Vec<Piece> {
    let t = light(cell);
    match c {
        '\u{2571}' => return vec![diagonal(cell, true, t)],
        '\u{2572}' => return vec![diagonal(cell, false, t)],
        '\u{2573}' => return vec![diagonal(cell, true, t), diagonal(cell, false, t)],
        _ => {}
    }
    let (left, right, up, down) = ARMS[c as usize - 0x2500];
    let mut out = Vec::new();
    // Horizontal arms run along x, vertical ones along y; `perp` is the arms on the other axis.
    for (w, forward, perp, opposite) in [(left, false, (up, down), right), (right, true, (up, down), left)] {
        for (a0, a1, c0, c1) in arm(w, forward, perp, opposite > 0, t, cell.x, cell.y) {
            out.push(Piece::Rect { rect: [a0, c0, a1, c1], alpha: 1.0 });
        }
    }
    for (w, forward, perp, opposite) in [(up, false, (left, right), down), (down, true, (left, right), up)] {
        for (a0, a1, c0, c1) in arm(w, forward, perp, opposite > 0, t, cell.y, cell.x) {
            out.push(Piece::Rect { rect: [c0, a0, c1, a1], alpha: 1.0 });
        }
    }
    out
}

fn diagonal(cell: Vec2, rising: bool, width: f32) -> Piece {
    let (from, to) = if rising { (pos2(cell.x, 0.0), pos2(0.0, cell.y)) } else { (pos2(0.0, 0.0), pos2(cell.x, cell.y)) };
    Piece::Line { from, to, width }
}

/// The strokes of one arm from the cell's centre to its edge, as `(along0, along1, across0,
/// across1)`. `along` is the cell's size on the arm's axis, `across` on the other. `perp` are the
/// weights of the arms on the other axis (before, after), `opposite` whether the arm on the
/// far side of the centre exists.
fn arm(w: u8, forward: bool, perp: (u8, u8), opposite: bool, t: f32, along: f32, across: f32) -> Vec<(f32, f32, f32, f32)> {
    if w == 0 {
        return Vec::new();
    }
    let width = |w: u8| match w {
        0 => 0.0,
        1 => t,
        2 => 2.0 * t,
        _ => 3.0 * t,
    };
    let ca = (along / 2.0).floor();
    let cc = (across / 2.0).floor();
    // (offset from the centre line, thickness, the perpendicular arm on its side)
    let strokes: &[(f32, f32, u8)] = match w {
        1 => &[(0.0, 1.0, 0)],
        2 => &[(0.0, 2.0, 0)],
        _ => &[(-1.0, 1.0, 1), (1.0, 1.0, 2)],
    };
    let ext = width(perp.0).max(width(perp.1)).max(width(w).min(2.0 * t));
    strokes
        .iter()
        .map(|&(offset, thick, side)| {
            let thick = thick * t;
            let side = match side {
                1 => perp.0,
                2 => perp.1,
                _ => 0,
            };
            // How far the stroke reaches past the centre: it stops at the inner line of a double
            // arm beside it, and otherwise covers the arms it meets.
            let reach = if side == 3 || (w < 3 && perp.0 == 3 && perp.1 == 3 && !opposite) { -t / 2.0 } else { ext / 2.0 };
            let (a0, a1) = if forward { (ca - reach, along) } else { (0.0, ca + reach) };
            let c0 = (cc + offset * t - thick / 2.0).round();
            (a0.round().max(0.0), a1.round().min(along), c0, c0 + thick)
        })
        .collect()
}

/// Braille: eight dots in two columns of four. Bits 0-2 are the left column from the top, 3-5
/// the right, 6 and 7 the bottom row, left then right.
fn braille(c: char, cell: Vec2) -> Vec<Piece> {
    let bits = c as u32 - 0x2800;
    let radius = (cell.x / 4.0).min(cell.y / 8.0) * 0.7;
    let dots = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)];
    dots.iter()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .map(|(_, &(col, row))| Piece::Circle {
            center: pos2(cell.x * (0.25 + 0.5 * col as f32), cell.y * (row as f32 + 0.5) / 4.0),
            radius,
        })
        .collect()
}

/// The VT100 scan lines U+23BA-23BD, at scan 1, 3, 7 and 9 of 9: the top, a quarter, three
/// quarters and the bottom of the cell.
fn scan_line(c: char, cell: Vec2) -> Vec<Piece> {
    let t = light(cell);
    let at = [0.0, 0.25, 0.75, 1.0][(c as u32 - 0x23BA) as usize];
    let y0 = (cell.y * at - t / 2.0).round().clamp(0.0, cell.y - t);
    vec![Piece::Rect { rect: [0.0, y0, cell.x, y0 + t], alpha: 1.0 }]
}

/// Sextants U+1FB00-1FB3B: a 2x3 grid of blocks. The 60 glyphs are the patterns 1-62 without
/// the left half (21) and the right half (42), which are U+258C and U+2590.
fn sextants(c: char, cell: Vec2) -> Vec<Piece> {
    let mut pattern = c as u32 - 0x1FB00 + 1;
    if pattern >= 21 {
        pattern += 1;
    }
    if pattern >= 42 {
        pattern += 1;
    }
    (0..6)
        .filter(|i| pattern & (1 << i) != 0)
        .map(|i| {
            let (col, row) = ((i % 2) as f32, (i / 2) as f32);
            let rect = [col * 0.5 * cell.x, row * cell.y / 3.0, (col + 1.0) * 0.5 * cell.x, (row + 1.0) * cell.y / 3.0];
            Piece::Rect { rect, alpha: 1.0 }
        })
        .collect()
}

fn blocks(c: char, cell: Vec2) -> Vec<Piece> {
    let part = |x0: f32, y0: f32, x1: f32, y1: f32| [x0 * cell.x, y0 * cell.y, x1 * cell.x, y1 * cell.y];
    let solid = |rects: &[[f32; 4]]| rects.iter().map(|&rect| Piece::Rect { rect, alpha: 1.0 }).collect();
    let n = (c as u32 - 0x2580) as f32;
    // Quadrants: upper left, upper right, lower left, lower right.
    let quads = |ul: bool, ur: bool, ll: bool, lr: bool| {
        let mut rects = Vec::new();
        for (on, x, y) in [(ul, 0.0, 0.0), (ur, 0.5, 0.0), (ll, 0.0, 0.5), (lr, 0.5, 0.5)] {
            if on {
                rects.push(part(x, y, x + 0.5, y + 0.5));
            }
        }
        solid(&rects)
    };
    match c as u32 {
        0x2580 => solid(&[part(0.0, 0.0, 1.0, 0.5)]),
        0x2581..=0x2588 => solid(&[part(0.0, 1.0 - n / 8.0, 1.0, 1.0)]),
        0x2589..=0x258F => solid(&[part(0.0, 0.0, (0x2590 - c as u32) as f32 / 8.0, 1.0)]),
        0x2590 => solid(&[part(0.5, 0.0, 1.0, 1.0)]),
        0x2591..=0x2593 => vec![Piece::Rect { rect: part(0.0, 0.0, 1.0, 1.0), alpha: (n - 0x10 as f32) / 4.0 }],
        0x2594 => solid(&[part(0.0, 0.0, 1.0, 0.125)]),
        0x2595 => solid(&[part(0.875, 0.0, 1.0, 1.0)]),
        0x2596 => quads(false, false, true, false),
        0x2597 => quads(false, false, false, true),
        0x2598 => quads(true, false, false, false),
        0x2599 => quads(true, false, true, true),
        0x259A => quads(true, false, false, true),
        0x259B => quads(true, true, true, false),
        0x259C => quads(true, true, false, true),
        0x259D => quads(false, true, false, false),
        0x259E => quads(false, true, true, false),
        _ => quads(false, true, true, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: Vec2 = Vec2::new(8.0, 18.0);

    /// Bounding box of all the rectangles of `c`.
    fn bounds(c: char) -> [f32; 4] {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in pieces(c, CELL) {
            if let Piece::Rect { rect, .. } = p {
                b = [b[0].min(rect[0]), b[1].min(rect[1]), b[2].max(rect[2]), b[3].max(rect[3])];
            }
        }
        b
    }

    #[test]
    fn nothing_leaves_the_cell() {
        for c in ('\u{2500}'..='\u{259F}')
            .chain('\u{2800}'..='\u{28FF}')
            .chain('\u{23BA}'..='\u{23BD}')
            .chain('\u{1FB00}'..='\u{1FB3B}')
            .chain(['\u{E0B0}', '\u{E0B2}'])
        {
            for p in pieces(c, CELL) {
                let points: Vec<Pos2> = match p {
                    Piece::Rect { rect, .. } => vec![pos2(rect[0], rect[1]), pos2(rect[2], rect[3])],
                    Piece::Line { from, to, .. } => vec![from, to],
                    Piece::Triangle(t) => t.to_vec(),
                    Piece::Circle { center, radius } => {
                        vec![center - Vec2::splat(radius), center + Vec2::splat(radius)]
                    }
                };
                for q in points {
                    assert!((0.0..=CELL.x).contains(&q.x) && (0.0..=CELL.y).contains(&q.y), "{c:?} has {q:?}");
                }
            }
        }
    }

    #[test]
    fn a_vertical_line_runs_the_full_height() {
        for c in ['\u{2502}', '\u{2503}', '\u{2551}'] {
            let b = bounds(c);
            assert_eq!((b[1], b[3]), (0.0, CELL.y), "{c:?}");
        }
        let b = bounds('\u{2502}');
        assert_eq!(b[2] - b[0], 1.0, "light line is one point wide");
    }

    #[test]
    fn corners_stop_at_the_cell_edge_they_do_not_reach() {
        let b = bounds('\u{250C}');
        assert_eq!((b[2], b[3]), (CELL.x, CELL.y));
        assert!(b[0] > 0.0 && b[1] > 0.0, "┌ doesn't reach the left or top: {b:?}");
    }

    #[test]
    fn full_block_fills_the_cell_and_halves_split_it() {
        assert_eq!(bounds('\u{2588}'), [0.0, 0.0, CELL.x, CELL.y]);
        assert_eq!(bounds('\u{2584}'), [0.0, CELL.y / 2.0, CELL.x, CELL.y]);
        assert_eq!(bounds('\u{258C}'), [0.0, 0.0, CELL.x / 2.0, CELL.y]);
    }

    #[test]
    fn shades_are_translucent() {
        let alpha = |c| match &pieces(c, CELL)[0] {
            Piece::Rect { alpha, .. } => *alpha,
            _ => unreachable!(),
        };
        assert_eq!((alpha('\u{2591}'), alpha('\u{2592}'), alpha('\u{2593}')), (0.25, 0.5, 0.75));
    }

    #[test]
    fn braille_has_a_dot_per_set_bit() {
        assert!(pieces('\u{2800}', CELL).is_empty());
        assert_eq!(pieces('\u{28FF}', CELL).len(), 8);
        assert_eq!(pieces('\u{2801}', CELL).len(), 1);
        let Piece::Circle { center, .. } = pieces('\u{2840}', CELL)[0] else { unreachable!() };
        assert!(center.x < CELL.x / 2.0 && center.y > CELL.y * 0.75, "dot 7 is bottom left");
    }

    #[test]
    fn scan_lines_run_across_from_top_to_bottom() {
        let ys: Vec<f32> = ('\u{23BA}'..='\u{23BD}').map(|c| bounds(c)[1]).collect();
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
        assert_eq!(bounds('\u{23BA}')[0..3], [0.0, 0.0, CELL.x][..]);
    }

    #[test]
    fn sextants_skip_the_halves() {
        let area = |c| pieces(c, CELL).len();
        assert_eq!(area('\u{1FB00}'), 1);
        assert_eq!(area('\u{1FB3B}'), 5);
        // U+1FB13 is pattern 20; U+1FB14 is 22, the left half (21) being skipped.
        assert_eq!(area('\u{1FB13}'), 2);
        assert_eq!(area('\u{1FB14}'), 3);
        assert_eq!(bounds('\u{1FB3B}'), [0.0, 0.0, CELL.x, CELL.y]);
    }

    #[test]
    fn only_our_ranges_are_handled() {
        assert!(handles('│') && handles('█') && handles('\u{E0B0}'));
        assert!(!handles('a') && !handles('\u{E0B1}') && !handles('\u{25A0}'));
        assert!(pieces('a', CELL).is_empty());
    }
}
