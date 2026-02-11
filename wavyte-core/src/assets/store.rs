use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;

use crate::{
    assets::decode as assets_decode,
    assets::media,
    composition::model,
    foundation::core::BezPath,
    foundation::error::{WavyteError, WavyteResult},
    foundation::math::Fnv1a64,
};

#[derive(Clone, Debug)]
/// Prepared raster image in premultiplied RGBA8 form.
pub struct PreparedImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel bytes in row-major premultiplied RGBA8.
    pub rgba8_premul: Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
/// Prepared SVG asset represented as a parsed `usvg` tree.
pub struct PreparedSvg {
    /// Parsed SVG tree.
    pub tree: Arc<usvg::Tree>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// RGBA8 brush color used by Parley text layout.
pub struct TextBrushRgba8 {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

#[derive(Clone)]
/// Prepared text asset: shaped layout plus backing font data.
pub struct PreparedText {
    /// Fully built text layout ready for rendering.
    pub layout: Arc<parley::Layout<TextBrushRgba8>>,
    /// Original font bytes used to build glyph outlines.
    pub font_bytes: Arc<Vec<u8>>,
    /// Primary detected family name from font data.
    pub font_family: String,
}

impl std::fmt::Debug for PreparedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedText")
            .field("layout_ptr", &Arc::as_ptr(&self.layout))
            .field("font_bytes_len", &self.font_bytes.len())
            .field("font_family", &self.font_family)
            .finish()
    }
}

#[derive(Clone, Debug)]
/// Prepared vector path asset parsed from SVG path data.
pub struct PreparedPath {
    /// Parsed Bezier path.
    pub path: BezPath,
}

#[derive(Clone, Debug)]
/// Prepared audio clip stored as interleaved `f32` PCM.
pub struct PreparedAudio {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Interleaved PCM samples.
    pub interleaved_f32: Arc<Vec<f32>>,
}

#[derive(Clone, Debug)]
/// Prepared video asset metadata and optional decoded audio track.
pub struct PreparedVideo {
    /// Probed source metadata.
    pub info: Arc<media::VideoSourceInfo>,
    /// Predecoded audio stream if present.
    pub audio: Option<PreparedAudio>,
}

#[derive(Clone, Debug)]
/// Union of all prepared asset kinds consumed by evaluator/compiler/renderers.
pub enum PreparedAsset {
    /// Prepared bitmap image.
    Image(PreparedImage),
    /// Prepared SVG vector tree.
    Svg(PreparedSvg),
    /// Prepared text layout.
    Text(PreparedText),
    /// Prepared path geometry.
    Path(PreparedPath),
    /// Prepared video metadata/audio.
    Video(PreparedVideo),
    /// Prepared audio PCM.
    Audio(PreparedAudio),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// Stable hashed identifier used for prepared assets.
pub struct AssetId(pub(crate) u64);

impl AssetId {
    /// Construct an [`AssetId`] from raw 64-bit value.
    pub fn from_u64(raw: u64) -> Self {
        Self(raw)
    }

    /// Access raw 64-bit identifier.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// Normalized identity key used to derive deterministic [`AssetId`] values.
pub struct AssetKey {
    /// Normalized relative path or inline identifier.
    pub norm_path: String,
    /// Canonicalized parameter key/value pairs.
    pub params: Vec<(String, String)>,
}

impl AssetKey {
    /// Build key with lexicographically sorted `params`.
    pub fn new(norm_path: String, mut params: Vec<(String, String)>) -> Self {
        params.sort();
        Self { norm_path, params }
    }
}

#[derive(Clone, Debug)]
/// Immutable store of prepared assets keyed by composition asset keys and hashed IDs.
pub struct PreparedAssetStore {
    root: PathBuf,
    ids_by_key: HashMap<String, AssetId>,
    assets_by_id: HashMap<AssetId, PreparedAsset>,
}

impl PreparedAssetStore {
    /// Prepare all assets referenced by `comp` using filesystem root `root`.
    ///
    /// This front-loads IO/decoding so render stages can remain deterministic and IO-free.
    pub fn prepare(comp: &model::Composition, root: impl Into<PathBuf>) -> WavyteResult<Self> {
        let root = root.into();
        let mut out = Self {
            root,
            ids_by_key: HashMap::new(),
            assets_by_id: HashMap::new(),
        };

        let mut text_engine = TextLayoutEngine::new();
        for (asset_key, asset) in &comp.assets {
            let (kind, key) = out.key_for(asset)?;
            let id = Self::hash_id_for_key(kind, &key);

            let prepared = match asset {
                model::Asset::Image(_) => {
                    let bytes = out.read_bytes(&key.norm_path)?;
                    PreparedAsset::Image(assets_decode::decode_image(&bytes)?)
                }
                model::Asset::Svg(_) => {
                    let bytes = out.read_bytes(&key.norm_path)?;
                    PreparedAsset::Svg(parse_svg_with_options(&out.root, &key.norm_path, &bytes)?)
                }
                model::Asset::Text(a) => {
                    let font_bytes = out.read_bytes(&key.norm_path)?;
                    let brush = TextBrushRgba8 {
                        r: a.color_rgba8[0],
                        g: a.color_rgba8[1],
                        b: a.color_rgba8[2],
                        a: a.color_rgba8[3],
                    };
                    let layout = text_engine.layout_plain(
                        &a.text,
                        font_bytes.as_slice(),
                        a.size_px,
                        brush,
                        a.max_width_px,
                    )?;
                    let family = text_engine
                        .last_family_name()
                        .unwrap_or_else(|| "unknown".to_string());
                    PreparedAsset::Text(PreparedText {
                        layout: Arc::new(layout),
                        font_bytes: Arc::new(font_bytes),
                        font_family: family,
                    })
                }
                model::Asset::Path(a) => PreparedAsset::Path(PreparedPath {
                    path: parse_svg_path(&a.svg_path_d)?,
                }),
                model::Asset::Video(a) => {
                    let source_path = out.root.join(Path::new(&key.norm_path));
                    let info = media::probe_video(&source_path)?;
                    let audio = if info.has_audio {
                        let pcm =
                            media::decode_audio_f32_stereo(&source_path, media::MIX_SAMPLE_RATE)?;
                        if pcm.interleaved_f32.is_empty() {
                            None
                        } else {
                            Some(PreparedAudio {
                                sample_rate: pcm.sample_rate,
                                channels: pcm.channels,
                                interleaved_f32: Arc::new(pcm.interleaved_f32),
                            })
                        }
                    } else {
                        None
                    };
                    let _ = a;
                    PreparedAsset::Video(PreparedVideo {
                        info: Arc::new(info),
                        audio,
                    })
                }
                model::Asset::Audio(_) => {
                    let source_path = out.root.join(Path::new(&key.norm_path));
                    let pcm = media::decode_audio_f32_stereo(&source_path, media::MIX_SAMPLE_RATE)?;
                    PreparedAsset::Audio(PreparedAudio {
                        sample_rate: pcm.sample_rate,
                        channels: pcm.channels,
                        interleaved_f32: Arc::new(pcm.interleaved_f32),
                    })
                }
            };

            out.ids_by_key.insert(asset_key.clone(), id);
            out.assets_by_id.insert(id, prepared);
        }

        Ok(out)
    }

    /// Return root directory used when resolving relative asset paths.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lookup prepared [`AssetId`] for a composition asset key.
    pub fn id_for_key(&self, key: &str) -> WavyteResult<AssetId> {
        self.ids_by_key
            .get(key)
            .copied()
            .ok_or_else(|| WavyteError::evaluation(format!("unknown asset key '{key}'")))
    }

    /// Lookup prepared asset data by [`AssetId`].
    pub fn get(&self, id: AssetId) -> WavyteResult<&PreparedAsset> {
        self.assets_by_id
            .get(&id)
            .ok_or_else(|| WavyteError::evaluation(format!("unknown AssetId {}", id.as_u64())))
    }

    fn key_for(&self, asset: &model::Asset) -> WavyteResult<(u8, AssetKey)> {
        match asset {
            model::Asset::Image(a) => {
                Ok((b'I', AssetKey::new(normalize_rel_path(&a.source)?, vec![])))
            }
            model::Asset::Svg(a) => {
                Ok((b'S', AssetKey::new(normalize_rel_path(&a.source)?, vec![])))
            }
            model::Asset::Text(a) => {
                let norm_path = normalize_rel_path(&a.font_source)?;
                let mut params = vec![
                    ("text".to_string(), a.text.clone()),
                    (
                        "size_px_bits".to_string(),
                        format!("0x{:08x}", a.size_px.to_bits()),
                    ),
                    (
                        "color_rgba8".to_string(),
                        format!(
                            "#{:02x}{:02x}{:02x}{:02x}",
                            a.color_rgba8[0], a.color_rgba8[1], a.color_rgba8[2], a.color_rgba8[3]
                        ),
                    ),
                ];
                if let Some(w) = a.max_width_px {
                    params.push((
                        "max_width_px_bits".to_string(),
                        format!("0x{:08x}", w.to_bits()),
                    ));
                }
                Ok((b'T', AssetKey::new(norm_path, params)))
            }
            model::Asset::Path(a) => Ok((
                b'P',
                AssetKey::new(
                    "inline:path".to_string(),
                    vec![("svg_path_d".to_string(), a.svg_path_d.clone())],
                ),
            )),
            model::Asset::Video(a) => {
                Ok((b'V', AssetKey::new(normalize_rel_path(&a.source)?, vec![])))
            }
            model::Asset::Audio(a) => {
                Ok((b'A', AssetKey::new(normalize_rel_path(&a.source)?, vec![])))
            }
        }
    }

    fn hash_id_for_key(kind_tag: u8, key: &AssetKey) -> AssetId {
        let mut hasher = Fnv1a64::new_default();
        hasher.write_u8(kind_tag);
        hasher.write_bytes(key.norm_path.as_bytes());
        hasher.write_u8(0);
        for (k, v) in &key.params {
            hasher.write_bytes(k.as_bytes());
            hasher.write_u8(0);
            hasher.write_bytes(v.as_bytes());
            hasher.write_u8(0);
        }
        AssetId(hasher.finish())
    }

    fn read_bytes(&self, norm_path: &str) -> WavyteResult<Vec<u8>> {
        let path = self.root.join(Path::new(norm_path));
        std::fs::read(&path)
            .with_context(|| format!("read asset bytes from '{}'", path.display()))
            .map_err(WavyteError::from)
    }
}

fn parse_svg_with_options(root: &Path, norm_path: &str, bytes: &[u8]) -> WavyteResult<PreparedSvg> {
    let abs = root.join(Path::new(norm_path));
    let resources_dir = abs.parent().map(|p| p.to_path_buf());

    let fontdb = build_svg_fontdb(root, resources_dir.as_deref());
    let font_resolver = make_svg_font_resolver();
    let opts = usvg::Options {
        resources_dir,
        fontdb,
        font_resolver,
        ..Default::default()
    };

    let tree = usvg::Tree::from_data(bytes, &opts).with_context(|| "parse svg tree")?;
    Ok(PreparedSvg {
        tree: Arc::new(tree),
    })
}

fn build_svg_fontdb(
    root: &Path,
    resources_dir: Option<&Path>,
) -> std::sync::Arc<usvg::fontdb::Database> {
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();

    load_fonts_from_dir(&mut db, &root.join("fonts"));
    load_fonts_from_dir(&mut db, &root.join("assets"));

    if let Some(dir) = resources_dir {
        load_fonts_from_dir(&mut db, dir);
        load_fonts_from_dir(&mut db, &dir.join("fonts"));
    }

    std::sync::Arc::new(db)
}

fn load_fonts_from_dir(db: &mut usvg::fontdb::Database, dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        if ext != "ttf" && ext != "otf" && ext != "ttc" {
            continue;
        }
        let _ = db.load_font_file(&path);
    }
}

fn make_svg_font_resolver() -> usvg::FontResolver<'static> {
    use usvg::FontResolver;

    FontResolver {
        select_font: Box::new(|font, fontdb| {
            let mut families = Vec::<usvg::fontdb::Family<'_>>::new();
            for family in font.families() {
                families.push(match family {
                    usvg::FontFamily::Serif => usvg::fontdb::Family::Serif,
                    usvg::FontFamily::SansSerif => usvg::fontdb::Family::SansSerif,
                    usvg::FontFamily::Cursive => usvg::fontdb::Family::Cursive,
                    usvg::FontFamily::Fantasy => usvg::fontdb::Family::Fantasy,
                    usvg::FontFamily::Monospace => usvg::fontdb::Family::Monospace,
                    usvg::FontFamily::Named(s) => usvg::fontdb::Family::Name(s),
                });
            }

            families.push(usvg::fontdb::Family::SansSerif);
            families.push(usvg::fontdb::Family::Serif);
            families.push(usvg::fontdb::Family::Monospace);

            let stretch = match font.stretch() {
                usvg::FontStretch::UltraCondensed => usvg::fontdb::Stretch::UltraCondensed,
                usvg::FontStretch::ExtraCondensed => usvg::fontdb::Stretch::ExtraCondensed,
                usvg::FontStretch::Condensed => usvg::fontdb::Stretch::Condensed,
                usvg::FontStretch::SemiCondensed => usvg::fontdb::Stretch::SemiCondensed,
                usvg::FontStretch::Normal => usvg::fontdb::Stretch::Normal,
                usvg::FontStretch::SemiExpanded => usvg::fontdb::Stretch::SemiExpanded,
                usvg::FontStretch::Expanded => usvg::fontdb::Stretch::Expanded,
                usvg::FontStretch::ExtraExpanded => usvg::fontdb::Stretch::ExtraExpanded,
                usvg::FontStretch::UltraExpanded => usvg::fontdb::Stretch::UltraExpanded,
            };

            let style = match font.style() {
                usvg::FontStyle::Normal => usvg::fontdb::Style::Normal,
                usvg::FontStyle::Italic => usvg::fontdb::Style::Italic,
                usvg::FontStyle::Oblique => usvg::fontdb::Style::Oblique,
            };

            let query = usvg::fontdb::Query {
                families: &families,
                weight: usvg::fontdb::Weight(font.weight()),
                stretch,
                style,
            };

            if let Some(id) = fontdb.query(&query) {
                return Some(id);
            }
            fontdb.faces().next().map(|f| f.id)
        }),
        select_fallback: FontResolver::default_fallback_selector(),
    }
}

/// Normalize and validate composition-relative asset paths.
///
/// The normalized result uses `/` separators, removes `.` segments, and rejects absolute paths or
/// parent traversals (`..`).
pub fn normalize_rel_path(source: &str) -> WavyteResult<String> {
    let s = source.replace('\\', "/");
    if s.starts_with('/') {
        return Err(WavyteError::validation("asset paths must be relative"));
    }
    if s.is_empty() {
        return Err(WavyteError::validation("asset path must be non-empty"));
    }

    let mut out = Vec::<&str>::new();
    for part in s.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(WavyteError::validation("asset paths must not contain '..'"));
        }
        out.push(part);
    }

    if out.is_empty() {
        return Err(WavyteError::validation(
            "asset path must contain a file name",
        ));
    }

    Ok(out.join("/"))
}

fn parse_svg_path(d: &str) -> WavyteResult<BezPath> {
    let d = d.trim();
    if d.is_empty() {
        return Err(WavyteError::validation(
            "path asset svg_path_d must be non-empty",
        ));
    }

    BezPath::from_svg(d).map_err(|e| WavyteError::validation(format!("invalid svg_path_d: {e}")))
}

/// Stateful helper for building Parley text layouts from raw font bytes.
pub struct TextLayoutEngine {
    font_ctx: parley::FontContext,
    layout_ctx: parley::LayoutContext<TextBrushRgba8>,
    last_family_name: Option<String>,
}

impl Default for TextLayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextLayoutEngine {
    /// Construct a new layout engine with fresh Parley contexts.
    pub fn new() -> Self {
        Self {
            font_ctx: parley::FontContext::default(),
            layout_ctx: parley::LayoutContext::new(),
            last_family_name: None,
        }
    }

    /// Return last successfully resolved family name, if any.
    pub fn last_family_name(&self) -> Option<String> {
        self.last_family_name.clone()
    }

    /// Shape and lay out plain text using provided font bytes and styling.
    pub fn layout_plain(
        &mut self,
        text: &str,
        font_bytes: &[u8],
        size_px: f32,
        brush: TextBrushRgba8,
        max_width_px: Option<f32>,
    ) -> WavyteResult<parley::Layout<TextBrushRgba8>> {
        if !size_px.is_finite() || size_px <= 0.0 {
            return Err(WavyteError::validation(
                "text size_px must be finite and > 0",
            ));
        }

        let families = self
            .font_ctx
            .collection
            .register_fonts(parley::fontique::Blob::from(font_bytes.to_vec()), None);
        let family_id = families.first().map(|(id, _)| *id).ok_or_else(|| {
            WavyteError::validation("no font families registered from font bytes")
        })?;

        let family_name = self
            .font_ctx
            .collection
            .family_name(family_id)
            .ok_or_else(|| WavyteError::validation("registered font family has no name"))?
            .to_string();
        self.last_family_name = Some(family_name.clone());

        let mut builder = self
            .layout_ctx
            .ranged_builder(&mut self.font_ctx, text, 1.0, true);
        builder.push_default(parley::style::StyleProperty::FontStack(
            parley::style::FontStack::Source(std::borrow::Cow::Owned(family_name)),
        ));
        builder.push_default(parley::style::StyleProperty::FontSize(size_px));
        builder.push_default(parley::style::StyleProperty::Brush(brush));

        let mut layout: parley::Layout<TextBrushRgba8> = builder.build(text);
        if let Some(w) = max_width_px {
            layout.break_all_lines(Some(w));
            layout.align(
                Some(w),
                parley::Alignment::Start,
                parley::AlignmentOptions::default(),
            );
        } else {
            layout.break_all_lines(None);
        }

        Ok(layout)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/assets/store.rs"]
mod tests;
