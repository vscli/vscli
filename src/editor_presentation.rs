//! Bounded, sealed editor hit geometry. No document, filesystem or input side effects.
use crate::{
    editor_groups::{GroupId, Groups, Membership, UiProof},
    editor_layout::{Divider, Geometry, Layout, MAX_LEAVES, MAX_NODES},
};
use anyhow::{Context, Result, ensure};
use ratatui::layout::{Position, Rect};
use std::sync::Arc;

/// A frame identity, distinct from layout ratios and group membership generations.
#[derive(Clone, Debug, Eq)]
pub struct GeometryProof {
    identity: Arc<()>,
    epoch: u64,
}
impl GeometryProof {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}
impl PartialEq for GeometryProof {
    fn eq(&self, other: &Self) -> bool {
        self.epoch == other.epoch && Arc::ptr_eq(&self.identity, &other.identity)
    }
}

/// Final drawn rectangles. `text` must include the actual line-number gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneRects {
    pub group: GroupId,
    pub outer: Rect,
    pub strip: Rect,
    pub breadcrumbs: Rect,
    pub text: Rect,
}
impl PaneRects {
    /// Reserve headers only. The renderer replaces `text` after drawing its gutter.
    pub fn reserve(group: GroupId, outer: Rect, show_breadcrumbs: bool) -> Result<Self> {
        endpoints(outer)?;
        let empty = Rect::new(outer.x, outer.y, 0, 0);
        if outer.width == 0 || outer.height == 0 {
            return Ok(Self {
                group,
                outer,
                strip: empty,
                breadcrumbs: empty,
                text: empty,
            });
        }
        let strip = Rect::new(outer.x, outer.y, outer.width, 1);
        let content = Rect::new(outer.x, outer.y + 1, outer.width, outer.height - 1);
        let (breadcrumbs, text) = if show_breadcrumbs && content.height >= 2 {
            (
                Rect::new(content.x, content.y, content.width, 1),
                Rect::new(content.x, content.y + 1, content.width, content.height - 1),
            )
        } else {
            (empty, content)
        };
        Ok(Self {
            group,
            outer,
            strip,
            breadcrumbs,
            text,
        })
    }
    fn validate(&self) -> Result<()> {
        endpoints(self.outer)?;
        endpoints(self.strip)?;
        endpoints(self.breadcrumbs)?;
        endpoints(self.text)?;
        if self.outer.width == 0 || self.outer.height == 0 {
            let empty = Rect::new(self.outer.x, self.outer.y, 0, 0);
            ensure!(
                self.strip == empty && self.breadcrumbs == empty && self.text == empty,
                "Hidden pane contains actionable rectangles"
            );
            return Ok(());
        }
        ensure!(
            self.strip == Rect::new(self.outer.x, self.outer.y, self.outer.width, 1),
            "Pane strip differs from its drawn first row"
        );
        let mut top = self.outer.y + 1;
        if self.breadcrumbs.width != 0 || self.breadcrumbs.height != 0 {
            ensure!(
                self.outer.height >= 3
                    && self.breadcrumbs == Rect::new(self.outer.x, top, self.outer.width, 1),
                "Pane Breadcrumbs overlap the strip or text"
            );
            top += 1;
        } else {
            ensure!(
                self.breadcrumbs == Rect::new(self.outer.x, self.outer.y, 0, 0),
                "Absent Breadcrumbs have a foreign origin"
            );
        }
        ensure!(
            self.text.x >= self.outer.x
                && self.text.y == top
                && self.text.height == self.outer.bottom() - top
                && self.text.right() == self.outer.right(),
            "Pane text does not match its actual gutter and header partition"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneHit {
    pub membership: Membership,
    pub group: GroupId,
    pub outer: Rect,
    pub strip: Rect,
    pub breadcrumbs: Rect,
    pub text: Rect,
    pub proof: GeometryProof,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DividerHit {
    pub divider: Divider,
    pub proof: GeometryProof,
}
#[derive(Debug)]
struct Frame {
    geometry: Geometry,
    groups: UiProof,
    rects: [Option<PaneRects>; MAX_LEAVES],
    panes: [Option<PaneHit>; MAX_LEAVES],
    dividers: [Option<DividerHit>; MAX_NODES / 2],
    proof: GeometryProof,
}
#[derive(Debug, Default)]
pub struct Presentation {
    epoch: u64,
    visible: bool,
    frame: Option<Frame>,
}
impl Presentation {
    /// Normal repaint hides all authorization until a complete map is sealed.
    /// Geometry-changing events and hidden/covered surfaces must also invalidate.
    pub fn begin_frame(&mut self) {
        self.visible = false;
    }
    /// Checked exhaustion always fails closed, including any previously held proof.
    pub fn invalidate(&mut self) -> Result<()> {
        self.visible = false;
        self.frame = None;
        self.epoch = self
            .epoch
            .checked_add(1)
            .context("Editor geometry epoch exhausted")?;
        Ok(())
    }
    pub fn proof(&self) -> Option<&GeometryProof> {
        if !self.visible {
            return None;
        }
        self.frame.as_ref().map(|frame| &frame.proof)
    }
    pub fn geometry(&self) -> Option<&Geometry> {
        if !self.visible {
            return None;
        }
        self.frame.as_ref().map(|frame| &frame.geometry)
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn panes(&self) -> &[Option<PaneHit>; MAX_LEAVES] {
        if self.visible {
            self.frame
                .as_ref()
                .map_or(&EMPTY_PANES, |frame| &frame.panes)
        } else {
            &EMPTY_PANES
        }
    }
    pub fn dividers(&self) -> &[Option<DividerHit>; MAX_NODES / 2] {
        if self.visible {
            self.frame
                .as_ref()
                .map_or(&EMPTY_DIVIDERS, |frame| &frame.dividers)
        } else {
            &EMPTY_DIVIDERS
        }
    }
    /// Lookups are bounded to four panes/three dividers. Callers check `current`
    /// before using a hit; focus changes may legitimately retire its old frame.
    pub fn pane_at(&self, point: Position) -> Option<PaneHit> {
        self.panes()
            .iter()
            .flatten()
            .find(|pane| pane.outer.contains(point))
            .cloned()
    }
    pub fn divider_at(&self, point: Position) -> Option<DividerHit> {
        self.dividers()
            .iter()
            .flatten()
            .find(|hit| hit.divider.rect.contains(point))
            .cloned()
    }
    pub fn current(&self, proof: &GeometryProof, layout: &Layout, groups: &Groups) -> bool {
        if !self.visible {
            return false;
        }
        let Some(frame) = self.frame.as_ref() else {
            return false;
        };
        &frame.proof == proof
            && groups.proof_current(&frame.groups)
            && layout.geometry_current(&frame.geometry)
            && layout
                .project(frame.geometry.area(), groups.active_group())
                .is_ok_and(|current| current == frame.geometry)
    }
    /// Validate all late entries before publishing a new authorization identity.
    /// Same frame repaints preserve its identity; any geometry/group/header/gutter
    /// change advances it. An invalid repaint stays hidden after `begin_frame`.
    pub fn present(
        &mut self,
        geometry: Geometry,
        groups: &Groups,
        rects: [Option<PaneRects>; MAX_LEAVES],
    ) -> Result<GeometryProof> {
        let mut count = 0;
        let mut members = [None; MAX_LEAVES];
        for (index, group) in groups.groups().iter().enumerate() {
            ensure!(
                index < MAX_LEAVES,
                "Editor presentation exceeds four groups"
            );
            let placement = geometry.placements()[index].context("Missing group placement")?;
            let pane = rects[index].context("Missing final pane rectangles")?;
            ensure!(
                placement.group == group.id()
                    && pane.group == group.id()
                    && pane.outer == placement.outer,
                "Pane/group placement identity differs"
            );
            pane.validate()?;
            let active = group
                .active()
                .context("Cannot present an empty editor group")?;
            members[index] = Some(Membership {
                group: group.id(),
                tab: active.id(),
                document: active.document(),
            });
            count += 1;
        }
        ensure!(
            geometry.placements()[count..].iter().all(Option::is_none)
                && rects[count..].iter().all(Option::is_none),
            "Extra pane placement outside current groups"
        );
        let group_proof = groups.proof();
        let same = self.frame.as_ref().is_some_and(|frame| {
            frame.geometry.same_revision(&geometry)
                && frame.geometry == geometry
                && frame.groups == group_proof
                && frame.rects == rects
        });
        let (epoch, proof) = if same {
            let frame = self
                .frame
                .as_ref()
                .context("Editor presentation disappeared")?;
            (self.epoch, frame.proof.clone())
        } else {
            let epoch = self
                .epoch
                .checked_add(1)
                .context("Editor geometry epoch exhausted")?;
            (
                epoch,
                GeometryProof {
                    identity: Arc::new(()),
                    epoch,
                },
            )
        };
        let panes = std::array::from_fn(|index| {
            let member = members[index]?;
            let pane = rects[index]?;
            Some(PaneHit {
                membership: member,
                group: pane.group,
                outer: pane.outer,
                strip: pane.strip,
                breadcrumbs: pane.breadcrumbs,
                text: pane.text,
                proof: proof.clone(),
            })
        });
        let dividers = std::array::from_fn(|index| {
            geometry.dividers()[index]
                .as_ref()
                .map(|divider| DividerHit {
                    divider: divider.clone(),
                    proof: proof.clone(),
                })
        });
        self.frame = Some(Frame {
            geometry,
            groups: group_proof,
            rects,
            panes,
            dividers,
            proof: proof.clone(),
        });
        self.epoch = epoch;
        self.visible = true;
        Ok(proof)
    }
}
static EMPTY_PANES: [Option<PaneHit>; MAX_LEAVES] = [const { None }; MAX_LEAVES];
static EMPTY_DIVIDERS: [Option<DividerHit>; MAX_NODES / 2] = [const { None }; MAX_NODES / 2];
fn endpoints(rect: Rect) -> Result<()> {
    ensure!(
        rect.x.checked_add(rect.width).is_some() && rect.y.checked_add(rect.height).is_some(),
        "Editor rectangle endpoint overflows"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_layout::{Axis, Direction};

    fn nested() -> (Groups, Layout) {
        let mut groups = Groups::default();
        groups.open(1).unwrap();
        let first = groups.active_group().unwrap();
        let mut layout = Layout::default();
        let initial = layout.prepare_flat(&[first], Axis::Columns).unwrap();
        layout.commit(initial).unwrap();
        groups.split_active().unwrap();
        let second = groups.active_group().unwrap();
        let right = layout
            .prepare_split(first, second, Direction::Right)
            .unwrap();
        layout.commit(right).unwrap();
        groups.split_active().unwrap();
        let third = groups.active_group().unwrap();
        let down = layout
            .prepare_split(second, third, Direction::Down)
            .unwrap();
        layout.commit(down).unwrap();
        (groups, layout)
    }
    fn rects(geometry: &Geometry, active: Option<GroupId>) -> [Option<PaneRects>; MAX_LEAVES] {
        std::array::from_fn(|index| {
            let placement = geometry.placements()[index]?;
            let mut pane = PaneRects::reserve(
                placement.group,
                placement.outer,
                active == Some(placement.group),
            )
            .unwrap();
            if pane.text.width > 0 {
                let gutter = 5.min(pane.text.width);
                pane.text.x += gutter;
                pane.text.width -= gutter;
            }
            Some(pane)
        })
    }
    #[test]
    fn nested_frame_maps_exact_groups_actual_gutters_and_disjoint_dividers() {
        let (groups, layout) = nested();
        let geometry = layout
            .project(Rect::new(26, 2, 93, 32), groups.active_group())
            .unwrap();
        let panes = rects(&geometry, groups.active_group());
        let mut presentation = Presentation::default();
        let proof = presentation
            .present(geometry.clone(), &groups, panes)
            .unwrap();
        assert!(presentation.current(&proof, &layout, &groups));
        assert_eq!(presentation.panes().iter().flatten().count(), 3);
        assert_eq!(presentation.dividers().iter().flatten().count(), 2);
        for pane in presentation.panes().iter().flatten() {
            let found = presentation
                .pane_at(Position::new(pane.strip.x, pane.strip.y))
                .unwrap();
            assert_eq!(found.membership, pane.membership);
            assert_eq!(pane.text.x, pane.outer.x + 5);
            assert!(groups.membership_current(pane.membership));
            for divider in presentation.dividers().iter().flatten() {
                assert!(!pane.outer.intersects(divider.divider.rect));
            }
        }
        for divider in presentation.dividers().iter().flatten() {
            let point = Position::new(divider.divider.rect.x, divider.divider.rect.y);
            assert!(presentation.pane_at(point).is_none());
            assert_eq!(presentation.divider_at(point).unwrap().proof, proof);
        }
        presentation.begin_frame();
        assert!(presentation.proof().is_none());
        assert!(presentation.pane_at(Position::new(26, 2)).is_none());
        assert_eq!(
            presentation.present(geometry, &groups, panes).unwrap(),
            proof
        );
    }
    #[test]
    fn late_bad_gutter_validation_preserves_old_frame_but_does_not_reauthorize_failed_repaint() {
        let (groups, layout) = nested();
        let geometry = layout
            .project(Rect::new(0, 0, 96, 32), groups.active_group())
            .unwrap();
        let panes = rects(&geometry, groups.active_group());
        let mut presentation = Presentation::default();
        let proof = presentation
            .present(geometry.clone(), &groups, panes)
            .unwrap();
        let epoch = presentation.epoch();
        let mut bad = panes;
        bad[2].as_mut().unwrap().text.y -= 1;
        assert!(
            presentation
                .present(geometry.clone(), &groups, bad)
                .is_err()
        );
        assert_eq!(presentation.epoch(), epoch);
        assert!(presentation.current(&proof, &layout, &groups));
        presentation.begin_frame();
        assert!(presentation.present(geometry, &groups, bad).is_err());
        assert_eq!(presentation.epoch(), epoch);
        assert!(!presentation.current(&proof, &layout, &groups));
    }
    #[test]
    fn sidebar_geometry_aba_and_header_or_gutter_changes_retire_original_hit_proofs() {
        let (groups, layout) = nested();
        let area = Rect::new(26, 2, 93, 32);
        let geometry = layout.project(area, groups.active_group()).unwrap();
        let panes = rects(&geometry, groups.active_group());
        let mut presentation = Presentation::default();
        let original = presentation
            .present(geometry.clone(), &groups, panes)
            .unwrap();
        presentation.invalidate().unwrap();
        let intermediate = layout
            .project(Rect::new(0, 2, 119, 32), groups.active_group())
            .unwrap();
        let intermediate_rects = rects(&intermediate, groups.active_group());
        presentation
            .present(intermediate, &groups, intermediate_rects)
            .unwrap();
        presentation.invalidate().unwrap();
        let returned = presentation
            .present(geometry.clone(), &groups, panes)
            .unwrap();
        assert_ne!(returned, original);
        assert!(!presentation.current(&original, &layout, &groups));
        let mut changed = panes;
        changed[0].as_mut().unwrap().text.x += 1;
        changed[0].as_mut().unwrap().text.width -= 1;
        let gutter = presentation.present(geometry, &groups, changed).unwrap();
        assert_ne!(gutter, returned);
        assert!(!presentation.current(&returned, &layout, &groups));
    }
    #[test]
    fn group_focus_aba_and_layout_resize_retire_maps_even_at_old_outer_area() {
        let (mut groups, mut layout) = nested();
        let area = Rect::new(0, 0, 96, 32);
        let geometry = layout.project(area, groups.active_group()).unwrap();
        let panes = rects(&geometry, groups.active_group());
        let mut presentation = Presentation::default();
        let proof = presentation
            .present(geometry.clone(), &groups, panes)
            .unwrap();
        let original = groups.active_group().unwrap();
        groups.focus_group(groups.groups()[0].id()).unwrap();
        groups.focus_group(original).unwrap();
        assert!(!presentation.current(&proof, &layout, &groups));
        let proof = presentation
            .present(geometry.clone(), &groups, panes)
            .unwrap();
        let divider = geometry.dividers()[0].as_ref().unwrap();
        let resize = layout
            .prepare_resize(
                &divider.proof,
                i32::from(divider.first_cells) + 4,
                &geometry,
            )
            .unwrap();
        layout.commit(resize).unwrap();
        assert!(!presentation.current(&proof, &layout, &groups));
    }
    #[test]
    fn tiny_projection_keeps_topology_but_exposes_only_current_nonzero_outer_pane() {
        let (groups, layout) = nested();
        let ids = layout.groups();
        let saved = layout.export(&ids).unwrap();
        let original_generation = layout.generation();
        let geometry = layout
            .project(Rect::new(4, 3, 1, 1), groups.active_group())
            .unwrap();
        assert!(geometry.active_only());
        let panes = rects(&geometry, groups.active_group());
        let mut presentation = Presentation::default();
        let proof = presentation.present(geometry, &groups, panes).unwrap();
        assert!(presentation.current(&proof, &layout, &groups));
        assert_eq!(
            presentation.pane_at(Position::new(4, 3)).unwrap().group,
            groups.active_group().unwrap()
        );
        assert!(presentation.dividers().iter().all(Option::is_none));
        assert_eq!(layout.groups(), ids);
        assert_eq!(layout.export(&ids).unwrap(), saved);
        assert_eq!(layout.generation(), original_generation);
        let zero = layout
            .project(Rect::new(4, 3, 0, 1), groups.active_group())
            .unwrap();
        let zero_rects = rects(&zero, groups.active_group());
        presentation.present(zero, &groups, zero_rects).unwrap();
        assert!(presentation.pane_at(Position::new(4, 3)).is_none());
    }
    #[test]
    fn epoch_exhaustion_fails_closed_and_cannot_reuse_any_previously_held_identity() {
        let (groups, layout) = nested();
        let geometry = layout
            .project(Rect::new(0, 0, 96, 32), groups.active_group())
            .unwrap();
        let panes = rects(&geometry, groups.active_group());
        let mut presentation = Presentation::default();
        let proof = presentation
            .present(geometry.clone(), &groups, panes)
            .unwrap();
        presentation.epoch = u64::MAX;
        assert!(presentation.invalidate().is_err());
        assert!(presentation.proof().is_none());
        assert!(!presentation.current(&proof, &layout, &groups));
        assert!(presentation.present(geometry, &groups, panes).is_err());
        assert!(presentation.panes().iter().all(Option::is_none));
    }
}
