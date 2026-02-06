# Performance Baseline (v0.2)

Date: 2026-02-06

Machine:
- 16 GB RAM
- 4 vCPU
- x86-64 Linux server

Methodology:
- Command template:
  - `cargo run --release -- --repeats 10 --warmup 1 --seconds 10 --parallel --threads <N>`
- Encoding enabled (default; no `--no-encode`)
- Each thread configuration run one-by-one (no concurrent benchmark jobs)
- 10 measured runs per configuration
- Reported metrics: p50 and p90 only

## Results (10s clip, 30 fps, encode on)

| Threads | Wall p50 (ms) | Wall p90 (ms) | Speedup vs 1T (p50) | Render p50 (ms) | Encode write p50 (ms) | Wall p50 / frame (ms) | Render p50 / frame (ms) |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1  | 6035.825 | 6168.430 | 1.00x | 4990.223 | 840.460 | 20.119 | 16.634 |
| 2  | 3876.942 | 4119.431 | 1.56x | 2903.349 | 765.864 | 12.923 | 9.678 |
| 4  | 3032.048 | 3468.811 | 1.99x | 2000.036 | 816.484 | 10.107 | 6.667 |
| 8  | 3137.818 | 3330.549 | 1.92x | 2142.020 | 803.633 | 10.459 | 7.140 |

## Gain and Saturation Analysis

- Clear scaling from 1 -> 2 -> 4 threads.
- Best p50 wall time in this run set is at 4 threads (`3032.048 ms`), a `1.99x` speedup vs 1 thread.
- 8 threads is slightly slower than 4 threads for p50 wall (`3137.818 ms`, about `3.5%` slower).
- Practical saturation point on this host: around 4 worker threads (matching 4 vCPU), with no consistent gain beyond that.
- At 4 threads, amortized p50 render cost is `6.667 ms/frame` for this 300-frame workload; amortized p50 wall is `10.107 ms/frame` with encode enabled.
