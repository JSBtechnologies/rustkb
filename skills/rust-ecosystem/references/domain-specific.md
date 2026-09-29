---
id: ecosystem/domain-specific
title: Domain crates — TLS/crypto pointers, GUI, WebAssembly, embedded, data/ML, FFI, cloud, files
summary: >-
  Default crate per specialised domain: rustls and RustCrypto (depth deferred to rust-security),
  desktop GUI and games, wasm in the browser and as plugins, embassy for firmware, polars/candle
  for data and ML, bindgen/cxx/pyo3/napi for FFI, cloud SDKs, filesystem and compression.
area: ecosystem
tags: [tls, rustls, crypto, gui, egui, tauri, wasm, wasm-bindgen, embedded, embassy, polars, candle, ml, ffi, pyo3, bindgen, cloud, compression]
rust: "1.96"
edition: "2024"
crates:
  rustls: "0.23"
  tokio-rustls: "0.26"
  rustls-platform-verifier: "0.7"
  rustls-pki-types: "1.15"
  webpki-roots: "1.0"
  aws-lc-rs: "1.18"
  ring: "0.17"
  argon2: "0.6"
  sha2: "0.11"
  blake3: "1.8"
  hmac: "0.13"
  aes-gcm: "0.11"
  chacha20poly1305: "0.11"
  ed25519-dalek: "3.0"
  secrecy: "0.10"
  zeroize: "1.9"
  subtle: "2.6"
  egui: "0.36"
  eframe: "0.36"
  iced: "0.14"
  slint: "1.18"
  tauri: "2.12"
  dioxus: "0.7"
  leptos: "0.8"
  wgpu: "30.0"
  bevy: "0.19"
  wasm-bindgen: "0.2"
  wasm-bindgen-futures: "0.4"
  web-sys: "0.3"
  js-sys: "0.3"
  web-time: "1.1"
  wasmtime: "49.0"
  wasm-pack: "0.15"
  trunk: "0.21"
  embassy-executor: "0.10"
  embassy-time: "0.5"
  embassy-sync: "0.8"
  embedded-hal: "1.0"
  embedded-hal-async: "1.0"
  rtic: "2.3"
  defmt: "1.1"
  heapless: "0.9"
  esp-hal: "1.2"
  probe-rs-tools: "0.32"
  polars: "0.55"
  arrow: "60.0"
  datafusion: "55.1"
  ndarray: "0.17"
  nalgebra: "0.35"
  candle-core: "0.11"
  burn: "0.21"
  tokenizers: "0.23"
  rmcp: "3.5"
  image: "0.25"
  bindgen: "0.73"
  cbindgen: "0.29"
  cxx: "1.0"
  pyo3: "0.29"
  maturin: "1.15"
  napi: "3.13"
  uniffi: "0.32"
  libc: "0.2"
  cc: "1.5"
  aws-sdk-s3: "1.150"
  aws-config: "1.12"
  object_store: "0.14"
  walkdir: "2.5"
  ignore: "0.4"
  notify: "8.2"
  flate2: "1.1"
  zstd: "0.14"
  zip: "8.6"
  nix: "0.31"
  windows: "0.62"
  autocxx: "0.30"
  candle-nn: "0.11"
  candle-transformers: "0.11"
  critical-section: "1.2"
  defmt-rtt: "1.3"
  embassy-nrf: "0.11"
  embassy-rp: "0.10"
  embassy-stm32: "0.6"
  extism: "1.30"
  faer: "0.24"
  getrandom: "0.4"
  glam: "0.33"
  gloo: "0.12"
  gtk4: "0.11"
  napi-derive: "3.6"
  notify-debouncer-full: "0.7"
  opendal: "0.59"
  panic-probe: "1.0"
  parquet: "60.0"
  duckdb: "1.10505"
  rustix: "1.1"
  signal-hook: "0.4"
  ctrlc: "3.5"
  static_cell: "2.1"
  tar: "0.4"
  wasmer: "7.4"
  windows-sys: "0.61"
  winit: "0.30"
  yew: "0.23"
  password-hash: "0.6"
verified: 2026-09-29
sources:
  - https://docs.rs/rustls/latest/rustls/
  - https://github.com/RustCrypto
  - https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org
  - https://embassy.dev/
  - https://pyo3.rs/
  - https://arewegameyet.rs/
---

# Domain crates

One section per domain; each gives the default and the main alternatives. Security depth
(crypto design, key handling, TLS configuration review) belongs to the `rust-security` skill.

## DOM-01: TLS — rustls by default

Default: `rustls` (0.23) everywhere: HTTP clients (`reqwest` 0.13 uses it by
default), servers, database drivers (`tls-rustls` features), `tokio-rustls` (0.26)
for raw streams. Its default crypto provider is `aws-lc-rs` (1.18); the `ring`
provider avoids aws-lc's cmake/NASM build requirements on some cross targets.

- Root certificates: `rustls-platform-verifier` (0.7) to use the
  OS verifier/trust store (enterprise CAs work), or `webpki-roots` (1.0) to
  embed Mozilla's roots for hermetic containers.
- PEM loading: `rustls-pki-types` (1.15) `PemObject`; `rustls-pemfile` is
  archived (RUSTSEC-2025-0134).

```rust
use std::path::Path;

use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

type TlsMaterial = (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>);

fn load_tls(cert: &Path, key: &Path) -> Result<TlsMaterial, rustls_pki_types::pem::Error> {
    let certs = CertificateDer::pem_file_iter(cert)?.collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(key)?;
    Ok((certs, key))
}
```

Use `openssl`/`native-tls` only when a platform or compliance rule mandates the OS/OpenSSL
stack; they complicate static linking and cross-compilation. FIPS: aws-lc-rs has a FIPS build.

## DOM-02: Cryptographic primitives — pointers only

Default: high-level, audited constructions — never compose primitives yourself. Details,
parameters and review checklists: `rust-security`.

| Need | Default crate |
|---|---|
| Password hashing | `argon2` (0.6) — Argon2id via the `password-hash` API |
| Hash (SHA-2) | `sha2` (0.11); fast content hashing: `blake3` (1.8) |
| MAC | `hmac` (0.13) |
| AEAD encryption | `aes-gcm` (0.11) or `chacha20poly1305` (0.11); or `aws-lc-rs` AEAD |
| Signatures | `ed25519-dalek` (3.0) |
| Secrets in memory | `secrecy` (0.10), `zeroize` (1.9) |
| Constant-time compare | `subtle` (2.6) |

- The RustCrypto crates moved to a new major generation in 2026 (`sha2` 0.11 and `hmac` 0.13
  on `digest` 0.11; `aes-gcm` 0.11 on `aead` 0.6/`cipher` 0.5; `argon2` 0.6 on
  `password-hash` 0.6; `ed25519-dalek` 3.0 on `curve25519-dalek` 5). Keep all RustCrypto
  crates on the same generation — mixing `sha2` 0.10 with `hmac` 0.13 fails with trait-bound
  errors, and examples from memory usually target the previous generation.
- Never: `rust-crypto` (dead since 2016), `sodiumoxide` (deprecated), MD5/SHA-1 for security,
  hand-rolled AES modes, `rand::random` for keys (use an OS CSPRNG).

## DOM-03: Desktop GUI, graphics and games

| Need | Default | Notes |
|---|---|---|
| App with a web-technology UI | `tauri` (2.12) | 2.x; system webview, desktop + mobile |
| Tools, editors, debug UIs, quick internal apps | `egui` (0.36) + `eframe` (0.36) | immediate mode; also runs in the browser |
| Native retained-mode app (Elm architecture) | `iced` (0.14) | |
| Declarative UI with designer tooling, embedded displays | `slint` (1.18) | GPL / royalty-free / commercial licences — check fit |
| React-like UI across web/desktop/mobile | `dioxus` (0.7) | |
| GPU rendering / compute | `wgpu` (30.0) | windowing via `winit` |
| Games | `bevy` (0.19) | ECS; breaking release every few months — pin minor |

GTK (`gtk4`) and Qt bindings exist for platform-native looks; they bring large C/C++
dependencies. Don't pick a GUI toolkit an agent can't build on the target OS in CI.

## DOM-04: WebAssembly in the browser

Default: `wasm-bindgen` (0.2) + `web-sys` (0.3) / `js-sys`
(0.3) + `wasm-bindgen-futures` (0.4), built with `trunk`
(0.21) for apps or `wasm-pack` (0.15) for npm packages.

- The `rustwasm` GitHub organisation was sunset in 2025: `wasm-bindgen` moved to its own
  `wasm-bindgen` organisation with new maintainers, `wasm-pack` to its original maintainer,
  `gloo` to its maintainer; other rustwasm repos were archived. Old links to
  `github.com/rustwasm/...` are stale, and archived templates shouldn't be used as a base.
- `web-sys` exposes each Web API behind a Cargo feature (`features = ["Window", "Document",
  "HtmlCanvasElement"]`); missing features are the #1 compile error.
- `std::time::Instant` panics on `wasm32-unknown-unknown`; use `web-time` (1.1).
  `instant` is unmaintained (RUSTSEC-2024-0384).
- Randomness: `getrandom` needs its wasm JS backend enabled for `wasm32-unknown-unknown`
  (see the getrandom docs for the current feature/cfg name).
- Don't use `wee_alloc` (unmaintained, leaks) or `stdweb` (dead).
- Full-stack web apps in Rust: `leptos` (0.8) (fine-grained reactivity, SSR) or
  `dioxus`; `yew` is older and slower-moving.

## DOM-05: WebAssembly as a plugin/sandbox runtime

Default: `wasmtime` (49.0) with the component model and WASI for embedding
untrusted plugins. Note wasmtime releases a new major monthly and currently requires the
latest stable Rust (MSRV 1.96 for 49.0) — pin it and budget for upgrades.
`wasmer` is an alternative with different packaging/licensing trade-offs; `extism` wraps
wasm plugins with a higher-level host API.

## DOM-06: Embedded and no_std

Default for async firmware: the Embassy stack — `embassy-executor` (0.10),
`embassy-time` (0.5), `embassy-sync` (0.8) and the HAL for your
chip family (`embassy-stm32`, `embassy-rp`, `embassy-nrf`, or Espressif's `esp-hal`
(1.2)).

| Need | Crate |
|---|---|
| Portable driver traits | `embedded-hal` (1.0) 1.0 + `embedded-hal-async` (1.0) |
| Interrupt-priority, hard real-time scheduling | `rtic` (2.3) |
| Logging over RTT | `defmt` (1.1) + `defmt-rtt` + `panic-probe` |
| Fixed-capacity collections | `heapless` (0.9) |
| Flash/debug/run | `probe-rs` tools (0.32): `cargo embed`, `probe-rs run` |

- Drivers written against `embedded-hal` 0.2 need porting to 1.0 (traits were reorganised).
- Keep `alloc` out unless needed; prefer static allocation (`static_cell`).
- `bare-metal` is deprecated (RUSTSEC, 2026-04); `critical-section` is the portable
  replacement for critical sections.

## DOM-07: Data processing and numerics

| Need | Default |
|---|---|
| DataFrames, ETL, analytics | `polars` (0.55) — prefer the lazy API; enable only needed features |
| Columnar interchange | `arrow` (60.0) (+ `parquet` crate for Parquet) |
| Embedded SQL engine over Arrow | `datafusion` (55.1); or `duckdb` for DuckDB |
| N-dimensional arrays | `ndarray` (0.17) |
| Linear algebra / geometry | `nalgebra` (0.35); `faer` for large dense decompositions; `glam` for graphics math |
| Images | `image` (0.25) |

polars and arrow/datafusion release breaking versions frequently; pin the minor and keep them
in sync with each other (arrow and parquet share version numbers).

## DOM-08: Machine learning and AI

| Need | Default |
|---|---|
| Transformer/LLM inference in Rust (CPU/CUDA/Metal) | `candle-core` (0.11) + `candle-nn`/`candle-transformers` |
| Training and inference with multiple backends | `burn` (0.21) |
| ONNX models in production | `ort` (ONNX Runtime bindings) — only 2.0 release candidates are published; pin exactly |
| Tokenization | `tokenizers` (0.23) |
| Model Context Protocol servers/clients | `rmcp` (3.5) — the official Rust MCP SDK |
| Calling hosted LLM APIs | the provider's HTTP API via `reqwest` + serde, or a maintained SDK crate |

## DOM-09: FFI and language bindings

| Direction | Default |
|---|---|
| Call C from Rust | `bindgen` (0.73) in `build.rs` + `cc` (1.5) for vendored C sources; publish as a `*-sys` crate |
| Expose Rust to C | `cbindgen` (0.29) generating headers; `extern "C"` + `#[unsafe(no_mangle)]` (edition 2024) |
| C++ ↔ Rust | `cxx` (1.0) — safe shared bridge; `autocxx` for larger C++ APIs |
| Python extensions | `pyo3` (0.29) + `maturin` (1.15) for building wheels |
| Node.js addons | `napi` (3.13) + `napi-derive` (napi-rs 3.x) |
| Kotlin/Swift/Python from one interface | `uniffi` (0.32) |
| Raw platform types | `libc` (0.2); Windows: `windows` (0.62) / `windows-sys`; Unix: `nix` (0.31) |

```rust
use pyo3::prelude::*;

/// Sum a list of integers in Rust.
#[pyfunction]
fn sum_as_string(values: Vec<i64>) -> PyResult<String> {
    Ok(values.iter().sum::<i64>().to_string())
}

#[pymodule]
fn fastmath(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sum_as_string, m)?)?;
    Ok(())
}
```

pyo3's API changed substantially across 0.2x releases (the `Bound<'py, T>` API replaced GIL
refs); code from memory using `&PyModule`/`Python::acquire_gil` won't compile. Unsafe/FFI
soundness rules are in `rust-idioms` and `rust-security`.

## DOM-10: Cloud SDKs and object storage

- AWS: official `aws-sdk-*` crates (`aws-sdk-s3` 1.150) + `aws-config`
  (1.12) for credentials/region. They release very frequently; update together.
- Provider-agnostic object storage (S3, GCS, Azure, local): `object_store` (0.14);
  `opendal` for dozens of storage backends behind one API.
- GCP/Azure: prefer the official SDK crates where they exist; the community `google-apis-rs`
  project is unmaintained (RUSTSEC, 2025-09).

## DOM-11: Files, compression and OS APIs

| Need | Default |
|---|---|
| Walk a directory tree | `walkdir` (2.5); gitignore-aware/parallel: `ignore` (0.4) |
| Watch for file changes | `notify` (8.2) (+ `notify-debouncer-full`) |
| gzip/deflate | `flate2` (1.1) (pure-Rust backend by default) |
| zstd | `zstd` (0.14) |
| ZIP archives | `zip` (8.6) — fast-moving majors; `zip-extract` is unmaintained |
| tar | `tar` |
| Unix syscalls | `nix` (0.31) or `rustix` |
| Windows APIs | `windows` (0.62) (COM/WinRT) or `windows-sys` (raw, faster compile) |
| File locking | `std::fs::File::lock`/`try_lock` (stable 1.89) before adding `fs4` |
| Signals | `tokio::signal` in async apps; `signal-hook` / `ctrlc` in sync apps |
