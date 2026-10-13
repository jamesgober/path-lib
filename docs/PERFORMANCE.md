<h1 align="center">
    <b>path-lib</b><br>
    <sub><sup>PERFORMANCE</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./API.md" title="API Reference"><b>API</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./GUIDELINES.md" title="Developer Guidelines"><b>GUIDELINES</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <span>PERFORMANCE</span>
    </sup>
</div>
<br>

> **Status:** Numbers below are the **0.1.0** baseline. Re-run the suite locally to compare on your hardware.

## Methodology

- **Harness:** [`criterion`](https://crates.io/crates/criterion) 0.5, `cargo bench --bench path_bench`.
- **Profile:** `release` with `lto = "fat"`, `codegen-units = 1`.
- **Machine:** Windows 11, x86_64, stable Rust. Each number is the criterion median of a 3-second measurement after a 1-second warm-up.

## Results

### Cached lookups

| Operation | Median |
|---|---|
| `home()` (after first call) | 0.94 ns |
| `documents()` (after first call) | 1.03 ns |
| `app.data()` | 0.23 ns |

After the first call these are a `OnceLock` load and a field read.

### Joining and sanitising

| Operation | Median |
|---|---|
| `join(base, "db")` | 57 ns |
| `join(base, "/big\\sloppy//floppy/nipple-slaps/")` | 139 ns |
| `join(base, "données/résumé/名前")` | 119 ns |
| `join(base, "a/b/../../etc/passwd")` (rejected) | 100 ns |
| `app.data_at("db/main")` | 78 ns |
| `sanitize("quarterly-report-2026.pdf")` | 44 ns |
| `sanitize("Q3: <final>?.pdf\u{202e}. ")` | 41 ns |

A successful join performs exactly one allocation, sized up front for the base plus the sub-path. ASCII segments are validated byte by byte; only non-ASCII segments decode characters. A rejected input pays for building the error, which copies a bounded preview of the input.

### Construction

| Operation | Median |
|---|---|
| `App::new("HiveDB")` | 567 ns |
| `App::global("HiveDB")` | 564 ns |

Construction validates the name and builds eight `PathBuf`s from the cached bases. It is a one-time cost; keep the `App` and pass it around.

## Running the Benchmarks

```bash
cargo bench --bench path_bench
```

## Interpreting the Numbers

Path resolution is not on a per-request hot path in most programs; it runs at startup and when files are opened. The numbers show that the validation the crate adds costs tens of nanoseconds, well below the cost of the system call that follows any real use of the path.
