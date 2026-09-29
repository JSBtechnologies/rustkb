---
name: rust-idioms
description: >-
  Current (Rust 1.96, edition 2024) idioms and decision rules for writing, reviewing, refactoring or
  debugging Rust code — any .rs file, Cargo.toml [lints]/rust-version, or Rust code review. Covers
  ownership and borrow-checker fixes without clone(), error handling (thiserror vs anyhow, unwrap/expect
  policy), traits and generics vs dyn, public API design (naming, conversions, builders, #[must_use],
  #[non_exhaustive], sealed traits), newtypes/typestate, iterators and closures, async/tokio
  (Send bounds, cancellation, blocking, locks across .await), threads/channels/Arc<Mutex>/atomics/
  LazyLock, unsafe and FFI (SAFETY comments, unsafe extern), performance, testing (proptest, insta,
  nextest), macros, edition 2024 migration and MSRV, clippy/rustfmt configuration. Use it to avoid
  the mistakes AI agents make in Rust: outdated crates (lazy_static, structopt, failure), unwrap
  everywhere, needless clone, Arc<Mutex> by reflex, Box<dyn Error> in library APIs, blocking in async,
  over-generic code.
---

# rust-idioms

Opinionated, verified rules for idiomatic modern Rust. Each rule has a stable ID (`ERR-01`) you can
cite in reviews. Read the core rules below, then open the one reference file that matches your task
(routing table). Don't load every file.

If the `rustkb` MCP server is available, use it for facts this skill deliberately doesn't copy:
`search` / `get_item` (versioned std and crate API docs), `crate_info` (current crate versions and
features), `check_advisories` (RustSec), `explain_lint` (clippy/rustc lint docs), `rust_release`
(what a Rust version stabilized). Never state a crate version or stabilization version from memory.

## Core rules

- **ED-01** New crates: `edition = "2024"` and an explicit `rust-version`; never generate edition 2021.
- **ED-08** Use std over legacy crates: `LazyLock`/`OnceLock` (not lazy_static/once_cell), `thread::scope`, `IsTerminal`, `cfg_select!` (1.95+).
- **OWN-01** Parameters borrow the most general type: `&str`, `&[T]`, `&Path` — never `&String`/`&Vec<T>`.
- **OWN-04** Fix borrow errors by restructuring (shorter borrows, field splitting, `mem::take`, entry API) before cloning.
- **ERR-01** Libraries return typed `thiserror` enums; never `Box<dyn Error>`, `anyhow::Error` or `String` in public APIs.
- **ERR-02** Applications use `anyhow::Result` and add `.context()` at every I/O/boundary call.
- **ERR-10** No `.unwrap()` in non-test code; `.expect("invariant that makes this infallible")` or `?`.
- **TRAIT-01** Start concrete; introduce a trait only for a second real implementation (no `FooService`+`FooServiceImpl`).
- **TRAIT-02** Closed set of types → `enum`; open/runtime set → `dyn Trait`; hot generic paths → generics.
- **ITER-04** Collect fallible iterators into `Result<Vec<_>, E>`; no `unwrap()` inside `map`.
- **TYPE-02** Parse, don't validate: newtypes with private fields and one fallible constructor; states as enum variants, not structs of `Option`s.
- **ASYNC-04** No blocking IO, `thread::sleep` or long CPU loops on async workers — `spawn_blocking` or rayon.
- **ASYNC-05** Never hold a `Mutex`/`RefCell` guard across `.await`; lock briefly with `std::sync::Mutex`.
- **CONC-02** Transfer ownership (channels, `thread::scope`) before reaching for `Arc<Mutex<_>>`.
- **UNSAFE-03** Every `unsafe` block has a `// SAFETY:` comment; every `unsafe fn` has a `# Safety` doc section.
- **PERF-01** Benchmark/profile before optimising; no `unsafe` or `#[inline]` sprinkling on a hunch.
- **LINT-04** Configure lints in `[workspace.lints]` (pedantic + cherry-picked restriction), not `#![deny(warnings)]`.
- **LINT-07** Silence a lint only with `#[expect(lint, reason = "..")]` on the smallest item; never crate-wide `#![allow]`.
- **LINT-11** Done means: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, tests pass.

## Routing table

| Situation | Read |
|---|---|
| Choosing parameter/return types; borrow-checker errors (E0499/E0502/E0505); `Rc<RefCell>`, `Cow`, lifetimes in structs | `references/ownership-borrowing.md` (OWN) |
| Defining error types; `?`, `unwrap`/`expect`, panics; `main` exit codes; thiserror vs anyhow | `references/error-handling.md` (ERR) |
| Adding a trait or type parameter; generics vs `impl Trait` vs `dyn` vs enum; orphan rule; dyn compatibility | `references/traits-generics.md` (TRAIT) |
| Loops vs iterator chains; `collect`; returning iterators; closure `Fn`/`FnMut`/`FnOnce`; `move` | `references/iterators-closures.md` (ITER) |
| Designing a public API: naming, `From`/`TryFrom`/`AsRef`, builders, `#[must_use]`, `#[non_exhaustive]`, sealed traits, docs | `references/api-design.md` (API) |
| Newtypes, parse-don't-validate, typestate, enums instead of bools, `NonZero`, making invalid states unrepresentable | `references/types-and-state.md` (TYPE) |
| async/await, tokio tasks, `Send` errors, `select!`, cancellation, blocking calls in async, `async fn` in traits | `references/async.md` (ASYNC) |
| Threads, channels, `Arc<Mutex>`/`RwLock`/atomics, rayon, scoped threads, global state | `references/concurrency.md` (CONC) |
| Any `unsafe`, FFI, `extern`, `#[no_mangle]`, raw pointers, `MaybeUninit`, transmute, Miri | `references/unsafe-ffi.md` (UNSAFE) |
| Something is slow or allocation-heavy; release profile, LTO, PGO; benchmarking | `references/performance.md` (PERF) |
| Writing or organising tests: unit/integration/doc tests, proptest, insta, mocks, nextest, coverage | `references/testing.md` (TEST) |
| Writing or reviewing `macro_rules!` or a proc-macro | `references/macros.md` (MAC) |
| New crate setup, edition 2024 migration, `rust-version`/MSRV, "is this feature stable?", replacing legacy crates | `references/editions-msrv.md` (ED) |
| `[lints]` table, clippy groups, `#[expect]`, `clippy.toml`, `rustfmt.toml`, CI commands | `references/lints-and-tooling.md` (LINT) |

## Reviewing Rust code

Walk the `## Review checklist` at the end of each relevant reference file and cite rule IDs in
findings (e.g. "OWN-04: clone only to escape the borrow checker"). Highest-yield checks for
generated code: stray `unwrap()`/`clone()`, `Box<dyn Error>` or `String` errors in library
signatures, locks or `RefCell` guards held across `.await`, blocking calls inside async functions,
single-implementation traits, legacy crates (lazy_static, once_cell, structopt, failure, async-std),
`edition = "2021"` in new manifests, and crate-level `#![allow(..)]`.

## Out of scope (other skills)

- Workspace/crate layout, layering, config, observability, services/CLIs, semver and feature flags,
  CI/CD, releases → `rust-architecture`.
- Which crate to use for a domain and its current version → `rust-ecosystem` (`catalog.toml`).
- Supply chain (cargo-deny/audit/vet), unsafe review process, crypto choices, fuzzing → `rust-security`.
