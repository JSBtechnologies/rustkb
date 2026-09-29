---
id: architecture/library-design
title: Library design, semver and features
summary: >-
  How to design a publishable Rust library: minimal public surface, semver discipline checked by
  cargo-semver-checks, public-dependency hygiene, strictly additive feature flags tested with
  cargo-hack, no global side effects, docs and docs.rs setup, MSRV policy and no_std support.
area: architecture
tags: [library, semver, api, features, cargo-features, msrv, no_std, docs, cargo-semver-checks, cargo-hack, publishing]
rust: "1.96"
edition: "2024"
crates:
  serde: "1.0"
  thiserror: "2.0"
  tracing: "0.1"
  futures: "0.3"
  cargo-semver-checks: "0.50"
  cargo-hack: "0.6"
  release-plz: "0.3"
verified: 2026-09-29
sources:
  - https://rust-lang.github.io/api-guidelines/
  - https://doc.rust-lang.org/cargo/reference/semver.html
  - https://doc.rust-lang.org/cargo/reference/features.html
  - https://doc.rust-lang.org/cargo/reference/rust-version.html
  - https://github.com/obi1kenobi/cargo-semver-checks
  - https://docs.rs/about/metadata
---

# Library design, semver and features

A library's architecture *is* its public API: every `pub` item, trait impl, feature flag and
public dependency is a promise. Type-level API idioms (builders, `impl Trait` arguments,
conversions) are in `idioms/api-design`; this file covers the structural decisions.

## Public surface

### LIB-01: Expose the minimum; re-export deliberately

Default: private modules + an explicit `pub use` facade in `lib.rs` (`modules-and-boundaries.md`
MOD-04). Before making anything `pub`, ask "do I want to support this for years?".

- Struct fields private; provide constructors/getters. Public fields freeze the representation.
- `#[non_exhaustive]` on public enums (errors!) and on structs/enum variants that may gain fields,
  so adding one is not a breaking change.
- No glob re-exports (`pub use inner::*`): new inner items silently become API.
- Seal traits users should call but not implement (private supertrait), so you can add methods:

```rust,ignore
mod sealed { pub trait Sealed {} }

/// Implemented for the unit types in this crate; not implementable downstream.
pub trait Unit: sealed::Sealed {
    fn symbol(&self) -> &'static str;
}
```

- Enable `missing_docs = "warn"` and `unreachable_pub = "warn"`: undocumented or accidentally
  public items then show up in review.

### LIB-02: Know what breaks semver; check it mechanically

Breaking (needs a major bump — or a minor bump while `0.y.z`, where `0.y` acts as the major):
removing/renaming a public item; adding a required trait method; adding a variant to an
exhaustive enum or a public field to an exhaustive struct; tightening generic bounds; changing a
function signature; removing a trait impl (including auto traits like `Send`/`Sync` lost through a
new private field); removing a feature; bumping a **public** dependency's major version.

Non-breaking: adding items, adding variants to `#[non_exhaustive]` enums, adding trait methods
with defaults (mostly), loosening bounds, adding optional features.

Default: run `cargo semver-checks` in CI on every PR to a library and before every release
(`ci-cd-release.md` CI-07; release-plz runs it for you). It catches most API-level breaks but not
behaviour changes — changelogs still need a human.

### LIB-03: Public dependencies are part of your API

If a dependency's type appears in your public API (`fn parse(input: bytes::Bytes)`,
`impl From<reqwest::Error> for Error`, a `pub use` of it), upgrading that dependency across a
major version is a breaking change for you.

- Prefer your own types or `std` types at the boundary; keep dependencies private.
- When exposing one is the point (a `serde` integration, a `tokio` I/O adaptor), gate it behind a
  feature and re-export the dependency (`pub use bytes;`) so users can name matching versions.
- Never expose a pre-1.0 dependency's types in a 1.x API without a feature gate.

## Feature flags

### LIB-04: Features are strictly additive

Cargo unifies features across the whole dependency graph: if *any* crate enables a feature, *every*
user of your crate gets it. So a feature may only **add** API or capabilities.

- ✅ `std` (default) enabling std-dependent APIs; `serde` adding derives; `tokio` adding an async
  adaptor.
- ❌ `no-std` feature (removes things — use a default `std` feature instead).
- ❌ Mutually exclusive features (`backend-a` vs `backend-b` with `compile_error!` when both are
  on) — some other crate in the graph *will* enable both. Make them coexist and pick at runtime or
  by explicit API.
- ❌ Features that change behaviour or types of existing items (a `u64-ids` feature changing a
  type alias).
- Use `dep:` syntax so optional dependencies don't create implicit features, and weak `?`
  features to forward without enabling:

```toml
[features]
default = ["std"]
std = ["serde?/std"]          # turn on serde's std only if serde is already enabled
serde = ["dep:serde"]

[dependencies]
serde = { version = "1.0", default-features = false, features = ["derive"], optional = true }
```

```rust,ignore
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Meters(f64);
```

- Test every combination that matters: `cargo hack check --feature-powerset --no-dev-deps`
  (CI, `ci-cd-release.md`), at minimum `--no-default-features` and `--all-features`.
- Keep the feature list short and documented in the crate docs; every feature doubles the build
  matrix.

## Behaviour a library must not have

### LIB-05: No global side effects

A library never:

- installs a tracing subscriber / logger, panic hook, global allocator or metrics recorder — it
  only *emits* via `tracing` (or `log`) macros; binaries decide where output goes;
- reads environment variables or config files implicitly — take a config struct or builder;
- calls `std::process::exit`, prints to stdout/stderr, or panics on recoverable errors;
- starts its own async runtime or spawns detached threads without giving the caller a handle to
  stop them;
- mutates process-global state (`std::env::set_var` is `unsafe` in edition 2024 for good reason).

### LIB-06: Typed, `Send + Sync + 'static` errors

Library errors are `thiserror` enums (often `#[non_exhaustive]`), implement `std::error::Error`,
and keep the underlying cause as `source()`. Never return `anyhow::Error` or `Box<dyn Error>` from a
public API. Full rules: `idioms/error-handling`.

### LIB-07: Async libraries don't choose the runtime for users

- Prefer runtime-agnostic code (`futures` traits, no `tokio::spawn`) where feasible; if you must
  depend on tokio, say so in the docs and keep tokio types out of the API unless feature-gated.
- Public futures should be `Send` (users run them on multi-threaded runtimes).
- Consider a sync core with a thin async layer: easier to test and reusable from both worlds.

## Documentation

### LIB-08: Crate docs, examples, and docs.rs metadata

- Crate-level `//!` docs with a runnable quick-start example (doctests run in `cargo test`).
- Document every public item; `# Errors` / `# Panics` sections where relevant.
- Show feature-gated items on docs.rs:

```toml
[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
```

```rust,ignore
#![cfg_attr(docsrs, feature(doc_cfg))] // docs.rs builds with nightly; `docsrs` is a known cfg
```

- `cargo doc --no-deps --all-features` with `RUSTDOCFLAGS="-D warnings"` in CI catches broken
  intra-doc links.
- Fill `description`, `license`, `repository`, `keywords`, `categories` in `Cargo.toml`;
  crates.io requires `description` and `license` (or `license-file`) to publish.

## MSRV

### LIB-09: Declare an MSRV, test it, bump it deliberately

Default policy: set `package.rust-version` to the oldest toolchain you test (for edition 2024 code
the floor is 1.85); support at least the last ~6 months of stable releases unless a dependency
forces more. Treat an MSRV bump as a minor-version change and note it in the changelog.

- CI job on exactly that toolchain (`dtolnay/rust-toolchain@1.85`, `cargo check --locked`).
- Resolver 3 (edition 2024) resolves dependencies to versions compatible with your
  `rust-version` when generating the lockfile, which keeps the MSRV job green.
- Your MSRV is bounded below by your dependencies' `rust-version` (`cargo info <crate>` shows
  it; e.g. sqlx 0.9.0 declares 1.94, config 0.15.26 declares 1.88). Check before promising an
  old MSRV.
- Don't declare an MSRV you don't test — it is a promise downstream resolvers rely on.
- Edition and version-specific features: `idioms/editions-msrv`.

## no_std

### LIB-10: Support `no_std` with a default `std` feature, if the domain allows it

For parsers, encoders, math, data structures and protocol crates:

```rust,ignore
//! Typed physical units.
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "std")]
extern crate std;
#[cfg(feature = "alloc")]
extern crate alloc; // if you add an `alloc` feature for Vec/String/Box

impl core::error::Error for ParseError {} // stable in core since 1.81
```

- Use `core::`/`alloc::` paths; gate std-only APIs (I/O, `HashMap`, time) behind `std`.
- Check both builds in CI: `cargo check --no-default-features` (plus an embedded target such as
  `thumbv7em-none-eabihf` if you claim bare-metal support).
- Don't retrofit `no_std` onto an IO-heavy crate — split out a `no_std` core crate instead.

## Release hygiene

### LIB-11: Changelog, semver check, then publish from CI

Default: `release-plz` opens a release PR with version bumps and changelog entries (running
cargo-semver-checks), and publishes when merged — see `ci-cd-release.md` CI-08. Never publish from
a laptop with uncommitted changes; never yank to "fix" a semver break — publish a fix release.

## Checklist

### LIB-12: Library readiness checklist

- [ ] Facade `lib.rs`; no glob re-exports; `missing_docs`/`unreachable_pub` warn.
- [ ] `#[non_exhaustive]` on public error enums and growable structs.
- [ ] Private fields, sealed traits where downstream impls aren't intended.
- [ ] No dependency types in the API unless feature-gated and re-exported.
- [ ] Features additive; `std` not `no-std`; `dep:` syntax; `cargo hack --feature-powerset` in CI.
- [ ] No logger/subscriber init, env reads, `process::exit`, prints, or runtime creation.
- [ ] Typed errors; `Send + Sync + 'static`.
- [ ] Crate docs with doctested example; docs.rs metadata.
- [ ] `rust-version` declared and tested; `cargo semver-checks` in CI.
