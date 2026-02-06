# v0.2 Baseline (Pre-Refactor)

Date: 2026-02-06
Branch: `wavyte-v0.2`
Mode: debug (`[profile.dev] debug = 0`)

## Command

```bash
cd bench
cargo run -- --repeats 1 --warmup 0 --seconds 1 --no-encode
```

## Result

- frames: 30 (1s @ 30fps)
- backend: CPU
- encode: disabled

```text
run 000: wall=27.251s eval=0.003s compile=0.002s render=27.245s encode_write=0.000s ffmpeg_spawn=0.000s ffmpeg_finish=0.000s

percentiles across runs (p50/p90/p99):
  backend_create     p50=   0.005ms  p90=   0.005ms  p99=   0.005ms
  ffmpeg_spawn       p50=   0.000ms  p90=   0.000ms  p99=   0.000ms
  eval_total         p50=   2.570ms  p90=   2.570ms  p99=   2.570ms
  compile_total      p50=   2.415ms  p90=   2.415ms  p99=   2.415ms
  render_total       p50=27245.497ms  p90=27245.497ms  p99=27245.497ms
  encode_write_total p50=   0.000ms  p90=   0.000ms  p99=   0.000ms
  ffmpeg_finish      p50=   0.000ms  p90=   0.000ms  p99=   0.000ms
  wall_total         p50=27251.054ms  p90=27251.054ms  p99=27251.054ms
```

## Notes

- This captures a pre-v0.2-refactor baseline for relative comparisons.
- Use the same command and machine profile when measuring post-change deltas.

---

## v0.2 Post-Revamp (Current)

Date: 2026-02-06
Branch: `wavyte-v0.2`
Mode: debug (`[profile.dev] debug = 0`)

## Command

```bash
cd bench
cargo run -- --repeats 1 --warmup 0 --seconds 1 --no-encode
```

## Result

```text
run 000: wall=25.939s eval=0.003s compile=0.001s render=24.123s encode_write=0.000s ffmpeg_spawn=0.000s ffmpeg_finish=0.000s

percentiles across runs (p50/p90/p99):
  backend_create     p50=   0.005ms  p90=   0.005ms  p99=   0.005ms
  ffmpeg_spawn       p50=   0.000ms  p90=   0.000ms  p99=   0.000ms
  eval_total         p50=   2.640ms  p90=   2.640ms  p99=   2.640ms
  compile_total      p50=   0.927ms  p90=   0.927ms  p99=   0.927ms
  render_total       p50=24122.730ms  p90=24122.730ms  p99=24122.730ms
  encode_write_total p50=   0.000ms  p90=   0.000ms  p99=   0.000ms
  ffmpeg_finish      p50=   0.000ms  p90=   0.000ms  p99=   0.000ms
  wall_total         p50=25938.508ms  p90=25938.508ms  p99=25938.508ms
```

## Delta vs Pre-Refactor Baseline

- wall total: `27.251s -> 25.939s` (`-1.312s`, about `-4.8%`)
- compile total: `2.415ms -> 0.927ms` (about `-61.6%`)
- render total: `27.245s -> 24.123s` (about `-11.5%`)
