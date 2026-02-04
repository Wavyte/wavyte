use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;

use crate::{
    assets_decode,
    error::{WavyteError, WavyteResult},
    model,
};

#[derive(Clone, Debug)]
pub struct PreparedImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8, row-major, tightly packed.
    pub rgba8_premul: Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct PreparedSvg {
    pub tree: Arc<usvg::Tree>,
}

#[derive(Clone, Debug)]
pub enum PreparedAsset {
    Image(PreparedImage),
    Svg(PreparedSvg),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetId(pub(crate) u64);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetKey {
    pub norm_path: String,
    pub params: Vec<(String, String)>,
}

impl AssetKey {
    pub fn new(norm_path: String, mut params: Vec<(String, String)>) -> Self {
        params.sort();
        Self { norm_path, params }
    }
}

pub trait AssetCache {
    fn id_for(&mut self, asset: &model::Asset) -> WavyteResult<AssetId>;
    fn get_or_load(&mut self, asset: &model::Asset) -> WavyteResult<PreparedAsset>;
}

pub struct FsAssetCache {
    root: PathBuf,
    keys_by_id: HashMap<AssetId, AssetKey>,
    prepared: HashMap<AssetId, PreparedAsset>,
    decode_counts: HashMap<AssetId, u32>,
}

impl FsAssetCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            keys_by_id: HashMap::new(),
            prepared: HashMap::new(),
            decode_counts: HashMap::new(),
        }
    }

    pub fn decode_count(&self, id: AssetId) -> u32 {
        self.decode_counts.get(&id).copied().unwrap_or(0)
    }

    pub fn key_for(&mut self, asset: &model::Asset) -> WavyteResult<(u8, AssetKey)> {
        match asset {
            model::Asset::Image(a) => Ok((
                b'I',
                AssetKey::new(self.normalize_source(&a.source)?, vec![]),
            )),
            model::Asset::Svg(a) => Ok((
                b'S',
                AssetKey::new(self.normalize_source(&a.source)?, vec![]),
            )),
            model::Asset::Video(_)
            | model::Asset::Audio(_)
            | model::Asset::Path(_)
            | model::Asset::Text(_) => Err(WavyteError::validation(
                "asset kind not yet supported by FsAssetCache in phase 3",
            )),
        }
    }

    pub fn normalize_source(&self, source: &str) -> WavyteResult<String> {
        normalize_rel_path(source)
    }

    fn id_for_key(kind_tag: u8, key: &AssetKey) -> AssetId {
        let mut hasher = Fnv1a64::new();
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

impl AssetCache for FsAssetCache {
    fn id_for(&mut self, asset: &model::Asset) -> WavyteResult<AssetId> {
        let (kind, key) = self.key_for(asset)?;
        let id = Self::id_for_key(kind, &key);
        self.keys_by_id.entry(id).or_insert(key);
        Ok(id)
    }

    fn get_or_load(&mut self, asset: &model::Asset) -> WavyteResult<PreparedAsset> {
        let (kind, key) = self.key_for(asset)?;
        let id = Self::id_for_key(kind, &key);
        self.keys_by_id.entry(id).or_insert_with(|| key.clone());

        if let Some(p) = self.prepared.get(&id) {
            return Ok(p.clone());
        }

        let prepared = match asset {
            model::Asset::Image(_) => {
                let bytes = self.read_bytes(&key.norm_path)?;
                PreparedAsset::Image(assets_decode::decode_image(&bytes)?)
            }
            model::Asset::Svg(_) => {
                let bytes = self.read_bytes(&key.norm_path)?;
                PreparedAsset::Svg(assets_decode::parse_svg(&bytes)?)
            }
            model::Asset::Video(_)
            | model::Asset::Audio(_)
            | model::Asset::Path(_)
            | model::Asset::Text(_) => {
                return Err(WavyteError::validation(
                    "asset kind not yet supported by FsAssetCache in phase 3",
                ));
            }
        };

        *self.decode_counts.entry(id).or_insert(0) += 1;
        self.prepared.insert(id, prepared.clone());
        Ok(prepared)
    }
}

fn normalize_rel_path(source: &str) -> WavyteResult<String> {
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

struct Fnv1a64(u64);

impl Fnv1a64 {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn write_u8(&mut self, b: u8) {
        self.write_bytes(&[b]);
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        self.0 = h;
    }

    fn finish(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn normalize_path_slash_normalization() {
        assert_eq!(normalize_rel_path("a/b.png").unwrap(), "a/b.png");
        assert_eq!(normalize_rel_path("a\\b.png").unwrap(), "a/b.png");
        assert!(normalize_rel_path("../x.png").is_err());
        assert!(normalize_rel_path("/abs.png").is_err());
    }

    #[test]
    fn asset_id_stability_same_input() {
        let key = AssetKey::new(
            "assets/img.png".to_string(),
            vec![
                ("dpi".to_string(), "96".to_string()),
                ("colorspace".to_string(), "srgb".to_string()),
            ],
        );

        let a = FsAssetCache::id_for_key(b'I', &key);
        let b = FsAssetCache::id_for_key(b'I', &key);
        assert_eq!(a, b);
        assert_eq!(a.0, 0xa23b14b8777d9f73);
    }

    #[test]
    fn cache_load_same_asset_only_decodes_once() {
        let tmp = std::env::temp_dir().join(format!(
            "wavyte_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        let png_path = tmp.join("img.png");
        let img = image::RgbaImage::from_raw(1, 1, vec![1u8, 2u8, 3u8, 255u8]).unwrap();
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&png_path, &buf).unwrap();

        let mut cache = FsAssetCache::new(&tmp);
        let asset = model::Asset::Image(model::ImageAsset {
            source: "img.png".to_string(),
        });
        let id = cache.id_for(&asset).unwrap();
        cache.get_or_load(&asset).unwrap();
        cache.get_or_load(&asset).unwrap();
        assert_eq!(cache.decode_count(id), 1);

        std::fs::remove_dir_all(&tmp).ok();
    }
}
