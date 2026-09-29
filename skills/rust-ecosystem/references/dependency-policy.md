---
id: ecosystem/dependency-policy
title: Dependency policy — evaluating and adding crates
summary: >-
  How to decide whether to add a crate at all (std-first), how to evaluate one (maintenance,
  adoption, unsafe, MSRV, license, features), and how to declare it in Cargo.toml.
area: ecosystem
tags: [dependencies, cargo, crates-io, maintenance, msrv, features, license, std, supply-chain]
rust: "1.96"
edition: "2024"
crates:
  cargo-deny: "0.20"
  cargo-audit: "0.22"
  cargo-machete: "0.9"
  cargo-shear: "1.14"
  cargo-msrv: "0.19"
  cargo-hack: "0.6"
  serde: "1.0"
  tokio: "1.53"
  reqwest: "0.13"
  uuid: "1.26"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html
  - https://doc.rust-lang.org/cargo/reference/features.html
  - https://rustsec.org/
  - https://embarkstudios.github.io/cargo-deny/
  - https://blessed.rs/crates
---

# Dependency policy — evaluating and adding crates

Every dependency is code you ship, compile, audit and upgrade. Coding agents over-add
crates, pick the one they saw most in training data (often deprecated), and enable default
features blindly. This file is the checklist to run *before* `cargo add`.
Supply-chain enforcement depth (cargo-deny config, cargo-vet, reviewing `unsafe`) lives in
the `rust-security` skill; this file covers the selection decision.

## DEP-01: Check std first

Default: if the standard library covers it on Rust 1.96, use std. Several once-essential
crates are now redundant, and agents still add them by reflex.

| Need | Use (std) | Stable since | Instead of |
|---|---|---|---|
| Lazy global / static | `std::sync::LazyLock`, `std::cell::LazyCell` | 1.80 | `lazy_static`, `once_cell::sync::Lazy` |
| Set-once cell | `std::sync::OnceLock`, `std::cell::OnceCell` | 1.70 | `once_cell` |
| "Is stdout a TTY?" | `std::io::IsTerminal` | 1.70 | `atty` (unmaintained), `is-terminal` |
| CPU count | `std::thread::available_parallelism` | 1.59 | `num_cpus` |
| Scoped threads | `std::thread::scope` | 1.63 | `crossbeam::scope` |
| MPSC channel | `std::sync::mpsc` (crossbeam-based since 1.67) | 1.67 | `crossbeam-channel` for plain MPSC |
| `async fn` in traits (static dispatch) | native `async fn` in trait | 1.75 | `async-trait` |
| Field offset | `core::mem::offset_of!` | 1.77 | `memoffset` |
| Compile-time assertion | `const { assert!(...) }` | 1.79 | `static_assertions` (value asserts) |
| `Error` in `no_std` | `core::error::Error` | 1.81 | custom shims |
| Backtraces | `std::backtrace::Backtrace` | 1.65 | `backtrace` for capture-only use |
| Advisory file locks | `File::lock` / `try_lock` / `lock_shared` | 1.89 | `fs2`, `fs4` for basic locking |
| Anonymous pipes | `std::io::pipe` | 1.87 | `os_pipe` |
| Home directory | `std::env::home_dir` (fixed on Windows in 1.85, un-deprecated) | 1.87 | `home`, `dirs` for just the home dir |
| Repeat N items | `std::iter::repeat_n` | 1.82 | `itertools::repeat_n` |
| Pattern test | `matches!` | 1.42 | `matches` crate |
| Benchmark barrier | `std::hint::black_box` | 1.66 | `criterion::black_box` |

✅ Instead of `lazy_static!`:

```rust
use std::collections::HashMap;
use std::sync::LazyLock;

static LIMITS: LazyLock<HashMap<&'static str, u32>> =
    LazyLock::new(|| HashMap::from([("free", 10), ("pro", 1_000)]));
```

Exceptions: `async-trait` is still needed when the trait must be used as `dyn Trait`
(native async-fn traits are not dyn-compatible on 1.96). `once_cell` still offers
`race` cells and fallible init (`get_or_try_init`) that std lacks on stable.

## DEP-02: Justify every new dependency

Default: add a crate only when it replaces meaningful, tricky code (parsing, crypto,
protocols, async runtimes, date/time math). Do not add a crate for a ten-line helper.

Ask, in order:
1. Does std do it (DEP-01)? Does a crate already in `Cargo.lock` do it?
2. Is the problem hard to get right (crypto, time zones, Unicode, HTTP, SQL)? Then use
   the ecosystem default from `catalog.toml` — never hand-roll these.
3. Is this a one-function convenience (`maplit`, `left-pad` equivalents)? Write the code.

Check what is already in the graph before adding: `cargo tree -i <crate>` shows whether
(and which version of) a crate is already pulled in.

## DEP-03: Evaluate maintenance before recommending

Default: recommend only crates with evidence of active maintenance. An abandoned crate
recommended today becomes a RustSec "unmaintained" advisory in CI tomorrow.

Check, cheapest first:
- **RustSec**: any `unmaintained` / `unsound` advisory? (`rustkb` MCP `check_advisories`, or
  `cargo deny check advisories` / `cargo audit`). Advisories are the strongest signal —
  e.g. `bincode` (2025-12), `serde_yml` (2025), `paste` (2024), `rustls-pemfile` (2025).
- **crates.io**: last release date, number of releases, recent downloads, reverse deps.
  `https://crates.io/api/v1/crates/<name>` → `crate.updated_at`, `max_stable_version`,
  `recent_downloads`.
- **Repository**: archived? README deprecation banner? Issues answered? A crate with no
  release for 2+ years is not automatically dead (small, "done" crates like `either` exist),
  but a large crate with an archived repo is.
- **Ecosystem position**: is it the crate that major projects (tokio, rust-lang, rustls,
  RustCrypto, Embark, BurntSushi, dtolnay) depend on?

Red flags: a fork with one maintainer and no releases after the initial fork burst;
crates whose README points elsewhere; crate names ending in `-next`/`-ng`/`2` without a
clear adoption story (check downloads before trusting a fork).

## DEP-04: Prefer the ecosystem default; deviate with a reason

Default: pick the `tier = "default"` crate for the category in `catalog.toml`
(queried via the `rustkb` MCP `recommend_crates`). Defaults interoperate — axum, reqwest,
tonic, sqlx, tower-http and tracing all assume tokio; `serde` derive is expected everywhere.

Choose an alternative only for a stated reason (no_std, compile time, license, existing
codebase, a specific feature). Write the reason in the PR or a comment next to the dependency.

## DEP-05: Pin versions as caret major.minor

Default: `crate = "MAJOR.MINOR"` (caret semantics), with the minor you actually need.

```toml
[dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "net", "signal"] }
```

- Never write `*` or unbounded `>=` requirements.
- Don't pin exact (`=1.2.3`) in libraries — it breaks resolution for downstream users.
  Exact pins are for proc-macro pairs that require lockstep versions, or a temporary
  workaround with a comment.
- Commit `Cargo.lock` for binaries *and* libraries (Cargo's current guidance); CI
  reproducibility matters more than the old library exception.
- For 0.x crates, the minor is the breaking axis: `"0.13"` accepts `0.13.*` only.
- Workspaces: declare shared versions once in `[workspace.dependencies]` and use
  `crate.workspace = true` in members (details: `rust-architecture` skill).

## DEP-06: Minimise features; use default-features = false deliberately

Default: enable only the features you use. Default features often pull in TLS stacks,
runtimes, or codecs you don't need.

- **Libraries**: set `default-features = false` on heavy dependencies and re-expose
  choices as your own features, so applications decide (e.g. which TLS backend).
- **Applications**: use `tokio` with explicit features instead of `"full"` when binary
  size/compile time matter; `"full"` is acceptable for prototypes.
- Some crates *require* opting in: `reqwest` 0.13 gates `json`, `query` and `form`
  behind features; `serde` needs `derive`; `uuid` needs `v4`/`v7`; `web-sys` needs one
  feature per Web API.

```toml
[dependencies]
reqwest = { version = "0.13", default-features = false, features = ["rustls", "json"] }
uuid = { version = "1.26", features = ["v7", "serde"] }
```

Check what features resolve with `cargo tree -e features -i <crate>`; verify feature
combinations in libraries with `cargo hack check --feature-powerset`.

## DEP-07: Respect MSRV and edition

Default: new crates use `edition = "2024"` and set `rust-version` (MSRV) explicitly.
Before adding a crate, compare its `rust-version` with yours: Cargo's MSRV-aware
resolver (`resolver = "3"`, the default for edition 2024) picks versions compatible with
your `rust-version`, which can silently hold you on old releases.

- Libraries: keep MSRV conservative (N-2 to N-6 stable releases) and verify with
  `cargo msrv verify` or a CI job on the MSRV toolchain.
- Applications: track latest stable; MSRV is mostly a documentation concern.
- Some ecosystem crates move MSRV fast: wasmtime's latest release requires 1.96,
  serde-saphyr 1.89, typos-cli 1.95; tokio's README announces MSRV 1.85 for releases after
  1.53. Check `rust-version` before promising an old MSRV.

## DEP-08: Check license compatibility

Default: the ecosystem norm is `MIT OR Apache-2.0`; these are compatible with almost
everything. Flag before adding: GPL/AGPL (copyleft: `slint` offers GPL/royalty-free/commercial
terms), `MPL-2.0` (file-level copyleft), `Unicode-3.0`, `OpenSSL`, `BSL`/source-available
licenses, and crates with no license field. Enforce an allow-list with `cargo deny check licenses`.

## DEP-09: Weigh unsafe, build scripts and native dependencies

Default: prefer pure-Rust crates when an equivalent exists; native (C/C++) dependencies
complicate cross-compilation, static linking, Windows builds and supply-chain review.

- TLS: `rustls` over `openssl`/`native-tls` unless a platform stack is mandated.
- YAML: `serde-saphyr` (pure Rust) over libyaml-based forks for new code.
- Compression: `flate2` with its default pure-Rust backend.
- `build.rs` that downloads files or probes the system (`*-sys` crates) deserves a look.
- Heavy `unsafe` is not disqualifying in foundational crates (tokio, hashbrown, bytes are
  heavily reviewed) but is a red flag in a small, young crate doing ordinary work.
  Review depth: `rust-security` skill.

## DEP-10: Keep the dependency graph clean

Default: run these regularly (CI for the first two):

| Tool | Purpose |
|---|---|
| `cargo deny check` | advisories, licenses, banned crates, duplicate versions, allowed sources |
| `cargo audit` | RustSec advisories only (subset of cargo-deny) |
| `cargo machete` / `cargo shear` | unused dependencies |
| `cargo tree -d` | duplicate versions of the same crate |
| `cargo update --dry-run` / `cargo outdated` | pending upgrades |
| `cargo hack` | feature-powerset and MSRV checks |

Duplicates of big crates (two `syn`, two `hyper`, two `rand`) usually mean one dependency
is behind a major version; upgrade or accept knowingly — don't paper over with patches.

## DEP-11: Never trust a version number from memory

Default: look up `max_stable_version` before writing a version into Cargo.toml or docs.
Model training data lags crates.io by months to years; the most common agent failure is
writing an old major (`reqwest = "0.11"`, `axum = "0.6"`, `rand = "0.8"`, `hyper = "0.14"`)
whose API differs from the code being written.

- Use `cargo add <crate>` (writes the current version) instead of typing versions.
- Or query the `rustkb` MCP `crate_info` tool, which returns live versions from crates.io
  alongside catalog notes.
- After upgrading across a breaking version, read the crate's CHANGELOG — don't assume
  the API you remember still exists.

## DEP-12: Replace deprecated crates when touching code

Default: when editing a file that uses a crate marked `tier = "avoid"` in `catalog.toml`,
migrate it if the change is mechanical (e.g. `lazy_static` → `LazyLock`, `structopt` →
`clap` derive, `dotenv` → `dotenvy`, `tempdir` → `tempfile`), or leave a TODO with the
replacement if not. Full mapping: `deprecated-and-replacements.md`.

## Quick evaluation template

Use this when proposing a new crate in a review:

```text
crate:        <name> <major.minor>   (crates.io max_stable_version, checked <date>)
why:          <what it replaces; why std/existing deps don't suffice>
maintenance:  last release <date>; <n> releases; repo <active|archived>; RustSec: <none|ID>
adoption:     <recent downloads>; used by <notable dependents>
license:      <SPDX>
msrv:         <rust-version> (ours: <x>)
features:     <enabled features; default-features on/off and why>
native/unsafe:<none | *-sys | notable unsafe>
```
