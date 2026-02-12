use crate::eval::context::NodeTimeCtx;
use crate::foundation::ids::NodeIdx;
use crate::normalize::ir::{CollectionModeIR, CompositionIR, NodeKindIR};

pub(crate) fn compute_node_time_ctxs(
    ir: &CompositionIR,
    global_frame: u64,
    out: &mut Vec<NodeTimeCtx>,
) {
    out.clear();
    out.resize(ir.nodes.len(), NodeTimeCtx::default());

    // Root node’s parent local is global frame.
    rec_time(ir, ir.root, global_frame as i64, out);
}

fn rec_time(ir: &CompositionIR, idx: NodeIdx, parent_local_i64: i64, out: &mut [NodeTimeCtx]) {
    let node = &ir.nodes[idx.0 as usize];
    let start = node.range.start as i64;
    let end = node.range.end as i64;
    let dur = (end - start).max(0) as u64;
    let dur_u32 = u32::try_from(dur).unwrap_or(u32::MAX);

    let local = parent_local_i64 - start;
    out[idx.0 as usize] = NodeTimeCtx {
        local_frame_i64: local,
        duration_frames: dur_u32,
    };

    let NodeKindIR::Collection {
        mode,
        children,
        sequence_prefix_starts,
    } = &node.kind
    else {
        return;
    };

    match mode {
        CollectionModeIR::Group | CollectionModeIR::Stack | CollectionModeIR::Switch => {
            for &c in children {
                rec_time(ir, c, local, out);
            }
        }
        CollectionModeIR::Sequence => {
            let Some(prefix) = sequence_prefix_starts.as_ref() else {
                return;
            };
            for (i, &c) in children.iter().enumerate() {
                let off = prefix.get(i).copied().unwrap_or(0) as i64;
                rec_time(ir, c, local - off, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::anim::{AnimDef, AnimTaggedDef};
    use crate::normalize::pass::normalize;
    use crate::scene::model::{
        AssetDef, CanvasDef, CollectionModeDef, CompositionDef, FpsDef, NodeDef, NodeKindDef,
    };
    use std::collections::BTreeMap;

    #[test]
    fn sequence_prefix_remap_maps_child_local_time() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let child_a = NodeDef {
            id: "a".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            blend: Default::default(),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };
        let child_b = NodeDef {
            id: "b".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Tagged(AnimTaggedDef::Expr("=1+2".to_owned())),
            blend: Default::default(),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 1,
                height: 1,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 20,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Sequence,
                    children: vec![child_a, child_b],
                },
                range: [0, 20],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                blend: Default::default(),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let ir = &norm.ir;

        let mut ctxs = Vec::new();
        compute_node_time_ctxs(ir, 10, &mut ctxs);
        // At global_frame=10, sequence child B becomes active and has local=0.
        let a_idx = *norm
            .node_idx_by_id
            .get(&norm.interner.lookup("a").unwrap())
            .unwrap();
        let b_idx = *norm
            .node_idx_by_id
            .get(&norm.interner.lookup("b").unwrap())
            .unwrap();
        assert!(!ctxs[a_idx.0 as usize].is_in_range());
        assert!(ctxs[b_idx.0 as usize].is_in_range());
        assert_eq!(ctxs[b_idx.0 as usize].local_frame_i64, 0);
    }
}
