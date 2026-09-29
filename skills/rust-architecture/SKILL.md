---
name: rust-architecture
description: >-
  Project and system design for Rust. Use when starting a new Rust project or crate (cargo new,
  scaffolding, "set up a Rust service/CLI/library"), deciding bin vs lib vs workspace, designing
  crate and module structure or visibility, applying layered/hexagonal (ports and adapters)
  architecture or dependency injection, structuring an axum/tokio web service (state, errors,
  graceful shutdown, timeouts, health checks), a clap CLI (exit codes, stdout/stderr, config
  precedence), or a publishable library (semver, feature flags, MSRV, no_std); for configuration
  and secrets, tracing/metrics/OpenTelemetry setup, and CI/CD, release (release-plz, cargo-dist)
  and copy-paste templates (Cargo.toml with workspace lints, clippy.toml, deny.toml, GitHub
  Actions workflow, rust-toolchain.toml).
---

# rust-architecture

Opinionated defaults for *shaping* Rust projects. Language-level idioms (errors, async, traits,
testing) live in `rust-idioms`; crate choices in `rust-ecosystem`; supply chain and secure coding in
`rust-security`. Every rule has a stable ID — cite it in reviews and plans.

## Core rules

1. **ARCH-01** Answer the kickoff questions (shape, edition, MSRV, async, errors, config,
   observability, targets) before scaffolding; state the defaults you picked.
2. **ARCH-02** Start with one package; binaries get a thin `main.rs` over a `lib.rs`.
3. **ARCH-03 / WS-06** Split crates only for separate deployables, reuse, heavy-dependency
   isolation, enforced dependency direction, or proc-macros — never one crate per layer.
4. **ARCH-06** Async only for concurrent network IO; sync + rayon for CPU work; never start a
   runtime inside a library.
5. **ARCH-09** No global mutable state: `main` is the composition root and passes dependencies down.
6. **WS-02** A virtual workspace manifest must say `resolver = "3"`.
7. **WS-04 / WS-05** Versions once in `[workspace.dependencies]`; lints once in
   `[workspace.lints]` with `[lints] workspace = true` in every member.
8. **MOD-01 / MOD-05** Modules by domain concept; no `utils`/`helpers`/`common` dumping grounds.
9. **MOD-03 / MOD-04** Private by default, `pub(crate)` in binaries, `lib.rs` re-exports a
   curated facade.
10. **HEX-01** Concrete types by default; a trait only at an external boundary that needs a second
    implementation (e.g. a test fake).
11. **HEX-05 / HEX-07** Inject with generics that stop at the composition root; no DI containers
    or service locators; `dyn`/enums only for runtime choice.
12. **SVC-04** One `ApiError: IntoResponse` maps domain errors to HTTP; 5xx bodies are generic.
13. **SVC-05 / SVC-06** SIGTERM + Ctrl-C graceful shutdown with cancellation of background tasks;
    request timeout, body limit and outbound timeouts everywhere.
14. **CLI-02 / CLI-04** `main` returns `ExitCode` and prints `error: {err:#}`; data to stdout,
    diagnostics to stderr; never `unwrap()` in `main`.
15. **LIB-04** Cargo features are strictly additive (`std`, never `no-std`); test with
    `cargo hack --feature-powerset`.
16. **LIB-02 / LIB-05** Semver-check every library PR; libraries never init loggers, read env vars
    or call `process::exit`.
17. **CFG-01 / CFG-04** One typed, validated `Settings` loaded once at startup; secrets are
    `SecretString`; no `env::var` outside the config module.
18. **OBS-01** `tracing` in every crate; the subscriber is installed only in binaries.
19. **OBS-04 / OBS-06** `#[instrument(skip_all, fields(..))]`; never log secrets or bodies.
20. **CI-01 / CI-03** CI runs fmt, clippy `-D warnings`, test, doc, cargo-deny with `--locked`;
    `Cargo.lock` is committed.

## Routing table

| Situation | Read |
|---|---|
| "Start/scaffold/set up a new Rust project"; choosing bin vs lib vs workspace, edition, MSRV, async | `references/project-kickoff.md` |
| Workspace manifests, `crates/` layout, shared deps/lints, when to split crates, xtask, build times | `references/workspace-layout.md` |
| Module tree, visibility, `pub(crate)`, re-export facade, god-modules, dependency direction | `references/modules-and-boundaries.md` |
| Ports & adapters, clean/hexagonal architecture, DI, traits vs generics vs `dyn`, fakes vs mocks | `references/layered-hexagonal.md` |
| axum/tokio HTTP service: state, handlers, error mapping, graceful shutdown, timeouts, health, DB pool | `references/web-services.md` |
| clap CLI: structure, exit codes, stdout/stderr, broken pipe, config precedence, progress | `references/cli-apps.md` |
| Publishable library: API surface, semver, feature flags, docs.rs, MSRV policy, no_std | `references/library-design.md` |
| Config files, env vars, secrets, validation, `.env` | `references/configuration.md` |
| Logging, spans, `#[instrument]`, log levels, OpenTelemetry, Prometheus metrics | `references/observability.md` |
| GitHub Actions CI, MSRV/semver/feature jobs, caching, release-plz, cargo-dist, cross-compiling | `references/ci-cd-release.md` |
| Copy-paste workspace/member `Cargo.toml` with lints, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, release profile | `references/templates.md` |
| Copy-paste `deny.toml`, `.github/workflows/ci.yml`, axum service and clap CLI `main.rs` skeletons | `references/templates-ci-and-skeletons.md` |

## Workflow for a new project

1. Read `project-kickoff.md`; write the ADR block with the chosen defaults.
2. Scaffold manifests from `templates.md`, then `deny.toml`, CI and the service (TPL-11) or CLI
   (TPL-12) skeleton from `templates-ci-and-skeletons.md`.
3. Lay out modules per `modules-and-boundaries.md`; add ports only per HEX-01.
4. Wire config (`configuration.md`) and telemetry (`observability.md`) in `main` before features.
5. Add CI (`ci-cd-release.md`) in the first commit, then run `cargo clippy --all-targets -- -D warnings`
   and `cargo test` locally before handing back.

## Versions and live data

Crate versions in these references were verified against crates.io on 2026-09-29 (see each
file's `crates:` frontmatter). When the `rustkb` MCP tools are available, prefer them for anything
version-sensitive: `crate_info` (current versions, features, MSRV), `search`/`get_item` (versioned
API docs), `check_advisories` (RustSec), `explain_lint` (rustc/clippy lint docs) and
`rust_release` (what changed in a Rust release). Never write a dependency version from memory.
