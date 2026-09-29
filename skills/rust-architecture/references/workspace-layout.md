---
id: architecture/workspace-layout
title: Workspace layout
summary: >-
  How to structure a Cargo workspace: virtual manifest with crates/ directory, explicit resolver 3,
  workspace.package / workspace.dependencies / workspace.lints inheritance, when to split a crate
  (and when not to), compile-time effects, default-members and the xtask pattern.
area: architecture
tags: [workspace, cargo, crates, monorepo, resolver, lints, xtask, compile-time, dependencies]
rust: "1.96"
edition: "2024"
crates:
  xshell: "0.2"
  anyhow: "1.0"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/cargo/reference/workspaces.html
  - https://doc.rust-lang.org/cargo/reference/resolver.html
  - https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section
  - https://github.com/matklad/cargo-xtask
  - https://matklad.github.io/2021/08/22/large-rust-workspaces.html
---

# Workspace layout

Use a workspace only when the project has more than one real unit (see `project-kickoff.md`
ARCH-03). Once you do, follow these rules; copy-paste manifests are in `templates.md`.

## Directory layout

### WS-01: Virtual manifest at the root, every crate under `crates/`

Default: the root `Cargo.toml` is a *virtual manifest* (only `[workspace]`, no `[package]`); all
member crates live flat under `crates/`, and the directory name equals the crate name.

```text
shop/
├── Cargo.toml            # [workspace] only
├── Cargo.lock            # committed
├── rust-toolchain.toml   # applications only
├── clippy.toml  deny.toml  rustfmt.toml
├── .cargo/config.toml    # xtask alias
├── crates/
│   ├── shop-domain/      # pure domain: types, rules, ports (traits)
│   ├── shop-postgres/    # adapter: implements domain ports with sqlx
│   ├── shop-server/      # binary: axum HTTP service (composition root)
│   └── shop-cli/         # binary: admin CLI
└── xtask/                # dev automation, never published
```

Why flat: `members = ["crates/*"]` picks up new crates automatically, paths never nest
(`crates/a/crates/b`), and every crate is one `ls` away. Prefix crate names with the project
(`shop-*`) so they are unambiguous in `Cargo.lock`, logs and crates.io.

Never make the root package also a member with code in `src/` when there are several binaries — it
blurs which crate is the "main" one and forces `--workspace` flags everywhere.

## Root manifest essentials

### WS-02: Write `resolver = "3"` explicitly in a virtual manifest

A virtual manifest has no `package.edition` to imply the resolver. Without the key, Cargo 1.96
warns *"virtual workspace defaulting to `resolver = "1"` despite one or more workspace members
being on edition 2024"* and uses the old resolver (feature unification across build/dev/normal
deps, no MSRV-aware resolution).

```toml
[workspace]
resolver = "3"
members = ["crates/*", "xtask"]
default-members = ["crates/*"]   # plain `cargo build/test` skips xtask
```

### WS-03: Inherit package metadata from `[workspace.package]`

Default: put `version`, `edition`, `rust-version`, `license`, `repository`, `publish` (and
`authors`/`homepage` if used) in `[workspace.package]`; members write `edition.workspace = true`
etc. One place to bump edition or MSRV.

```toml
[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "MIT OR Apache-2.0"
repository = "https://github.com/example/shop"
publish = false            # internal crates; published crates override with their own key
```

Exception: published crates that version independently set their own `version` instead of
inheriting it.

### WS-04: Declare every dependency version once in `[workspace.dependencies]`

Default: all external and internal dependencies are declared in the root; members inherit with
`dep.workspace = true` and may only **add** `features` (and `optional`).

```toml
# root Cargo.toml
[workspace.dependencies]
shop-domain = { path = "crates/shop-domain", version = "0.1.0" }
serde = { version = "1.0", features = ["derive"] }
tokio = "1.53"                        # no features here…
tower-http = "0.7"

# crates/shop-server/Cargo.toml
[dependencies]
shop-domain.workspace = true
serde.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "signal", "net"] }  # …members add them
tower-http = { workspace = true, features = ["trace", "timeout", "request-id", "limit"] }
```

Rules that trip agents:

- Features in `[workspace.dependencies]` apply to **every** member that inherits the dependency.
  Keep the root entry minimal; let each member ask for what it uses.
- A member cannot turn off default features the root entry leaves on. If any member needs
  `default-features = false`, put that in the root entry and have members add features back.
- Internal path dependencies carry `version` too, so `cargo publish` works if a crate is ever
  published (path is stripped on publish, version remains).
- Dev-only tools used by one crate (`insta`, `proptest`) can still live in the root table — one
  version for the whole repo is the point.

### WS-05: Share lints through `[workspace.lints]`; opt every member in

Default: define lint levels once in the root and put `[lints] workspace = true` in **every**
member (including xtask). A crate without it silently gets no workspace lints.

```toml
# root
[workspace.lints.rust]
unsafe_code = "deny"
unreachable_pub = "warn"
[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "warn"

# every member
[lints]
workspace = true
```

Cargo refuses a member manifest that combines `workspace = true` with its own `[lints.*]`
entries — there is no per-crate merge. For a crate-specific exception, put an attribute in code
with a reason: `#![expect(clippy::print_stdout, reason = "CLI writes reports")]` or
`#[allow(unsafe_code)]` on the single FFI module (that is why the template uses
`unsafe_code = "deny"`: a `forbid` level cannot be overridden by `allow`). Use `priority = -1` on
lint *groups* so individual lints override them. Full recommended table: `templates.md` TPL-01; lint rationale:
`idioms/lints-and-tooling`.

Note: `unreachable_pub` fires in binary crates too; write `pub(crate)` there (see
`modules-and-boundaries.md` MOD-03).

## When to split a crate

### WS-06: Split along deployment, reuse and dependency-weight boundaries

Split a new crate out when it:

1. **Is a separate deployable** (each binary gets its own crate so its dependencies stay its own).
2. **Is reused** by two or more crates, or published.
3. **Isolates a heavy or risky dependency** (sqlx, a C library, a GUI toolkit) so other crates
   compile and test without it.
4. **Enforces a dependency rule** the compiler should check: the domain crate *cannot* import the
   Postgres adapter if it does not depend on it.
5. **Must be separate by construction**: proc-macros, `build.rs`-heavy FFI `-sys` crates.

Do **not** split when it only mirrors a layer name (`-models`, `-utils`, `-common`, `-types`
grab-bags), when the crate would be < ~500 lines with a single consumer, or when two crates would
need to change together in every PR. Merge crates that always change together.

❌ Common agent failure: `app-models`, `app-services`, `app-controllers`, `app-utils`,
`app-errors`, `app-config` — six crates, one consumer each, every change touches four manifests,
and `app-utils` becomes the dumping ground. ✅ One `app` crate with those as modules, split later
along rule 1–5 lines.

### WS-07: Keep the crate graph wide, not deep

Cargo compiles independent crates in parallel; a chain `a → b → c → d` compiles serially. Prefer a
small domain core that many leaf crates depend on in parallel. Generic code is monomorphised in
the crate that instantiates it, so a generic-heavy core does not "pre-compile" work for its
dependents.

Measure before restructuring: `cargo build --timings` writes an HTML report showing the critical
path. Cheap wins before splitting crates:

- Turn off unused default features of heavy dependencies.
- Keep proc-macro-heavy deps (`sqlx` macros, large derive sets) out of crates that don't need them.
- `debug = "line-tables-only"` or `debug = false` in `[profile.dev]` if link time dominates.

## Build targets and automation

### WS-08: Use `default-members` to keep tooling out of the default build

`default-members = ["crates/*"]` makes `cargo build`/`cargo test` at the root skip `xtask` (and
examples crates), while `--workspace` still includes everything. CI should always pass
`--workspace` explicitly.

### WS-09: Automate with an `xtask` crate, not shell scripts or Makefiles

Default: an `xtask` binary crate (unpublished) plus a Cargo alias; tasks are plain Rust, so they
work identically on Windows, macOS and Linux.

```toml
# .cargo/config.toml
[alias]
xtask = "run --package xtask --"
```

```rust
// xtask/src/main.rs
use anyhow::bail;
use xshell::{Shell, cmd};

fn main() -> anyhow::Result<()> {
    let task = std::env::args().nth(1);
    let sh = Shell::new()?;
    sh.change_dir(env!("CARGO_MANIFEST_DIR")); // xtask/
    sh.change_dir(".."); // workspace root, regardless of caller cwd

    match task.as_deref() {
        Some("ci") => {
            cmd!(sh, "cargo fmt --all --check").run()?;
            cmd!(sh, "cargo clippy --workspace --all-targets --all-features -- -D warnings").run()?;
            cmd!(sh, "cargo test --workspace --all-features").run()?;
            cmd!(sh, "cargo doc --workspace --no-deps --all-features")
                .env("RUSTDOCFLAGS", "-D warnings")
                .run()?;
        }
        Some(other) => bail!("unknown task: {other}"),
        None => bail!("usage: cargo xtask <ci>"),
    }
    Ok(())
}
```

Run with `cargo xtask ci`. Keep xtask dependencies light (`xshell`, `anyhow`); it is compiled on
every developer machine.

### WS-10: Commit `Cargo.lock` and build with `--locked` in CI

Default: commit `Cargo.lock` for every repository — applications and libraries (Cargo's own
guidance since 2023). CI and release builds use `--locked` so a stale lockfile fails instead of
silently resolving new versions. Update deliberately (`cargo update`, Dependabot/Renovate).

### WS-11: Mark internal crates `publish = false`

Default: set `publish = false` in `[workspace.package]` and override with `publish = true` (or a
registry list) only in crates meant for crates.io. Prevents accidental publication of internal
code and lets tools (release-plz, cargo-semver-checks) skip them.

## Anti-patterns

### WS-12: Workspace anti-patterns agents produce

- Versions duplicated in member manifests instead of `[workspace.dependencies]` → drift.
- Missing `resolver = "3"` in a virtual manifest → resolver 1 behaviour.
- A member without `[lints] workspace = true` → lints silently off.
- `path = "../foo"` without `version` on a crate that might be published.
- Circular "fix" via a `common` crate that everything depends on and that depends on everything's
  types — extract the shared *concept* into the domain crate instead.
- Nested workspaces or `[patch]` hacks to paper over a bad split — flatten instead.
- One crate per microservice *and* per layer: N×M crates for no compile-time benefit.

Related: crate boundaries inside a crate → `modules-and-boundaries.md`; dependency direction
between domain and adapters → `layered-hexagonal.md`; full manifests → `templates.md`.
