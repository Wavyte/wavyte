use std::path::Path;

use wavyte::{
    Composition, CpuBackendOpts, FrameIndex, FrameRange, InMemorySink, RenderSession,
    RenderSessionOpts,
};

fn load_fixture(name: &str) -> Composition {
    Composition::from_path(Path::new("tests/data/v03").join(name)).unwrap()
}

#[test]
fn render_single_frame_minimal_solid() {
    let comp = load_fixture("minimal_solid.json");
    let mut session = RenderSession::new(&comp, ".", RenderSessionOpts::default()).unwrap();
    let frame = session
        .render_frame(FrameIndex(0), CpuBackendOpts::default())
        .unwrap();
    assert_eq!(frame.width, 64);
    assert_eq!(frame.height, 64);
    assert_eq!(frame.data.len(), 64 * 64 * 4);
}

#[test]
fn render_range_inmemory_sequential_and_parallel_match() {
    let comp = load_fixture("moving_opacity.json");
    let range = FrameRange::new(FrameIndex(0), FrameIndex(16)).unwrap();

    let mut s_seq =
        RenderSession::new(&comp, ".", RenderSessionOpts::default()).expect("seq session");
    let mut sink_seq = InMemorySink::new();
    s_seq
        .render_range(range, CpuBackendOpts::default(), &mut sink_seq)
        .unwrap();

    let opts_par = RenderSessionOpts {
        parallel: true,
        chunk_size: 8,
        ..Default::default()
    };
    let mut s_par = RenderSession::new(&comp, ".", opts_par).expect("par session");
    let mut sink_par = InMemorySink::new();
    s_par
        .render_range(range, CpuBackendOpts::default(), &mut sink_par)
        .unwrap();

    assert_eq!(sink_seq.frames().len(), sink_par.frames().len());
    for ((idx_a, a), (idx_b, b)) in sink_seq.frames().iter().zip(sink_par.frames().iter()) {
        assert_eq!(idx_a, idx_b);
        assert_eq!(a.width, b.width);
        assert_eq!(a.height, b.height);
        assert_eq!(a.data, b.data);
    }
}
