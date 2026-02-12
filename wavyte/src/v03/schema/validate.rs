use crate::v03::scene::model::{
    AssetDef, CollectionModeDef, CompositionDef, MaskSourceDef, NodeDef, NodeKindDef,
    TransitionSpecDef,
};
use crate::v03::schema::version::V03_VERSION_STR;
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaPathElem {
    Field(&'static str),
    Index(usize),
}

#[derive(Debug, Clone)]
pub(crate) struct SchemaError {
    pub(crate) path: Vec<SchemaPathElem>,
    pub(crate) message: String,
}

impl SchemaError {
    fn at(path: &[SchemaPathElem], message: impl Into<String>) -> Self {
        Self {
            path: path.to_vec(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            return write!(f, "{}", self.message);
        }
        write!(f, "{}: {}", format_path(&self.path), self.message)
    }
}

fn format_path(path: &[SchemaPathElem]) -> String {
    let mut s = String::from("$");
    for p in path {
        match *p {
            SchemaPathElem::Field(name) => {
                s.push('.');
                s.push_str(name);
            }
            SchemaPathElem::Index(i) => {
                s.push('[');
                s.push_str(&i.to_string());
                s.push(']');
            }
        }
    }
    s
}

#[derive(Debug, Clone)]
pub(crate) struct SchemaErrors {
    pub(crate) errors: Vec<SchemaError>,
}

impl fmt::Display for SchemaErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.errors.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for SchemaErrors {}

pub(crate) fn validate_composition(def: &CompositionDef) -> Result<(), SchemaErrors> {
    let mut errors = Vec::new();

    if def.version != V03_VERSION_STR {
        errors.push(SchemaError::at(
            &[SchemaPathElem::Field("version")],
            format!("version must be \"{V03_VERSION_STR}\""),
        ));
    }

    // Root range should match the declared composition duration to avoid ambiguity.
    if def.root.range[0] != 0 || def.root.range[1] != def.duration {
        errors.push(SchemaError::at(
            &[
                SchemaPathElem::Field("root"),
                SchemaPathElem::Field("range"),
            ],
            "root.range must be [0, composition.duration]",
        ));
    }

    // Pass 1: collect node IDs and basic node invariants in DFS order.
    let mut ids = HashSet::<String>::new();
    let mut all_node_ids = Vec::<String>::new();
    collect_ids_and_validate_ranges(
        &def.root,
        &mut vec![SchemaPathElem::Field("root")],
        &mut ids,
        &mut all_node_ids,
        &mut errors,
    );

    let id_set: HashSet<&str> = all_node_ids.iter().map(|s| s.as_str()).collect();

    // Pass 2: validate references that require the global id set / asset table.
    validate_refs(
        &def.root,
        &mut vec![SchemaPathElem::Field("root")],
        &id_set,
        def,
        &mut errors,
    );
    validate_assets(def, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SchemaErrors { errors })
    }
}

fn validate_assets(def: &CompositionDef, errors: &mut Vec<SchemaError>) {
    for (key, asset) in &def.assets {
        match asset {
            AssetDef::Video {
                trim_start_sec,
                trim_end_sec,
                playback_rate,
                volume,
                mute: _,
                fade_in_sec,
                fade_out_sec,
                source: _,
            }
            | AssetDef::Audio {
                trim_start_sec,
                trim_end_sec,
                playback_rate,
                volume,
                mute: _,
                fade_in_sec,
                fade_out_sec,
                source: _,
            } => {
                if !trim_start_sec.is_finite() || *trim_start_sec < 0.0 {
                    errors.push(SchemaError::at(
                        &[SchemaPathElem::Field("assets")],
                        format!("asset '{key}': trim_start_sec must be finite and >= 0"),
                    ));
                }
                if let Some(end) = trim_end_sec {
                    if !end.is_finite() || *end < 0.0 {
                        errors.push(SchemaError::at(
                            &[SchemaPathElem::Field("assets")],
                            format!("asset '{key}': trim_end_sec must be finite and >= 0"),
                        ));
                    }
                    if *end < *trim_start_sec {
                        errors.push(SchemaError::at(
                            &[SchemaPathElem::Field("assets")],
                            format!("asset '{key}': trim_end_sec must be >= trim_start_sec"),
                        ));
                    }
                }
                if !playback_rate.is_finite() || *playback_rate <= 0.0 {
                    errors.push(SchemaError::at(
                        &[SchemaPathElem::Field("assets")],
                        format!("asset '{key}': playback_rate must be finite and > 0"),
                    ));
                }
                if !volume.is_finite() || *volume < 0.0 {
                    errors.push(SchemaError::at(
                        &[SchemaPathElem::Field("assets")],
                        format!("asset '{key}': volume must be finite and >= 0"),
                    ));
                }
                if !fade_in_sec.is_finite() || *fade_in_sec < 0.0 {
                    errors.push(SchemaError::at(
                        &[SchemaPathElem::Field("assets")],
                        format!("asset '{key}': fade_in_sec must be finite and >= 0"),
                    ));
                }
                if !fade_out_sec.is_finite() || *fade_out_sec < 0.0 {
                    errors.push(SchemaError::at(
                        &[SchemaPathElem::Field("assets")],
                        format!("asset '{key}': fade_out_sec must be finite and >= 0"),
                    ));
                }
            }
            _ => {}
        }
    }
}

fn collect_ids_and_validate_ranges(
    node: &NodeDef,
    path: &mut Vec<SchemaPathElem>,
    ids: &mut HashSet<String>,
    all_node_ids: &mut Vec<String>,
    errors: &mut Vec<SchemaError>,
) {
    // id
    if node.id.trim().is_empty() {
        errors.push(SchemaError::at(
            &[path.as_slice(), &[SchemaPathElem::Field("id")]].concat(),
            "node id must be non-empty",
        ));
    } else {
        all_node_ids.push(node.id.clone());
        if !ids.insert(node.id.clone()) {
            errors.push(SchemaError::at(
                &[path.as_slice(), &[SchemaPathElem::Field("id")]].concat(),
                format!("duplicate node id \"{}\"", node.id),
            ));
        }
    }

    // range
    if node.range[0] > node.range[1] {
        errors.push(SchemaError::at(
            &[path.as_slice(), &[SchemaPathElem::Field("range")]].concat(),
            "range must satisfy start <= end",
        ));
    }

    // sequence child rule: start == 0
    if let NodeKindDef::Collection { mode, children } = &node.kind {
        if matches!(mode, CollectionModeDef::Sequence) {
            for (i, child) in children.iter().enumerate() {
                if child.range[0] != 0 {
                    errors.push(SchemaError::at(
                        &[
                            path.as_slice(),
                            &[
                                SchemaPathElem::Field("kind"),
                                SchemaPathElem::Field("children"),
                                SchemaPathElem::Index(i),
                                SchemaPathElem::Field("range"),
                            ],
                        ]
                        .concat(),
                        "sequence child range.start must be 0",
                    ));
                }
            }
        }

        // Recurse.
        for (i, child) in children.iter().enumerate() {
            path.push(SchemaPathElem::Field("kind"));
            path.push(SchemaPathElem::Field("children"));
            path.push(SchemaPathElem::Index(i));
            collect_ids_and_validate_ranges(child, path, ids, all_node_ids, errors);
            path.pop();
            path.pop();
            path.pop();
        }
    }
}

fn validate_refs(
    node: &NodeDef,
    path: &mut Vec<SchemaPathElem>,
    id_set: &HashSet<&str>,
    def: &CompositionDef,
    errors: &mut Vec<SchemaError>,
) {
    // Transition ease names (static set in v0.3 boundary model).
    if let Some(t) = node.transition_in.as_ref() {
        validate_transition_spec(
            t,
            [path.as_slice(), &[SchemaPathElem::Field("transition_in")]].concat(),
            errors,
        );
    }
    if let Some(t) = node.transition_out.as_ref() {
        validate_transition_spec(
            t,
            [path.as_slice(), &[SchemaPathElem::Field("transition_out")]].concat(),
            errors,
        );
    }

    // Leaf asset refs.
    if let NodeKindDef::Leaf { asset } = &node.kind
        && !def.assets.contains_key(asset)
    {
        errors.push(SchemaError::at(
            &[
                path.as_slice(),
                &[
                    SchemaPathElem::Field("kind"),
                    SchemaPathElem::Field("asset"),
                ],
            ]
            .concat(),
            format!("unknown asset \"{asset}\""),
        ));
    }

    // Mask refs.
    if let Some(mask) = &node.mask {
        match &mask.source {
            MaskSourceDef::Node(id) => {
                if id == &node.id {
                    errors.push(SchemaError::at(
                        &[
                            path.as_slice(),
                            &[
                                SchemaPathElem::Field("mask"),
                                SchemaPathElem::Field("source"),
                            ],
                        ]
                        .concat(),
                        "mask source cannot refer to the same node id (self-mask)",
                    ));
                } else if !id_set.contains(id.as_str()) {
                    errors.push(SchemaError::at(
                        &[
                            path.as_slice(),
                            &[
                                SchemaPathElem::Field("mask"),
                                SchemaPathElem::Field("source"),
                            ],
                        ]
                        .concat(),
                        format!("unknown mask source node id \"{id}\""),
                    ));
                }
            }
            MaskSourceDef::Asset(key) => {
                if !def.assets.contains_key(key) {
                    errors.push(SchemaError::at(
                        &[
                            path.as_slice(),
                            &[
                                SchemaPathElem::Field("mask"),
                                SchemaPathElem::Field("source"),
                            ],
                        ]
                        .concat(),
                        format!("unknown mask source asset \"{key}\""),
                    ));
                }
            }
            MaskSourceDef::Shape(_shape) => {}
        }
    }

    // Transition kind basic sanity.
    if let Some(t) = &node.transition_in
        && t.kind.trim().is_empty()
    {
        errors.push(SchemaError::at(
            &[
                path.as_slice(),
                &[
                    SchemaPathElem::Field("transition_in"),
                    SchemaPathElem::Field("kind"),
                ],
            ]
            .concat(),
            "transition kind must be non-empty",
        ));
    }
    if let Some(t) = &node.transition_out
        && t.kind.trim().is_empty()
    {
        errors.push(SchemaError::at(
            &[
                path.as_slice(),
                &[
                    SchemaPathElem::Field("transition_out"),
                    SchemaPathElem::Field("kind"),
                ],
            ]
            .concat(),
            "transition kind must be non-empty",
        ));
    }

    // Recurse.
    if let NodeKindDef::Collection { children, .. } = &node.kind {
        for (i, child) in children.iter().enumerate() {
            path.push(SchemaPathElem::Field("kind"));
            path.push(SchemaPathElem::Field("children"));
            path.push(SchemaPathElem::Index(i));
            validate_refs(child, path, id_set, def, errors);
            path.pop();
            path.pop();
            path.pop();
        }
    }
}

fn validate_transition_spec(
    t: &TransitionSpecDef,
    base_path: Vec<SchemaPathElem>,
    errors: &mut Vec<SchemaError>,
) {
    if t.kind.trim().is_empty() {
        errors.push(SchemaError::at(
            &[base_path.as_slice(), &[SchemaPathElem::Field("kind")]].concat(),
            "transition kind must be non-empty",
        ));
    } else {
        let k = t.kind.trim().to_ascii_lowercase();
        let ok = matches!(k.as_str(), "crossfade" | "wipe" | "slide" | "zoom" | "iris");
        if !ok {
            errors.push(SchemaError::at(
                &[base_path.as_slice(), &[SchemaPathElem::Field("kind")]].concat(),
                format!("unknown transition kind \"{}\"", t.kind),
            ));
        }
    }

    if let Some(e) = t.ease.as_deref() {
        let ok = matches!(
            e,
            "hold"
                | "linear"
                | "ease_in"
                | "ease_out"
                | "ease_in_out"
                | "elastic_out"
                | "bounce_out"
        );
        if !ok {
            errors.push(SchemaError::at(
                &[base_path.as_slice(), &[SchemaPathElem::Field("ease")]].concat(),
                format!("unknown transition ease \"{e}\""),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::animation::anim::AnimDef;
    use crate::v03::scene::model::{
        AssetDef, CanvasDef, FpsDef, MaskDef, MaskModeDef, NodeKindDef,
    };
    use std::collections::BTreeMap;

    fn minimal_ok() -> CompositionDef {
        let mut assets = BTreeMap::new();
        assets.insert(
            "a".to_owned(),
            AssetDef::Image {
                source: "x.png".to_owned(),
            },
        );

        CompositionDef {
            version: V03_VERSION_STR.to_owned(),
            canvas: CanvasDef {
                width: 1920,
                height: 1080,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 60,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Leaf {
                    asset: "a".to_owned(),
                },
                range: [0, 60],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        }
    }

    #[test]
    fn ok_scene_validates() {
        validate_composition(&minimal_ok()).unwrap();
    }

    #[test]
    fn rejects_wrong_version() {
        let mut c = minimal_ok();
        c.version = "0.2".to_owned();
        let err = validate_composition(&c).unwrap_err();
        assert!(err.to_string().contains("version must be \"0.3\""));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut c = minimal_ok();
        c.root = NodeDef {
            id: "r".to_owned(),
            kind: NodeKindDef::Collection {
                mode: CollectionModeDef::Group,
                children: vec![
                    NodeDef {
                        id: "dup".to_owned(),
                        kind: NodeKindDef::Leaf {
                            asset: "a".to_owned(),
                        },
                        range: [0, 10],
                        transform: Default::default(),
                        opacity: AnimDef::Constant(1.0),
                        layout: None,
                        effects: vec![],
                        mask: None,
                        transition_in: None,
                        transition_out: None,
                    },
                    NodeDef {
                        id: "dup".to_owned(),
                        kind: NodeKindDef::Leaf {
                            asset: "a".to_owned(),
                        },
                        range: [0, 10],
                        transform: Default::default(),
                        opacity: AnimDef::Constant(1.0),
                        layout: None,
                        effects: vec![],
                        mask: None,
                        transition_in: None,
                        transition_out: None,
                    },
                ],
            },
            range: [0, 60],
            transform: Default::default(),
            opacity: AnimDef::Constant(1.0),
            layout: None,
            effects: vec![],
            mask: None,
            transition_in: None,
            transition_out: None,
        };
        c.duration = 60;
        let err = validate_composition(&c).unwrap_err();
        assert!(err.to_string().contains("duplicate node id"));
    }

    #[test]
    fn rejects_unknown_asset_ref() {
        let mut c = minimal_ok();
        c.root.kind = NodeKindDef::Leaf {
            asset: "missing".to_owned(),
        };
        let err = validate_composition(&c).unwrap_err();
        assert!(err.to_string().contains("unknown asset"));
    }

    #[test]
    fn rejects_sequence_child_start_nonzero() {
        let mut c = minimal_ok();
        c.root.kind = NodeKindDef::Collection {
            mode: CollectionModeDef::Sequence,
            children: vec![NodeDef {
                id: "c".to_owned(),
                kind: NodeKindDef::Leaf {
                    asset: "a".to_owned(),
                },
                range: [2, 10],
                transform: Default::default(),
                opacity: AnimDef::Constant(1.0),
                layout: None,
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            }],
        };
        let err = validate_composition(&c).unwrap_err();
        assert!(
            err.to_string()
                .contains("sequence child range.start must be 0")
        );
    }

    #[test]
    fn rejects_mask_node_ref_unknown() {
        let mut c = minimal_ok();
        c.root.mask = Some(MaskDef {
            source: MaskSourceDef::Node("nope".to_owned()),
            mode: MaskModeDef::Alpha,
            inverted: false,
        });
        let err = validate_composition(&c).unwrap_err();
        assert!(err.to_string().contains("unknown mask source node id"));
    }

    #[test]
    fn rejects_root_range_mismatch_duration() {
        let mut c = minimal_ok();
        c.duration = 61;
        c.root.range = [0, 60];
        let err = validate_composition(&c).unwrap_err();
        assert!(
            err.to_string()
                .contains("root.range must be [0, composition.duration]")
        );
    }
}
