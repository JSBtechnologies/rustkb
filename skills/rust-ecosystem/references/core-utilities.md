---
id: ecosystem/core-utilities
title: Error, concurrency, collection and macro-helper crates
summary: >-
  thiserror/anyhow for errors, rayon and friends for parallelism and shared state (and where
  std now suffices), collections/hashing/ids/randomness crates, and proc-macro tooling.
area: ecosystem
tags: [errors, thiserror, anyhow, concurrency, rayon, dashmap, parking_lot, arc-swap, collections, indexmap, smallvec, hashing, rand, uuid, macros, syn]
rust: "1.96"
edition: "2024"
crates:
  thiserror: "2.0"
  anyhow: "1.0"
  eyre: "0.6"
  snafu: "0.9"
  rayon: "1.12"
  crossbeam-channel: "0.5"
  dashmap: "6.2"
  papaya: "0.2"
  parking_lot: "0.12"
  arc-swap: "1.9"
  indexmap: "2.14"
  smallvec: "1.16"
  arrayvec: "0.7"
  bytes: "1.12"
  hashbrown: "0.17"
  rustc-hash: "2.1"
  foldhash: "0.2"
  petgraph: "0.8"
  slotmap: "1.1"
  bitflags: "2.13"
  uuid: "1.26"
  rand: "0.10"
  fastrand: "2.5"
  itertools: "0.15"
  strum: "0.28"
  derive_more: "2.1"
  compact_str: "0.10"
  rust_decimal: "1.43"
  semver: "1.0"
  syn: "3.0"
  quote: "1.0"
  proc-macro2: "1.0"
  darling: "0.24"
  manyhow: "0.14"
  pastey: "0.2"
  derive-where: "1.7"
  bon: "3.10"
  educe: "0.8"
  imbl: "7.0"
  getrandom: "0.4"
verified: 2026-09-29
sources:
  - https://docs.rs/thiserror/latest/thiserror/
  - https://docs.rs/anyhow/latest/anyhow/
  - https://docs.rs/rayon/latest/rayon/
  - https://rust-random.github.io/book/update.html
  - https://rustsec.org/advisories/RUSTSEC-2024-0436.html
---

# Error, concurrency, collection and macro-helper crates

Error-handling *design* (what goes in an error, when to panic) and concurrency *patterns*
(ownership across threads, avoiding `Arc<Mutex<_>>` by reflex) are in `rust-idioms`. This file
picks crates.

## UTIL-01: Errors — thiserror in libraries, anyhow in applications

Default: `thiserror` (2.0) to define error enums; `anyhow` (1.0) at
application edges (`main`, request handlers, scripts). thiserror is 2.x — don't write
`thiserror = "1"` from memory.

```rust
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    #[error("request failed")]
    Http(#[from] std::io::Error),
    #[error("record {id} not found")]
    NotFound { id: u64 },
}
```

- Never expose `anyhow::Error` or `Box<dyn Error>` from a library's public API.
- `eyre` (0.6) / `color-eyre`: anyhow-like with pluggable report handlers (nice CLI output).
- `snafu` (0.9): context selectors that force context at each error site; good for
  large codebases that want that discipline.
- `miette`: diagnostics with source spans (see `cli-and-tui.md`).
- Dead: `failure` (RUSTSEC-2020-0036), `error-chain`, `quick-error` — replace on sight.

## UTIL-02: Data parallelism with rayon

Default: `rayon` (1.12) for CPU-bound work over collections — change `iter()` to
`par_iter()`.

```rust
use rayon::prelude::*;

fn checksum(chunks: &[Vec<u8>]) -> u64 {
    chunks
        .par_iter()
        .map(|c| c.iter().map(|&b| u64::from(b)).sum::<u64>())
        .sum()
}
```

- Inside async code, run rayon work via `tokio::task::spawn_blocking` or a oneshot channel —
  never block a tokio worker on a long `par_iter`.
- Don't parallelise tiny loops; measure. Don't use rayon for IO (use async).

## UTIL-03: std first for threads, channels and lazy values

Default: std primitives. Many crates agents add here are obsolete:

| Instead of | Use |
|---|---|
| `lazy_static!`, `once_cell::sync::Lazy` | `std::sync::LazyLock` (1.80) |
| `once_cell::sync::OnceCell` | `std::sync::OnceLock` (1.70) |
| `crossbeam::scope` | `std::thread::scope` (1.63) |
| `num_cpus::get()` | `std::thread::available_parallelism()` (1.59) |
| `crossbeam-channel` for plain MPSC | `std::sync::mpsc` (crossbeam-based since 1.67) |
| `parking_lot::Mutex` by default | `std::sync::Mutex` (futex-based since 1.62, fast) |

Still worth a crate:
- `crossbeam-channel` (0.5): MPMC and `select!` over channels.
- `parking_lot` (0.12): no poisoning, `const` constructors for more types,
  upgradable/fair locks, `lock_api` integration — pick for those features, not speed folklore.
- `arc-swap` (1.9): read-mostly shared config/state that is swapped atomically.

```rust
use std::sync::Arc;

use arc_swap::ArcSwap;

#[derive(Debug, Default)]
struct Config {
    max_conns: usize,
}

fn hot_reload(shared: &ArcSwap<Config>) {
    let current = shared.load(); // cheap, lock-free read
    let next = Config { max_conns: current.max_conns + 1 };
    shared.store(Arc::new(next)); // readers see old or new, never a torn value
}
```

## UTIL-04: Concurrent maps

Default: `Mutex<HashMap<K, V>>` / `RwLock<HashMap>` owned by one component, or message passing
to a task that owns the map. Reach for a concurrent map only with measured contention.

- `dashmap` (6.2): sharded map; its `Ref` guards hold a shard lock — never keep
  one across `.await` or while touching the same map again (deadlock).
- `papaya` (0.2): lock-free, read-optimised, safe to use from async code.
- For caches with eviction, use `moka` (see `databases.md`), not a raw concurrent map.

## UTIL-05: Collections

| Need | Crate |
|---|---|
| Insertion-ordered map/set (deterministic output) | `indexmap` (2.14) |
| Byte buffers shared across tasks (network code) | `bytes` (1.12) |
| Small inline vectors (measured hot path) | `smallvec` (1.16) / `arrayvec` (0.7) |
| Graphs and graph algorithms | `petgraph` (0.8) |
| Arena with generational keys (instead of `Rc<RefCell>` graphs) | `slotmap` (1.1) |
| Bit flags | `bitflags` (2.13) |
| Small-string optimisation | `compact_str` (0.10) — `smartstring` is unmaintained (RUSTSEC-2026-0249) |
| Persistent/immutable collections | `imbl` — `im` is unmaintained (RUSTSEC-2026-0248) |
| no_std hash map / raw-table APIs | `hashbrown` (0.17) — std's `HashMap` already *is* hashbrown |
| Exact decimals (money) | `rust_decimal` (1.43); never `f64` for currency |
| Version strings | `semver` (1.0) |

## UTIL-06: Hashing (non-cryptographic)

Default: std's `HashMap` with its default SipHash-based `RandomState` — DoS-resistant and
fast enough for most code.

- Hot maps with trusted keys (integers, interned strings, compiler-like workloads):
  `rustc-hash` (2.1) `FxHashMap`. `fxhash` is unmaintained (RUSTSEC-2025-0057).
- Faster general hashing with some DoS resistance: `foldhash` (0.2) (hashbrown's
  default hasher). `ahash` is still common transitively but no longer the hashbrown default.
- Never use a fast non-DoS-resistant hasher for maps keyed by untrusted input (HTTP headers,
  user IDs from requests). Cryptographic hashing is a different job (see `domain-specific.md`).

## UTIL-07: Randomness and IDs

Default: `rand` (0.10). rand 0.9 and 0.10 renamed core APIs, so memory-based code
fails to compile:

| Old (≤ 0.8) | Current |
|---|---|
| `rand::thread_rng()` | `rand::rng()` |
| `rng.gen()` / `rng.gen_range(a..b)` | `rng.random()` / `rng.random_range(a..b)` |
| `use rand::Rng;` for those methods | `use rand::RngExt;` (0.10 moved the convenience methods to `RngExt`) |
| `rand::random::<T>()` | unchanged; also `rand::random_range(a..=b)` |

```rust
use rand::RngExt;
use rand::seq::SliceRandom;

fn roll_and_shuffle() -> (u32, Vec<i32>) {
    let mut rng = rand::rng();
    let roll = rng.random_range(1..=6);
    let mut v = vec![1, 2, 3];
    v.shuffle(&mut rng);
    (roll, v)
}
```

- Tiny/fast non-crypto RNG (jitter, tests): `fastrand` (2.5).
- Security tokens/keys: an OS-backed CSPRNG (`rand::rngs::OsRng`/`getrandom`, or the crypto
  library's RNG) — see `rust-security`.
- IDs: `uuid` (1.26) with `v7` (`Uuid::now_v7()`) for database keys, `v4` for opaque random IDs.

## UTIL-08: Iterator and derive helpers

- `itertools` (0.15): `chunk_by`, `tuple_windows`, `join`, `sorted_by_key`,
  `izip!`. Check std first — slice `windows`/`chunks`/`chunk_by` (1.77), `iter::repeat_n`
  (1.82) and `Iterator::is_sorted` (1.82) are stable.
- `strum` (0.28): enum ↔ string, `EnumIter`, `EnumCount`.
- `derive_more` (2.1): `From`, `Display`, `Deref`, arithmetic for newtypes
  (2.x requires opting into each derive via features).
- Builders: `bon` (3.10) (compile-time checked, works on functions too). `derive_builder`
  checks required fields only at runtime.
- `derivative` is unmaintained (RUSTSEC-2024-0388): use `derive-where` (1.7)
  for bounds-free `#[derive]`s, or `educe`.

## UTIL-09: Proc-macro tooling

Default: `syn` (3.0) + `quote` (1.0) + `proc-macro2` (1.0).
**syn is now 3.x** — macros written against syn 1/2 from memory need updating; check the
syn release notes when porting.

- Attribute parsing into structs: `darling` (0.24).
- Errors: return `syn::Error::to_compile_error()`; for multiple errors use `manyhow`
  (0.14). `proc-macro-error` and its fork `proc-macro-error2` are both unmaintained
  (RUSTSEC-2024-0370, RUSTSEC-2026-0173).
- Identifier pasting in `macro_rules!`: `pastey` (0.2) — `paste` is archived
  (RUSTSEC-2024-0436).
- `#[cfg]` dispatch: `cfg_select!` (std, 1.95+) instead of `cfg-if`.
- Offsets and compile-time checks: `core::mem::offset_of!` (1.77) replaces `memoffset`;
  `const { assert!(..) }` (1.79) replaces most of `static_assertions`.
