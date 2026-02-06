# Performance Baseline (v0.2)

Date: 2026-02-06

Machine:
- 16 GB RAM
- 4 vCPU
- x86-64 Linux server

Methodology:
- Command template:
  - `cargo run --release -- --repeats 10 --warmup 1 --seconds 1 --parallel --threads <N>`
- Encoding enabled (default; no `--no-encode`)
- Each thread configuration run one-by-one (no concurrent benchmark jobs)
- 10 measured runs per configuration
- Reported metrics: p50 and p90 only

## Results (1s clip, 30 fps, encode on)

| Threads | Wall p50 (ms) | Wall p90 (ms) | Speedup vs 1T (p50) | Render p50 (ms) | Encode write p50 (ms) |
|---:|---:|---:|---:|---:|---:|
| 1  | 843.171 | 872.478 | 1.00x | 542.902 | 69.181 |
| 2  | 657.195 | 712.354 | 1.28x | 345.500 | 86.781 |
| 4  | 488.485 | 538.309 | 1.73x | 227.641 | 62.920 |
| 8  | 585.337 | 606.760 | 1.44x | 265.160 | 91.395 |
| 16 | 494.501 | 514.777 | 1.70x | 239.520 | 56.266 |

## Gain and Saturation Analysis

- Clear scaling from 1 -> 2 -> 4 threads.
- Best p50 wall time in this run set is at 4 threads (`488.485 ms`).
- 16 threads is close to 4 threads (`494.501 ms`, about 1.2% slower), but not better.
- 8 threads regresses versus 4 threads.
- Practical saturation point on this host: around 4 worker threads (matching 4 vCPU), with no consistent gain beyond that.
