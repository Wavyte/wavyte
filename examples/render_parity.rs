#[cfg(all(feature = "cpu", feature = "gpu"))]
use std::collections::BTreeMap;

#[cfg(all(feature = "cpu", feature = "gpu"))]
use wavyte::{
    Anim, Asset, BackendKind, BlendMode, Canvas, Clip, ClipProps, Composition, FrameIndex,
    FrameRange, PathAsset, RenderSettings, Track, Transform2D, create_backend, render_frame,
};

#[cfg(all(feature = "cpu", feature = "gpu"))]
fn rmse_u8(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len().min(b.len()).max(1) as f64;
    let mut sum = 0.0f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let d = (x as f64) - (y as f64);
        sum += d * d;
    }
    (sum / n).sqrt()
}

#[cfg(all(feature = "cpu", feature = "gpu"))]
struct NoAssets;
#[cfg(all(feature = "cpu", feature = "gpu"))]
impl wavyte::AssetCache for NoAssets {
    fn id_for(&mut self, _asset: &Asset) -> wavyte::WavyteResult<wavyte::AssetId> {
        Err(wavyte::WavyteError::evaluation("no assets in this example"))
    }
    fn get_or_load(&mut self, _asset: &Asset) -> wavyte::WavyteResult<wavyte::PreparedAsset> {
        Err(wavyte::WavyteError::evaluation("no assets in this example"))
    }
    fn get_or_load_by_id(
        &mut self,
        _id: wavyte::AssetId,
    ) -> wavyte::WavyteResult<wavyte::PreparedAsset> {
        Err(wavyte::WavyteError::evaluation("no assets in this example"))
    }
}

#[cfg(all(feature = "cpu", feature = "gpu"))]
fn build_comp() -> Composition {
    let mut assets = BTreeMap::new();
    assets.insert(
        "p0".to_string(),
        Asset::Path(PathAsset {
            svg_path_d: "M-100,-100 L1000,-100 L1000,1000 L-100,1000 Z".to_string(),
        }),
    );

    Composition {
        fps: wavyte::Fps::new(30, 1).unwrap(),
        canvas: Canvas {
            width: 128,
            height: 128,
        },
        duration: FrameIndex(1),
        assets,
        tracks: vec![Track {
            name: "main".to_string(),
            z_base: 0,
            clips: vec![Clip {
                id: "c0".to_string(),
                asset: "p0".to_string(),
                range: FrameRange::new(FrameIndex(0), FrameIndex(1)).unwrap(),
                props: ClipProps {
                    transform: Anim::constant(Transform2D::default()),
                    opacity: Anim::constant(1.0),
                    blend: BlendMode::Normal,
                },
                z_offset: 0,
                effects: vec![],
                transition_in: None,
                transition_out: None,
            }],
        }],
        seed: 1,
    }
}

#[cfg(not(all(feature = "cpu", feature = "gpu")))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("build with `--features gpu,cpu`")
}

#[cfg(all(feature = "cpu", feature = "gpu"))]
fn main() -> anyhow::Result<()> {
    let comp = build_comp();
    comp.validate()?;

    let settings = RenderSettings {
        clear_rgba: Some([0, 0, 0, 255]),
    };

    let mut assets = NoAssets;

    let mut cpu = create_backend(BackendKind::Cpu, &settings)?;
    let mut gpu = create_backend(BackendKind::Gpu, &settings)?;

    let a = render_frame(&comp, FrameIndex(0), cpu.as_mut(), &mut assets)?;
    let b = match render_frame(&comp, FrameIndex(0), gpu.as_mut(), &mut assets) {
        Ok(v) => v,
        Err(e) if e.to_string().contains("no gpu adapter available") => {
            eprintln!("skipping gpu parity (no adapter)");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let rmse = rmse_u8(&a.data, &b.data);
    println!("rmse={rmse:.4}");
    Ok(())
}
