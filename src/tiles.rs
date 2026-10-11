//! The tiled layout (ADR-0005, ADR-0008): a tree of rows and columns,
//! each holding any number of children with a weight each, as a tiling
//! window manager arranges windows. Splitting a pane halves it; moving one
//! takes it out of its row or column and inserts it beside another pane;
//! swapping exchanges two leaves.

use egui::{Pos2, Rect, pos2};

pub type PaneId = u64;

/// Which way a split runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// A row: children side by side, new ones to the right.
    Right,
    /// A column: children stacked, new ones below.
    Down,
}

/// Which side of a pane another one is put on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn dir(self) -> Dir {
        match self {
            Side::Left | Side::Right => Dir::Right,
            Side::Top | Side::Bottom => Dir::Down,
        }
    }

    fn after(self) -> bool {
        matches!(self, Side::Right | Side::Bottom)
    }

    /// The side of `rect` that `pos` is nearest, relative to its size.
    pub fn nearest(rect: Rect, pos: Pos2) -> Side {
        let u = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let v = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        let sides = [(u, Side::Left), (1.0 - u, Side::Right), (v, Side::Top), (1.0 - v, Side::Bottom)];
        sides.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).map_or(Side::Right, |(_, side)| side)
    }

    /// The half of `rect` a pane dropped on this side would take, for the preview.
    pub fn half(self, rect: Rect) -> Rect {
        let c = rect.center();
        match self {
            Side::Left => Rect::from_min_max(rect.min, pos2(c.x, rect.bottom())),
            Side::Right => Rect::from_min_max(pos2(c.x, rect.top()), rect.max),
            Side::Top => Rect::from_min_max(rect.min, pos2(rect.right(), c.y)),
            Side::Bottom => Rect::from_min_max(pos2(rect.left(), c.y), rect.max),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    /// Two or more children in a row or column; each `f32` is a weight, its
    /// share of the space.
    Split { dir: Dir, children: Vec<(Node, f32)> },
}

/// The gap between two neighbours in a row or column, for dragging.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub rect: Rect,
    pub dir: Dir,
    /// Child indexes from the root to the split.
    pub path: Vec<usize>,
    /// The divider sits after child `index` and before `index + 1`.
    pub index: usize,
    /// The two neighbours' area together, to turn a drag into weights.
    pub area: Rect,
}

/// The least share of a pair either neighbour can be dragged down to.
pub const MIN_RATIO: f32 = 0.1;

impl Node {
    pub fn panes(&self) -> Vec<PaneId> {
        match self {
            Node::Leaf(id) => vec![*id],
            Node::Split { children, .. } => children.iter().flat_map(|(c, _)| c.panes()).collect(),
        }
    }

    /// Splits `pane` in two, putting `new` to its right or below it.
    /// Returns false if `pane` isn't in the tree.
    pub fn split(&mut self, pane: PaneId, dir: Dir, new: PaneId) -> bool {
        self.put(pane, if dir == Dir::Right { Side::Right } else { Side::Bottom }, new, false)
    }

    /// Puts `new` beside `target` on `side`. In a row or column already
    /// running that way it joins it and the children share the space evenly
    /// (`even`), or it takes half of `target`'s share; otherwise `target`
    /// becomes a split of the two.
    fn put(&mut self, target: PaneId, side: Side, new: PaneId, even: bool) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                let pair = if side.after() { [*id, new] } else { [new, *id] };
                *self = Node::Split { dir: side.dir(), children: pair.map(|id| (Node::Leaf(id), 1.0)).to_vec() };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { dir, children } => {
                let here = children.iter().position(|(c, _)| *c == Node::Leaf(target));
                if let Some(i) = here
                    && *dir == side.dir()
                {
                    let at = if side.after() { i + 1 } else { i };
                    if even {
                        children.insert(at, (Node::Leaf(new), 1.0));
                        let total: f32 = children.iter().map(|(_, w)| w).sum();
                        let each = total / children.len() as f32;
                        children.iter_mut().for_each(|(_, w)| *w = each);
                    } else {
                        let half = children[i].1 / 2.0;
                        children[i].1 = half;
                        children.insert(at, (Node::Leaf(new), half));
                    }
                    return true;
                }
                children.iter_mut().any(|(c, _)| c.put(target, side, new, even))
            }
        }
    }

    /// Removes `pane`; its row or column's others share its space. A split
    /// left with one child becomes that child. The last pane can't be
    /// removed (returns false); the window closes instead.
    pub fn remove(&mut self, pane: PaneId) -> bool {
        let Node::Split { children, .. } = self else { return false };
        let found = if let Some(i) = children.iter().position(|(c, _)| *c == Node::Leaf(pane)) {
            children.remove(i);
            true
        } else {
            children.iter_mut().any(|(c, _)| c.remove(pane))
        };
        if found {
            self.tidy();
        }
        found
    }

    /// Adds a new pane beside `target` on `side`, sharing its row or column evenly.
    pub fn insert(&mut self, target: PaneId, side: Side, new: PaneId) -> bool {
        self.put(target, side, new, true)
    }

    /// Moves `pane` beside `target`, on `side`, as a tiling window manager
    /// does: it leaves its place and joins `target`'s row or column, which
    /// then shares its space evenly.
    pub fn move_pane(&mut self, pane: PaneId, target: PaneId, side: Side) -> bool {
        if pane == target || !self.panes().contains(&target) || !self.remove(pane) {
            return false;
        }
        self.put(target, side, pane, true)
    }

    /// Swaps two panes' places.
    pub fn swap(&mut self, a: PaneId, b: PaneId) {
        match self {
            Node::Leaf(id) if *id == a => *id = b,
            Node::Leaf(id) if *id == b => *id = a,
            Node::Leaf(_) => {}
            Node::Split { children, .. } => children.iter_mut().for_each(|(c, _)| c.swap(a, b)),
        }
    }

    /// A split of one becomes its child; a split inside a split running the
    /// same way is merged into it, its children sharing its weight.
    fn tidy(&mut self) {
        let Node::Split { dir, children } = self else { return };
        let dir = *dir;
        let mut merged = Vec::new();
        for (mut child, weight) in std::mem::take(children) {
            child.tidy();
            match child {
                Node::Split { dir: d, children: inner } if d == dir => {
                    let total: f32 = inner.iter().map(|(_, w)| w).sum();
                    merged.extend(inner.into_iter().map(|(c, w)| (c, weight * w / total)));
                }
                other => merged.push((other, weight)),
            }
        }
        if merged.len() == 1 {
            *self = merged.pop().map(|(c, _)| c).unwrap_or(Node::Leaf(0));
        } else {
            *children = merged;
        }
    }

    /// Moves the divider after child `index` of the split at `path`, giving
    /// the first of the pair `ratio` of their space together.
    pub fn set_ratio(&mut self, path: &[usize], index: usize, ratio: f32) {
        let Node::Split { children, .. } = self else { return };
        match path.split_first() {
            Some((&step, rest)) => {
                if let Some((child, _)) = children.get_mut(step) {
                    child.set_ratio(rest, index, ratio);
                }
            }
            None if index + 1 < children.len() => {
                let total = children[index].1 + children[index + 1].1;
                let ratio = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
                children[index].1 = total * ratio;
                children[index + 1].1 = total * (1.0 - ratio);
            }
            None => {}
        }
    }

    /// Each pane's rectangle within `area`, with `gap` between neighbours,
    /// and the dividers in those gaps. A collapsed pane is only `title`
    /// tall (its title bar); in a column, the open panes share what the
    /// folded ones give up. In a row, a folded pane keeps its width.
    pub fn layout(
        &self,
        area: Rect,
        gap: f32,
        collapsed: &dyn Fn(PaneId) -> bool,
        title: f32,
    ) -> (Vec<(PaneId, Rect)>, Vec<Divider>) {
        let mut out = Layout { gap, collapsed, title, panes: Vec::new(), dividers: Vec::new() };
        self.layout_into(area, &mut Vec::new(), &mut out);
        (out.panes, out.dividers)
    }

    /// Whether every pane under this node is collapsed.
    fn folded(&self, collapsed: &dyn Fn(PaneId) -> bool) -> bool {
        match self {
            Node::Leaf(id) => collapsed(*id),
            Node::Split { children, .. } => children.iter().all(|(c, _)| c.folded(collapsed)),
        }
    }

    /// The height a fully collapsed node needs: its title bars, stacked or side by side.
    fn folded_height(&self, title: f32, gap: f32) -> f32 {
        match self {
            Node::Leaf(_) => title,
            Node::Split { dir: Dir::Down, children } => {
                children.iter().map(|(c, _)| c.folded_height(title, gap)).sum::<f32>()
                    + gap * (children.len() - 1) as f32
            }
            Node::Split { dir: Dir::Right, children } => {
                children.iter().map(|(c, _)| c.folded_height(title, gap)).fold(0.0, f32::max)
            }
        }
    }

    fn layout_into(&self, area: Rect, path: &mut Vec<usize>, out: &mut Layout<'_>) {
        let gap = out.gap;
        let (dir, children) = match self {
            Node::Leaf(id) if (out.collapsed)(*id) => {
                let height = out.title.min(area.height());
                out.panes.push((*id, Rect::from_min_size(area.min, egui::vec2(area.width(), height))));
                return;
            }
            Node::Leaf(id) => {
                out.panes.push((*id, area));
                return;
            }
            Node::Split { dir, children } => (*dir, children),
        };

        // Each child's length along the split: folded children in a column get their
        // title bars' height, the rest share what's left by weight.
        let length = match dir {
            Dir::Right => area.width(),
            Dir::Down => area.height(),
        } - gap * (children.len() - 1) as f32;
        let folded: Vec<bool> =
            children.iter().map(|(c, _)| dir == Dir::Down && c.folded(out.collapsed)).collect();
        let fixed: f32 =
            children.iter().zip(&folded).filter(|(_, f)| **f).map(|((c, _), _)| c.folded_height(out.title, gap)).sum();
        let weights: f32 = children.iter().zip(&folded).filter(|(_, f)| !**f).map(|((_, w), _)| w).sum();
        let flexible = (length - fixed).max(0.0);
        let lengths: Vec<f32> = children
            .iter()
            .zip(&folded)
            .map(|((c, w), f)| if *f { c.folded_height(out.title, gap) } else { flexible * w / weights })
            .collect();

        let mut start = match dir {
            Dir::Right => area.left(),
            Dir::Down => area.top(),
        };
        let mut rects = Vec::new();
        for len in &lengths {
            rects.push(match dir {
                Dir::Right => Rect::from_min_max(pos2(start, area.top()), pos2(start + len, area.bottom())),
                Dir::Down => Rect::from_min_max(pos2(area.left(), start), pos2(area.right(), start + len)),
            });
            start += len + gap;
        }
        for i in 0..children.len() {
            // Only two open neighbours have anything to resize between them.
            if i + 1 < children.len() && !folded[i] && !folded[i + 1] {
                let (a, b) = (rects[i], rects[i + 1]);
                let rect = match dir {
                    Dir::Right => Rect::from_min_max(pos2(a.right(), area.top()), pos2(b.left(), area.bottom())),
                    Dir::Down => Rect::from_min_max(pos2(area.left(), a.bottom()), pos2(area.right(), b.top())),
                };
                out.dividers.push(Divider { rect, dir, path: path.clone(), index: i, area: a.union(b) });
            }
            path.push(i);
            children[i].0.layout_into(rects[i], path, out);
            path.pop();
        }
    }
}

struct Layout<'a> {
    gap: f32,
    collapsed: &'a dyn Fn(PaneId) -> bool,
    title: f32,
    panes: Vec<(PaneId, Rect)>,
    dividers: Vec<Divider>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(104.0, 52.0))
    }

    fn open(tree: &Node) -> Vec<(PaneId, Rect)> {
        tree.layout(area(), 4.0, &|_| false, 10.0).0
    }

    /// Two on the left (1 over 2), two on the right (3 over 4).
    fn two_by_two() -> Node {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 3);
        tree.split(1, Dir::Down, 2);
        tree.split(3, Dir::Down, 4);
        tree
    }

    #[test]
    fn split_right_then_down() {
        let mut tree = Node::Leaf(1);
        assert!(tree.split(1, Dir::Right, 2));
        assert!(tree.split(2, Dir::Down, 3));
        assert_eq!(tree.panes(), [1, 2, 3]);
        let (panes, dividers) = tree.layout(area(), 4.0, &|_| false, 10.0);
        assert_eq!(panes[0], (1, Rect::from_min_max(pos2(0.0, 0.0), pos2(50.0, 52.0))));
        assert_eq!(panes[1], (2, Rect::from_min_max(pos2(54.0, 0.0), pos2(104.0, 24.0))));
        assert_eq!(panes[2], (3, Rect::from_min_max(pos2(54.0, 28.0), pos2(104.0, 52.0))));
        assert_eq!(dividers.len(), 2);
        assert_eq!((dividers[1].path.as_slice(), dividers[1].index), (&[1usize][..], 0));
    }

    #[test]
    fn splitting_in_a_row_halves_the_pane() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        tree.split(2, Dir::Right, 3);
        let Node::Split { children, .. } = &tree else { panic!() };
        assert_eq!(children.iter().map(|(_, w)| *w).collect::<Vec<_>>(), [1.0, 0.5, 0.5]);
    }

    #[test]
    fn moving_one_from_the_left_gives_one_left_three_right() {
        let mut tree = two_by_two();
        // Drop pane 2 below pane 4.
        assert!(tree.move_pane(2, 4, Side::Bottom));
        assert_eq!(tree.panes(), [1, 3, 4, 2]);
        let panes = open(&tree);
        assert_eq!(panes[0].1.height(), 52.0, "pane 1 has the whole left");
        let heights: Vec<f32> = panes[1..].iter().map(|(_, r)| r.height()).collect();
        assert!(heights.iter().all(|h| (h - heights[0]).abs() < 0.01), "even thirds: {heights:?}");
        assert!(panes[1..].iter().all(|(_, r)| r.left() == 54.0));
    }

    #[test]
    fn dropping_on_a_side_across_the_split_makes_a_new_split() {
        let mut tree = two_by_two();
        // Pane 2 to the left of pane 3: the right column's top becomes a row.
        assert!(tree.move_pane(2, 3, Side::Left));
        assert_eq!(tree.panes(), [1, 2, 3, 4]);
        let panes = open(&tree);
        assert_eq!(panes[1].1.top(), panes[2].1.top(), "2 and 3 side by side");
        assert!(panes[1].1.left() >= 54.0);
    }

    #[test]
    fn moving_the_last_pane_out_of_a_split_dissolves_it() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        assert!(tree.move_pane(1, 2, Side::Bottom));
        assert_eq!(tree, Node::Split { dir: Dir::Down, children: vec![(Node::Leaf(2), 1.0), (Node::Leaf(1), 1.0)] });
        assert!(!tree.move_pane(1, 1, Side::Top), "onto itself does nothing");
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_its_neighbours() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        tree.split(2, Dir::Down, 3);
        assert!(tree.remove(2));
        assert_eq!(tree.panes(), [1, 3]);
        assert!(tree.remove(1));
        assert_eq!(tree, Node::Leaf(3));
        assert!(!tree.remove(3), "the last pane stays");
    }

    #[test]
    fn removing_merges_splits_running_the_same_way() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Down, 2);
        tree.split(2, Dir::Right, 3);
        tree.split(3, Dir::Down, 4); // column [1, row [2, column [3, 4]]]
        tree.remove(2); // the row dissolves: its column joins the outer column
        let Node::Split { dir, children } = &tree else { panic!() };
        assert_eq!((*dir, children.len()), (Dir::Down, 3));
        assert_eq!(tree.panes(), [1, 3, 4]);
    }

    #[test]
    fn swapping_keeps_the_shape() {
        let mut tree = two_by_two();
        let before = open(&tree);
        tree.swap(1, 4);
        let after = open(&tree);
        assert_eq!(after[0], (4, before[0].1));
        assert_eq!(after[3], (1, before[3].1));
    }

    #[test]
    fn a_collapsed_pane_gives_its_height_to_its_neighbours() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Down, 2);
        tree.split(2, Dir::Down, 3);
        let tall = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 200.0));
        // 1 and 3 folded: two 10-high title bars, 2 gets the rest.
        let (panes, dividers) = tree.layout(tall, 4.0, &|id| id != 2, 10.0);
        assert_eq!(panes[0].1, Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 10.0)));
        assert_eq!(panes[1].1, Rect::from_min_max(pos2(0.0, 14.0), pos2(100.0, 186.0)));
        assert_eq!(panes[2].1, Rect::from_min_max(pos2(0.0, 190.0), pos2(100.0, 200.0)));
        assert!(dividers.is_empty(), "nothing to resize next to a folded pane");
    }

    #[test]
    fn in_a_row_a_collapsed_pane_keeps_its_width() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        let (panes, dividers) = tree.layout(area(), 4.0, &|id| id == 1, 10.0);
        assert_eq!(panes[0].1, Rect::from_min_max(pos2(0.0, 0.0), pos2(50.0, 10.0)));
        assert_eq!(panes[1].1, Rect::from_min_max(pos2(54.0, 0.0), pos2(104.0, 52.0)));
        assert_eq!(dividers.len(), 1);
    }

    #[test]
    fn dividers_move_only_their_two_neighbours() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        tree.split(2, Dir::Right, 3);
        tree.set_ratio(&[], 1, 0.0);
        let Node::Split { children, .. } = &tree else { panic!() };
        let w: Vec<f32> = children.iter().map(|(_, w)| *w).collect();
        assert_eq!(w[0], 1.0);
        assert!((w[1] - 0.1).abs() < 1e-6 && (w[2] - 0.9).abs() < 1e-6, "{w:?}");
    }

    #[test]
    fn the_nearest_side_picks_the_drop_zone() {
        let r = Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 100.0));
        assert_eq!(Side::nearest(r, pos2(10.0, 50.0)), Side::Left);
        assert_eq!(Side::nearest(r, pos2(190.0, 50.0)), Side::Right);
        assert_eq!(Side::nearest(r, pos2(100.0, 5.0)), Side::Top);
        assert_eq!(Side::nearest(r, pos2(100.0, 95.0)), Side::Bottom);
    }
}
