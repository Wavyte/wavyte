use crate::v03::animation::anim::{Anim, SampleCtx};
use crate::v03::eval::context::NodeTimeCtx;
use crate::v03::expression::bytecode::TimeField;
use crate::v03::expression::program::{ExprProgram, PropertyProgram, ValueType};
use crate::v03::expression::vm::{ValueSlot, VmError, eval_program_with_stack};
use crate::v03::foundation::ids::{NodeIdx, PropertyId, VarId};
use crate::v03::normalize::ir::{CompositionIR, VarValueIR};
use crate::v03::normalize::property::PropertyKey;

#[derive(Debug)]
pub(crate) struct PropertyValues {
    values_by_pid: Vec<ValueSlot>,
}

#[derive(Debug)]
pub(crate) struct PropertyEvalScratch {
    // Scratch stack reused across all bytecode evals (hot-path allocation avoidance).
    vm_stack: Vec<ValueSlot>,
}

impl PropertyValues {
    pub(crate) fn new(program: &ExprProgram) -> Self {
        Self {
            values_by_pid: vec![ValueSlot::U64(0); program.entry_by_pid.len()],
        }
    }

    pub(crate) fn get(&self, pid: PropertyId) -> Result<ValueSlot, VmError> {
        self.values_by_pid
            .get(pid.0 as usize)
            .copied()
            .ok_or_else(|| VmError::new("property id out of range"))
    }

    fn set(&mut self, pid: PropertyId, v: ValueSlot) -> Result<(), VmError> {
        let Some(slot) = self.values_by_pid.get_mut(pid.0 as usize) else {
            return Err(VmError::new("property id out of range"));
        };
        *slot = v;
        Ok(())
    }
}

impl PropertyEvalScratch {
    pub(crate) fn new() -> Self {
        Self {
            vm_stack: Vec::with_capacity(32),
        }
    }
}

pub(crate) fn eval_expr_program_frame(
    ir: &CompositionIR,
    time_ctxs: &[NodeTimeCtx],
    program: &ExprProgram,
    out: &mut PropertyValues,
    scratch: &mut PropertyEvalScratch,
) -> Result<(), VmError> {
    // Ensure backing store is consistent with current program shape.
    if out.values_by_pid.len() != program.entry_by_pid.len() {
        *out = PropertyValues::new(program);
    }

    for &pid in &program.eval_order {
        let entry_idx = program
            .entry_by_pid
            .get(pid.0 as usize)
            .and_then(|x| *x)
            .ok_or_else(|| VmError::new("missing entry for property id"))?
            as usize;
        let entry = &program.entries[entry_idx];

        let v = match &entry.program {
            PropertyProgram::SampleNodeLane { node, lane } => {
                sample_node_lane(ir, time_ctxs, *node, *lane, out)?
            }
            PropertyProgram::Expr(bc) => {
                let owner = entry.owner_node;
                let t = time_ctxs
                    .get(owner.0 as usize)
                    .copied()
                    .ok_or_else(|| VmError::new("owner node idx out of range"))?;

                let v = eval_program_with_stack(
                    bc,
                    &mut scratch.vm_stack,
                    |dep| out.get(dep),
                    |vid| load_var(ir, vid),
                    |tf| load_time(ir, t, tf),
                )?;

                coerce_value(v, entry.value_type)?
            }
        };

        out.set(pid, v)?;
    }

    Ok(())
}

fn sample_node_lane(
    ir: &CompositionIR,
    time_ctxs: &[NodeTimeCtx],
    node: NodeIdx,
    lane: PropertyKey,
    vals: &PropertyValues,
) -> Result<ValueSlot, VmError> {
    let node_ir = ir
        .nodes
        .get(node.0 as usize)
        .ok_or_else(|| VmError::new("node idx out of range"))?;
    let t = time_ctxs
        .get(node.0 as usize)
        .copied()
        .ok_or_else(|| VmError::new("node time ctx out of range"))?;
    let frame = t.sample_frame_u64();

    match lane {
        PropertyKey::Opacity => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.opacity,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformTranslateX => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.translate_x,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformTranslateY => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.translate_y,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformRotationRad => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.rotation_rad,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformScaleX => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.scale_x,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformScaleY => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.scale_y,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformAnchorX => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.anchor_x,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformAnchorY => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.anchor_y,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformSkewX => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.skew_x_deg,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::TransformSkewY => Ok(ValueSlot::F64(sample_anim_f64(
            &node_ir.props.skew_y_deg,
            ir,
            frame,
            vals,
        )?)),
        PropertyKey::SwitchActiveIndex => {
            let Some(anim) = node_ir.props.switch_active.as_ref() else {
                return Ok(ValueSlot::U64(0));
            };
            Ok(ValueSlot::U64(sample_anim_u64(anim, ir, frame, vals)?))
        }
        PropertyKey::LayoutX
        | PropertyKey::LayoutY
        | PropertyKey::LayoutWidth
        | PropertyKey::LayoutHeight => Err(VmError::new(
            "layout lanes are forbidden in v0.3 expressions",
        )),
    }
}

fn sample_anim_f64(
    a: &Anim<f64>,
    ir: &CompositionIR,
    frame: u64,
    vals: &PropertyValues,
) -> Result<f64, VmError> {
    match a {
        Anim::Constant(v) => Ok(*v),
        Anim::Keyframes(k) => Ok(k.sample(frame)),
        Anim::Procedural(p) => Ok(p.sample(SampleCtx {
            fps: ir.fps,
            frame,
            seed: ir.seed,
        })),
        Anim::Reference(pid) => Ok(vals.get(*pid)?.as_f64()?),
    }
}

fn sample_anim_u64(
    a: &Anim<u64>,
    ir: &CompositionIR,
    frame: u64,
    vals: &PropertyValues,
) -> Result<u64, VmError> {
    match a {
        Anim::Constant(v) => Ok(*v),
        Anim::Keyframes(k) => Ok(k.sample(frame)),
        Anim::Procedural(p) => Ok(p.sample(SampleCtx {
            fps: ir.fps,
            frame,
            seed: ir.seed,
        })),
        Anim::Reference(pid) => Ok(vals.get(*pid)?.as_u64_floor()?),
    }
}

fn load_var(ir: &CompositionIR, vid: VarId) -> Result<ValueSlot, VmError> {
    let v = ir
        .vars
        .get(vid.0 as usize)
        .ok_or_else(|| VmError::new("VarId out of range"))?;
    match *v {
        VarValueIR::Bool(b) => Ok(ValueSlot::Bool(b)),
        VarValueIR::F64(x) => Ok(ValueSlot::F64(x)),
        VarValueIR::Vec2 { .. } => Err(VmError::new(
            "vars.vec2 is not supported in v0.3 expressions",
        )),
        VarValueIR::Color(_) => Err(VmError::new(
            "vars.color is not supported in v0.3 expressions",
        )),
    }
}

fn load_time(ir: &CompositionIR, t: NodeTimeCtx, tf: TimeField) -> Result<ValueSlot, VmError> {
    match tf {
        TimeField::Frame => Ok(ValueSlot::F64(t.time_frame_f64())),
        TimeField::Fps => Ok(ValueSlot::F64(ir.fps.as_f64())),
        TimeField::Duration => Ok(ValueSlot::F64(t.duration_f64())),
        TimeField::Progress => Ok(ValueSlot::F64(t.progress_f64())),
        TimeField::Seconds => Ok(ValueSlot::F64(ir.fps.frames_to_secs(t.time_frame_u64()))),
    }
}

fn coerce_value(v: ValueSlot, ty: ValueType) -> Result<ValueSlot, VmError> {
    match ty {
        ValueType::F64 => Ok(ValueSlot::F64(v.as_f64()?)),
        ValueType::Bool => Ok(ValueSlot::Bool(v.as_bool()?)),
        ValueType::U64 => Ok(ValueSlot::U64(v.as_u64_floor()?)),
        ValueType::Color => Err(VmError::new(
            "Color is not supported in v0.3 expression runtime yet",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::animation::anim::{
        AnimDef, AnimTaggedDef, InterpModeDef, KeyframeDef, KeyframesDef,
    };
    use crate::v03::eval::time::compute_node_time_ctxs;
    use crate::v03::expression::compile::compile_expr_program;
    use crate::v03::normalize::pass::normalize;
    use crate::v03::scene::model::{
        AssetDef, CanvasDef, CollectionModeDef, CompositionDef, FpsDef, NodeDef, NodeKindDef,
    };
    use std::collections::BTreeMap;

    #[test]
    fn time_progress_hits_1_at_end_boundary_even_when_out_of_range() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let child_a = NodeDef {
            id: "a".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Tagged(AnimTaggedDef::Expr("=time.progress".to_owned())),
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
                    mode: CollectionModeDef::Group,
                    children: vec![child_a],
                },
                range: [0, 20],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut values = PropertyValues::new(&program);
        let mut scratch = PropertyEvalScratch::new();

        let mut t = Vec::new();
        compute_node_time_ctxs(&norm.ir, 10, &mut t);
        eval_expr_program_frame(&norm.ir, &t, &program, &mut values, &mut scratch).unwrap();

        let a_idx = *norm
            .node_idx_by_id
            .get(&norm.interner.lookup("a").unwrap())
            .unwrap();
        let pid = crate::v03::normalize::property::PropertyIndex::property_id(
            a_idx,
            PropertyKey::Opacity,
        );
        let v = values.get(pid).unwrap().as_f64().unwrap();
        assert!((v - 1.0).abs() < 1e-9);
    }

    #[test]
    fn sample_node_lane_uses_sample_frame_clamp_to_duration_minus_one() {
        let mut assets = BTreeMap::new();
        assets.insert("a".to_owned(), AssetDef::Null);

        let keyframes = KeyframesDef {
            keys: vec![
                KeyframeDef {
                    frame: 0,
                    value: 0.0,
                },
                KeyframeDef {
                    frame: 9,
                    value: 9.0,
                },
            ],
            mode: InterpModeDef::Linear,
            default: None,
        };

        let child_a = NodeDef {
            id: "a".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Tagged(AnimTaggedDef::Expr(
                "=nodes.b.transform.translate.x".to_owned(),
            )),
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };

        let mut child_b = NodeDef {
            id: "b".to_owned(),
            kind: NodeKindDef::Leaf {
                asset: "a".to_owned(),
            },
            range: [0, 10],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };
        // Put the keyframes on B.translate_x and reference B from A expression; this forces a SampleNodeLane dep.
        child_b.transform.translate.x = AnimDef::Tagged(AnimTaggedDef::Keyframes(keyframes));

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
                    mode: CollectionModeDef::Group,
                    children: vec![child_a, child_b],
                },
                range: [0, 20],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        let program = compile_expr_program(&norm).unwrap();
        let mut values = PropertyValues::new(&program);
        let mut scratch = PropertyEvalScratch::new();

        let mut t = Vec::new();
        // Global frame 10 is out of range for children; sample frame should clamp to 9 for duration 10.
        compute_node_time_ctxs(&norm.ir, 10, &mut t);
        eval_expr_program_frame(&norm.ir, &t, &program, &mut values, &mut scratch).unwrap();

        let a_idx = *norm
            .node_idx_by_id
            .get(&norm.interner.lookup("a").unwrap())
            .unwrap();
        let pid = crate::v03::normalize::property::PropertyIndex::property_id(
            a_idx,
            PropertyKey::Opacity,
        );
        let v = values.get(pid).unwrap().as_f64().unwrap();
        assert!((v - 9.0).abs() < 1e-9);
    }
}
