---
id: architecture/observability
title: "Observability: tracing, metrics, OpenTelemetry"
summary: >-
  Where and how to instrument Rust code: tracing events and spans everywhere, subscriber setup
  only in binaries, structured fields, #[instrument] hygiene, log levels, never logging secrets,
  span propagation across tasks, OpenTelemetry export via tracing-opentelemetry, and metrics.
area: architecture
tags: [observability, tracing, tracing-subscriber, logging, spans, instrument, opentelemetry, otlp, metrics, prometheus]
rust: "1.96"
edition: "2024"
crates:
  tracing: "0.1"
  tracing-subscriber: "0.3"
  tracing-opentelemetry: "0.34"
  opentelemetry: "0.33"
  opentelemetry_sdk: "0.33"
  opentelemetry-otlp: "0.33"
  metrics: "0.24"
  metrics-exporter-prometheus: "0.18"
  tower-http: "0.7"
  secrecy: "0.10"
  anyhow: "1.0"
verified: 2026-09-29
sources:
  - https://docs.rs/tracing/0.1/tracing/
  - https://docs.rs/tracing-subscriber/0.3/tracing_subscriber/
  - https://docs.rs/tracing-opentelemetry/0.34/tracing_opentelemetry/
  - https://opentelemetry.io/docs/languages/rust/
  - https://docs.rs/metrics/0.24/metrics/
---

# Observability: tracing, metrics, OpenTelemetry

Default: the `tracing` crate for all diagnostic output (events *and* spans), `tracing-subscriber`
configured once in each binary, `metrics` for counters/histograms, and OpenTelemetry export
added in `main` when there is a collector. All snippets compile on Rust 1.96 with the versions in
the frontmatter.

## Who does what

### OBS-01: Every crate emits with `tracing`; only binaries install a subscriber

- Libraries and domain crates depend on `tracing` only and call `info!`, `warn!`,
  `#[instrument]`. They never depend on `tracing-subscriber`, `env_logger` or an exporter.
- Binaries install exactly one global subscriber at the top of `main` (after config is loaded,
  before anything that logs).

❌ Common agent failure: a library `fn new()` that calls `tracing_subscriber::fmt::init()` or
`env_logger::init()` — it panics or silently loses output when the application also installs one,
and it takes the output decision away from the application.

Don't use `println!`/`eprintln!` for diagnostics in services, and don't start new code on `log` +
`env_logger`. (If a dependency uses `log`, `tracing-subscriber`'s default `tracing-log` feature
forwards those records into your subscriber.)

### OBS-02: Subscriber setup: `EnvFilter` + pretty locally, JSON in production

```rust
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Install the global subscriber. Binaries only — libraries never call this.
pub(crate) fn init(json: bool) {
    // RUST_LOG wins; otherwise info for us, less for noisy deps.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn"));

    let registry = tracing_subscriber::registry().with(filter);
    if json {
        registry.with(fmt::layer().json().with_current_span(true)).init();
    } else {
        registry.with(fmt::layer()).init();
    }
}
```

- Features: `tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }`.
- `fmt` writes to **stdout** by default — right for containers, wrong for CLIs (use
  `.with_writer(std::io::stderr)`, `cli-apps.md` CLI-04).
- In tests, don't install a global subscriber per test; if you need output, use
  `tracing_subscriber::fmt().with_test_writer().try_init()` (ignore the error when already set).

## Writing good events and spans

### OBS-03: Structured fields, not formatted strings

```rust,ignore
// ❌ values baked into the message: unsearchable, ungroupable
tracing::info!("placed order {} for {} x{}", id, sku, qty);

// ✅ fields are indexable in any log backend; message is a constant
tracing::info!(order_id = id.0, sku = %sku.as_str(), quantity = qty, "order placed");
```

`%` records with `Display`, `?` with `Debug`, bare values for primitives. Keep the message a
fixed string so events can be grouped. Use consistent field names across the codebase
(`order_id`, `user_id`, `error`).

### OBS-04: `#[instrument]` with `skip_all` (or `skip`) and explicit fields

```rust,ignore
#[tracing::instrument(skip(state, body), fields(sku = %body.sku.as_str()))]
pub(super) async fn create(State(state): State<AppState>, Json(body): Json<CreateOrder>)
    -> Result<(StatusCode, Json<OrderId>), ApiError> { … }

#[tracing::instrument(skip_all, fields(order_id = id.0), err)]
async fn charge(&self, id: OrderId, card: &SecretString) -> Result<Receipt, PaymentError> { … }
```

- By default `#[instrument]` records **every argument** with `Debug` — large bodies, whole state
  structs, secrets. Default to `skip_all` and list the fields you want.
- `err` records a returned `Err` as an error event; `ret` records the return value (use sparingly).
- Instrument boundaries (handlers, service methods, outbound calls, jobs), not tiny helpers or
  hot loops.

### OBS-05: Levels mean something; log an error once

| Level | Use for | Volume |
|---|---|---|
| `error!` | Something failed and needs a human (5xx, job gave up, data loss risk) | Rare |
| `warn!` | Degraded but handled (retry succeeded later, fallback used, readiness failed) | Low |
| `info!` | Lifecycle and business events (startup, config summary, order placed, shutdown) | Moderate |
| `debug!` | Diagnostic detail for developers | Off in prod |
| `trace!` | Very verbose internals | Off |

Log an error where it is **handled** (the HTTP error mapper, the job runner, `main`), not at every
layer it passes through — otherwise one failure produces five log lines. Lower layers add
context to the error instead (`idioms/error-handling`).

### OBS-06: Never log secrets or personal data

- Credentials are `secrecy::SecretString` (`configuration.md` CFG-04), so `?settings` is safe.
- `#[instrument(skip_all)]` on anything handling passwords, tokens, card data.
- Don't log full request/response bodies or headers (`Authorization`, `Cookie`) by default.
- Hash or truncate identifiers where policy requires; the `rust-security` skill covers PII.

### OBS-07: Propagate spans into spawned tasks with `.instrument`

Spans follow the *future*, not the thread. A spawned task starts with no parent span unless you
attach one:

```rust,ignore
use tracing::Instrument;

let span = tracing::info_span!("reindex", job_id = %job.id);
tokio::spawn(async move { reindex(job).await }.instrument(span));
```

Never hold a `span.enter()` guard across an `.await` — the guard stays entered while other tasks
run on the thread, attributing their events to your span. Use `.instrument(span)` or
`#[instrument]` for async code; `enter()` only in synchronous code.

## HTTP services

### OBS-08: Request spans and request ids at the edge

Use tower-http's `TraceLayer::new_for_http()` for a span per request (method, URI, version)
with response status and latency recorded when it completes, plus
`SetRequestIdLayer`/`PropagateRequestIdLayer` so logs and responses share an
`x-request-id` (`web-services.md` SVC-06). Handlers' `#[instrument]` spans nest inside the request
span automatically.

## OpenTelemetry

### OBS-09: Export traces with `tracing-opentelemetry`; match its OpenTelemetry version

Keep using `tracing` macros; add an OpenTelemetry layer in `main`. The OTel crates move fast and
must be version-matched: `tracing-opentelemetry` 0.34 pairs with `opentelemetry`,
`opentelemetry_sdk` and `opentelemetry-otlp` 0.33. Check `tracing-opentelemetry`'s dependency on
`opentelemetry` before upgrading either.

```rust
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Returns the provider so `main` can flush it on shutdown.
fn init_telemetry(service: &'static str) -> anyhow::Result<SdkTracerProvider> {
    // Endpoint, headers, protocol come from the standard OTEL_EXPORTER_OTLP_* env vars.
    let exporter = SpanExporter::builder().with_http().build()?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(Resource::builder().with_service_name(service).build())
        .build();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer())
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer(service)))
        .init();
    Ok(provider)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = init_telemetry("shop-server")?;
    tracing::info!("running");
    // ... serve until shutdown ...
    provider.shutdown()?; // flush buffered spans before exit
    Ok(())
}
```

- Call `provider.shutdown()` after graceful shutdown, or the last batch of spans is lost.
- Make export optional (enabled when `OTEL_EXPORTER_OTLP_ENDPOINT` is set or by config) so local
  runs don't need a collector.
- Incoming/outgoing trace-context propagation (W3C `traceparent`) needs a propagator plus
  middleware on the HTTP server and client; follow the `opentelemetry` docs for the version you pin
  rather than copying older blog posts — the APIs changed substantially across 0.2x releases.

## Metrics

### OBS-10: `metrics` facade in code, exporter chosen in `main`

Like `tracing`, the `metrics` crate separates emitting (anywhere) from recording (binary only):

```rust,ignore
// anywhere, including libraries
metrics::counter!("orders_placed_total").increment(1);
metrics::histogram!("payment_latency_seconds").record(elapsed.as_secs_f64());

// main: Prometheus text format served on /metrics
let handle = metrics_exporter_prometheus::PrometheusBuilder::new()
    .install_recorder()
    .context("installing metrics recorder")?;
// route: .route("/metrics", get(move || async move { handle.render() }))
```

Rules: measure RED for every service (Rate, Errors, Duration per route) and USE for pools/queues;
suffix units (`_seconds`, `_bytes`, `_total`); keep label cardinality bounded — never label with
user ids, raw URLs or error messages. If you already export traces via OTel, OTel metrics are an
alternative; don't run both for the same numbers.

## Checklist

### OBS-11: Observability checklist

- [ ] `tracing` in all crates; subscriber and exporters only in binaries.
- [ ] `EnvFilter` honouring `RUST_LOG`; JSON output in production.
- [ ] Structured fields with constant messages; consistent field names.
- [ ] `#[instrument(skip_all, fields(..))]` on boundaries; no secrets in spans.
- [ ] Errors logged once, where handled; levels per OBS-05.
- [ ] Spawned tasks carry spans via `.instrument`.
- [ ] Request spans + request ids on HTTP services.
- [ ] OTel crates version-matched; provider flushed on shutdown.
- [ ] RED metrics with bounded labels; `/metrics` or OTLP export.
