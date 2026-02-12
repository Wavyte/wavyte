use std::fs;
use std::io::Cursor;
use std::path::Path;

use wavyte::{Composition, CpuBackendOpts, FrameIndex, RenderSession, RenderSessionOpts};

fn unpremultiply_in_place(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u16;
        if a == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            continue;
        }
        px[0] = ((px[0] as u16 * 255 + a / 2) / a).min(255) as u8;
        px[1] = ((px[1] as u16 * 255 + a / 2) / a).min(255) as u8;
        px[2] = ((px[2] as u16 * 255 + a / 2) / a).min(255) as u8;
    }
}

fn main() -> anyhow::Result<()> {
    // Minimal v0.3 composition with a solid rectangle.
    let json = r##"
{
  "version": "0.3",
  "canvas": { "width": 256, "height": 256 },
  "fps": { "num": 30, "den": 1 },
  "duration": 30,
  "assets": {
    "solid": { "solid_rect": { "color": "#ff3366" } }
  },
  "root": {
    "id": "root",
    "kind": { "leaf": { "asset": "solid" } },
    "range": [0, 30]
  }
}
"##;

    let comp = Composition::from_reader(Cursor::new(json))?;
    let mut session = RenderSession::new(&comp, ".", RenderSessionOpts::default())?;
    let frame = session.render_frame(FrameIndex(0), CpuBackendOpts::default())?;

    let out_dir = Path::new("target/v03_examples");
    fs::create_dir_all(out_dir)?;
    let out_path = out_dir.join("frame0.png");

    let mut straight = frame.data;
    unpremultiply_in_place(&mut straight);
    let img = image::RgbaImage::from_raw(frame.width, frame.height, straight)
        .ok_or_else(|| anyhow::anyhow!("invalid rgba buffer size"))?;
    img.save(&out_path)?;

    eprintln!("wrote {}", out_path.display());
    Ok(())
}
