use crate::{
    assets::store::{PreparedAsset, PreparedAssetStore},
    composition::model::{Composition, LayoutAlignX, LayoutAlignY, LayoutMode, Track},
    foundation::core::Vec2,
    foundation::error::WavyteResult,
};

#[derive(Clone, Debug, Default)]
pub struct LayoutOffsets {
    per_track: Vec<Vec<Vec2>>,
}

impl LayoutOffsets {
    pub fn offset_for(&self, track_idx: usize, clip_idx: usize) -> Vec2 {
        self.per_track
            .get(track_idx)
            .and_then(|v| v.get(clip_idx))
            .copied()
            .unwrap_or_else(|| Vec2::new(0.0, 0.0))
    }
}

pub fn resolve_layout_offsets(
    comp: &Composition,
    assets: &PreparedAssetStore,
) -> WavyteResult<LayoutOffsets> {
    let mut per_track = Vec::<Vec<Vec2>>::with_capacity(comp.tracks.len());
    for track in &comp.tracks {
        per_track.push(resolve_track_offsets(comp, track, assets)?);
    }
    Ok(LayoutOffsets { per_track })
}

fn resolve_track_offsets(
    comp: &Composition,
    track: &Track,
    assets: &PreparedAssetStore,
) -> WavyteResult<Vec<Vec2>> {
    let mut sizes = Vec::<(f64, f64)>::with_capacity(track.clips.len());
    for clip in &track.clips {
        sizes.push(intrinsic_size_for_asset_key(&clip.asset, assets)?);
    }

    let mut offsets = vec![Vec2::new(0.0, 0.0); track.clips.len()];
    if track.layout_mode == LayoutMode::Absolute || track.clips.is_empty() {
        return Ok(offsets);
    }

    let x0 = track.layout_padding.left;
    let y0 = track.layout_padding.top;
    let avail_w =
        (comp.canvas.width as f64 - track.layout_padding.left - track.layout_padding.right)
            .max(0.0);
    let avail_h =
        (comp.canvas.height as f64 - track.layout_padding.top - track.layout_padding.bottom)
            .max(0.0);

    match track.layout_mode {
        LayoutMode::Absolute => {}
        LayoutMode::Center => {
            for (idx, &(w, h)) in sizes.iter().enumerate() {
                offsets[idx] = Vec2::new(
                    x0 + align_offset(avail_w, w, LayoutAlignX::Center),
                    y0 + align_offset(avail_h, h, LayoutAlignY::Center),
                );
            }
        }
        LayoutMode::HStack => {
            let total_w = sizes.iter().map(|(w, _)| *w).sum::<f64>()
                + (track.clips.len().saturating_sub(1) as f64) * track.layout_gap_px;
            let mut x = x0 + align_offset(avail_w, total_w, track.layout_align_x);
            for (idx, &(w, h)) in sizes.iter().enumerate() {
                let y = y0 + align_offset(avail_h, h, track.layout_align_y);
                offsets[idx] = Vec2::new(x, y);
                x += w + track.layout_gap_px;
            }
        }
        LayoutMode::VStack => {
            let total_h = sizes.iter().map(|(_, h)| *h).sum::<f64>()
                + (track.clips.len().saturating_sub(1) as f64) * track.layout_gap_px;
            let mut y = y0 + align_offset(avail_h, total_h, track.layout_align_y);
            for (idx, &(w, h)) in sizes.iter().enumerate() {
                let x = x0 + align_offset(avail_w, w, track.layout_align_x);
                offsets[idx] = Vec2::new(x, y);
                y += h + track.layout_gap_px;
            }
        }
        LayoutMode::Grid => {
            let cols = usize::try_from(track.layout_grid_columns.max(1)).unwrap_or(1);
            let cell_w = sizes.iter().map(|(w, _)| *w).fold(0.0, f64::max);
            let cell_h = sizes.iter().map(|(_, h)| *h).fold(0.0, f64::max);
            for (idx, &(w, h)) in sizes.iter().enumerate() {
                let row = idx / cols;
                let col = idx % cols;
                let base_x = x0 + (col as f64) * (cell_w + track.layout_gap_px);
                let base_y = y0 + (row as f64) * (cell_h + track.layout_gap_px);
                offsets[idx] = Vec2::new(
                    base_x + align_offset(cell_w, w, track.layout_align_x),
                    base_y + align_offset(cell_h, h, track.layout_align_y),
                );
            }
        }
    }
    Ok(offsets)
}

fn intrinsic_size_for_asset_key(
    key: &str,
    assets: &PreparedAssetStore,
) -> WavyteResult<(f64, f64)> {
    let id = assets.id_for_key(key)?;
    let prepared = assets.get(id)?;
    match prepared {
        PreparedAsset::Image(i) => Ok((f64::from(i.width), f64::from(i.height))),
        PreparedAsset::Svg(s) => Ok((
            f64::from(s.tree.size().width()),
            f64::from(s.tree.size().height()),
        )),
        PreparedAsset::Text(t) => {
            let mut w = 0.0f64;
            let mut h = 0.0f64;
            for line in t.layout.lines() {
                let m = line.metrics();
                w = w.max(f64::from(m.advance));
                h += f64::from(m.ascent + m.descent + m.leading);
            }
            Ok((w.max(1.0), h.max(1.0)))
        }
        PreparedAsset::Path(p) => {
            use kurbo::Shape;
            let bbox = p.path.bounding_box();
            Ok((bbox.width().max(1.0), bbox.height().max(1.0)))
        }
        PreparedAsset::Video(v) => Ok((f64::from(v.info.width), f64::from(v.info.height))),
        PreparedAsset::Audio(_) => Ok((0.0, 0.0)),
    }
}

fn align_offset<W: Into<f64>, C: Into<f64>, A>(container: W, content: C, align: A) -> f64
where
    A: Into<AlignKind>,
{
    let container = container.into();
    let content = content.into();
    let rem = (container - content).max(0.0);
    match align.into() {
        AlignKind::Start => 0.0,
        AlignKind::Center => rem * 0.5,
        AlignKind::End => rem,
    }
}

enum AlignKind {
    Start,
    Center,
    End,
}

impl From<LayoutAlignX> for AlignKind {
    fn from(value: LayoutAlignX) -> Self {
        match value {
            LayoutAlignX::Start => AlignKind::Start,
            LayoutAlignX::Center => AlignKind::Center,
            LayoutAlignX::End => AlignKind::End,
        }
    }
}

impl From<LayoutAlignY> for AlignKind {
    fn from(value: LayoutAlignY) -> Self {
        match value {
            LayoutAlignY::Start => AlignKind::Start,
            LayoutAlignY::Center => AlignKind::Center,
            LayoutAlignY::End => AlignKind::End,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/layout/solver.rs"]
mod tests;
