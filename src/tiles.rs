//! The tiled layout (ADR-0005): a binary tree of splits whose leaves are
//! panes. Splitting a pane replaces its leaf with a split holding it and a
//! new pane; closing one hands its space to its sibling; swapping exchanges
//! two leaves, so the shape stays and the panes trade places.

use egui::{Rect, pos2};

pub type PaneId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// Side by side: the new pane goes to the right.
    Right,
    /// Stacked: the new pane goes below.
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split { dir: Dir, ratio: f32, first: Box<Node>, second: Box<Node> },
}

/// A split's divider, for dragging: where it is and which split it moves.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub rect: Rect,
    pub dir: Dir,
    /// Steps from the root: `false` for first, `true` for second.
    pub path: Vec<bool>,
    /// The split's whole area, to turn a drag into a ratio.
    pub area: Rect,
}

pub const MIN_RATIO: f32 = 0.1;

impl Node {
    pub fn panes(&self) -> Vec<PaneId> {
        match self {
            Node::Leaf(id) => vec![*id],
            Node::Split { first, second, .. } => {
                let mut ids = first.panes();
                ids.extend(second.panes());
                ids
            }
        }
    }

    /// Splits `pane`, putting `new` to its right or below it. Returns false
    /// if `pane` isn't in the tree.
    pub fn split(&mut self, pane: PaneId, dir: Dir, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == pane => {
                *self = Node::Split {
                    dir,
                    ratio: 0.5,
                    first: Box::new(Node::Leaf(pane)),
                    second: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => first.split(pane, dir, new) || second.split(pane, dir, new),
        }
    }

    /// Removes `pane`, giving its space to its sibling. The last pane can't
    /// be removed (returns false); the window closes instead.
    pub fn remove(&mut self, pane: PaneId) -> bool {
        let Node::Split { first, second, .. } = self else { return false };
        let keep = if **first == Node::Leaf(pane) {
            second
        } else if **second == Node::Leaf(pane) {
            first
        } else {
            return first.remove(pane) || second.remove(pane);
        };
        *self = std::mem::replace(keep.as_mut(), Node::Leaf(0));
        true
    }

    /// Swaps two panes' places.
    pub fn swap(&mut self, a: PaneId, b: PaneId) {
        match self {
            Node::Leaf(id) if *id == a => *id = b,
            Node::Leaf(id) if *id == b => *id = a,
            Node::Leaf(_) => {}
            Node::Split { first, second, .. } => {
                first.swap(a, b);
                second.swap(a, b);
            }
        }
    }

    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        match (self, path.split_first()) {
            (Node::Split { ratio: r, .. }, None) => *r = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO),
            (Node::Split { first, second, .. }, Some((&step, rest))) => {
                if step { second } else { first }.set_ratio(rest, ratio)
            }
            (Node::Leaf(_), _) => {}
        }
    }

    /// Each pane's rectangle within `area`, with `gap` between neighbours,
    /// and the dividers in those gaps. A collapsed pane is only `title`
    /// tall (its title bar); in a stack, its sibling takes the height it
    /// gives up. Side by side, it keeps its width.
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
            Node::Split { first, second, .. } => first.folded(collapsed) && second.folded(collapsed),
        }
    }

    /// The height a fully collapsed node needs: its title bars, stacked or side by side.
    fn folded_height(&self, title: f32, gap: f32) -> f32 {
        match self {
            Node::Leaf(_) => title,
            Node::Split { dir: Dir::Down, first, second, .. } => {
                first.folded_height(title, gap) + gap + second.folded_height(title, gap)
            }
            Node::Split { dir: Dir::Right, first, second, .. } => {
                first.folded_height(title, gap).max(second.folded_height(title, gap))
            }
        }
    }

    fn layout_into(&self, area: Rect, path: &mut Vec<bool>, out: &mut Layout<'_>) {
        let gap = out.gap;
        match self {
            Node::Leaf(id) if (out.collapsed)(*id) => {
                let height = out.title.min(area.height());
                out.panes.push((*id, Rect::from_min_size(area.min, egui::vec2(area.width(), height))));
            }
            Node::Leaf(id) => out.panes.push((*id, area)),
            Node::Split { dir, ratio, first, second } => {
                let (a, divider, b) = match dir {
                    Dir::Right => {
                        let x = area.left() + (area.width() - gap) * ratio;
                        (
                            Rect::from_min_max(area.min, pos2(x, area.bottom())),
                            Rect::from_min_max(pos2(x, area.top()), pos2(x + gap, area.bottom())),
                            Rect::from_min_max(pos2(x + gap, area.top()), area.max),
                        )
                    }
                    Dir::Down => {
                        // A folded side is as tall as its title bars; the other gets the rest.
                        let first_folded = first.folded(out.collapsed);
                        let second_folded = second.folded(out.collapsed);
                        let y = if first_folded {
                            area.top() + first.folded_height(out.title, gap)
                        } else if second_folded {
                            area.bottom() - gap - second.folded_height(out.title, gap)
                        } else {
                            area.top() + (area.height() - gap) * ratio
                        };
                        let y = y.clamp(area.top(), (area.bottom() - gap).max(area.top()));
                        (
                            Rect::from_min_max(area.min, pos2(area.right(), y)),
                            Rect::from_min_max(pos2(area.left(), y), pos2(area.right(), y + gap)),
                            Rect::from_min_max(pos2(area.left(), y + gap), area.max),
                        )
                    }
                };
                // A stack with a folded side has nothing to resize.
                let fixed = *dir == Dir::Down && (first.folded(out.collapsed) || second.folded(out.collapsed));
                if !fixed {
                    out.dividers.push(Divider { rect: divider, dir: *dir, path: path.clone(), area });
                }
                path.push(false);
                first.layout_into(a, path, out);
                path.pop();
                path.push(true);
                second.layout_into(b, path, out);
                path.pop();
            }
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
        assert_eq!(dividers[1].path, [true]);
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_its_sibling() {
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
    fn swapping_keeps_the_shape() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        tree.split(2, Dir::Down, 3);
        let before = tree.layout(area(), 4.0, &|_| false, 10.0).0;
        tree.swap(1, 3);
        let after = tree.layout(area(), 4.0, &|_| false, 10.0).0;
        assert_eq!(after[0], (3, before[0].1));
        assert_eq!(after[2], (1, before[2].1));
    }

    #[test]
    fn a_collapsed_pane_gives_its_height_to_its_sibling() {
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
    fn side_by_side_a_collapsed_pane_keeps_its_width() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        let (panes, dividers) = tree.layout(area(), 4.0, &|id| id == 1, 10.0);
        assert_eq!(panes[0].1, Rect::from_min_max(pos2(0.0, 0.0), pos2(50.0, 10.0)));
        assert_eq!(panes[1].1, Rect::from_min_max(pos2(54.0, 0.0), pos2(104.0, 52.0)));
        assert_eq!(dividers.len(), 1);
    }

    #[test]
    fn ratios_are_clamped() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Dir::Right, 2);
        tree.set_ratio(&[], 0.0);
        let Node::Split { ratio, .. } = tree else { unreachable!() };
        assert_eq!(ratio, MIN_RATIO);
    }
}
