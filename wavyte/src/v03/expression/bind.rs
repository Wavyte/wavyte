use crate::v03::expression::ast::Expr;
use crate::v03::expression::bytecode::TimeField;
use crate::v03::expression::error::ExprError;
use crate::v03::foundation::ids::{NodeIdx, VarId};
use crate::v03::normalize::intern::{InternId, StringInterner};
use crate::v03::normalize::property::{PropertyIndex, PropertyKey};
use std::collections::HashMap;

pub(crate) struct BindCtx<'a> {
    pub(crate) owner_node: NodeIdx,
    pub(crate) interner: &'a StringInterner,
    pub(crate) node_idx_by_id: &'a HashMap<InternId, NodeIdx>,
    pub(crate) var_id_by_key: &'a HashMap<InternId, VarId>,
}

pub(crate) fn bind_expr(e: Expr, ctx: &BindCtx<'_>) -> Result<Expr, ExprError> {
    match e {
        Expr::Lit(_) => Ok(e),
        Expr::Unary { op, expr } => Ok(Expr::Unary {
            op,
            expr: Box::new(bind_expr(*expr, ctx)?),
        }),
        Expr::Binary { op, left, right } => Ok(Expr::Binary {
            op,
            left: Box::new(bind_expr(*left, ctx)?),
            right: Box::new(bind_expr(*right, ctx)?),
        }),
        Expr::Call { func, args } => {
            let mut out_args = Vec::with_capacity(args.len());
            for a in args {
                out_args.push(bind_expr(a, ctx)?);
            }
            Ok(Expr::Call {
                func,
                args: out_args,
            })
        }
        Expr::Path(p) => bind_path(p, ctx),
        Expr::Prop(_) | Expr::Var(_) | Expr::Time(_) => Ok(e),
    }
}

fn bind_path(p: Vec<String>, ctx: &BindCtx<'_>) -> Result<Expr, ExprError> {
    if p.is_empty() {
        return Err(ExprError::new(0, "empty path"));
    }

    match p[0].as_str() {
        "self" => {
            if p.len() < 2 {
                return Err(ExprError::new(0, "self.<lane> expected"));
            }
            let key = property_key_from_segments(&p[1..])
                .ok_or_else(|| ExprError::new(0, format!("unknown property path {:?}", &p[1..])))?;
            let pid = PropertyIndex::property_id(ctx.owner_node, key);
            Ok(Expr::Prop(pid))
        }
        "nodes" => {
            if p.len() < 3 {
                return Err(ExprError::new(0, "nodes.<id>.<lane> expected"));
            }
            let node_id_str = &p[1];
            let node_intern = ctx
                .interner
                .lookup(node_id_str)
                .ok_or_else(|| ExprError::new(0, format!("unknown node id \"{node_id_str}\"")))?;
            let node_idx = ctx
                .node_idx_by_id
                .get(&node_intern)
                .copied()
                .ok_or_else(|| ExprError::new(0, format!("unknown node id \"{node_id_str}\"")))?;
            let key = property_key_from_segments(&p[2..])
                .ok_or_else(|| ExprError::new(0, format!("unknown property path {:?}", &p[2..])))?;
            let pid = PropertyIndex::property_id(node_idx, key);
            Ok(Expr::Prop(pid))
        }
        "vars" => {
            if p.len() != 2 {
                return Err(ExprError::new(0, "vars.<name> expected"));
            }
            let name = &p[1];
            let key = ctx
                .interner
                .lookup(name)
                .ok_or_else(|| ExprError::new(0, format!("unknown var \"{name}\"")))?;
            let vid = ctx
                .var_id_by_key
                .get(&key)
                .copied()
                .ok_or_else(|| ExprError::new(0, format!("unknown var \"{name}\"")))?;
            Ok(Expr::Var(vid))
        }
        "time" => {
            if p.len() != 2 {
                return Err(ExprError::new(0, "time.<field> expected"));
            }
            let tf = match p[1].as_str() {
                "frame" => TimeField::Frame,
                "fps" => TimeField::Fps,
                "duration" => TimeField::Duration,
                "progress" => TimeField::Progress,
                "seconds" => TimeField::Seconds,
                other => {
                    return Err(ExprError::new(0, format!("unknown time field \"{other}\"")));
                }
            };
            Ok(Expr::Time(tf))
        }
        other => Err(ExprError::new(
            0,
            format!("unknown top-level namespace \"{other}\""),
        )),
    }
}

fn property_key_from_segments(segs: &[String]) -> Option<PropertyKey> {
    match segs.len() {
        1 if segs[0] == "opacity" => Some(PropertyKey::Opacity),
        2 if segs[0] == "transform" && segs[1] == "rotation_rad" => {
            Some(PropertyKey::TransformRotationRad)
        }
        2 if segs[0] == "switch" && segs[1] == "active" => Some(PropertyKey::SwitchActiveIndex),
        3 if segs[0] == "transform" && segs[1] == "translate" && segs[2] == "x" => {
            Some(PropertyKey::TransformTranslateX)
        }
        3 if segs[0] == "transform" && segs[1] == "translate" && segs[2] == "y" => {
            Some(PropertyKey::TransformTranslateY)
        }
        3 if segs[0] == "transform" && segs[1] == "scale" && segs[2] == "x" => {
            Some(PropertyKey::TransformScaleX)
        }
        3 if segs[0] == "transform" && segs[1] == "scale" && segs[2] == "y" => {
            Some(PropertyKey::TransformScaleY)
        }
        3 if segs[0] == "transform" && segs[1] == "anchor" && segs[2] == "x" => {
            Some(PropertyKey::TransformAnchorX)
        }
        3 if segs[0] == "transform" && segs[1] == "anchor" && segs[2] == "y" => {
            Some(PropertyKey::TransformAnchorY)
        }
        3 if segs[0] == "transform" && segs[1] == "skew" && segs[2] == "x" => {
            Some(PropertyKey::TransformSkewX)
        }
        3 if segs[0] == "transform" && segs[1] == "skew" && segs[2] == "y" => {
            Some(PropertyKey::TransformSkewY)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::expression::parser::parse_expr;
    use crate::v03::normalize::intern::StringInterner;

    #[test]
    fn binds_self_and_nodes_and_vars_and_time() {
        let mut interner = StringInterner::new();
        let node_a = interner.intern("a");
        let var_x = interner.intern("x");

        let mut node_idx_by_id = HashMap::new();
        node_idx_by_id.insert(node_a, NodeIdx(7));

        let mut var_id_by_key = HashMap::new();
        var_id_by_key.insert(var_x, VarId(1));

        let ctx = BindCtx {
            owner_node: NodeIdx(3),
            interner: &interner,
            node_idx_by_id: &node_idx_by_id,
            var_id_by_key: &var_id_by_key,
        };

        let e = bind_expr(parse_expr("self.opacity").unwrap(), &ctx).unwrap();
        assert!(matches!(e, Expr::Prop(_)));

        let e = bind_expr(parse_expr("nodes.a.opacity").unwrap(), &ctx).unwrap();
        assert_eq!(
            e,
            Expr::Prop(PropertyIndex::property_id(NodeIdx(7), PropertyKey::Opacity))
        );

        let e = bind_expr(parse_expr("vars.x").unwrap(), &ctx).unwrap();
        assert_eq!(e, Expr::Var(VarId(1)));

        let e = bind_expr(parse_expr("time.fps").unwrap(), &ctx).unwrap();
        assert_eq!(e, Expr::Time(TimeField::Fps));
    }
}
