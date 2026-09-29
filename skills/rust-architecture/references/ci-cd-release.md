---
id: architecture/ci-cd-release
title: CI, CD and releases
summary: >-
  The CI gate set for Rust repositories on GitHub Actions (fmt, clippy -D warnings, tests, docs,
  MSRV, cargo-deny, feature powerset, semver checks), caching and hardening, library releases with
  release-plz, binary releases with cargo-dist, reproducible builds and cross-compilation.
area: architecture
tags: [ci, github-actions, clippy, rustfmt, msrv, cargo-deny, cargo-semver-checks, cargo-hack, release-plz, cargo-dist, cross-compilation, reproducible-builds, caching]
rust: "1.96"
edition: "2024"
crates:
  cargo-deny: "0.20"
  cargo-semver-checks: "0.50"
  cargo-hack: "0.6"
  cargo-nextest: "0.9"
  release-plz: "0.3"
  cargo-dist: "0.32"
  cargo-auditable: "0.7"
  cargo-zigbuild: "0.23"
  cargo-chef: "0.1"
verified: 2026-09-29
sources:
  - https://github.com/dtolnay/rust-toolchain
  - https://github.com/Swatinem/rust-cache
  - https://github.com/EmbarkStudios/cargo-deny-action
  - https://github.com/obi1kenobi/cargo-semver-checks-action
  - https://github.com/taiki-e/install-action
  - https://release-plz.dev/docs/github/quickstart
  - https://axodotdev.github.io/cargo-dist/book/
---

# CI, CD and releases

Action versions below were checked on 2026-09-29: `actions/checkout@v7`,
`dtolnay/rust-toolchain@stable` (branch-based, no releases), `Swatinem/rust-cache@v2`,
`EmbarkStudios/cargo-deny-action@v2`, `obi1kenobi/cargo-semver-checks-action@v2`,
`taiki-e/install-action@v2` (or `@<tool>`), `release-plz/action@v0.5`. The complete CI workflow is
`templates-ci-and-skeletons.md` TPL-07.

## The gate set

### CI-01: Every PR runs fmt, clippy, tests, docs and cargo-deny

| Job | Command | Why |
|---|---|---|
| fmt | `cargo fmt --all --check` | No style debates in review |
| clippy | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Lints (incl. `[workspace.lints]`) are errors in CI, warnings locally |
| test | `cargo test --workspace --all-features --locked` | Unit, integration and doc tests |
| doc | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked` | Broken intra-doc links |
| deny | `cargo deny check` (via `cargo-deny-action`) | Advisories, licenses, banned/duplicate crates, sources |

Libraries add (CI-06, CI-07):

| Job | Command |
|---|---|
| msrv | `cargo check --workspace --all-features --locked` on the `rust-version` toolchain |
| features | `cargo hack check --workspace --feature-powerset --no-dev-deps` |
| semver | `cargo-semver-checks-action` on pull requests |

`--all-targets` makes clippy see tests, benches and examples. `--all-features` alone never tests
`--no-default-features`; that is what the features job is for. `cargo-nextest` is a faster test
runner for big workspaces, but it does not run doctests — keep a `cargo test --doc` step if you
switch.

### CI-02: Deny warnings in CI via flags, not a global `RUSTFLAGS`

Default: `-D warnings` on the clippy command line and `RUSTDOCFLAGS` scoped to the doc job. A
workflow-wide `RUSTFLAGS: -D warnings` changes the compiler flags for every job and every
dependency, so caches built by other jobs (or `cargo install` steps) stop matching and everything
recompiles. Never put `#![deny(warnings)]` in source: a new compiler lint would break downstream
builds of a library.

### CI-03: `--locked` everywhere; lockfile updates are their own PRs

`--locked` fails the build if `Cargo.lock` is out of date instead of silently re-resolving
(WS-10). Let Dependabot or Renovate propose updates for both ecosystems:

```yaml
# .github/dependabot.yml
version: 2
updates:
  - package-ecosystem: cargo
    directory: /
    schedule: { interval: weekly }
    groups:
      cargo-minor: { update-types: [minor, patch] }
  - package-ecosystem: github-actions
    directory: /
    schedule: { interval: weekly }
```

## Speed and matrix

### CI-04: Install the toolchain explicitly, then cache

- `dtolnay/rust-toolchain@stable` (or `@1.85` for MSRV, `@nightly` for Miri/fuzz) with
  `components: clippy` / `rustfmt` as needed. In a repo with `rust-toolchain.toml`, cargo
  commands use the pinned toolchain (rustup installs it on first use) regardless of what the
  action installed — so list `clippy` and `rustfmt` under `components` in that file too.
- `Swatinem/rust-cache@v2` **after** the toolchain step (the toolchain version is part of the cache
  key). It caches `~/.cargo` and dependency builds in `target/`, not workspace crates.
- Cancel superseded PR runs with a `concurrency` group.
- Split fast jobs (fmt, deny) from slow ones so feedback on trivial mistakes is quick.

### CI-05: Matrix only what varies

Run `test` on `ubuntu-latest`, `windows-latest` and `macos-latest` when the code touches paths,
processes, signals, terminals or filesystems — or when you ship binaries for those OSes. Run
clippy, doc, deny, msrv and semver once on Linux. Don't matrix over stable/beta/nightly unless you
maintain a widely used library (then a scheduled beta job catches regressions early).

## Library-specific gates

### CI-06: Verify the MSRV you declare

Use the exact `rust-version` from `[workspace.package]` in a separate job:

```yaml
msrv:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v7
      with: { persist-credentials: false }
    - uses: dtolnay/rust-toolchain@1.85   # keep in sync with rust-version
    - uses: Swatinem/rust-cache@v2
    - run: cargo check --workspace --all-features --locked
```

Check (not test) on the MSRV: dev-dependencies often need newer compilers. If the repo has a
`rust-toolchain.toml`, it overrides the action's choice — set `RUSTUP_TOOLCHAIN: "1.85"` in the
job's `env` to force the MSRV toolchain. Alternative:
`cargo hack check --rust-version --workspace` reads each package's `rust-version`. Resolver 3's
MSRV-aware resolution keeps the committed lockfile compatible (ARCH-04).

### CI-07: Semver-check every PR and every release

`obi1kenobi/cargo-semver-checks-action@v2` compares the PR against the latest published version on
crates.io and fails on API breaks without a matching version bump. It installs its own stable
toolchain and ignores `rust-toolchain.toml` by design (it needs a recent rustdoc JSON format).
Unpublished crates have no baseline — mark internal crates `publish = false` (WS-11). release-plz
runs the same checks when preparing a release.

## Releasing

### CI-08: Libraries: release-plz opens the release PR and publishes on merge

Default for crates.io libraries: `release-plz` computes version bumps from commits and semver
checks, updates `CHANGELOG.md`, opens/updates a release PR, and when that PR merges, publishes the
crates, tags them and creates GitHub releases. Minimal workflow (from the release-plz quickstart):

```yaml
name: Release-plz
on:
  push:
    branches: [main]
jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write
      pull-requests: read
      id-token: write          # crates.io trusted publishing; no CARGO_REGISTRY_TOKEN secret
    steps:
      - uses: actions/checkout@v7
        with: { fetch-depth: 0, persist-credentials: false }
      - uses: dtolnay/rust-toolchain@stable
      - uses: release-plz/action@v0.5
        with: { command: release }
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
  release-pr:
    runs-on: ubuntu-latest
    permissions:
      contents: write
      pull-requests: write
    concurrency:
      group: release-plz-${{ github.ref }}
      cancel-in-progress: false
    steps:
      - uses: actions/checkout@v7
        with: { fetch-depth: 0, persist-credentials: false }
      - uses: dtolnay/rust-toolchain@stable
      - uses: release-plz/action@v0.5
        with: { command: release-pr }
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

Repository settings must allow Actions to create pull requests. Trusted publishing must be
configured per crate on crates.io, and a brand-new crate's first publish has to be done manually;
otherwise use a `CARGO_REGISTRY_TOKEN` secret. Workspaces can also publish in dependency order with
`cargo publish --workspace` (stable Cargo).

### CI-09: Binaries: cargo-dist builds, packages and uploads on tag push

Default for CLIs and other distributed binaries: install `dist` and run `dist init` — it
interactively writes its config and `.github/workflows/release.yml`. Releasing is then: bump the
version, commit, push a tag like `v0.4.0`; CI builds every target, creates archives, checksums and
optional shell/PowerShell/Homebrew/MSI installers, and publishes a GitHub release. `dist plan`
previews what CI will build; `dist build` builds locally. Re-run `dist init` after upgrading dist.
It requires `repository` in `Cargo.toml`. Don't hand-write a matrix of `cargo build --target`
jobs plus upload scripts unless dist cannot do what you need.

Services usually ship as container images instead: multi-stage Dockerfile, `cargo-chef` to cache
dependency layers, `--locked` build, copy the single binary into a small runtime image
(distroless/`debian-slim`, or `scratch` for static musl builds).

### CI-10: Reproducible, auditable release builds

- `--locked`, a pinned toolchain (`rust-toolchain.toml` for apps), and release profile settings
  committed in `Cargo.toml` (TPL-08) — not ad-hoc `RUSTFLAGS` in CI.
- Build releases in CI only, from a tag, never from a developer machine.
- `cargo auditable build --release` embeds the dependency list in the binary so scanners
  (`cargo audit bin`, Trivy, Syft) can check shipped artifacts for advisories.
- Publish checksums (dist does) and consider GitHub artifact attestations for provenance.

### CI-11: Cross-compile on native runners first

| Target | Default approach |
|---|---|
| Linux x86_64 / aarch64 (glibc) | Native runners: `ubuntu-latest`, `ubuntu-24.04-arm` |
| Windows x86_64 / aarch64 | `windows-latest`, `windows-11-arm` |
| macOS arm64 / x86_64 | `macos-latest` (arm64); `x86_64-apple-darwin` target via `rustup target add` |
| Linux musl (static) | `rustup target add x86_64-unknown-linux-musl`; pure-Rust deps (rustls) |
| Older glibc baseline | `cargo-zigbuild` (`cargo zigbuild --target x86_64-unknown-linux-gnu.2.28`) |
| Exotic targets (armv7, riscv, …) | `cross` (Docker-based; install from its git repository — the crates.io release is old) |

C dependencies (OpenSSL, libgit2 without vendoring) are the usual cross-compilation blocker;
choose pure-Rust alternatives early (ARCH-10). cargo-dist drives most of these setups for you.

## Hardening

### CI-12: Least-privilege workflows

- Top-level `permissions: contents: read`; grant `contents: write`/`pull-requests: write`/
  `id-token: write` only on the release jobs that need them.
- `actions/checkout` with `persist-credentials: false` unless a later step pushes.
- Never run untrusted PR code with secrets: no `pull_request_target` + checkout of the PR head.
- High-assurance repos pin third-party actions to full commit SHAs (Dependabot keeps them
  updated); first-party `actions/*` on major tags is the common baseline.
- Supply-chain policy (`deny.toml` contents, `cargo vet`, advisories): the `rust-security` skill.

## Checklist

### CI-13: CI/CD checklist

- [ ] fmt, clippy `-D warnings`, test, doc, deny on every PR; `--locked` everywhere.
- [ ] Toolchain action before `Swatinem/rust-cache@v2`; concurrency cancels stale runs.
- [ ] OS matrix only for OS-sensitive code or shipped targets.
- [ ] Libraries: MSRV check, `cargo hack --feature-powerset`, semver checks, release-plz.
- [ ] Binaries: `dist init` release workflow, or container build with `cargo-chef`.
- [ ] Release builds from tags in CI, `cargo auditable`, checksums.
- [ ] `permissions: contents: read` by default; no persisted checkout credentials.
- [ ] Dependabot/Renovate for `cargo` and `github-actions`.
