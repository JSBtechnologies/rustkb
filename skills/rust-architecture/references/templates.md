---
id: architecture/templates
title: Project templates
summary: >-
  Copy-paste manifests and config files verified with cargo 1.96: workspace Cargo.toml with shared
  dependencies and [workspace.lints], member and single-package manifests, rust-toolchain.toml,
  rustfmt.toml, clippy.toml, release profile, .cargo/config.toml and .gitignore.
area: architecture
tags: [templates, scaffold, cargo-toml, workspace, lints, clippy, rustfmt, rust-toolchain, profile, new-project]
rust: "1.96"
edition: "2024"
crates:
  anyhow: "1.0"
  axum: "0.8"
  clap: "4.6"
  config: "0.15"
  metrics: "0.24"
  metrics-exporter-prometheus: "0.18"
  secrecy: "0.10"
  serde: "1.0"
  serde_json: "1.0"
  sqlx: "0.9"
  thiserror: "2.0"
  tokio: "1.53"
  tokio-util: "0.7"
  tower: "0.5"
  tower-http: "0.7"
  tracing: "0.1"
  tracing-subscriber: "0.3"
  xshell: "0.2"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/cargo/reference/workspaces.html
  - https://doc.rust-lang.org/cargo/reference/profiles.html
  - https://rust-lang.github.io/rustup/overrides.html#the-toolchain-file
  - https://doc.rust-lang.org/clippy/lint_configuration.html
---

# Project templates

Every template here was assembled into a scratch workspace (domain lib + Postgres adapter + axum
server + clap CLI + xtask) and passed `cargo fmt --check`, `cargo clippy --workspace --all-targets
-- -D warnings` (with the lint table below), `cargo test` and `cargo doc` on Rust 1.96. Replace
`shop`/`example` names. `deny.toml` (TPL-06), the CI workflow (TPL-07) and `main.rs` skeletons
(TPL-11, TPL-12) are in `templates-ci-and-skeletons.md`. Rationale lives in the other reference
files; rule IDs are cited inline.

## Workspace root manifest

### TPL-01: Root `Cargo.toml` (virtual manifest)

```toml
[workspace]
resolver = "3"                      # required in a virtual manifest (WS-02)
members = ["crates/*", "xtask"]
default-members = ["crates/*"]      # plain `cargo build` skips xtask (WS-08)

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.96"               # apps: = pinned toolchain; libs: tested MSRV (ARCH-05)
license = "MIT OR Apache-2.0"
repository = "https://github.com/example/shop"
publish = false                     # opt in per published crate (WS-11)

[workspace.dependencies]
# internal crates: path + version
shop-domain = { path = "crates/shop-domain", version = "0.1.0" }
shop-postgres = { path = "crates/shop-postgres", version = "0.1.0" }

# external: one version for the repo; keep features minimal, members add their own (WS-04)
anyhow = "1.0"
axum = "0.8"
clap = { version = "4.6", features = ["derive", "env"] }
config = { version = "0.15", default-features = false, features = ["toml"] }
metrics = "0.24"
metrics-exporter-prometheus = { version = "0.18", default-features = false }
secrecy = { version = "0.10", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio", "tls-rustls", "postgres", "migrate"] }
thiserror = "2.0"
tokio = "1.53"
tokio-util = "0.7"
tower = "0.5"
tower-http = "0.7"
tracing = "0.1"
tracing-subscriber = "0.3"
xshell = "0.2"

[workspace.lints.rust]
unsafe_code = "deny"                # deny, not forbid: one FFI module can #[allow] it
missing_debug_implementations = "warn"
unreachable_pub = "warn"            # binaries: use pub(crate) (MOD-03)
rust_2018_idioms = { level = "warn", priority = -1 }

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
# pedantic lints that are mostly noise in application code
module_name_repetitions = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
must_use_candidate = "allow"
# restriction lints worth opting into
unwrap_used = "warn"                # tests allowed via clippy.toml (TPL-05)
dbg_macro = "warn"
todo = "warn"
print_stdout = "warn"               # data output goes through a locked writer (CLI-05)

[profile.release]                   # see TPL-08
lto = "thin"
codegen-units = 1
debug = "line-tables-only"
```

Libraries meant for crates.io: lower `rust-version` to the MSRV you test (LIB-09) — but it cannot
be lower than your dependencies' own `rust-version` (e.g. sqlx 0.9.0 declares 1.94, config
0.15.26 declares 1.88; check with `cargo info <crate>`). Add `missing_docs = "warn"` via
`#![warn(missing_docs)]` in each library's `lib.rs`.

## Member manifests

### TPL-02: Library and binary member `Cargo.toml`

```toml
# crates/shop-domain/Cargo.toml — library
[package]
name = "shop-domain"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
serde.workspace = true
thiserror.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["rt", "macros"] }

[lints]
workspace = true                    # every member, or it gets no lints (WS-05)
```

```toml
# crates/shop-server/Cargo.toml — binary
[package]
name = "shop-server"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
shop-domain.workspace = true
shop-postgres.workspace = true
anyhow.workspace = true
axum.workspace = true
config.workspace = true
metrics.workspace = true
metrics-exporter-prometheus.workspace = true
secrecy.workspace = true
serde.workspace = true
serde_json.workspace = true
sqlx.workspace = true
thiserror.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "signal", "net"] }
tokio-util = { workspace = true, features = ["rt"] }
tower.workspace = true
tower-http = { workspace = true, features = ["trace", "timeout", "request-id", "limit"] }
tracing.workspace = true
tracing-subscriber = { workspace = true, features = ["env-filter", "json"] }

[dev-dependencies]
tower = { workspace = true, features = ["util"] }   # ServiceExt::oneshot in router tests

[lints]
workspace = true
```

## Toolchain and formatting

### TPL-03: `rust-toolchain.toml` (applications only)

```toml
[toolchain]
channel = "1.96"                    # bump deliberately, in its own PR
components = ["rustfmt", "clippy"]
profile = "minimal"
# targets = ["x86_64-unknown-linux-musl"]   # if you cross-compile
```

Libraries: omit the file (or use `channel = "stable"`) so contributors and CI test on current
stable; the MSRV is checked by a separate CI job (CI-06). Never pin `nightly` for a project that
doesn't need nightly features.

### TPL-04: `rustfmt.toml`

```toml
style_edition = "2024"
use_field_init_shorthand = true
use_try_shorthand = true
```

Keep it tiny: `cargo fmt` already takes the edition from `Cargo.toml`, and every extra option is a
diff against the ecosystem's default style. Only stable options — unstable ones are ignored
(with a warning) on stable rustfmt.

### TPL-05: `clippy.toml`

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-dbg-in-tests = true
allow-print-in-tests = true
```

Lint *levels* belong in `[workspace.lints]`; `clippy.toml` only configures lint *behaviour*.
Clippy reads the MSRV from `rust-version`, so don't duplicate it here.

## Profiles and small files

### TPL-08: Release profile

```toml
[profile.release]
lto = "thin"                  # most of fat LTO's win at a fraction of the link time
codegen-units = 1             # better optimisation, slower build
debug = "line-tables-only"    # file:line in backtraces and profilers, small size cost

# Optional, for distributed binaries built by cargo-dist or your release job:
[profile.dist]
inherits = "release"
lto = "fat"
```

Add `strip = true` for CLIs where size matters more than symbolised backtraces. Avoid
`panic = "abort"` in tokio services unless you have a reason: it turns a panicking request task
(normally caught and logged by the runtime) into a process crash. Set `opt-level` per profile,
never via `RUSTFLAGS`.

### TPL-09: Single-package project (no workspace)

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "2024"
rust-version = "1.96"
license = "MIT OR Apache-2.0"
publish = false

[dependencies]
anyhow = "1.0"
clap = { version = "4.6", features = ["derive", "env"] }

[lints.rust]
unsafe_code = "deny"
unreachable_pub = "warn"

[lints.clippy]
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
missing_errors_doc = "allow"
unwrap_used = "warn"
dbg_macro = "warn"
```

Resolver 3 is implied by `edition = "2024"` here. Convert to TPL-01 when a second crate appears.

### TPL-10: `.cargo/config.toml` and `.gitignore`

```toml
# .cargo/config.toml
[alias]
xtask = "run --package xtask --"
```

```text
# .gitignore
/target
.env
```

`Cargo.lock` is **not** ignored (WS-10). Don't put `RUSTFLAGS`/`target-cpu=native` in the
committed `.cargo/config.toml`: it makes binaries non-portable and busts caches.

