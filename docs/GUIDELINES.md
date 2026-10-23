<h1 align="center">
    <b>path-lib</b><br>
    <sub><sup>DEVELOPER GUIDELINES</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./API.md" title="API Reference"><b>API</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./PERFORMANCE.md" title="Performance"><b>PERFORMANCE</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <a href="./STABILITY-1.0.md" title="Stability Contract"><b>STABILITY</b></a>
        <span>&nbsp;&mdash;&nbsp;</span>
        <span>GUIDELINES</span>
    </sup>
</div>
<br>

This document describes the engineering standards this project is built to. [REPS](../REPS.md) is the governing standard; this page records how it applies here.

## Standards

- **MSRV:** Rust 1.85
- **Edition:** 2024
- **License:** Apache-2.0 OR MIT (dual)
- **Cross-platform:** Linux, macOS, Windows tested in CI; FreeBSD type-checked

## Quality Bar

- **Zero runtime dependencies.** `criterion` and `proptest` are dev-dependencies only.
- **`unsafe` confined to the Windows bindings.** The crate root denies `unsafe_code`; only the two Windows binding modules opt out (the Known Folder lookup, and the private-directory access control list since 1.1), and every block there carries a `SAFETY` comment.
- **No panics in library code** &mdash; no `unwrap`, `expect`, `todo`, or `unimplemented`.
- **Reject before building.** Input validation happens before any path is constructed; nothing is accepted and corrected afterwards. `sanitize` is the explicit, opt-in exception.
- **Testable platform rules.** Every platform table resolves through an injected environment lookup, so its rules are unit-tested without touching the process environment.
- **Property-tested invariants.** Confinement of `join`, validity and idempotence of `sanitize`, and idempotence of `clean` are checked against thousands of generated inputs per run.
- **Benchmark-backed claims** &mdash; performance statements are verified by committed benchmarks.

## Contributing

1. `cargo fmt --all -- --check` passes.
2. `cargo clippy --all-targets --all-features -- -D warnings` is clean, also with `--target` set to a macOS and a Linux triple.
3. `cargo test --all-features` passes.
4. New public items include documentation and examples.
5. A change to any directory table updates the tables in `src/lib.rs`, `docs/API.md`, and the platform module together. The tables are part of the [1.x stability contract](./STABILITY-1.0.md), so changing a default directory requires a major version.
6. A change to the public API is checked against the [stability contract](./STABILITY-1.0.md) before it is merged.
