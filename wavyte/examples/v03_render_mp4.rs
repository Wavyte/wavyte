use std::io::Cursor;
use std::path::Path;

use wavyte::{
    Composition, CpuBackendOpts, FfmpegSink, FfmpegSinkOpts, FrameRange, RenderSession,
    RenderSessionOpts,
};

fn main() -> anyhow::Result<()> {
    let json = r##"
{
  "version": "0.3",
  "canvas": { "width": 256, "height": 256 },
  "fps": { "num": 30, "den": 1 },
  "duration": 60,
  "assets": {
    "solid": { "solid_rect": { "color": "#22aaee" } }
  },
  "root": {
    "id": "root",
    "kind": { "leaf": { "asset": "solid" } },
    "range": [0, 60]
  }
}
"##;

    let comp = Composition::from_reader(Cursor::new(json))?;
    let mut session = RenderSession::new(&comp, ".", RenderSessionOpts::default())?;

    let out_path = Path::new("target/v03_examples/out.mp4");
    let mut sink = FfmpegSink::new(FfmpegSinkOpts::new(out_path));

    session.render_range(
        FrameRange::new(wavyte::FrameIndex(0), wavyte::FrameIndex(60))?,
        CpuBackendOpts::default(),
        &mut sink,
    )?;

    eprintln!("wrote {}", out_path.display());
    Ok(())
}
