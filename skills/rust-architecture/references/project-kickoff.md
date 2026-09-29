---
id: architecture/project-kickoff
title: Project kickoff decisions
summary: >-
  The questions an agent must answer before writing the first line of a new Rust project — package
  shape (bin, lib, workspace), edition/MSRV/toolchain, sync vs async, error strategy, config,
  observability, target platforms — with an opinionated default for each.
area: architecture
tags: [kickoff, new-project, cargo-new, workspace, msrv, edition, async, architecture, checklist]
rust: "1.96"
edition: "2024"
crates:
  tokio: "1.53"
  rayon: "1.12"
  thiserror: "2.0"
  anyhow: "1.0"
  tracing: "0.1"
  tracing-subscriber: "0.3"
  clap: "4.6"
  axum: "0.8"
  rustls: "0.23"
  tonic: "0.14"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/cargo/guide/project-layout.html
  - https://doc.rust-lang.org/cargo/reference/rust-version.html
  - https://doc.rust-lang.org/edition-guide/rust-2024/index.html
---

# Project kickoff decisions

Use this when asked to "start", "scaffold", "bootstrap" or "set up" a Rust project. Answer the
questions below in order, write the answers down (README "Architecture" section or
`docs/adr/0001-kickoff.md`), then scaffold. Every later file in this skill assumes these answers.

## The kickoff flow

### ARCH-01: Answer the kickoff questions before scaffolding

Default: walk this table top to bottom. If the user did not specify an answer, take the default and
state it explicitly in your reply so it can be corrected cheaply.

| # | Question | Default | Deviate when |
|---|---|---|---|
| 1 | What is the deliverable? | One binary **or** one library | Several deployables → workspace (ARCH-03) |
| 2 | Package shape? | Single package; bin gets `src/main.rs` + `src/lib.rs` | See ARCH-02/03 |
| 3 | Edition / resolver? | `edition = "2024"`, resolver 3 (implied) | Never start new code on an older edition |
| 4 | Toolchain / MSRV? | Apps: pin stable in `rust-toolchain.toml`. Libs: declare `rust-version` | ARCH-05 |
| 5 | Async? | Only for concurrent network IO (servers, many clients) | ARCH-06 |
| 6 | Error strategy? | `thiserror` in library/domain code, `anyhow` in `main` | ARCH-07 |
| 7 | Config? | Typed struct, loaded once in `main`, passed down | `configuration.md` |
| 8 | Observability? | `tracing` everywhere, subscriber only in binaries | `observability.md` |
| 9 | Target platforms? | Linux x86_64 + the dev's OS; Windows/macOS if users run it | ARCH-10 |
| 10 | Distribution? | Services: container image. CLIs: `cargo-dist`. Libs: crates.io via `release-plz` | `ci-cd-release.md` |
| 11 | CI gates? | fmt, clippy `-D warnings`, test, doc, cargo-deny (+ semver/MSRV for libs) | `ci-cd-release.md` |

❌ Common agent failure: generating ten crates, a trait per struct and a DI container for a
200-line tool. The right amount of architecture for day one is the smallest shape that keeps the
next six months of growth cheap — usually one package with a clean module tree.

## Package shape: binary, library, or workspace

### ARCH-02: Start with one package; give binaries a thin `main.rs` over a `lib.rs`

Default: `cargo new --bin app`, then move logic into `src/lib.rs` and keep `src/main.rs` to
argument parsing, config loading, telemetry init, wiring, and exit-code mapping.

```text
app/
├── Cargo.toml
├── src/
│   ├── main.rs        # ~50–150 lines: parse → load config → init tracing → build → run
│   ├── lib.rs         # pub API used by main.rs, integration tests and benches
│   ├── config.rs
│   └── <domain modules>.rs
└── tests/
    └── cli.rs         # integration tests exercise lib.rs or the built binary
```

Why: integration tests in `tests/` and benches can only import a library target; a logic-heavy
`main.rs` is untestable without spawning the binary.

Use a library-only package (`cargo new --lib`) when there is no executable. Never put business
logic in `build.rs`.

### ARCH-03: Use a workspace only when there is a second real unit

Default: become a workspace when **any** of these is true:

- Two or more deployables (API server + worker, server + CLI) share code.
- A reusable core is published or consumed separately from the application.
- A proc-macro is needed (it must be its own crate).
- A heavy optional dependency (DB driver, GUI toolkit) should compile only for one binary.
- Measured build times (`cargo build --timings`) show one huge crate on the critical path.

Never split by technical layer alone (`models`, `services`, `utils` crates) and never create a
crate per struct or per endpoint. 3–8 crates is typical for a mid-sized service; 20+ needs a
reason. Layout and manifests: `workspace-layout.md`.

## Edition, toolchain and MSRV

### ARCH-04: New code uses edition 2024 and resolver 3

Default: `edition = "2024"` (requires Rust 1.85+). In a single package resolver 3 is implied by the
edition; in a virtual workspace manifest you must write `resolver = "3"` yourself or Cargo falls
back to resolver 1 with a warning. Resolver 3 makes dependency resolution MSRV-aware
(`incompatible-rust-versions = "fallback"`).

Never start new code on 2018/2021 because an example or a memory of an old tutorial did.
Edition-specific language changes: see `idioms/editions-msrv`.

### ARCH-05: Applications pin a toolchain; libraries declare an MSRV

| Project | `rust-toolchain.toml` | `package.rust-version` | CI |
|---|---|---|---|
| Application / service / CLI | Pin `channel = "1.96"` (current stable); bump deliberately | Same as the pin, or omit | Build with the pinned toolchain |
| Library (published) | Don't pin a version (contributors use latest stable) | Oldest supported, e.g. `"1.85"` | Extra job on the MSRV toolchain |
| Internal library in an app workspace | Inherits the workspace pin | Inherit from `[workspace.package]` | Covered by the app build |

Why: apps benefit from reproducible builds; libraries must not force a toolchain on downstream
users. MSRV policy and verification: `library-design.md` (LIB-09). Templates: `templates.md` (TPL-03).

## Sync or async

### ARCH-06: Choose async only for concurrent network IO

Default decision:

| Workload | Choice |
|---|---|
| HTTP/gRPC server, proxy, many concurrent sockets | `tokio` (multi-thread runtime) + `axum`/`tonic` |
| CLI doing a handful of HTTP calls | Blocking client is fine; async only if you fan out many requests |
| CPU-bound batch work (parsing, compression, image processing) | Sync + `rayon` |
| File-processing CLI | Sync `std::fs`; parallelise with `rayon` if needed |
| Library | Sync core; offer async only where IO is inherent. Never start a runtime inside a library |
| Embedded / `no_std` | `embassy` or bare-metal, not tokio |

Never mix runtimes in one process, never call `block_on` inside async code, and never make
everything `async fn` "just in case" — async colours every caller. Runtime usage rules: see
`idioms/async`.

## Errors, config, observability

### ARCH-07: Typed errors inside, `anyhow` at the edge

Default: domain/library crates define `thiserror` enums per operation family; the binary's
`main`/`run` returns `anyhow::Result` and adds `.context()` at IO boundaries. HTTP services map
domain errors to responses in exactly one place (`web-services.md` SVC-04). CLIs map errors to exit
codes in `main` (`cli-apps.md` CLI-02). Never `unwrap()` in `main`; never `Box<dyn Error>` in a
library API. Details: `idioms/error-handling`.

### ARCH-08: Tracing from the first commit, initialised only in `main`

Default: every crate depends on `tracing` for events/spans; only binaries depend on
`tracing-subscriber` and install it, once, at the top of `main`. Adding observability later means
touching every function; adding it on day one costs one dependency. See `observability.md`.

### ARCH-09: No global mutable state; `main` is the composition root

Default: `main` loads config, builds clients/pools/services, and passes them down explicitly
(function arguments, struct fields, axum `State`). Globals are acceptable only for immutable,
lazily-initialised values that are truly process-wide (a compiled `Regex` in a
`static LazyLock`) and for the tracing/metrics recorders the ecosystem already makes global.

❌ Common agent failure:

```rust,ignore
static DB: OnceLock<Mutex<PgPool>> = OnceLock::new(); // hidden dependency, untestable
```

✅ Pass `PgPool` (already `Clone` + internally `Arc`) through state. Never reach for
`lazy_static!` (superseded by `std::sync::LazyLock`) or a `static mut`.

## Target platforms

### ARCH-10: Decide targets early; they constrain crate choice

Decide on day one, because the answer changes dependencies:

- **Windows support** → no shelling out to `sh`, use `std::path::Path` not string paths, handle
  Ctrl-C without Unix signals (see SVC-05), test on `windows-latest` in CI.
- **Static Linux binaries / musl / cross-compilation** → prefer pure-Rust TLS (`rustls`) over
  OpenSSL; avoid C dependencies you cannot cross-compile.
- **WebAssembly** → no threads/filesystem by default; keep the core `wasm32`-clean and feature-gate
  IO.
- **Embedded / `no_std`** → core crate must be `#![no_std]`; see `library-design.md` (LIB-10).
- **ARM servers** → build natively on ARM CI runners where available (see `ci-cd-release.md`).

## Day-one deliverables

### ARCH-11: Scaffold the guardrails with the code, not later

A new project is "set up" when it has all of these:

- [ ] `Cargo.toml` with `edition = "2024"`, `rust-version`, license, `[lints]` (or
  `[workspace.lints]`) — templates in `templates.md` (TPL-01/TPL-02)
- [ ] `Cargo.lock` committed (applications **and** libraries)
- [ ] `rust-toolchain.toml` for applications (TPL-03)
- [ ] `clippy.toml` allowing `unwrap`/`expect` in tests only (TPL-05)
- [ ] `deny.toml` (TPL-06) and `.github/workflows/ci.yml` (TPL-07) — `templates-ci-and-skeletons.md`
- [ ] `src/main.rs` that returns an error or `ExitCode` instead of panicking
- [ ] tracing subscriber initialised in `main` (binaries)
- [ ] README with: what it is, how to run, config/env vars, the kickoff decisions from ARCH-01

## Decision record template

Record the answers so later agents do not relitigate them:

```markdown
# ADR 0001: Project kickoff
- Deliverable: HTTP API `shop-server` + admin CLI `shop`
- Shape: workspace (2 binaries share `shop-domain`)
- Edition 2024, resolver 3; toolchain pinned to 1.96; MSRV = pinned
- Async: tokio multi-thread (server only); CLI is sync
- Errors: thiserror in shop-domain, anyhow in binaries; ApiError → HTTP in one place
- Config: `config` crate, TOML + APP__* env; secrets via env only
- Observability: tracing + JSON logs in prod; OTLP export optional
- Targets: linux x86_64/aarch64 containers; CLI also Windows/macOS
- Distribution: container image (server), cargo-dist (CLI)
```

## Related references

- Workspace manifests and crate splitting → `workspace-layout.md`
- Module trees and visibility → `modules-and-boundaries.md`
- Ports/adapters without over-abstraction → `layered-hexagonal.md`
- Service skeleton → `web-services.md`; CLI skeleton → `cli-apps.md`; libraries → `library-design.md`
- Crate choices per domain → the `rust-ecosystem` skill; security baseline → the `rust-security` skill
