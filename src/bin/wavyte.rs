use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "wavyte", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Render a single frame as a PNG.
    Frame(FrameArgs),
    /// Render an MP4 video (requires `ffmpeg` on PATH).
    Render(RenderArgs),
}

#[derive(Parser, Debug)]
struct FrameArgs {
    /// Input composition JSON.
    #[arg(long = "in")]
    in_path: PathBuf,

    /// Frame index (0-based).
    #[arg(long)]
    frame: u64,

    /// Output PNG path.
    #[arg(long)]
    out: PathBuf,

    /// Backend to use.
    #[arg(long, value_enum, default_value_t = BackendChoice::Cpu)]
    backend: BackendChoice,
}

#[derive(Parser, Debug)]
struct RenderArgs {
    /// Input composition JSON.
    #[arg(long = "in")]
    in_path: PathBuf,

    /// Output MP4 path.
    #[arg(long)]
    out: PathBuf,

    /// Backend to use.
    #[arg(long, value_enum, default_value_t = BackendChoice::Cpu)]
    backend: BackendChoice,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BackendChoice {
    Cpu,
    Gpu,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Frame(args) => cmd_frame(args),
        Command::Render(args) => cmd_render(args),
    }
}

fn read_comp_json(path: &Path) -> anyhow::Result<wavyte::Composition> {
    let f = File::open(path).with_context(|| format!("open composition '{}'", path.display()))?;
    let r = BufReader::new(f);
    let comp: wavyte::Composition =
        serde_json::from_reader(r).with_context(|| "parse composition JSON")?;
    Ok(comp)
}

fn make_backend(
    choice: BackendChoice,
    settings: &wavyte::RenderSettings,
) -> anyhow::Result<Box<dyn wavyte::RenderBackend>> {
    let kind = match choice {
        BackendChoice::Cpu => {
            #[cfg(feature = "cpu")]
            {
                wavyte::BackendKind::Cpu
            }
            #[cfg(not(feature = "cpu"))]
            {
                anyhow::bail!("built without `cpu` feature")
            }
        }
        BackendChoice::Gpu => {
            #[cfg(feature = "gpu")]
            {
                wavyte::BackendKind::Gpu
            }
            #[cfg(not(feature = "gpu"))]
            {
                anyhow::bail!("built without `gpu` feature")
            }
        }
    };

    Ok(wavyte::create_backend(kind, settings)?)
}

fn cmd_frame(args: FrameArgs) -> anyhow::Result<()> {
    let comp = read_comp_json(&args.in_path)?;
    comp.validate()?;

    let settings = wavyte::RenderSettings {
        clear_rgba: Some([18, 20, 28, 255]),
    };

    let mut backend = make_backend(args.backend, &settings)?;

    let assets_root = args.in_path.parent().unwrap_or_else(|| Path::new("."));
    let mut assets = wavyte::FsAssetCache::new(assets_root);

    let frame = wavyte::render_frame(
        &comp,
        wavyte::FrameIndex(args.frame),
        backend.as_mut(),
        &mut assets,
    )?;

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output dir '{}'", parent.display()))?;
    }

    image::save_buffer_with_format(
        &args.out,
        &frame.data,
        frame.width,
        frame.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .with_context(|| format!("write png '{}'", args.out.display()))?;

    eprintln!("wrote {}", args.out.display());
    Ok(())
}

fn cmd_render(args: RenderArgs) -> anyhow::Result<()> {
    let comp = read_comp_json(&args.in_path)?;
    comp.validate()?;

    let settings = wavyte::RenderSettings {
        clear_rgba: Some([18, 20, 28, 255]),
    };
    let mut backend = make_backend(args.backend, &settings)?;

    let assets_root = args.in_path.parent().unwrap_or_else(|| Path::new("."));
    let mut assets = wavyte::FsAssetCache::new(assets_root);

    let opts = wavyte::RenderToMp4Opts {
        range: wavyte::FrameRange::new(wavyte::FrameIndex(0), comp.duration)?,
        bg_rgba: settings.clear_rgba.unwrap_or([0, 0, 0, 255]),
        overwrite: true,
    };

    wavyte::render_to_mp4(&comp, &args.out, opts, backend.as_mut(), &mut assets)?;

    eprintln!("wrote {}", args.out.display());
    Ok(())
}
