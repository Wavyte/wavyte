use crate::v03::foundation::ids::NodeIdx;
use crate::v03::layout::RectPx;
use crate::v03::normalize::ir::{CompositionIR, NodeKindIR};
use taffy::prelude::{AvailableSpace, NodeId, Size};
use taffy::style::Style;

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
        }
    }

    pub(crate) fn ensure_tree(&mut self, ir: &CompositionIR) -> Result<(), taffy::TaffyError> {
        if self.node_to_taffy.len() == ir.nodes.len() && self.taffy_root.is_some() {
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
