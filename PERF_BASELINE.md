# Performance Baseline (v0.2.1)

Date: 2026-02-11

Machine:
- 16 GB RAM
- 4 vCPU
- x86-64 Linux server (KVM VPS)

Methodology:
- Command template:
  - `cd bench && cargo run --release -- --repeats 10 --warmup 1 --seconds 10 --parallel --threads <N>`
- Encoding enabled (default; no `--no-encode`)
- Each thread configuration run one-by-one (no concurrent benchmark jobs)
- 10 measured runs per configuration
- Reported metrics in table: p50 and p90
- Raw logs are saved at:
  - `bench/results/perf_2026-02-11_t1.log`
  - `bench/results/perf_2026-02-11_t2.log`
  - `bench/results/perf_2026-02-11_t4.log`
  - `bench/results/perf_2026-02-11_t8.log`

## Results (10s clip, 30 fps, encode on)

| Threads | Wall p50 (ms) | Wall p90 (ms) | Speedup vs 1T (p50) | Render p50 (ms) | Encode write p50 (ms) | Wall p50 / frame (ms) | Render p50 / frame (ms) |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1  | 6494.955 | 7196.449 | 1.00x | 5255.402 | 1077.597 | 21.650 | 17.518 |
| 2  | 4938.074 | 5801.970 | 1.32x | 3441.560 | 1150.140 | 16.460 | 11.472 |
| 4  | 4010.889 | 4779.223 | 1.62x | 2545.693 | 1144.565 | 13.370 | 8.486 |
| 8  | 3963.202 | 5097.982 | 1.64x | 2555.783 | 1098.049 | 13.211 | 8.519 |

## Gain and Saturation Analysis

- Strong gain from 1 -> 2 -> 4 threads.
- 8 threads has the best p50 wall in this run set (`3963.202 ms`), but only marginally better than 4 threads (`4010.889 ms`, ~1.2%).
- 4 threads is more stable than 8 threads on tail behavior (`wall p90`: `4779.223 ms` at 4T vs `5097.982 ms` at 8T).
- Practical sweet spot on this 4 vCPU host remains around 4 threads; 8 threads offers minimal median gain with weaker consistency.
