use crate::{
    composition::model::TransitionSpec,
    foundation::error::{WavyteError, WavyteResult},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Wipe direction used by [`TransitionKind::Wipe`].
pub enum WipeDir {
    /// Reveal new clip from left edge toward right.
    LeftToRight,
    /// Reveal new clip from right edge toward left.
    RightToLeft,
    /// Reveal new clip from top edge toward bottom.
    TopToBottom,
    /// Reveal new clip from bottom edge toward top.
    BottomToTop,
}

#[derive(Clone, Debug, PartialEq)]
/// Parsed transition kind used during compile/composite.
pub enum TransitionKind {
    /// Linear blend between outgoing/incoming surfaces.
    Crossfade,
    /// Directional wipe with optional softened boundary.
    Wipe {
        /// Wipe travel direction.
        dir: WipeDir,
        /// Edge softness in `[0, 1]`.
        soft_edge: f32,
    },
}

/// Parse transition kind and params into a typed representation.
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

/// Parse a full [`TransitionSpec`] object.
pub fn parse_transition(spec: &TransitionSpec) -> WavyteResult<TransitionKind> {
    parse_transition_kind_params(&spec.kind, &spec.params)
}

#[cfg(test)]
#[path = "../../tests/unit/effects/transitions.rs"]
mod tests;
