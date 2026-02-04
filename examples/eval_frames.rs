use wavyte::{Composition, Evaluator, FrameIndex};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let s = include_str!("../tests/data/simple_comp.json");
    let comp: Composition = serde_json::from_str(s)?;

    for f in [0u64, 1, 2, 9, 10, 19] {
        let g = Evaluator::eval_frame(&comp, FrameIndex(f))?;
        println!("frame {f}: {} nodes", g.nodes.len());
    }

    Ok(())
}
