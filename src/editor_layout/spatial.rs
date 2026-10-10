//! Exact weighted logical adjacency, independent of terminal presentation seals.
use super::*;
use std::cmp::Ordering;

#[derive(Clone, Copy)]
struct Interval {
    start: u128,
    end: u128,
    denominator: u128,
}
impl Interval {
    const UNIT: Self = Self {
        start: 0,
        end: 1,
        denominator: 1,
    };
    fn split(self, weights: (u32, u32)) -> Result<(Self, Self)> {
        ensure!(
            self.denominator != 0 && self.start < self.end && self.end <= self.denominator,
            "Invalid spatial interval"
        );
        ensure!(
            (1..=MAX_WEIGHT).contains(&weights.0) && (1..=MAX_WEIGHT).contains(&weights.1),
            "Invalid spatial layout weights"
        );
        let sum = u128::from(weights.0) + u128::from(weights.1);
        let multiply = |a: u128, b: u128| {
            a.checked_mul(b)
                .context("Spatial layout arithmetic exhausted")
        };
        let denominator = multiply(self.denominator, sum)?;
        let start = multiply(self.start, sum)?;
        let end = multiply(self.end, sum)?;
        let span = self
            .end
            .checked_sub(self.start)
            .context("Invalid spatial interval")?;
        let cut = start
            .checked_add(multiply(span, weights.0.into())?)
            .context("Spatial layout arithmetic exhausted")?;
        Ok((
            Self {
                start,
                end: cut,
                denominator,
            }
            .reduced(),
            Self {
                start: cut,
                end,
                denominator,
            }
            .reduced(),
        ))
    }
    fn reduced(self) -> Self {
        fn gcd(mut a: u128, mut b: u128) -> u128 {
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a
        }
        let factor = gcd(gcd(self.start, self.end), self.denominator);
        Self {
            start: self.start / factor,
            end: self.end / factor,
            denominator: self.denominator / factor,
        }
    }
    fn overlaps(self, other: Self) -> Result<bool> {
        Ok(
            compare(self.start, self.denominator, other.end, other.denominator)? == Ordering::Less
                && compare(other.start, other.denominator, self.end, self.denominator)?
                    == Ordering::Less,
        )
    }
}
fn compare(a: u128, ad: u128, b: u128, bd: u128) -> Result<Ordering> {
    let left = a
        .checked_mul(bd)
        .context("Spatial layout comparison exhausted")?;
    let right = b
        .checked_mul(ad)
        .context("Spatial layout comparison exhausted")?;
    Ok(left.cmp(&right))
}
#[derive(Clone, Copy)]
struct LogicalBox {
    group: GroupId,
    x: Interval,
    y: Interval,
}
fn collect(
    node: &Node,
    x: Interval,
    y: Interval,
    depth: usize,
    nodes: &mut usize,
    boxes: &mut [Option<LogicalBox>; MAX_LEAVES],
    leaves: &mut usize,
) -> Result<()> {
    ensure!(
        depth <= MAX_DEPTH && *nodes < MAX_NODES,
        "Spatial layout exceeds node/depth bounds"
    );
    *nodes += 1;
    match node {
        Node::Leaf(group) => {
            ensure!(*leaves < MAX_LEAVES, "Spatial layout exceeds four groups");
            ensure!(
                !boxes.iter().flatten().any(|old| old.group == *group),
                "Duplicate spatial layout group"
            );
            boxes[*leaves] = Some(LogicalBox {
                group: *group,
                x,
                y,
            });
            *leaves += 1;
        }
        Node::Split {
            axis,
            weights,
            first,
            second,
            ..
        } => {
            let (a, b) = match axis {
                Axis::Columns => x.split(*weights)?,
                Axis::Rows => y.split(*weights)?,
            };
            match axis {
                Axis::Columns => {
                    collect(first, a, y, depth + 1, nodes, boxes, leaves)?;
                    collect(second, b, y, depth + 1, nodes, boxes, leaves)?;
                }
                Axis::Rows => {
                    collect(first, x, a, depth + 1, nodes, boxes, leaves)?;
                    collect(second, x, b, depth + 1, nodes, boxes, leaves)?;
                }
            }
        }
    }
    Ok(())
}
impl Layout {
    /// Existing boundary neighbors in tree appearance order. Visual dividers have
    /// zero logical width; exact positive interval overlap excludes corner-only
    /// contact. Wrapping preserves the perpendicular interval at the outer edge.
    /// This query changes no layout identity, generation, IDs or presentation.
    pub fn neighbor_candidates(
        &self,
        source: GroupId,
        direction: Direction,
        wrap: bool,
    ) -> Result<[Option<GroupId>; MAX_LEAVES]> {
        let root = self
            .root
            .as_ref()
            .context("Spatial focus requires an existing group")?;
        let mut boxes = [None; MAX_LEAVES];
        let mut nodes = 0;
        let mut leaves = 0;
        collect(
            root,
            Interval::UNIT,
            Interval::UNIT,
            0,
            &mut nodes,
            &mut boxes,
            &mut leaves,
        )?;
        let source = boxes
            .iter()
            .flatten()
            .find(|item| item.group == source)
            .context("Spatial source group was closed")?;
        let horizontal = matches!(direction, Direction::Left | Direction::Right);
        let before = matches!(direction, Direction::Left | Direction::Up);
        let axis = if horizontal { source.x } else { source.y };
        let perpendicular = if horizontal { source.y } else { source.x };
        let mut boundary = if before { axis.start } else { axis.end };
        let mut denominator = axis.denominator;
        if wrap && ((before && boundary == 0) || (!before && boundary == denominator)) {
            boundary = u128::from(before);
            denominator = 1;
        }
        let mut candidates = [None; MAX_LEAVES];
        let mut count = 0;
        for item in boxes.iter().flatten() {
            let next = if horizontal { item.x } else { item.y };
            let overlap = if horizontal { item.y } else { item.x };
            let opposite = if before { next.end } else { next.start };
            if compare(boundary, denominator, opposite, next.denominator)? == Ordering::Equal
                && perpendicular.overlaps(overlap)?
            {
                candidates[count] = Some(item.group);
                count += 1;
            }
        }
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids() -> [GroupId; 4] {
        let mut groups = crate::editor_groups::Groups::default();
        groups.open(1).unwrap();
        for _ in 1..4 {
            groups.split_active().unwrap();
        }
        std::array::from_fn(|i| groups.groups()[i].id())
    }
    fn split(axis: Axis, weights: (u32, u32), first: SavedNode, second: SavedNode) -> SavedNode {
        SavedNode::Split {
            axis,
            first_weight: weights.0,
            second_weight: weights.1,
            first: Box::new(first),
            second: Box::new(second),
        }
    }
    fn leaf(group: usize) -> SavedNode {
        SavedNode::Leaf { group }
    }
    fn imported(groups: &[GroupId], node: SavedNode) -> Layout {
        let mut layout = Layout::default();
        layout
            .commit(layout.prepare_import(groups, Some(&node)).unwrap())
            .unwrap();
        layout
    }
    fn candidates(
        layout: &Layout,
        source: GroupId,
        direction: Direction,
        wrap: bool,
    ) -> Vec<GroupId> {
        layout
            .neighbor_candidates(source, direction, wrap)
            .unwrap()
            .into_iter()
            .flatten()
            .collect()
    }
    #[test]
    fn whole_edge_has_both_stacked_neighbors_and_outer_edges_wrap() {
        let g = ids();
        let layout = imported(
            &g[..3],
            split(
                Axis::Columns,
                (1, 1),
                leaf(0),
                split(Axis::Rows, (1, 1), leaf(1), leaf(2)),
            ),
        );
        assert_eq!(candidates(&layout, g[0], Direction::Right, true), g[1..3]);
        assert_eq!(candidates(&layout, g[0], Direction::Left, true), g[1..3]);
        assert_eq!(candidates(&layout, g[1], Direction::Left, true), vec![g[0]]);
        assert_eq!(
            candidates(&layout, g[1], Direction::Right, true),
            vec![g[0]]
        );
        assert_eq!(candidates(&layout, g[1], Direction::Up, true), vec![g[2]]);
        assert_eq!(candidates(&layout, g[2], Direction::Down, true), vec![g[1]]);
        assert!(candidates(&layout, g[0], Direction::Left, false).is_empty());
    }
    #[test]
    fn aligned_weighted_corners_are_excluded_exactly_without_cell_rounding() {
        let g = ids();
        let layout = imported(
            &g,
            split(
                Axis::Columns,
                (1, 1),
                split(Axis::Rows, (1, 2), leaf(0), leaf(1)),
                split(Axis::Rows, (2, 4), leaf(2), leaf(3)),
            ),
        );
        assert_eq!(
            candidates(&layout, g[0], Direction::Right, false),
            vec![g[2]]
        );
        assert_eq!(
            candidates(&layout, g[1], Direction::Right, false),
            vec![g[3]]
        );
        assert_eq!(candidates(&layout, g[2], Direction::Left, true), vec![g[0]]);
        assert_eq!(candidates(&layout, g[3], Direction::Down, true), vec![g[2]]);
    }
    #[test]
    fn unequal_nested_edges_preserve_all_positive_overlap_candidates() {
        let g = ids();
        let layout = imported(
            &g,
            split(
                Axis::Columns,
                (999999, 1000000),
                split(Axis::Rows, (1, 2), leaf(0), leaf(1)),
                split(Axis::Rows, (1, 1), leaf(2), leaf(3)),
            ),
        );
        assert_eq!(
            candidates(&layout, g[0], Direction::Right, false),
            vec![g[2]]
        );
        assert_eq!(
            candidates(&layout, g[1], Direction::Right, false),
            vec![g[2], g[3]]
        );
        assert_eq!(
            candidates(&layout, g[2], Direction::Left, false),
            vec![g[0], g[1]]
        );
        assert_eq!(
            candidates(&layout, g[3], Direction::Left, false),
            vec![g[1]]
        );
    }
    #[test]
    fn deepest_extreme_weights_and_tiny_projection_leave_query_and_revision_unchanged() {
        let g = ids();
        let layout = imported(
            &g,
            split(
                Axis::Columns,
                (999999, 1000000),
                leaf(0),
                split(
                    Axis::Columns,
                    (999999, 1000000),
                    leaf(1),
                    split(Axis::Columns, (999999, 1000000), leaf(2), leaf(3)),
                ),
            ),
        );
        let before = layout
            .project(Rect::new(0, 0, 120, 36), Some(g[0]))
            .unwrap();
        for i in 0..4 {
            assert_eq!(
                candidates(&layout, g[i], Direction::Right, true),
                vec![g[(i + 1) % 4]]
            );
            assert_eq!(
                candidates(&layout, g[i], Direction::Left, true),
                vec![g[(i + 3) % 4]]
            );
        }
        let tiny = layout.project(Rect::new(0, 0, 1, 1), Some(g[3])).unwrap();
        assert!(tiny.active_only());
        assert!(layout.geometry_current(&before));
        assert!(before.same_revision(&tiny));
        assert_eq!(
            candidates(&layout, g[3], Direction::Right, true),
            vec![g[0]]
        );
    }
    #[test]
    fn welcome_unknown_source_and_single_group_are_explicit() {
        let g = ids();
        assert!(
            Layout::default()
                .neighbor_candidates(g[0], Direction::Left, true)
                .is_err()
        );
        let layout = imported(&g[..1], leaf(0));
        assert!(
            layout
                .neighbor_candidates(g[1], Direction::Left, true)
                .is_err()
        );
        for d in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            assert_eq!(candidates(&layout, g[0], d, true), vec![g[0]]);
            assert!(candidates(&layout, g[0], d, false).is_empty());
        }
    }
    #[test]
    fn orthogonal_reflected_faces_and_query_at_counter_exhaustion_are_read_only() {
        let g = ids();
        let mut layout = imported(
            &g[..3],
            split(
                Axis::Rows,
                (2, 1),
                split(Axis::Columns, (1, 1), leaf(0), leaf(1)),
                leaf(2),
            ),
        );
        layout.generation = u64::MAX;
        layout.next_split = u64::MAX;
        let identity = layout.identity.clone();
        assert_eq!(
            candidates(&layout, g[2], Direction::Up, true),
            vec![g[0], g[1]]
        );
        assert_eq!(
            candidates(&layout, g[2], Direction::Down, true),
            vec![g[0], g[1]]
        );
        assert_eq!(
            candidates(&layout, g[0], Direction::Right, true),
            vec![g[1]]
        );
        assert_eq!(
            candidates(&layout, g[1], Direction::Right, true),
            vec![g[0]]
        );
        assert!(Arc::ptr_eq(&layout.identity, &identity));
        assert_eq!(layout.generation, u64::MAX);
        assert_eq!(layout.next_split, u64::MAX);
    }
    #[test]
    fn duplicate_private_tree_and_excess_depth_refuse_boundedly() {
        let g = ids();
        let mut layout = imported(&g[..2], split(Axis::Columns, (1, 1), leaf(0), leaf(1)));
        if let Some(Node::Split { second, .. }) = &mut layout.root {
            **second = Node::Leaf(g[0]);
        }
        assert!(
            layout
                .neighbor_candidates(g[0], Direction::Right, true)
                .is_err()
        );
        let mut node = Node::Leaf(g[0]);
        for _ in 0..4 {
            node = Node::Split {
                id: SplitId(1),
                axis: Axis::Columns,
                weights: (1, 1),
                first: Box::new(node),
                second: Box::new(Node::Leaf(g[1])),
            };
        }
        layout.root = Some(node);
        assert!(
            layout
                .neighbor_candidates(g[0], Direction::Right, true)
                .is_err()
        );
    }
    #[test]
    fn invalid_private_weights_and_arithmetic_refuse_without_mutation() {
        let g = ids();
        let mut layout = imported(&g[..2], split(Axis::Columns, (1, 1), leaf(0), leaf(1)));
        if let Some(Node::Split { weights, .. }) = &mut layout.root {
            *weights = (0, 1);
        }
        let identity = layout.identity.clone();
        let generation = layout.generation;
        let next = layout.next_split;
        assert!(
            layout
                .neighbor_candidates(g[0], Direction::Right, true)
                .is_err()
        );
        assert!(Arc::ptr_eq(&identity, &layout.identity));
        assert_eq!(layout.generation, generation);
        assert_eq!(layout.next_split, next);
        assert!(
            Interval {
                start: 0,
                end: 1,
                denominator: u128::MAX
            }
            .split((1, 1))
            .is_err()
        );
        assert!(compare(u128::MAX, 1, 1, 2).is_err());
    }
}
