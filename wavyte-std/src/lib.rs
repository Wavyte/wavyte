//! `wavyte-std` provides higher-level helpers on top of the `wavyte` v0.3 JSON-first API.
//!
//! The goal is to keep `wavyte` focused on a production-grade runtime and schema, while `wavyte-std`
//! layers conveniences like presets and small JSON builders.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// v0.3 helpers.
pub mod v03 {
    use serde_json::json;
    use std::io::Cursor;

    /// Build a minimal v0.3 composition (solid rectangle) as a `serde_json::Value`.
    ///
    /// - `color` must be a v0.3-compatible color string (e.g. `#rrggbb` or `#rrggbbaa`).
    pub fn minimal_solid_value(
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        duration_frames: u64,
        color: &str,
    ) -> serde_json::Value {
        json!({
            "version": "0.3",
            "canvas": { "width": width, "height": height },
            "fps": { "num": fps_num, "den": fps_den },
            "duration": duration_frames,
            "assets": {
                "solid": { "solid_rect": { "color": color } }
            },
            "root": {
                "id": "root",
                "kind": { "leaf": { "asset": "solid" } },
                "range": [0, duration_frames]
            }
        })
    }

    /// Build and parse a minimal v0.3 solid-rect composition into a `wavyte::Composition`.
    pub fn minimal_solid_composition(
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        duration_frames: u64,
        color: &str,
    ) -> wavyte::WavyteResult<wavyte::Composition> {
        let v = minimal_solid_value(width, height, fps_num, fps_den, duration_frames, color);
        let bytes = serde_json::to_vec(&v)
            .map_err(|e| wavyte::WavyteError::validation(format!("json serialize failed: {e}")))?;
        wavyte::Composition::from_reader(Cursor::new(bytes))
    }
}
