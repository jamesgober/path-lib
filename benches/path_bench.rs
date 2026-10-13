//! Benchmarks for the hot paths: sub-path joining, sanitising, cached OS
//! lookups, and `App` construction.

#![allow(missing_docs)]

use std::hint::black_box;
use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use path_lib::App;

fn bench_join(c: &mut Criterion) {
    let base = Path::new("/var/lib/hivedb");
    let mut g = c.benchmark_group("join");
    g.bench_function("short", |b| {
        b.iter(|| path_lib::join(black_box(base), black_box("db")));
    });
    g.bench_function("nested_mixed_separators", |b| {
        b.iter(|| {
            path_lib::join(
                black_box(base),
                black_box("/big\\sloppy//floppy/nipple-slaps/"),
            )
        });
    });
    g.bench_function("unicode", |b| {
        b.iter(|| path_lib::join(black_box(base), black_box("données/résumé/名前")));
    });
    g.bench_function("rejected_traversal", |b| {
        b.iter(|| path_lib::join(black_box(base), black_box("a/b/../../etc/passwd")));
    });
    g.finish();
}

fn bench_sanitize(c: &mut Criterion) {
    let mut g = c.benchmark_group("sanitize");
    g.bench_function("clean_input", |b| {
        b.iter(|| path_lib::sanitize(black_box("quarterly-report-2026.pdf")));
    });
    g.bench_function("dirty_input", |b| {
        b.iter(|| path_lib::sanitize(black_box("Q3: <final>?.pdf\u{202e}. ")));
    });
    g.finish();
}

fn bench_os(c: &mut Criterion) {
    let mut g = c.benchmark_group("os");
    g.bench_function("home_cached", |b| b.iter(path_lib::home));
    g.bench_function("documents_cached", |b| b.iter(path_lib::documents));
    g.finish();
}

fn bench_app(c: &mut Criterion) {
    let mut g = c.benchmark_group("app");
    g.bench_function("new_user", |b| b.iter(|| App::new(black_box("HiveDB"))));
    g.bench_function("new_global", |b| {
        b.iter(|| App::global(black_box("HiveDB")));
    });
    if let Ok(app) = App::new("HiveDB") {
        g.bench_function("data_accessor", |b| {
            b.iter(|| black_box(&app).data().as_os_str().len());
        });
        g.bench_function("data_at", |b| {
            b.iter(|| black_box(&app).data_at(black_box("db/main")));
        });
    }
    g.finish();
}

criterion_group!(benches, bench_join, bench_sanitize, bench_os, bench_app);
criterion_main!(benches);
