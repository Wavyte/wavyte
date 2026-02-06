use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use clap::{Parser, Subcommand, ValueEnum};
use sha2::Digest as _;

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

    /// Print diagnostics about text font resolution (family name + SHA-256 of font bytes).
    #[arg(long)]
    dump_fonts: bool,

    /// Print diagnostics about SVG font resolution (fontdb face count + text node count).
    #[arg(long)]
    dump_svg_fonts: bool,
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

    /// Print diagnostics about text font resolution (family name + SHA-256 of font bytes).
    #[arg(long)]
    dump_fonts: bool,

    /// Print diagnostics about SVG font resolution (fontdb face count + text node count).
    #[arg(long)]
    dump_svg_fonts: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BackendChoice {
    Cpu,
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
        BackendChoice::Cpu => wavyte::BackendKind::Cpu,
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
    let assets = wavyte::PreparedAssetStore::prepare(&comp, assets_root)?;

    if args.dump_fonts || args.dump_svg_fonts {
        dump_font_diagnostics(&comp, &assets, args.dump_fonts, args.dump_svg_fonts)?;
    }

    let frame = wavyte::render_frame(
        &comp,
        wavyte::FrameIndex(args.frame),
        backend.as_mut(),
        &assets,
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
    let assets = wavyte::PreparedAssetStore::prepare(&comp, assets_root)?;

    if args.dump_fonts || args.dump_svg_fonts {
        dump_font_diagnostics(&comp, &assets, args.dump_fonts, args.dump_svg_fonts)?;
    }

    let opts = wavyte::RenderToMp4Opts {
        range: wavyte::FrameRange::new(wavyte::FrameIndex(0), comp.duration)?,
        bg_rgba: settings.clear_rgba.unwrap_or([0, 0, 0, 255]),
        overwrite: true,
    };

    wavyte::render_to_mp4(&comp, &args.out, opts, backend.as_mut(), &assets)?;

    eprintln!("wrote {}", args.out.display());
    Ok(())
}

fn dump_font_diagnostics(
    comp: &wavyte::Composition,
    assets: &wavyte::PreparedAssetStore,
    dump_text: bool,
    dump_svg: bool,
) -> anyhow::Result<()> {
    if dump_text {
        eprintln!("text font diagnostics:");
        for (key, asset) in &comp.assets {
            let wavyte::Asset::Text(a) = asset else {
                continue;
            };

            let asset_id = assets
                .id_for_key(key)
                .with_context(|| format!("resolve text asset id '{key}'"))?;
            let prepared = assets
                .get(asset_id)
                .with_context(|| format!("load text asset '{key}'"))?;
            let wavyte::PreparedAsset::Text(p) = prepared else {
                anyhow::bail!("text asset '{key}' did not prepare as text (bug)");
            };

            let sha = sha256_hex(&p.font_bytes);
            eprintln!("  {key}:");
            eprintln!("    font_source: {}", a.font_source);
            eprintln!("    family:      {}", p.font_family);
            eprintln!("    sha256:      {}", sha);
        }
    }

    if dump_svg {
        eprintln!("svg font diagnostics:");
        for (key, asset) in &comp.assets {
            let wavyte::Asset::Svg(a) = asset else {
                continue;
            };

            let asset_id = assets
                .id_for_key(key)
                .with_context(|| format!("resolve svg asset id '{key}'"))?;
            let prepared = assets
                .get(asset_id)
                .with_context(|| format!("load svg asset '{key}'"))?;
            let wavyte::PreparedAsset::Svg(p) = prepared else {
                anyhow::bail!("svg asset '{key}' did not prepare as svg (bug)");
            };

            let face_count = p.tree.fontdb().faces().count();
            let text_nodes = count_svg_text_nodes(p.tree.root());
            eprintln!("  {key}:");
            eprintln!("    source:       {}", a.source);
            eprintln!("    text_nodes:   {text_nodes}");
            eprintln!("    font_faces:   {face_count}");
        }
    }

    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = sha2::Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn count_svg_text_nodes(group: &usvg::Group) -> usize {
    let mut n = 0usize;
    for child in group.children() {
        match child {
            usvg::Node::Group(g) => n += count_svg_text_nodes(g.as_ref()),
            usvg::Node::Text(_) => n += 1,
            usvg::Node::Path(_) | usvg::Node::Image(_) => {}
        }
    }
    n
}
