---
id: idioms/lints-and-tooling
title: Lints, clippy and rustfmt
summary: >-
  Lint policy lives in Cargo.toml [workspace.lints] with a tested baseline (clippy pedantic plus
  cherry-picked restriction lints), suppression uses #[expect(.., reason)], project bans go in
  clippy.toml, rustfmt stays near-default with style_edition 2024, and CI denies warnings on the
  command line rather than in source.
area: idioms
tags: [clippy, lints, rustfmt, cargo-toml, expect, ci, rust-toolchain, rustdoc]
rust: "1.96"
edition: "2024"
crates:
  cargo-nextest: "0.9"
  cargo-hack: "0.6"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section
  - https://doc.rust-lang.org/cargo/reference/workspaces.html#the-lints-table
  - https://doc.rust-lang.org/clippy/lint_configuration.html
  - https://rust-lang.github.io/rust-clippy/stable/index.html
  - https://github.com/rust-lang/rustfmt/blob/main/Configurations.md
  - https://doc.rust-lang.org/cargo/reference/config.html#buildwarnings
---

# Lints, clippy and rustfmt

Decides where lint configuration lives, which lints to turn on, how to silence one correctly, and
how to run fmt/clippy/doc locally and in CI. Read it when creating a crate or workspace, when a
lint fires and you are tempted to `#[allow]` it, or when reviewing a PR's tooling config. Every
lint name and config key below was checked against clippy 0.1.96 / Cargo 1.96. Use the `rustkb`
MCP tool `explain_lint` for any individual lint's rationale and examples.

## Where lint configuration lives

### LINT-01: Configure lints in `Cargo.toml`, not in source files or `RUSTFLAGS`

**Default:** a `[workspace.lints]` table in the root manifest and `[lints] workspace = true` in
every member (single crate: `[lints]` directly; available since 1.74). **Never** put
`#![deny(warnings)]` in `lib.rs`/`main.rs` — every new toolchain adds warnings and turns into a
broken build for contributors and path/git dependents. **Never** set `-D warnings` in
`RUSTFLAGS` or `.cargo/config.toml` for local builds: changing `RUSTFLAGS` invalidates the whole
build cache, including dependencies. Deny warnings only on the CI command line (LINT-11).

```toml
# Cargo.toml (workspace root)
[workspace.lints.rust]
unsafe_code = "deny"

# crates/foo/Cargo.toml
[lints]
workspace = true
```

### LINT-02: Members cannot override inherited lints — plan escape hatches in source

With `[lints] workspace = true`, adding any `[lints.rust]`/`[lints.clippy]` key in the same
member is a hard error (`cannot override workspace.lints in lints`). Consequences:

- Use `unsafe_code = "deny"`, **not** `"forbid"`, workspace-wide, so the one FFI module can opt in
  with `#![expect(unsafe_code, reason = "FFI bindings to libfoo")]`. `forbid` cannot be lifted.
- Lints that only make sense for published libraries (`missing_docs`, `unreachable_pub`) either go
  in the workspace table (binaries then satisfy them trivially) or at the top of that library's
  `lib.rs` as `#![warn(missing_docs)]`.
- A crate with genuinely different policy (generated code, a `-sys` crate) gets its own complete
  `[lints]` table instead of `workspace = true`.

### LINT-03: Give lint groups a lower priority than individual lints

**Default:** `pedantic = { level = "warn", priority = -1 }`. Groups and individual lints at the
same priority are ambiguous; clippy's deny-by-default `lint_groups_priority` rejects the manifest.
Individual overrides keep the default priority `0` so they win.

## Recommended baseline

### LINT-04: Start every workspace from this baseline

**Default:** copy this table, then remove lines you have a reason to drop. It is tuned against the
failure modes of generated code: `unwrap()` everywhere, leftover `dbg!`/`todo!`, silent `#[allow]`,
undocumented `unsafe`, `x.clone()` on an `Arc` that reads like a deep copy.

```toml
[workspace.lints.rust]
unsafe_code = "deny"                    # opt in per module with #[expect(unsafe_code, reason = "..")]
unsafe_op_in_unsafe_fn = "deny"         # warn-by-default in 2024; make it hard
rust_2018_idioms = { level = "warn", priority = -1 }
let_underscore_drop = "warn"            # `let _ = guard;` drops immediately — usually a bug
missing_debug_implementations = "warn"  # libraries: every public type is Debug
missing_docs = "warn"                   # libraries: see LINT-02 for binaries
unreachable_pub = "warn"                # `pub` that isn't reachable should be `pub(crate)`

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
# pedantic lints with poor signal-to-noise:
must_use_candidate = "allow"
similar_names = "allow"
too_many_lines = "allow"
# cherry-picked restriction lints:
allow_attributes = "warn"               # forces #[expect] instead of #[allow]
allow_attributes_without_reason = "warn"
dbg_macro = "warn"
todo = "warn"
unimplemented = "warn"
unwrap_used = "warn"                    # expect("invariant") or `?` instead; tests allowed via clippy.toml
print_stdout = "warn"                   # libraries/services log via tracing
print_stderr = "warn"
exit = "warn"                           # return ExitCode so destructors run
undocumented_unsafe_blocks = "warn"     # every unsafe block needs `// SAFETY:`
multiple_unsafe_ops_per_block = "warn"
clone_on_ref_ptr = "warn"               # Arc::clone(&x), visibly cheap

[workspace.lints.rustdoc]
broken_intra_doc_links = "warn"
missing_crate_level_docs = "warn"
```

CLI tools that legitimately print: set `print_stdout = "allow"` in that binary's own `[lints]`
table, or keep printing in one `output` module annotated with
`#![expect(clippy::print_stdout, reason = "CLI output module")]`.

### LINT-05: Keep clippy's default groups at their default levels

**Never** write `all = "warn"` or `correctness = "warn"`: `clippy::correctness` is **deny** by
default (`mut_from_ref`, `unused_io_amount`, `panicking_unwrap` …) and a blanket `all = "warn"`
downgrades real bugs to warnings. Only raise levels, never lower the defaults wholesale.

### LINT-06: Cherry-pick `restriction` lints; enable `nursery` and `cargo` only deliberately

**Default:** never enable `clippy::restriction` as a group — its lints contradict each other
(`implicit_return` vs `needless_return`) and many encode style choices, not quality. Pick
individual ones as in LINT-04. `clippy::nursery` lints have known false positives; run them as an
occasional audit (`cargo clippy -- -W clippy::nursery`) and keep the useful hits
(`redundant_clone`, `needless_collect`, `significant_drop_tightening`, `future_not_send` for
libraries exposing futures). `clippy::cargo` is for crates.io libraries (metadata checks);
`multiple_crate_versions` is noisy in any real dependency graph — allow it.

## Suppressing a lint

### LINT-07: Silence with `#[expect(lint, reason = "...")]` at the narrowest scope

**Default:** fix the code. When the lint is wrong for this spot, use `#[expect]` (1.81+) on the
smallest item, with a reason. `#[expect]` warns (`unfulfilled_lint_expectations`) once the lint no
longer fires, so suppressions don't rot. **Never** add crate-level `#![allow(clippy::pedantic)]`,
`#![allow(dead_code)]` or `#![allow(unused)]` to make warnings go away — that is the most common
agent "fix" and it hides real defects.

```rust
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "inputs are clamped to 0.0..=1.0 so the scaled value fits in u8"
    )]
    #[must_use]
    pub fn from_unit_floats(r: f32, g: f32, b: f32) -> Self {
        let to_u8 = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self { r: to_u8(r), g: to_u8(g), b: to_u8(b) }
    }
}
```

### LINT-08: Fix the lints agents most often silence

| Lint (default level) | Real fix |
|---|---|
| `needless_borrow`, `needless_borrows_for_generic_args` (warn) | drop the `&`; the callee already borrows |
| `clone_on_copy` (warn), `redundant_clone` (nursery) | remove the clone; see `ownership-borrowing.md` |
| `ptr_arg` (warn) | take `&str` / `&[T]` / `&Path`, not `&String` / `&Vec<T>` / `&PathBuf` |
| `needless_range_loop` (warn) | iterate (`iter().enumerate()`, `zip`); see `iterators-closures.md` |
| `large_enum_variant`, `result_large_err` (warn) | box the large variant / large error payload |
| `await_holding_lock`, `await_holding_refcell_ref` (warn) | end the guard's scope before `.await`; see `async.md` |
| `let_underscore_future` (warn) | `.await` it or spawn it; a dropped future never runs |
| `arc_with_non_send_sync` (warn) | use `Rc` (single thread) or make the payload `Send + Sync` |
| `new_without_default` (warn) | derive or implement `Default` |
| `missing_errors_doc`, `missing_panics_doc` (pedantic) | write the `# Errors` / `# Panics` doc section; see `api-design.md` |
| `needless_pass_by_value` (pedantic) | take `&T`, or keep by-value if you store/consume it and say why |
| `cast_possible_truncation` (pedantic) | `u8::try_from(x)?` or a documented `#[expect]` (LINT-07) |
| `manual_let_else` (pedantic) | `let Some(x) = opt else { return … };` |
| `non_std_lazy_statics` (pedantic) | `std::sync::LazyLock` instead of `lazy_static!`/`once_cell` |

## clippy.toml

### LINT-09: Put thresholds and project-specific bans in `clippy.toml`

**Default:** a `clippy.toml` at the workspace root for things `[lints]` can't express. Don't set
`msrv` there — clippy reads `package.rust-version`. Use `disallowed-methods`/`disallowed-types`
to encode project rules an agent can't infer (the `reason` is shown in the warning, so the agent
learns the fix).

```toml
# clippy.toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-print-in-tests = true
disallowed-methods = [
  { path = "std::thread::sleep", reason = "blocks a runtime worker; use tokio::time::sleep" },
  { path = "std::process::exit", reason = "skips destructors; return ExitCode from main" },
]
await-holding-invalid-types = [
  { path = "tracing::span::Entered", reason = "guard held across .await corrupts spans; use Instrument" },
]
```

The `std::thread::sleep` ban belongs only in async crates; `await-holding-invalid-types` extends
`await_holding_invalid_type` to your own guard types.

## rustfmt

### LINT-10: Default rustfmt, `style_edition = "2024"`, nothing nightly-only

**Default:** no `rustfmt.toml`, or a minimal one. `cargo fmt` infers the style edition from the
package edition but bare `rustfmt` (editors, pre-commit hooks) defaults to 2015 — pin it:

```toml
# rustfmt.toml
style_edition = "2024"
```

`imports_granularity` and `group_imports` are **unstable**: stable `rustfmt` prints
"unstable features are only available in nightly channel" and ignores them. Only add them if the
project already runs `cargo +nightly fmt` in CI. **Never** reformat unrelated code in a feature
PR; run `cargo fmt` on the whole workspace in a dedicated commit.

## Running the tools

### LINT-11: Run this command set locally and in CI

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
cargo test --workspace --all-features --locked     # or cargo nextest run + cargo test --doc
```

- `--all-targets` lints tests, examples and benches too — without it agents ship warning-free
  `src/` and a `tests/` dir full of `unwrap` misuse and dead code.
- `-- -D warnings` after `clippy` makes warnings fatal for workspace crates only, without touching
  `RUSTFLAGS` (LINT-01). From Cargo 1.97 (newer than this file's verified toolchain),
  `CARGO_BUILD_WARNINGS=deny` / `build.warnings = "deny"` is the built-in equivalent.
- Feature-matrix checks (`cargo hack --each-feature`) and CI layout belong to `rust-architecture`;
  `cargo deny` / `cargo audit` to `rust-security`.
- `cargo clippy --fix --allow-dirty` applies machine-applicable suggestions; run it on a clean tree
  and review the diff — it can change public signatures.

### LINT-12: Pin the toolchain for applications

**Default:** applications and services commit a `rust-toolchain.toml`; libraries test on stable
plus their MSRV (see `editions-msrv.md`, ED-05) instead. Clippy adds lints every release, so an
unpinned CI starts failing on unrelated PRs; bump the pin in its own PR and fix new lints there.

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.96"
components = ["clippy", "rustfmt"]
```

### LINT-13: Make the agent loop lint-clean before declaring done

**Default:** after every edit batch run `cargo clippy --all-targets` (fast) and treat any warning
in touched code as a failure to fix, not to suppress. Finish with the full LINT-11 set. An agent
that stops at "it compiles" leaves the `needless_borrow`/`redundant_clone`/`unwrap` noise that
LINT-04 exists to catch.

## Review checklist

- [ ] Lints configured in `[workspace.lints]` + `[lints] workspace = true`; no `#![deny(warnings)]` — LINT-01
- [ ] `unsafe_code = "deny"` (not `forbid`) with scoped `#[expect(unsafe_code, reason)]` where needed — LINT-02
- [ ] Groups use `priority = -1` — LINT-03
- [ ] Baseline from LINT-04 present; `clippy::all`/`correctness` never lowered — LINT-04, LINT-05
- [ ] No `restriction` group enabled wholesale — LINT-06
- [ ] Every suppression is `#[expect(.., reason = "..")]` on the smallest item — LINT-07
- [ ] No crate-level `#![allow(..)]` added to hide warnings — LINT-07, LINT-08
- [ ] Project bans in `clippy.toml` with `reason`s; no duplicate `msrv` — LINT-09
- [ ] `rustfmt.toml` pins `style_edition = "2024"`, no nightly-only keys on stable — LINT-10
- [ ] CI runs fmt, clippy `--all-targets -D warnings`, doc `-D warnings`, tests — LINT-11
- [ ] Applications pin `rust-toolchain.toml` — LINT-12
