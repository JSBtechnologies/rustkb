---
id: architecture/templates-ci-and-skeletons
title: CI, deny and code skeleton templates
summary: >-
  Copy-paste deny.toml, GitHub Actions CI workflow (fmt, clippy, test, doc, deny, MSRV, feature
  powerset, semver) and minimal main.rs skeletons for an axum service and a clap CLI, all verified
  with cargo 1.96 and cargo-deny 0.20.
area: architecture
tags: [templates, cargo-deny, github-actions, ci, workflow, axum, clap, main, skeleton, scaffold]
rust: "1.96"
edition: "2024"
crates:
  anyhow: "1.0"
  axum: "0.8"
  clap: "4.6"
  metrics-exporter-prometheus: "0.18"
  secrecy: "0.10"
  serde_json: "1.0"
  thiserror: "2.0"
  tokio: "1.53"
  tokio-util: "0.7"
  tracing: "0.1"
  cargo-deny: "0.20"
  cargo-hack: "0.6"
  cargo-semver-checks: "0.50"
verified: 2026-09-29
sources:
  - https://embarkstudios.github.io/cargo-deny/
  - https://github.com/dtolnay/rust-toolchain
  - https://github.com/Swatinem/rust-cache
  - https://github.com/EmbarkStudios/cargo-deny-action
  - https://github.com/obi1kenobi/cargo-semver-checks-action
---

# CI, deny and code skeleton templates

Second half of the template set (manifests, toolchain, lint and profile files are in
`templates.md`, TPL-01…TPL-05 and TPL-08…TPL-10). Everything here passed `cargo deny check`,
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo test` in a scratch workspace on
Rust 1.96; the workflow YAML was parse-checked and every action tag was confirmed to exist on
2026-09-29. Rationale: `ci-cd-release.md`, `web-services.md`, `cli-apps.md`.

## Supply chain

### TPL-06: `deny.toml`

```toml
[graph]
all-features = true

[advisories]
unmaintained = "workspace"          # only flag unmaintained crates you depend on directly
yanked = "deny"
ignore = [
    # { id = "RUSTSEC-0000-0000", reason = "not reachable: we never call X" },
]

[licenses]
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "Zlib",
    "CDLA-Permissive-2.0",
]
confidence-threshold = 0.9

[licenses.private]
ignore = true                       # unpublished workspace crates need no license

[bans]
multiple-versions = "warn"
wildcards = "deny"
allow-wildcard-paths = true         # path deps inside the workspace
deny = [
    # { crate = "openssl", reason = "use rustls" },
]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

Adjust the license allow-list to your policy (this list passed `cargo deny check` for the axum/sqlx/tokio stack of `templates.md` TPL-01).
`cargo deny init` prints the fully commented default. Policy choices: the `rust-security` skill.

## Continuous integration

### TPL-07: `.github/workflows/ci.yml`

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

env:
  CARGO_TERM_COLOR: always

jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all --check

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --all-features --locked

  doc:
    runs-on: ubuntu-latest
    env:
      RUSTDOCFLAGS: -D warnings
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo doc --workspace --all-features --no-deps --locked

  deny:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: EmbarkStudios/cargo-deny-action@v2

  # ---- Libraries only: delete the jobs below for application-only repos ----
  msrv:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@1.85 # keep in sync with rust-version
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --workspace --all-features --locked

  features:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@cargo-hack
      - uses: Swatinem/rust-cache@v2
      - run: cargo hack check --workspace --feature-powerset --no-dev-deps

  semver:
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - uses: obi1kenobi/cargo-semver-checks-action@v2
```

Application repos with a `rust-toolchain.toml`: the pinned toolchain overrides
`dtolnay/rust-toolchain@stable` for cargo commands (CI-04), which is what you want. Release
workflows: `ci-cd-release.md` CI-08 (release-plz) and CI-09 (`dist init` generates its own).

## Code skeletons

### TPL-11: axum service `main.rs`

Complete version of the startup sequence from `web-services.md` (router, `ApiError`, config and
telemetry modules are shown there and in `configuration.md`/`observability.md`):

```rust,ignore
mod config;
mod http;
mod telemetry;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use metrics_exporter_prometheus::PrometheusBuilder;
use secrecy::ExposeSecret;
use shop_domain::OrderService;
use shop_postgres::PgOrderRepository;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::config::Settings;
use crate::http::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Config first: fail fast, before binding ports or opening pools.
    let settings = Settings::load()?;
    telemetry::init(settings.log.json);
    tracing::info!(?settings, "starting"); // secrets are redacted by SecretString

    // 2. Build adapters and services once (composition root).
    let db = shop_postgres::connect(
        settings.database.url.expose_secret(),
        settings.database.max_connections,
    )
    .await
    .context("connecting to database")?;
    let orders = Arc::new(OrderService::new(PgOrderRepository::new(db.clone())));
    let metrics = PrometheusBuilder::new()
        .install_recorder()
        .context("installing metrics recorder")?;

    // 3. Background work shares one cancellation token and one tracker.
    let shutdown = CancellationToken::new();
    let tasks = TaskTracker::new();
    tasks.spawn(cleanup_loop(shutdown.clone()));

    // 4. Serve until a signal arrives, then drain in-flight requests.
    let state = AppState { orders, db: db.clone(), metrics };
    let app = http::router(state, settings.http.request_timeout());
    let listener = TcpListener::bind(settings.http.addr)
        .await
        .with_context(|| format!("binding {}", settings.http.addr))?;
    tracing::info!(addr = %settings.http.addr, "listening");

    let token = shutdown.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await; // see web-services.md SVC-05
            token.cancel();
        })
        .await
        .context("server error")?;

    // 5. Stop background tasks (bounded), then close the pool.
    tasks.close();
    if tokio::time::timeout(Duration::from_secs(10), tasks.wait()).await.is_err() {
        tracing::warn!("background tasks did not finish within 10s");
    }
    db.close().await;
    tracing::info!("shutdown complete");
    Ok(())
}

async fn cleanup_loop(shutdown: CancellationToken) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = tick.tick() => tracing::debug!("cleanup tick"),
        }
    }
}
```

### TPL-12: clap CLI `main.rs`

```rust,ignore
mod cli;

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;

use crate::cli::{Cli, Command, OutputFormat};

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("input file not found: {0}")]
    InputMissing(std::path::PathBuf),
}

impl CliError {
    fn exit_code(&self) -> u8 {
        match self {
            Self::InputMissing(_) => 66, // EX_NOINPUT
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(err.downcast_ref::<CliError>().map_or(1, CliError::exit_code))
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    match &cli.command {
        Command::Import { path, .. } => {
            if !path.exists() {
                return Err(CliError::InputMissing(path.clone()).into());
            }
            let _text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            Ok(())
        }
        Command::List { format } => {
            let items = ["A-1", "B-2"];
            let mut out = BufWriter::new(io::stdout().lock());
            match format {
                OutputFormat::Text => {
                    for i in items {
                        writeln!(out, "{i}")?;
                    }
                }
                OutputFormat::Json => {
                    serde_json::to_writer_pretty(&mut out, &items)?;
                    writeln!(out)?;
                }
            }
            out.flush()?;
            Ok(())
        }
    }
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain()
        .filter_map(|e| e.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::BrokenPipe)
}
```

`cli.rs` (the clap types) is in `cli-apps.md` CLI-01.
