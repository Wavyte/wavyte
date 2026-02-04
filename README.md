# wavyte
Fast and ergonomic programmatic video generation

## Development (v0.1.0 Phase 1+2)

Quality gate before commits:

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --no-default-features`
- `cargo test --all-targets --features gpu`
- `cargo test --all-targets --features cpu`
- `cargo test --all-targets --features gpu,cpu`

Examples:

- `cargo run --example build_dsl_and_dump_json`
- `cargo run --example eval_frames`
