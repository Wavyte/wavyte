use crate::foundation::math::Fnv1a64;
use crate::v03::animation::anim::{Anim, SampleCtx};
use crate::v03::foundation::ids::NodeIdx;
use crate::v03::layout::RectPx;
use crate::v03::normalize::ir::{CompositionIR, NodeKindIR};
use crate::v03::normalize::property::PropertyKey;
use crate::v03::{
    eval::{context::NodeTimeCtx, properties::PropertyValues},
    normalize::ir::{AnimDimensionIR, LayoutPropsIR},
};
use taffy::prelude::{AvailableSpace, NodeId, Rect, Size};
use taffy::style::{
    AlignContent, AlignItems, Dimension, Display, FlexDirection, FlexWrap, JustifyContent,
    LengthPercentage, LengthPercentageAuto, Position, Style,
};

#[derive(Debug, Clone, Copy)]
struct LayoutNodeCtx {
    node: NodeIdx,
}

/// Session-owned Taffy bridge.
///
/// Phase 4 progressively wires this into per-frame evaluation. The tree structure is intended
/// to be rebuilt only when the composition structure changes.
#[derive(Debug)]
pub(crate) struct TaffyBridge {
    taffy: taffy::TaffyTree<LayoutNodeCtx>,
    pub(crate) node_to_taffy: Vec<Option<NodeId>>,
    pub(crate) taffy_root: Option<NodeId>,
    pub(crate) layout_rects: Vec<RectPx>,
    built_for_nodes_len: usize,
}

impl Default for TaffyBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl TaffyBridge {
    pub(crate) fn new() -> Self {
        Self {
            taffy: taffy::TaffyTree::new(),
            node_to_taffy: Vec::new(),
            taffy_root: None,
            layout_rects: Vec::new(),
            built_for_nodes_len: 0,
        }
    }

    pub(crate) fn ensure_tree(&mut self, ir: &CompositionIR) -> Result<(), taffy::TaffyError> {
        if self.built_for_nodes_len == ir.nodes.len() && self.layout_rects.len() == ir.nodes.len() {
            return Ok(());
        }
        self.rebuild_tree(ir)
    }

    pub(crate) fn rebuild_tree(&mut self, ir: &CompositionIR) -> Result<(), taffy::TaffyError> {
        self.taffy = taffy::TaffyTree::new();
        self.node_to_taffy.clear();
        self.node_to_taffy.resize(ir.nodes.len(), None);
        self.layout_rects.clear();
        self.layout_rects.resize(ir.nodes.len(), RectPx::default());
        self.built_for_nodes_len = ir.nodes.len();

        // Build a minimal tree containing only nodes that opt into layout.
        let root = self.build_subtree(ir, ir.root, None)?;
        self.taffy_root = root;
        Ok(())
    }

    pub(crate) fn compute_layout_canvas(
        &mut self,
        canvas_w: f32,
        canvas_h: f32,
    ) -> Result<(), taffy::TaffyError> {
        let Some(root) = self.taffy_root else {
            return Ok(());
        };

        let available = Size {
            width: AvailableSpace::Definite(canvas_w),
            height: AvailableSpace::Definite(canvas_h),
        };
        self.taffy.compute_layout(root, available)?;

        for (i, maybe) in self.node_to_taffy.iter().enumerate() {
            if let Some(nid) = maybe {
                let l = self.taffy.layout(*nid)?;
                self.layout_rects[i] = RectPx {
                    x: l.location.x,
                    y: l.location.y,
                    w: l.size.width,
                    h: l.size.height,
                };
            }
        }

        Ok(())
    }

    pub(crate) fn update_styles_for_frame(
        &mut self,
        ir: &CompositionIR,
        time_ctxs: &[NodeTimeCtx],
        props: Option<&PropertyValues>,
    ) -> Result<(), crate::v03::expression::vm::VmError> {
        for (i, maybe) in self.node_to_taffy.iter().enumerate() {
            let Some(nid) = maybe else {
                continue;
            };
            let idx = NodeIdx(i as u32);
            let Some(lp) = ir.nodes.get(i).and_then(|n| n.layout.as_ref()) else {
                continue;
            };

            let t = time_ctxs
                .get(i)
                .copied()
                .ok_or_else(|| crate::v03::expression::vm::VmError::new("layout time ctx OOB"))?;
            let frame = t.sample_frame_u64();

            let style = style_for_node(ir, idx, lp, frame, props)?;
            self.taffy
                .set_style(*nid, style)
                .map_err(|e| crate::v03::expression::vm::VmError::new(e.to_string()))?;
        }
        Ok(())
    }

    fn build_subtree(
        &mut self,
        ir: &CompositionIR,
        idx: NodeIdx,
        parent: Option<NodeId>,
    ) -> Result<Option<NodeId>, taffy::TaffyError> {
        let node = &ir.nodes[idx.0 as usize];
        let Some(_layout) = node.layout.as_ref() else {
            // If the node doesn't opt into layout, we treat the entire subtree as non-layout for now.
            return Ok(None);
        };

        let mut children_ids = Vec::<NodeId>::new();
        if let NodeKindIR::Collection { children, .. } = &node.kind {
            for &c in children {
                if let Some(cid) = self.build_subtree(ir, c, None)? {
                    children_ids.push(cid);
                }
            }
        }

        let style = Style::default();
        let nid = if children_ids.is_empty() {
            self.taffy
                .new_leaf_with_context(style, LayoutNodeCtx { node: idx })?
        } else {
            let nid = self.taffy.new_with_children(style, &children_ids)?;
            self.taffy
                .set_node_context(nid, Some(LayoutNodeCtx { node: idx }))?;
            nid
        };

        self.node_to_taffy[idx.0 as usize] = Some(nid);

        if let Some(parent) = parent {
            // Caller may attach explicitly; not used in the initial scaffold.
            let _ = parent;
        }

        Ok(Some(nid))
    }
}

fn style_for_node(
    ir: &CompositionIR,
    node: NodeIdx,
    lp: &LayoutPropsIR,
    frame: u64,
    props: Option<&PropertyValues>,
) -> Result<Style, crate::v03::expression::vm::VmError> {
    let display = match lp.display {
        crate::v03::normalize::ir::LayoutDisplayIR::None => Display::None,
        crate::v03::normalize::ir::LayoutDisplayIR::Flex => Display::Flex,
        crate::v03::normalize::ir::LayoutDisplayIR::Grid => Display::Grid,
    };

    let position = match lp.position {
        crate::v03::normalize::ir::LayoutPositionIR::Relative => Position::Relative,
        crate::v03::normalize::ir::LayoutPositionIR::Absolute => Position::Absolute,
    };

    let flex_direction = match lp.direction {
        crate::v03::normalize::ir::LayoutDirectionIR::Row => FlexDirection::Row,
        crate::v03::normalize::ir::LayoutDirectionIR::Column => FlexDirection::Column,
    };
    let flex_wrap = match lp.wrap {
        crate::v03::normalize::ir::LayoutWrapIR::NoWrap => FlexWrap::NoWrap,
        crate::v03::normalize::ir::LayoutWrapIR::Wrap => FlexWrap::Wrap,
    };

    let justify_content = Some(match lp.justify_content {
        crate::v03::normalize::ir::LayoutJustifyContentIR::Start => JustifyContent::Start,
        crate::v03::normalize::ir::LayoutJustifyContentIR::End => JustifyContent::End,
        crate::v03::normalize::ir::LayoutJustifyContentIR::Center => JustifyContent::Center,
        crate::v03::normalize::ir::LayoutJustifyContentIR::SpaceBetween => {
            JustifyContent::SpaceBetween
        }
        crate::v03::normalize::ir::LayoutJustifyContentIR::SpaceAround => {
            JustifyContent::SpaceAround
        }
        crate::v03::normalize::ir::LayoutJustifyContentIR::SpaceEvenly => {
            JustifyContent::SpaceEvenly
        }
    });
    let align_items = Some(match lp.align_items {
        crate::v03::normalize::ir::LayoutAlignItemsIR::Start => AlignItems::Start,
        crate::v03::normalize::ir::LayoutAlignItemsIR::End => AlignItems::End,
        crate::v03::normalize::ir::LayoutAlignItemsIR::Center => AlignItems::Center,
        crate::v03::normalize::ir::LayoutAlignItemsIR::Stretch => AlignItems::Stretch,
    });
    let align_content = Some(match lp.align_content {
        crate::v03::normalize::ir::LayoutAlignContentIR::Start => AlignContent::Start,
        crate::v03::normalize::ir::LayoutAlignContentIR::End => AlignContent::End,
        crate::v03::normalize::ir::LayoutAlignContentIR::Center => AlignContent::Center,
        crate::v03::normalize::ir::LayoutAlignContentIR::SpaceBetween => AlignContent::SpaceBetween,
        crate::v03::normalize::ir::LayoutAlignContentIR::SpaceAround => AlignContent::SpaceAround,
        crate::v03::normalize::ir::LayoutAlignContentIR::SpaceEvenly => AlignContent::SpaceEvenly,
        crate::v03::normalize::ir::LayoutAlignContentIR::Stretch => AlignContent::Stretch,
    });

    let gap_x = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutGapX,
        &lp.gap_x_px,
        frame,
        props,
    )? as f32;
    let gap_y = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutGapY,
        &lp.gap_y_px,
        frame,
        props,
    )? as f32;
    let gap = Size {
        width: LengthPercentage::length(gap_x.max(0.0)),
        height: LengthPercentage::length(gap_y.max(0.0)),
    };

    let pad_top = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutPaddingTopPx,
        &lp.padding_px.top,
        frame,
        props,
    )? as f32;
    let pad_right = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutPaddingRightPx,
        &lp.padding_px.right,
        frame,
        props,
    )? as f32;
    let pad_bottom = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutPaddingBottomPx,
        &lp.padding_px.bottom,
        frame,
        props,
    )? as f32;
    let pad_left = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutPaddingLeftPx,
        &lp.padding_px.left,
        frame,
        props,
    )? as f32;
    let padding = Rect {
        left: LengthPercentage::length(pad_left.max(0.0)),
        right: LengthPercentage::length(pad_right.max(0.0)),
        top: LengthPercentage::length(pad_top.max(0.0)),
        bottom: LengthPercentage::length(pad_bottom.max(0.0)),
    };

    let mar_top = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutMarginTopPx,
        &lp.margin_px.top,
        frame,
        props,
    )? as f32;
    let mar_right = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutMarginRightPx,
        &lp.margin_px.right,
        frame,
        props,
    )? as f32;
    let mar_bottom = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutMarginBottomPx,
        &lp.margin_px.bottom,
        frame,
        props,
    )? as f32;
    let mar_left = sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutMarginLeftPx,
        &lp.margin_px.left,
        frame,
        props,
    )? as f32;
    let margin = Rect {
        left: LengthPercentageAuto::length(mar_left),
        right: LengthPercentageAuto::length(mar_right),
        top: LengthPercentageAuto::length(mar_top),
        bottom: LengthPercentageAuto::length(mar_bottom),
    };

    let flex_grow = (sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutFlexGrow,
        &lp.flex_grow,
        frame,
        props,
    )? as f32)
        .max(0.0);
    let flex_shrink = (sample_lane_f64(
        ir,
        node,
        PropertyKey::LayoutFlexShrink,
        &lp.flex_shrink,
        frame,
        props,
    )? as f32)
        .max(0.0);

    let size = Size {
        width: sample_dimension(
            ir,
            node,
            &lp.size.width,
            PropertyKey::LayoutWidthPx,
            frame,
            props,
        )?,
        height: sample_dimension(
            ir,
            node,
            &lp.size.height,
            PropertyKey::LayoutHeightPx,
            frame,
            props,
        )?,
    };
    let min_size = Size {
        width: sample_dimension(
            ir,
            node,
            &lp.min_size.width,
            PropertyKey::LayoutMinWidthPx,
            frame,
            props,
        )?,
        height: sample_dimension(
            ir,
            node,
            &lp.min_size.height,
            PropertyKey::LayoutMinHeightPx,
            frame,
            props,
        )?,
    };
    let max_size = Size {
        width: sample_dimension(
            ir,
            node,
            &lp.max_size.width,
            PropertyKey::LayoutMaxWidthPx,
            frame,
            props,
        )?,
        height: sample_dimension(
            ir,
            node,
            &lp.max_size.height,
            PropertyKey::LayoutMaxHeightPx,
            frame,
            props,
        )?,
    };

    Ok(Style {
        display,
        position,
        flex_direction,
        flex_wrap,
        justify_content,
        align_items,
        align_content,
        gap,
        padding,
        margin,
        flex_grow,
        flex_shrink,
        size,
        min_size,
        max_size,
        ..Style::default()
    })
}

fn sample_dimension(
    ir: &CompositionIR,
    node: NodeIdx,
    d: &AnimDimensionIR,
    px_key: PropertyKey,
    frame: u64,
    props: Option<&PropertyValues>,
) -> Result<Dimension, crate::v03::expression::vm::VmError> {
    Ok(match d {
        AnimDimensionIR::Auto => Dimension::auto(),
        AnimDimensionIR::Percent(p) => Dimension::percent((*p).max(0.0)),
        AnimDimensionIR::Px(px) => {
            let v = sample_lane_f64(ir, node, px_key, px, frame, props)? as f32;
            Dimension::length(v.max(0.0))
        }
    })
}

fn sample_lane_f64(
    ir: &CompositionIR,
    node: NodeIdx,
    lane: PropertyKey,
    a: &Anim<f64>,
    frame: u64,
    props: Option<&PropertyValues>,
) -> Result<f64, crate::v03::expression::vm::VmError> {
    match a {
        Anim::Constant(v) => Ok(*v),
        Anim::Keyframes(k) => Ok(k.sample(frame)),
        Anim::Procedural(p) => Ok(p.sample(SampleCtx {
            fps: ir.fps,
            frame,
            seed: mixed_seed(ir.seed, node, lane),
        })),
        Anim::Reference(pid) => {
            let Some(props) = props else {
                return Err(crate::v03::expression::vm::VmError::new(
                    "layout anim references an expression property but no properties were provided",
                ));
            };
            Ok(props.get(*pid)?.as_f64()?)
        }
    }
}

fn mixed_seed(base: u64, node: NodeIdx, lane: PropertyKey) -> u64 {
    let mut h = Fnv1a64::new(base);
    h.write_u64(node.0 as u64);
    h.write_u64(lane.as_u32() as u64);
    h.finish()
}
