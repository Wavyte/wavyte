use crate::{
    composition::model::TransitionSpec,
    foundation::error::{WavyteError, WavyteResult},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WipeDir {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransitionKind {
    Crossfade,
    Wipe { dir: WipeDir, soft_edge: f32 },
}

pub fn parse_transition_kind_params(
    kind: &str,
    params: &serde_json::Value,
) -> WavyteResult<TransitionKind> {
    let kind = kind.trim().to_ascii_lowercase();
    if kind.is_empty() {
        return Err(WavyteError::validation("transition kind must be non-empty"));
    }

    match kind.as_str() {
        "crossfade" => Ok(TransitionKind::Crossfade),
        "wipe" => {
            let params = if params.is_null() {
                None
            } else {
                Some(
                    params
                        .as_object()
                        .ok_or_else(|| WavyteError::validation("wipe params must be an object"))?,
                )
            };

            let dir = match params.and_then(|p| p.get("dir")).and_then(|v| v.as_str()) {
                None => WipeDir::LeftToRight,
                Some(s) => match s.trim().to_ascii_lowercase().as_str() {
                    "left_to_right" | "lefttoright" | "ltr" => WipeDir::LeftToRight,
                    "right_to_left" | "righttoleft" | "rtl" => WipeDir::RightToLeft,
                    "top_to_bottom" | "toptobottom" | "ttb" => WipeDir::TopToBottom,
                    "bottom_to_top" | "bottomtotop" | "btt" => WipeDir::BottomToTop,
                    other => {
                        return Err(WavyteError::validation(format!(
                            "unknown wipe.dir '{other}'"
                        )));
                    }
                },
            };

            let soft_edge = match params
                .and_then(|p| p.get("soft_edge"))
                .and_then(|v| v.as_f64())
            {
                None => 0.0,
                Some(v) => {
                    let f = v as f32;
                    if !f.is_finite() {
                        return Err(WavyteError::validation(
                            "wipe.soft_edge must be finite when set",
                        ));
                    }
                    f.clamp(0.0, 1.0)
                }
            };

            Ok(TransitionKind::Wipe { dir, soft_edge })
        }
        _ => Err(WavyteError::validation(format!(
            "unknown transition kind '{kind}'"
        ))),
    }
}

pub fn parse_transition(spec: &TransitionSpec) -> WavyteResult<TransitionKind> {
    parse_transition_kind_params(&spec.kind, &spec.params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::ease::Ease;

    #[test]
    fn wipe_dir_parses_aliases() {
        let spec = TransitionSpec {
            kind: "wipe".to_string(),
            duration_frames: 10,
            ease: Ease::Linear,
            params: serde_json::json!({ "dir": "ttb", "soft_edge": 0.1 }),
        };
        assert_eq!(
            parse_transition(&spec).unwrap(),
            TransitionKind::Wipe {
                dir: WipeDir::TopToBottom,
                soft_edge: 0.1
            }
        );
    }

    #[test]
    fn wipe_soft_edge_is_clamped() {
        let spec = TransitionSpec {
            kind: "wipe".to_string(),
            duration_frames: 10,
            ease: Ease::Linear,
            params: serde_json::json!({ "soft_edge": -5.0 }),
        };
        assert_eq!(
            parse_transition(&spec).unwrap(),
            TransitionKind::Wipe {
                dir: WipeDir::LeftToRight,
                soft_edge: 0.0
            }
        );
    }
}
