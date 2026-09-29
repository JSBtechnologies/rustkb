---
name: rust-ecosystem
description: >-
  Choosing Rust crates with verified current versions. Use when adding a dependency or running
  cargo add, asking "which crate for X" or "what's the best Rust library for Y", comparing
  crates (axum vs actix-web, sqlx vs diesel vs sea-orm, jiff vs chrono, reqwest vs ureq),
  replacing deprecated or unmaintained crates (lazy_static, once_cell, structopt, failure,
  async-std, serde_yaml, bincode, atty, dotenv...), reviewing or writing Cargo.toml
  dependencies, versions and features, or picking crates for web, HTTP, async, serialization,
  databases, CLI/TUI, logging/tracing, testing, crypto/TLS, GUI, wasm, embedded, ML/data or FFI.
---

# Rust ecosystem: which crate, which version

Agents fail here in predictable ways: an old major version typed from memory, a deprecated
crate from old training data, a crate std has replaced, or every default feature switched on.
This skill gives the current default per domain, verified against crates.io, GitHub and
RustSec on 2026-09-29 (Rust 1.96, edition 2024).

**`catalog.toml`** (in this skill) is the machine-readable catalog: about 200 crates with
`category`, `tier` (default / recommended / situational / avoid), verified `version`,
`use_for`, `avoid_when`, `alternatives` and `replaces`. The `rustkb` MCP server reads it.
- `recommend_crates` — ranked catalog entries for a need, with **live** crates.io versions.
- `crate_info` — one crate's catalog entry, live version, maintenance and advisory status.
- `check_advisories` — RustSec advisories for a crate or lockfile.
- `search` / `get_item` — versioned API docs for tracked crates.
When these tools are available, prefer them over the static versions in this skill.

## Core rules

| ID | Rule |
|---|---|
| DEP-01 | Check std first: `LazyLock`/`OnceLock`, `IsTerminal`, `available_parallelism`, `thread::scope`, `File::lock`, `cfg_select!`, native `async fn` in traits |
| DEP-03 | Check maintenance before recommending: RustSec advisory, last release, archived repo |
| DEP-04 | Pick the catalog's `tier = "default"` crate unless you can state a reason |
| DEP-05 | Write versions as caret `"MAJOR.MINOR"`; commit `Cargo.lock` |
| DEP-06 | Enable only the features you use; libraries set `default-features = false` on heavy deps |
| DEP-07 | Check `rust-version` (MSRV) of new deps against yours |
| DEP-11 | Never type a version from memory: `cargo add`, `crate_info`, or crates.io |
| DEP-12 | Replace `avoid`-tier crates when you touch the code that uses them |
| NET-01 | Async runtime: tokio; never async-std (discontinued) |
| WEB-01 | Web server: axum + tower-http; HTTP client: reqwest (async, rustls by default since 0.13) or ureq (sync) |
| SER-03 | YAML: serde-saphyr (serde_yaml deprecated, serde_yml unsound) |
| SER-04 | Binary serde: postcard (bincode discontinued; 3.0.0 is a compile-error tombstone) |
| DB-01 | SQL: sqlx (raw SQL, checked); diesel or sea-orm when you want an ORM/DSL |
| TXT-01 | Date/time: jiff for new code; chrono where APIs require it; never `Local::now()` in servers |
| TEL-01 | Instrumentation: tracing + tracing-subscriber; OTel crates all on one minor |
| UTIL-01 | Errors: thiserror in libraries, anyhow at application edges |
| UTIL-07 | rand 0.10: `rand::rng()`, `random_range`, `use rand::RngExt` |
| TST-07 | Test runner: cargo-nextest (+ `cargo test --doc`); coverage: cargo-llvm-cov |
| DOM-01 | TLS: rustls (aws-lc-rs provider); openssl only when mandated |

## Current defaults at a glance (verified 2026-09-29)

| Need | Default | Version |
|---|---|---|
| Async runtime | tokio | 1.53 |
| Web framework / middleware | axum / tower-http | 0.8 / 0.7 |
| HTTP client async / sync | reqwest / ureq | 0.13 / 3.4 |
| gRPC | tonic + prost (codegen: tonic-prost-build) | 0.14 / 0.14 |
| Serialization | serde + serde_json | 1.0 / 1.0 |
| TOML / YAML | toml / serde-saphyr | 1.1 / 1.3 |
| SQL / SQLite | sqlx / rusqlite | 0.9 / 0.40 |
| CLI / TUI | clap (derive) / ratatui | 4.6 / 0.30 |
| Errors | thiserror / anyhow | 2.0 / 1.0 |
| Logging & tracing | tracing / tracing-subscriber | 0.1 / 0.3 |
| Date & time | jiff | 0.2 |
| Parsing | winnow | 1.0 |
| Parallelism | rayon | 1.12 |
| Testing | proptest / insta / rstest | 1.11 / 1.48 / 0.27 |
| TLS | rustls | 0.23 |
| Proc macros | syn / quote | 3.0 / 1.0 |

## Routing: which reference to read

| Situation | Read |
|---|---|
| Should I add this crate? Evaluating maintenance, features, MSRV, license; std replacements | `references/dependency-policy.md` |
| Crate is old/deprecated/unmaintained; migration targets; stale APIs agents write from memory | `references/deprecated-and-replacements.md` |
| tokio, futures, cancellation, channels, retries, WebSockets, gRPC, QUIC, DNS, brokers | `references/async-and-networking.md` |
| Web server, middleware, OpenAPI, sessions/JWT, HTTP clients, email | `references/web-and-http.md` |
| serde, JSON/TOML/YAML/binary formats, CSV/XML/protobuf, config loading, .env | `references/data-and-serialization.md` |
| Dates/times/time zones, regex, parsing (winnow/nom/pest/logos), Unicode, Markdown, templates | `references/text-time-and-parsing.md` |
| SQL toolkits/ORMs, SQLite, Redis, MongoDB, embedded KV, pools, caches | `references/databases.md` |
| CLI parsing, colours, prompts, progress, config dirs, TUIs | `references/cli-and-tui.md` |
| tracing, log, OpenTelemetry, metrics, Sentry, tokio-console | `references/observability-crates.md` |
| Error crates, rayon, locks/concurrent maps, collections, hashing, rand, UUIDs, proc-macro tooling | `references/core-utilities.md` |
| Property/snapshot/parameterized tests, mocks, HTTP fakes, nextest, coverage, benchmarks, profiling | `references/testing-and-benchmarking.md` |
| TLS/crypto pointers, GUI, wasm, embedded, data/ML, FFI (pyo3, napi, cxx, bindgen), cloud, files | `references/domain-specific.md` |
| A crate not covered above | `catalog.toml`, then `recommend_crates` / `crate_info` |

## Related skills

- `rust-idioms` — how to use std and the language well (errors, async, concurrency, testing style).
- `rust-architecture` — workspaces, service/CLI structure, config and observability design, CI.
- `rust-security` — supply-chain policy (cargo-deny/vet), crypto choices in depth, unsafe review, fuzzing.

## Using the catalog directly

```toml
[[crate]]
name = "serde-saphyr"
category = "serialization"
tier = "default"                # default | recommended | situational | avoid
version = "1.3"                 # crates.io major.minor at verification time
replaces = ["serde_yaml", "serde_yml"]
```

- To find the default for a category: filter `tier = "default"` and `category`.
- Before using any crate, check it is not `tier = "avoid"`; if it is, use its `alternatives`.
- `category = "tool"` entries are cargo subcommands to install (`cargo install` /
  `cargo binstall`), never `[dependencies]`.
- Versions drift: the catalog records what was current on the verification date; live
  versions come from `crate_info` or crates.io.
