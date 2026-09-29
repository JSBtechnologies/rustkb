---
id: ecosystem/observability-crates
title: Logging, tracing, metrics and error-reporting crates
summary: >-
  tracing + tracing-subscriber as the logging/tracing default, the OpenTelemetry crate set and
  version lockstep, metrics exporters, the log-crate bridge, and error/panic reporting services.
area: ecosystem
tags: [tracing, logging, log, opentelemetry, otlp, metrics, prometheus, sentry, observability]
rust: "1.96"
edition: "2024"
crates:
  tracing: "0.1"
  tracing-subscriber: "0.3"
  tracing-appender: "0.2"
  tracing-opentelemetry: "0.34"
  opentelemetry: "0.33"
  opentelemetry_sdk: "0.33"
  opentelemetry-otlp: "0.33"
  opentelemetry-appender-tracing: "0.33"
  metrics: "0.24"
  metrics-exporter-prometheus: "0.18"
  prometheus-client: "0.25"
  log: "0.4"
  env_logger: "0.11"
  sentry: "0.49"
  tokio-console: "0.1"
  console-subscriber: "0.5"
  tracing-test: "0.2"
  test-log: "0.2"
verified: 2026-09-29
sources:
  - https://docs.rs/tracing/latest/tracing/
  - https://docs.rs/tracing-subscriber/latest/tracing_subscriber/
  - https://github.com/open-telemetry/opentelemetry-rust
  - https://docs.rs/metrics/latest/metrics/
---

# Logging, tracing, metrics and error-reporting crates

What to instrument, log levels, correlation IDs and dashboards are covered by the
`rust-architecture` observability reference. This file picks crates and shows the wiring.

## TEL-01: tracing is the instrumentation default

Default: `tracing` (0.1) for all logging and spans, in libraries and applications.
It records structured fields and spans that `log` cannot express, and bridges both ways.

- Libraries: depend on `tracing` only; **never install a subscriber** in a library.
- Use structured fields, not interpolated strings:
  `tracing::info!(user_id, order_id = %id, "order placed")`, not `info!("order {id} placed by {user_id}")`.
- `#[tracing::instrument(skip(secret, big_payload))]` creates a span per call; always `skip`
  secrets and large arguments (or `skip_all` + explicit `fields(...)`).
- Don't hold a `span.enter()` guard across `.await`; use `#[instrument]` or
  `future.instrument(span)`.

```rust
#[tracing::instrument(skip(password), fields(user.id = %user_id))]
async fn login(user_id: u64, password: &str) -> bool {
    tracing::info!(attempt = 1, "checking credentials");
    !password.is_empty()
}
```

## TEL-02: tracing-subscriber configures output in the binary

Default: `tracing-subscriber` (0.3) with the `env-filter` feature
(`RUST_LOG`-style filtering); add `json` for structured logs in production.

```rust
use tracing_subscriber::EnvFilter;

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env()) // RUST_LOG=info,my_crate=debug
        .init();
}
```

- Compose layers with `tracing_subscriber::registry().with(filter).with(fmt_layer).with(otel_layer)`
  when you need more than one sink.
- Log to stdout/stderr in containers; use `tracing-appender` (0.2) for
  non-blocking writers or rolling files on hosts that need files. Keep its `WorkerGuard`
  alive until exit or buffered lines are lost.
- `log`-based dependencies are captured automatically by `tracing-subscriber`'s default
  `tracing-log` bridge — don't also install `env_logger`.

## TEL-03: `log` + `env_logger` only for small or legacy code

`log` (0.4) is still the facade many libraries use and is fine for a library that
wants zero tracing dependency. `env_logger` (0.11) suits tiny tools. For
services and anything async, use tracing (TEL-01) — `log` has no spans, so concurrent
requests' lines can't be correlated.

## TEL-04: OpenTelemetry export — keep the crate set in lockstep

Default: `tracing` spans → `tracing-opentelemetry` (0.34) →
`opentelemetry_sdk` (0.33) → `opentelemetry-otlp` (0.33)
exporter → an OTel Collector.

The `opentelemetry*` crates are 0.x and release together; **all of them must share the same
minor** (currently 0.33), and `tracing-opentelemetry` must be the release built
for that minor (currently 0.34). Mixed minors produce "trait
`Tracer` is not implemented" errors that agents often try to fix with the wrong APIs.
`opentelemetry-jaeger` is unmaintained (RUSTSEC, 2025-11) — Jaeger ingests OTLP directly.

```rust
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

fn init_telemetry() -> anyhow::Result<SdkTracerProvider> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317")
        .build()?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(Resource::builder().with_service_name("my-service").build())
        .build();
    let tracer = provider.tracer("my-service");

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().json())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .init();
    Ok(provider) // call provider.shutdown() before exit to flush spans
}
```

Cargo features used above: `opentelemetry-otlp` with `grpc-tonic` + `trace`,
`tracing-subscriber` with `env-filter` + `json`. Use `http-proto` + a reqwest client feature
instead of tonic if you don't otherwise depend on tonic. To export *logs* via OTLP, add
`opentelemetry-appender-tracing` (0.33).

## TEL-05: Metrics

Default: the `metrics` (0.24) facade with `metrics-exporter-prometheus`
(0.18) for a Prometheus scrape endpoint. Libraries emit through
the facade; the binary picks the exporter.

```rust
fn record_login(ok: bool) {
    let result = if ok { "ok" } else { "denied" };
    metrics::counter!("login_attempts_total", "result" => result).increment(1);
}

fn install_exporter() -> anyhow::Result<()> {
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9000))
        .install()?; // needs the exporter's http-listener feature
    Ok(())
}
```

Alternatives:
- `prometheus-client` (0.25) — the official Prometheus/OpenMetrics client
  with typed metric families, when you want no facade.
- OpenTelemetry metrics (`opentelemetry_sdk` metrics + OTLP) when the whole pipeline is OTel.
- The older `prometheus` crate (TiKV) is still published but slower-moving; prefer the two above.

Keep label cardinality bounded: never use user IDs, raw paths or error messages as labels.

## TEL-06: Error and panic reporting services

- `sentry` (0.49) with its `tracing` integration for error tracking; initialise it
  first in `main` and keep the guard alive.
- For CLIs, `human-panic` / `color-eyre` (see `cli-and-tui.md`).
- `std::panic::set_hook` can forward panics into tracing (`tracing::error!(panic = %info)`)
  so they reach the same sink as logs.

## TEL-07: Debugging async runtimes and tests

- `tokio-console` (0.1) + `console-subscriber` (0.5):
  live view of tasks, wakeups and blocking; requires building with `--cfg tokio_unstable`.
  Enable only in debug/profiling builds.
- Capture logs in tests with `tracing-test` (0.2) (`#[traced_test]` +
  `logs_contain`) or `test-log`, rather than initialising a global subscriber in every test.
- CPU profiling tools (samply, cargo-flamegraph) are covered in `testing-and-benchmarking.md`.
