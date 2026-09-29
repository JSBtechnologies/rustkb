---
id: idioms/performance
title: Performance
summary: >-
  Measure before optimising (criterion/divan, samply/flamegraph), tune the release profile (LTO,
  codegen-units, panic, debug info) and PGO only with evidence, then remove allocations and clones,
  pick hashers/allocators/containers deliberately, and let iterators eliminate bounds checks.
area: idioms
tags: [performance, benchmarking, profiling, criterion, divan, release-profile, lto, pgo, allocation, clone, cow, smallvec, hashing, allocator, inline, bounds-checks]
rust: "1.96"
edition: "2024"
crates:
  criterion: "0.8"
  divan: "0.1"
  smallvec: "1.16"
  arrayvec: "0.7"
  compact_str: "0.10"
  rustc-hash: "2.1"
  foldhash: "0.2"
  mimalloc: "0.1"
  tikv-jemallocator: "0.7"
  cargo-pgo: "0.3"
  samply: "0.13"
  flamegraph: "0.6"
verified: 2026-09-29
sources:
  - https://nnethercote.github.io/perf-book/
  - https://doc.rust-lang.org/cargo/reference/profiles.html
  - https://bheisler.github.io/criterion.rs/book/
  - https://github.com/Kobzol/cargo-pgo
  - https://github.com/mstange/samply
  - https://doc.rust-lang.org/std/hint/fn.black_box.html
---

# Performance

Decides how to find and fix performance problems in Rust: what to measure with, which build
settings matter, and which code-level changes (allocation, cloning, containers, hashing, bounds
checks, inlining, IO buffering) pay off. Read before "optimising" anything, adding `unsafe` for
speed, or tuning `Cargo.toml` profiles. Build-time (compile speed) tuning is in rust-architecture.

## Measure first

### PERF-01: Benchmark and profile before changing code
**Default:** reproduce the slowness in a benchmark or a release-build run, profile it, then change
the hottest thing. `criterion` for statistically sound micro-benchmarks (default); `divan` for
lighter attribute-style benches; `samply record ./target/release/app` or `cargo flamegraph` for
where time goes; `std::hint::black_box` to stop the optimiser deleting benchmarked work.
**Never** benchmark a debug build, "optimise" without a before/after number, or keep an
optimisation that doesn't move the benchmark — it is just complexity.

```toml
# Cargo.toml
[dev-dependencies]
criterion = "0.8"

[[bench]]
name = "parse"
harness = false   # criterion provides main()
```

```rust
// benches/parse.rs
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn parse_all(input: &str) -> u64 {
    input.split(',').filter_map(|s| s.trim().parse::<u64>().ok()).sum()
}

fn bench_parse(c: &mut Criterion) {
    let input = (0..1_000).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    c.bench_function("parse_all 1k", |b| b.iter(|| parse_all(black_box(&input))));
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
```

### PERF-02: Profile with symbols; keep a `profiling` profile
**Default:** add a custom profile so profilers see function names and lines without slowing the
release build:

```toml
[profile.profiling]
inherits = "release"
debug = "line-tables-only"   # file/line info for profilers and backtraces
```

Build with `cargo build --profile profiling`, run under `samply record` (Linux/macOS/Windows),
`perf`, VTune or Instruments. **Never** draw conclusions from `cargo run` (debug) timings or from
`Instant::now()` around a single call.

## Build configuration

### PERF-03: Tune the release profile only with measurements
**Default:** Cargo's release profile (`opt-level = 3`, `lto = false` = thin-local LTO,
`codegen-units = 16`) is a good start. For a shipped binary where 5–20 % matters, benchmark:

```toml
[profile.release]
lto = "thin"          # or "fat"/true: cross-crate inlining; slower links
codegen-units = 1     # better optimisation, slower builds
panic = "abort"       # smaller/faster; no unwinding, no catch_unwind recovery
strip = "symbols"     # smaller binary; keep debug info elsewhere if you need crash symbols
```

`panic = "abort"` is only for binaries that never recover from panics (tests and benches ignore
it). For size-constrained targets try `opt-level = "s"`/`"z"`. **Never** copy a "max perf" profile
into a library's `Cargo.toml` — profiles are ignored in dependencies; only the final workspace's
profile applies.

### PERF-04: Use `target-cpu=native` only for binaries that run where they are built
**Default:** portable builds. **Use** `-C target-cpu=native` (in `.cargo/config.toml` `rustflags`)
for local tools and single-machine deployments; for distributed binaries pick a baseline such as
`-C target-cpu=x86-64-v3` only if every target machine supports it, or keep the baseline and use
runtime detection (`is_x86_feature_detected!` + `#[target_feature(enable = "avx2")]`, which may be
applied to safe functions since 1.86). **Never** ship `native` builds to machines you don't
control: they crash with illegal-instruction on older CPUs.

### PERF-05: Reach for PGO/BOLT last, for hot long-running binaries
**Default:** after algorithmic and allocation work, and with LTO on, profile-guided optimisation
can add 10 %+ for large CPU-bound programs (compilers, databases, proxies). **Use** `cargo-pgo`:
`rustup component add llvm-tools-preview`, `cargo pgo build`, run a representative workload,
`cargo pgo optimize build` (and `cargo pgo bolt ...` on Linux). The workload must represent
production or the result gets slower.

## Allocations and copies

### PERF-06: Pre-size collections and reuse buffers
**Default:** `Vec::with_capacity(n)` / `String::with_capacity(n)` / `reserve(n)` when the size is
known or bounded; `collect()` already pre-sizes from exact-size iterators. In loops, allocate once
outside, `clear()` inside, and pass `&mut Vec<T>` / `&mut String` / `impl Write` to helpers instead
of returning a fresh `String` per call. **Never** build strings with repeated `format!` +
`push_str` (`clippy::format_push_string`) — `write!` into the existing buffer.

```rust
use std::fmt::Write;
use std::io::BufRead;

pub fn render_rows(rows: &[(u32, &str)]) -> String {
    let mut out = String::with_capacity(rows.len() * 16);
    for (id, name) in rows {
        writeln!(out, "{id}\t{name}").expect("writing to a String cannot fail"); // no temp String
    }
    out
}

pub fn longest_line(mut input: impl BufRead) -> std::io::Result<usize> {
    let mut line = String::new(); // one buffer for every line
    let mut longest = 0;
    while input.read_line(&mut line)? != 0 {
        longest = longest.max(line.trim_end().len());
        line.clear();
    }
    Ok(longest)
}
```

### PERF-07: Borrow instead of cloning; clone at ownership boundaries only
**Default:** accept `&str`/`&[T]`, return borrows or iterators, and move values you no longer
need (see `ownership-borrowing.md`). Clone when you genuinely need a second owner; for shared
immutable data clone an `Arc` (`Arc::clone(&x)`), not the data. Watch the hidden clones:
`.to_string()` / `.to_owned()` / `.to_vec()` / `.cloned()` in iterator chains, `String` keys built
just to call `HashMap::get` (look up with `&str`). `clippy::unnecessary_to_owned`,
`clippy::redundant_clone` (nursery) and `clippy::implicit_clone` (pedantic) find them.

```rust
use std::collections::HashMap;

pub fn total_for(prices: &HashMap<String, u64>, items: &[&str]) -> u64 {
    items.iter().filter_map(|name| prices.get(*name)).sum() // &str lookup, no String allocation
}
```

### PERF-08: Return `Cow` when the common path needs no allocation
**Default:** when a function usually returns its input unchanged (escaping, normalising,
trimming), return `Cow<'_, str>` / `Cow<'_, [T]>`: borrow on the fast path, allocate only when you
modify. **Never** use `Cow` in struct fields "for flexibility" without a measured reason — the
lifetime spreads through the API (see `ownership-borrowing.md`).

```rust
use std::borrow::Cow;

pub fn escape_tabs(s: &str) -> Cow<'_, str> {
    if s.contains('\t') {
        Cow::Owned(s.replace('\t', "\\t"))
    } else {
        Cow::Borrowed(s) // no allocation for the common case
    }
}
```

### PERF-09: Use inline-storage and compact types only where profiles show many tiny allocations
**Default:** `Vec<T>` and `String`. **Use** `SmallVec<[T; N]>` (spills to heap) or
`ArrayVec<T, N>` (fixed capacity, never allocates) when a profile shows many short-lived small
vectors; `Box<str>` / `Arc<str>` / `Box<[T]>` for immutable data kept in bulk (no capacity field;
`Arc<str>` instead of `Arc<String>` avoids a double indirection); `compact_str::CompactString` for
many short strings. **Never** default to `SmallVec` everywhere — larger values, branchy access and
a slower spill path can make code slower.

```rust
use smallvec::SmallVec;
use std::sync::Arc;

pub fn split_path(path: &str) -> SmallVec<[&str; 8]> {
    path.split('/').filter(|s| !s.is_empty()).collect() // ≤ 8 segments: no heap allocation
}

pub fn intern_names(names: Vec<String>) -> Vec<Arc<str>> {
    names.into_iter().map(Arc::from).collect() // one allocation per name, cheap clones after
}
```

### PERF-10: Choose the hasher by who controls the keys
**Default:** std `HashMap`/`HashSet` (SipHash-1-3, randomly seeded) whenever keys can come from
untrusted input — it resists hash-flooding DoS. **Use** `rustc_hash::FxHashMap` or
`foldhash` for hot maps with trusted keys (integers, internal IDs, compiler-like workloads);
`BTreeMap` when you need ordering or deterministic iteration. **Never** swap the hasher of a map
keyed by request data for speed (see rust-security).

```rust
use rustc_hash::FxHashMap;

pub fn histogram(ids: &[u32]) -> FxHashMap<u32, usize> {
    let mut counts = FxHashMap::default();
    for &id in ids {
        *counts.entry(id).or_insert(0) += 1; // internal u32 keys: fast non-DoS-resistant hasher is fine
    }
    counts
}
```

### PERF-11: Swap the global allocator only for allocation-heavy multi-threaded binaries
**Default:** the system allocator. **Use** `mimalloc` or `tikv-jemallocator` in a *binary* when
profiling shows allocator contention or fragmentation (busy multi-threaded services; musl targets
especially, whose allocator is slow). **Never** set `#[global_allocator]` in a library — that
decision belongs to the final binary.

```rust
// main.rs of the service binary
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
```

## Code generation

### PERF-12: Let iterators and slicing remove bounds checks; don't reach for `get_unchecked`
**Default:** iterate (`iter`, `zip`, `chunks_exact`, `windows`) instead of indexing in loops; when
you must index, re-slice once (`let a = &a[..n];`) or `assert!` lengths up front so the optimiser
can prove every access in range. **Never** use `get_unchecked` without a benchmark showing the
check matters *and* a `SAFETY:` proof (see `unsafe-ffi.md`, UNSAFE-01).

```rust
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "dot: length mismatch");
    a.iter().zip(b).map(|(x, y)| x * y).sum() // no per-element bounds checks
}
```

### PERF-13: Don't sprinkle `#[inline]`; mark cold paths instead
**Default:** no attribute. Generic functions and functions within one crate are already inlining
candidates; with `lto` everything is. **Use** `#[inline]` on small, hot, non-generic `pub`
functions in libraries called across crate boundaries (without LTO); `#[cold]` on error/slow-path
functions (or `core::hint::cold_path()`, 1.95) so the hot path stays compact. **Use**
`#[inline(always)]` / `#[inline(never)]` only with benchmark evidence — both can make code slower
and binaries bigger.

### PERF-14: Prefer contiguous data and the cheapest correct algorithm
**Default:** `Vec`/slices (cache-friendly) over `LinkedList` (`clippy::linkedlist`) and over maps
for small N — a linear scan of ≤ ~32 elements often beats hashing; `sort_unstable*` when stable
order isn't needed; `binary_search` on a sorted `Vec` for read-mostly lookup tables. Keep hot
structs small: box rarely-used large enum variants (`clippy::large_enum_variant`).

### PERF-15: Buffer IO and lock stdout once
**Default:** wrap `File`/`TcpStream` in `BufReader`/`BufWriter`; for many writes to stdout, take
`io::stdout().lock()` once and wrap it in `BufWriter`. **Never** `println!` in a hot loop — each
call locks stdout, and Rust's stdout is line-buffered, so every line is a write syscall. Call
`flush()` explicitly at the end of a `BufWriter` to observe write errors (drop ignores them).

```rust
use std::io::{self, BufWriter, Write};

pub fn print_all(values: &[u64]) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    for v in values {
        writeln!(out, "{v}")?;
    }
    out.flush() // surface write errors instead of losing them on drop
}
```

## Review checklist

- [ ] A benchmark or profile justifies each optimisation; before/after numbers recorded (PERF-01, PERF-02)
- [ ] Release-profile changes (LTO, codegen-units, panic) are measured and live in the binary's workspace (PERF-03)
- [ ] No `target-cpu=native` in distributed builds (PERF-04); PGO only for hot binaries (PERF-05)
- [ ] Collections pre-sized; buffers reused; `write!` instead of `format!` + `push_str` (PERF-06)
- [ ] No clones/`to_string` to satisfy lookups or borrowck; `Arc::clone` for shared data (PERF-07)
- [ ] `Cow` on mostly-unchanged return paths; `SmallVec`/`compact_str` only where profiled (PERF-08, PERF-09)
- [ ] Fast hashers only for trusted keys (PERF-10); `#[global_allocator]` only in binaries (PERF-11)
- [ ] Loops iterate instead of index; no unproven `get_unchecked` (PERF-12)
- [ ] No blanket `#[inline(always)]`; cold paths marked (PERF-13)
- [ ] Contiguous containers; no `LinkedList` (PERF-14); stdout/file IO buffered, no `println!` in loops (PERF-15)
