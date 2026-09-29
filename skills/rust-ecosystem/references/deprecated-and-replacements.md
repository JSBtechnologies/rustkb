---
id: ecosystem/deprecated-and-replacements
title: Deprecated crates and patterns, and their replacements
summary: >-
  Lookup table of crates and APIs that coding agents still reach for but that are deprecated,
  unmaintained, archived, or superseded by std — with the modern replacement, reason and date.
area: ecosystem
tags: [deprecated, unmaintained, replacement, migration, rustsec, lazy_static, structopt, serde_yaml, bincode, async-std]
rust: "1.96"
edition: "2024"
crates:
  clap: "4.6"
  thiserror: "2.0"
  anyhow: "1.0"
  tokio: "1.53"
  smol: "2.0"
  serde: "1.0"
  serde-saphyr: "1.3"
  serde_norway: "0.9"
  postcard: "1.1"
  wincode: "0.6"
  ciborium: "0.2"
  dotenvy: "0.15"
  owo-colors: "4.4"
  ratatui: "0.30"
  tempfile: "3.27"
  web-time: "1.1"
  pastey: "0.2"
  manyhow: "0.14"
  derive-where: "1.7"
  educe: "0.8"
  rustls-pki-types: "1.15"
  backon: "1.6"
  hickory-resolver: "0.26"
  reqwest: "0.13"
  ureq: "3.4"
  axum: "0.8"
  rustc-hash: "2.1"
  compact_str: "0.10"
  imbl: "7.0"
  bacon: "3.26"
  opentelemetry-otlp: "0.33"
  redb: "4.3"
  jiff: "0.2"
  cc: "1.5"
  memmap2: "0.9"
  terminal_size: "0.4"
  similar: "3.2"
  adler2: "2.0"
  yaml-rust2: "0.13"
  rustls: "0.23"
  sha2: "0.11"
  aes-gcm: "0.11"
  gungraun: "0.20"
  anstream: "1.0"
  askama: "0.16"
  aws-lc-rs: "1.18"
  bitcode: "0.6"
  bon: "3.10"
  directories: "6.0"
  dirs: "7.0"
  lexopt: "0.3"
  minijinja: "2.24"
  moka: "0.12"
  rkyv: "0.8"
  smol_str: "0.3"
  unicode-normalization: "0.1"
  unicode-segmentation: "1.13"
  unicode-ident: "1.0"
  unit-prefix: "0.5"
  wasm-bindgen: "0.2"
  web-sys: "0.3"
  wasm-encoder: "0.259"
  wasmparser: "0.259"
  watchexec-cli: "2.7"
  tracing: "0.1"
  tracing-subscriber: "0.3"
  winnow: "1.0"
  tonic-prost: "0.14"
  tonic-prost-build: "0.14"
  hyper-util: "0.1"
  http-body-util: "0.1"
  syn: "3.0"
  sqlx: "0.9"
  pyo3: "0.29"
  embedded-hal: "1.0"
  rand: "0.10"
  opentelemetry: "0.33"
  saphyr: "0.1"
verified: 2026-09-29
sources:
  - https://rustsec.org/advisories/
  - https://github.com/rustsec/advisory-db
  - https://doc.rust-lang.org/stable/releases.html
  - https://github.com/dtolnay/serde-yaml
  - https://github.com/rust-lang/cfg-if
---

# Deprecated crates and patterns, and their replacements

Status was verified on 2026-09-29 against crates.io, GitHub (archived flag) and the RustSec
advisory database. When you touch code that uses anything in the left column, migrate it if
the change is mechanical; otherwise leave a `TODO` naming the replacement. The rustkb MCP
tool `check_advisories` gives live RustSec status; `catalog.toml` marks these `tier = "avoid"`.

## REPL-01: Superseded by the standard library

| Old | Replacement | Why | Since |
|---|---|---|---|
| `lazy_static` | `std::sync::LazyLock` | std has lazy statics | Rust 1.80 (2024-07) |
| `once_cell::sync::Lazy` / `unsync::Lazy` | `std::sync::LazyLock` / `std::cell::LazyCell` | upstreamed | 1.80 |
| `once_cell::sync::OnceCell` | `std::sync::OnceLock` | upstreamed | 1.70 (2023-06) |
| `atty` | `std::io::IsTerminal` | unmaintained + unsound (RUSTSEC-2024-0375) | 1.70 |
| `is-terminal` | `std::io::IsTerminal` | polyfill for the std trait | 1.70 |
| `num_cpus` | `std::thread::available_parallelism` | std | 1.59 |
| `crossbeam::scope` | `std::thread::scope` | std | 1.63 |
| `crossbeam-channel` (plain MPSC only) | `std::sync::mpsc` | std reimplemented on crossbeam's design | 1.67 |
| `async-trait` (static dispatch only) | native `async fn` in traits | language feature; still need `async-trait` for `dyn` | 1.75 |
| `memoffset` | `core::mem::offset_of!` | std | 1.77 |
| `static_assertions::const_assert!` | `const { assert!(...) }` | inline const | 1.79 |
| `matches` crate | `matches!` | std | 1.42 |
| `cfg-if` | `cfg_select!` | std macro; cfg-if repo archived 2026-09 | 1.95 |
| `fs2`/`fs4` (basic advisory locks) | `File::lock` / `try_lock` / `lock_shared` | std | 1.89 |
| `os_pipe` | `std::io::pipe` | std | 1.87 |
| `home` / `dirs` just for `$HOME` | `std::env::home_dir` | fixed (1.85) and un-deprecated | 1.87 |
| `criterion::black_box` | `std::hint::black_box` | std | 1.66 |
| `backtrace` (capture/print only) | `std::backtrace::Backtrace` | std | 1.65 |
| `itertools::repeat_n` | `std::iter::repeat_n` | std | 1.82 |

## REPL-02: Deprecated or unmaintained crates

| Old | Replacement | Why | Since |
|---|---|---|---|
| `structopt` | `clap` derive | merged into clap 3; maintenance mode (RUSTSEC-2022-0104) | 2022 |
| `failure` | `thiserror` + `anyhow` | deprecated (RUSTSEC-2020-0036) | 2020 |
| `error-chain`, `quick-error` | `thiserror` + `anyhow` | unmaintained / superseded | ~2020 |
| `async-std` | `tokio` (or `smol`) | discontinued, points to smol (RUSTSEC-2025-0052) | 2025-08 |
| `surf`, `tide` | `reqwest`/`ureq`, `axum` | unmaintained async-std-era crates (RUSTSEC-2025-0036, RUSTSEC-2026-0170) | 2025–2026 |
| `iron`, `nickel`, `gotham` | `axum` | unmaintained frameworks (iron: RUSTSEC-2025-0061) | — |
| `serde_yaml` | `serde-saphyr` (new code) / `serde_norway` (drop-in) | deprecated by author, archived | 2024-03 |
| `serde_yml` | `serde-saphyr` | unsound + unmaintained, archived (RUSTSEC-2025-0068) | 2025 |
| `yaml-rust` | `saphyr` / `yaml-rust2` | unmaintained (RUSTSEC-2024-0320) | 2024 |
| `bincode` (any version; `3.0.0` is a compile-error tombstone) | `postcard`, `wincode` (bincode-compatible), `bitcode`, `rkyv` | development ceased (RUSTSEC-2025-0141) | 2025-12 |
| `serde_cbor` | `ciborium` | unmaintained (RUSTSEC-2021-0127) | 2021 |
| `rustc-serialize` | `serde` | unmaintained (RUSTSEC-2025-0025) | — |
| `dotenv` | `dotenvy` | unmaintained (RUSTSEC-2021-0141) | 2021 |
| `ansi_term` | `owo-colors` / `anstream` | unmaintained (RUSTSEC-2021-0139) | 2021 |
| `tui` (tui-rs) | `ratatui` | unmaintained (RUSTSEC-2023-0049) | 2023 |
| `term_size` | `terminal_size` | unmaintained (RUSTSEC-2020-0163) | 2020 |
| `tempdir` | `tempfile::TempDir` | deprecated (RUSTSEC-2018-0017) | 2018 |
| `instant` | `web-time` (wasm) / `std::time` | unmaintained (RUSTSEC-2024-0384) | 2024 |
| `paste` | `pastey` | archived (RUSTSEC-2024-0436) | 2024 |
| `proc-macro-error`, `proc-macro-error2` | `manyhow` or `syn::Error::to_compile_error` | unmaintained (RUSTSEC-2024-0370, RUSTSEC-2026-0173) | 2024 / 2026 |
| `derivative` | `derive-where` / `educe` | unmaintained (RUSTSEC-2024-0388) | 2024 |
| `rustls-pemfile` | `rustls-pki-types` `PemObject` | archived (RUSTSEC-2025-0134) | 2025-08 |
| `backoff` | `backon` | unmaintained (RUSTSEC-2025-0012) | 2025 |
| `trust-dns-resolver` / `trust-dns-proto` | `hickory-resolver` | project renamed | 2023 |
| `fxhash` | `rustc-hash` | unmaintained (RUSTSEC-2025-0057) | 2025 |
| `smartstring` | `compact_str` / `smol_str` | unmaintained, archived (RUSTSEC-2026-0249) | 2026-05 |
| `im` / `im-rc` | `imbl` | unmaintained, archived (RUSTSEC-2026-0248) | 2026-05 |
| `number_prefix` | `unit-prefix` (what indicatif switched to) | unmaintained (RUSTSEC, 2025-11) | 2025 |
| `gumdrop` | `clap` / `lexopt` | unmaintained (RUSTSEC, 2026-07) | 2026 |
| `unic-*` | `unicode-segmentation`, `unicode-normalization`, `unicode-ident` … | unmaintained family (RUSTSEC, 2025-10) | 2025 |
| `difference` | `similar` | unmaintained (RUSTSEC-2020-0095) | 2020 |
| `adler` | `adler2` | unmaintained | 2025 |
| `mmap` | `memmap2` | unmaintained | 2024 |
| `gcc` (build crate) | `cc` | renamed | 2018 |
| `directories-next`, `dirs-next` | `directories`, `dirs` | the `-next` forks stopped at 2.0.0 (2020); originals are maintained (now on Codeberg) | — |
| `rust-crypto` | RustCrypto crates (`sha2`, `aes-gcm`, …) or `aws-lc-rs` | abandoned 2016 with known issues | 2016 |
| `sodiumoxide` | RustCrypto / `aws-lc-rs` | deprecated (RUSTSEC-2021-0137) | 2021 |
| `wee_alloc` | default allocator | unmaintained, leaks (RUSTSEC-2022-0054) | 2022 |
| `stdweb`, `parity-wasm` | `wasm-bindgen`/`web-sys`; `wasm-encoder`/`wasmparser` | unmaintained / deprecated | 2020 / 2022 |
| `tokio-core`, `tokio-io`, `tokio-*` 0.1 crates | `tokio` 1.x | tokio 0.1 era crates flagged unmaintained (2026-03) | 2026 |
| `opentelemetry-jaeger` | `opentelemetry-otlp` (Jaeger ingests OTLP) | unmaintained (RUSTSEC, 2025-11) | 2025 |
| `opentelemetry_api` | `opentelemetry` | merged back into the main crate | 2024 |
| `cargo-watch` | `bacon` (or `watchexec`) | repository archived | 2025-01 |
| `iai-callgrind` | `gungraun` | project renamed (same repository) | — |
| `sled` | `redb` (or SQLite via `rusqlite`) | stuck in 0.34 beta, unstable format | — |

## REPL-03: Maintained but not the default

These are not dead — don't rip them out of working code — but don't pick them for new work
without a reason.

| Crate | Prefer for new code | Reason |
|---|---|---|
| `warp` | `axum` | filter types hurt errors/compile times; ecosystem moved to axum |
| `rocket` | `axum` | last release 0.5.1 (2024-05) |
| `chrono` + `chrono-tz` | `jiff` | jiff has built-in tz handling and DST-correct arithmetic; keep chrono where APIs require it |
| `openssl` / `native-tls` as default TLS | `rustls` | pure Rust, no system libs; reqwest 0.13 made it the default |
| `ring` directly | `aws-lc-rs` / RustCrypto | rustls' default provider is aws-lc-rs; ring remains maintained |
| `log` + `env_logger` in services | `tracing` + `tracing-subscriber` | spans and structured fields |
| `colored` | `owo-colors` / `anstream` | global state, TTY handling |
| `derive_builder` | `bon` | compile-time checked builders |
| `handlebars`, `tera` | `minijinja` / `askama` | lighter, better errors / compile-time checked |
| `serde_norway`, `serde_yaml_ng` | `serde-saphyr` | libyaml-based forks with sparse releases |
| `nom` | `winnow` | winnow is the nom lineage with better ergonomics and errors (nom is fine in existing code) |
| `once_cell` | `std` | only needed for `race` cells or fallible init not yet in std |

## REPL-04: API-level breaking changes agents write from stale memory

The crate is right but the remembered API is not.

| Crate | Stale code | Current |
|---|---|---|
| `axum` 0.8 | `.route("/users/:id", …)`, `#[async_trait]` on extractors | `"/users/{id}"`, `"/{*rest}"`; extractors use native async fn |
| `hyper` 1.x | `hyper::Server::bind(..)`, `hyper::Body` | `axum::serve` or `hyper-util` server/client; `http-body-util` bodies |
| `reqwest` 0.13 | `features = ["rustls-tls"]`, `.json()`/`.query()` always available | feature `rustls` (now default); enable `json`, `query`, `form` features |
| `ureq` 3.x | `.call()?.into_json()` | `.call()?.body_mut().read_json()` |
| `rand` 0.9/0.10 | `thread_rng()`, `gen()`, `gen_range()`, `use rand::Rng` | `rng()`, `random()`, `random_range()`, `use rand::RngExt` (0.10) |
| `thiserror` 2.x | `thiserror = "1"` | `"2"`; mostly source-compatible |
| `clap` 4.x | `App::new`, `Arg::with_name`, `#[clap(...)]` | `Command::new`, `Arg::new`, `#[arg]`/`#[command]` |
| `syn` 3.x | syn 1/2 macros from memory | check syn release notes; update visitor/parse APIs |
| `pyo3` 0.2x | `&PyModule`, `Python::acquire_gil`, GIL refs | `Bound<'py, T>` API, `#[pymodule] fn m(m: &Bound<'_, PyModule>)` |
| `sqlx` 0.9 | 0.6-era combined features such as `runtime-tokio-rustls`; 0.7/0.8 code from memory | separate `runtime-tokio` + `tls-rustls` (or `tls-rustls-ring-webpki`, `tls-native-tls`, `tls-none`) |
| `tonic` 0.14 | `tonic-build` generating prost code directly | `tonic-prost` / `tonic-prost-build` |
| `opentelemetry` 0.2x–0.33 | `opentelemetry_jaeger`, `new_pipeline()`, `TracerProvider` from `opentelemetry::sdk` | `opentelemetry_otlp::SpanExporter::builder()`, `SdkTracerProvider`; keep all otel crates on one minor |
| `embedded-hal` 1.0 | 0.2 traits (`blocking::i2c::Write`, `digital::v2`) | reorganised 1.0 traits (`i2c::I2c`, `digital::OutputPin`) |
| RustCrypto 2026 majors | `sha2 0.10` + `hmac 0.12` examples | `sha2 0.11`, `hmac 0.13` (digest 0.11); keep one generation |
| `rustls` 0.23 | `ServerConfig::builder().with_safe_defaults()` | `ServerConfig::builder()` (safe defaults implied); crypto provider via features |

## REPL-05: Patterns to replace

| Pattern | Replacement |
|---|---|
| `lazy_static! { static ref RE: Regex = … }` | `static RE: LazyLock<Regex> = LazyLock::new(|| …)` |
| `Box<dyn Error + Send + Sync>` in a library's public API | a `thiserror` enum |
| `Arc<Mutex<HashMap<..>>>` as a cache | `moka` with eviction |
| New `reqwest::Client` per request | one shared `Client` |
| `std::thread::sleep` in async code | `tokio::time::sleep` |
| `tokio::spawn` + hand-rolled `AtomicBool` shutdown | `tokio_util::sync::CancellationToken` + `TaskTracker` |
| `chrono::Local::now()` in servers | UTC timestamps (`jiff::Timestamp::now()`, `Utc::now()`) |
| `format!` building SQL | bound parameters (`sqlx::query(...).bind(..)`) |
| `tokio = { features = ["full"] }` in a library | only the features used |
| `println!`-based logging | `tracing` macros |
