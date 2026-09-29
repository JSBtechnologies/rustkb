---
id: ecosystem/async-and-networking
title: Async runtime, utilities and networking crates
summary: >-
  Tokio as the default runtime and which companion crates to use for cancellation, streams,
  channels, retries, WebSockets, gRPC, QUIC, DNS and message brokers.
area: ecosystem
tags: [async, tokio, futures, smol, channels, retry, websocket, grpc, tonic, quic, messaging]
rust: "1.96"
edition: "2024"
crates:
  tokio: "1.53"
  tokio-util: "0.7"
  tokio-stream: "0.1"
  futures: "0.3"
  futures-lite: "2.6"
  async-trait: "0.1"
  pin-project-lite: "0.2"
  smol: "2.0"
  async-channel: "2.5"
  flume: "0.12"
  backon: "1.6"
  tokio-tungstenite: "0.30"
  tonic: "0.14"
  prost: "0.14"
  tonic-prost: "0.14"
  tonic-prost-build: "0.14"
  quinn: "0.11"
  hickory-resolver: "0.26"
  socket2: "0.6"
  async-nats: "0.50"
  rdkafka: "0.39"
  lapin: "4.12"
  governor: "0.10"
  futures-util: "0.3"
  pin-project: "1.1"
  prost-types: "0.14"
  async-tungstenite: "0.35"
  tokio-rustls: "0.26"
  crossbeam-channel: "0.5"
verified: 2026-09-29
sources:
  - https://tokio.rs/tokio/tutorial
  - https://github.com/tokio-rs/tokio#supported-rust-versions
  - https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html
  - https://rustsec.org/advisories/RUSTSEC-2025-0052.html
---

# Async runtime, utilities and networking crates

Which crates to pick for async IO. *How* to write correct async code (cancellation safety,
not blocking the executor, locks across `.await`) is covered by the `rust-idioms` skill;
service structure and graceful shutdown design by `rust-architecture`.

## NET-01: Default runtime is tokio

Default: `tokio` (1.53). The major async libraries — axum, hyper, reqwest, tonic, sqlx,
redis, tower — are built on it, so choosing anything else forks you from the ecosystem.

```toml
[dependencies]
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "net", "time", "signal", "sync"] }
```

- Applications may use `features = ["full"]`; libraries must enable only what they use.
- Tokio designates LTS minors (currently 1.51.x until March 2027 and 1.53.x until
  September 2027, per its README) — useful for conservative services.
- tokio 1.53 declares `rust-version = 1.71`; its README announces MSRV 1.85 for newer
  releases — check before promising an older MSRV for your crate.

Use **`smol`** (2.0) only for small tools or runtime-agnostic experiments where tokio's
ecosystem is not needed. Use **`embassy-executor`** for embedded/no_std (see `domain-specific.md`).
**Never** start new work on `async-std`: it is discontinued (RUSTSEC-2025-0052) and its
maintainers point users to `smol`.

## NET-02: Libraries should not force a runtime

Default: library crates depend on runtime-agnostic traits (`futures-core`/`futures-util`,
`http`, `bytes`) and let the application pick the runtime. When a library genuinely needs
timers, spawning or sockets, depend on tokio with the minimum features and document it.

- Don't create a runtime inside a library function (`Runtime::new()` / `#[tokio::main]` in a
  lib): it panics when called from within another runtime.
- Offer a blocking API by wrapping a *caller-provided* or internal current-thread runtime
  only in a clearly separate `blocking` module (the `reqwest::blocking` pattern).

## NET-03: tokio-util for cancellation, graceful shutdown and framing

Default: `tokio-util` (0.7) for `CancellationToken`, `TaskTracker`, codecs
(`Framed`, `LengthDelimitedCodec`, `LinesCodec`) and IO/Stream bridges.

✅ Graceful shutdown of background tasks:

```rust
use std::time::Duration;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[tokio::main]
async fn main() {
    let token = CancellationToken::new();
    let tracker = TaskTracker::new();

    for id in 0..4 {
        let token = token.clone();
        tracker.spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        tracing::debug!(id, "tick");
                    }
                }
            }
        });
    }

    tokio::signal::ctrl_c().await.expect("install Ctrl-C handler");
    token.cancel();      // ask every task to stop
    tracker.close();     // no more tasks will be spawned
    tracker.wait().await // wait for all of them to finish
}
```

❌ Don't hand-roll shutdown with `Arc<AtomicBool>` polling loops or `broadcast` channels
used as one-shot signals — `CancellationToken` supports child tokens and `select!` directly.

## NET-04: futures for combinators; keep feature footprint small

Default: `futures` (0.3) in applications for `StreamExt`, `join_all`,
`FuturesUnordered`, `select_all`. In libraries prefer `futures-util` with
`default-features = false` plus only the features you need.

- `tokio-stream` (0.1) only for wrappers such as `ReceiverStream`,
  `IntervalStream`, `BroadcastStream`; don't add it just for `StreamExt`.
- `futures-lite` (2.6) in the smol ecosystem or when compile time matters.
- For bounded concurrency over a stream: `stream.map(fut).buffer_unordered(n)` — not
  spawning an unbounded number of tasks.

## NET-05: async fn in traits — native first, async-trait for dyn

Default: write `async fn` directly in traits (stable since 1.75) when used with generics.
Use `async-trait` (0.1) only when you need `dyn Trait` (e.g. `Box<dyn Store>`
in a plugin registry): native async-fn traits are still not dyn-compatible on Rust 1.96.

```rust
// Static dispatch: no macro needed. (For a `pub` trait, rustc's `async_fn_in_trait` lint
// asks you to decide on `Send` bounds — see below.)
trait Store {
    async fn get(&self, key: &str) -> Option<String>;
}
```

If the returned future must be `Send` for generic callers that spawn it, declare the method
as `fn get(&self, key: &str) -> impl Future<Output = Option<String>> + Send;` (implementers can
still write `async fn`). Deeper guidance: `rust-idioms` async reference.

## NET-06: Channels — use the runtime's own first

Default inside tokio: `tokio::sync::{mpsc, oneshot, broadcast, watch}`.

| Need | Crate |
|---|---|
| async ↔ async inside tokio | `tokio::sync::mpsc` (bounded by default choice) |
| latest-value config/state fan-out | `tokio::sync::watch` |
| sync thread ↔ async task | `flume` (0.12) or tokio mpsc with `blocking_send`/`blocking_recv` |
| runtime-agnostic MPMC | `async-channel` (2.5) |
| sync-only MPSC | `std::sync::mpsc` |
| sync MPMC with `select!` | `crossbeam-channel` |

Always prefer **bounded** channels; unbounded channels turn backpressure bugs into memory leaks.

## NET-07: Retries with backon; rate limits with governor

Default: `backon` (1.6) for retry with exponential backoff and jitter (sync or async).
`backoff` is unmaintained (RUSTSEC-2025-0012) — don't add it. `tokio-retry` still works but
is tokio-only and less featureful; prefer backon for new code.

```rust
use backon::{ExponentialBuilder, Retryable};

async fn fetch() -> Result<String, std::io::Error> {
    Ok("ok".to_owned())
}

async fn fetch_with_retry() -> Result<String, std::io::Error> {
    fetch
        .retry(ExponentialBuilder::default().with_max_times(5).with_jitter())
        .when(|e| e.kind() == std::io::ErrorKind::TimedOut)
        .await
}
```

For client-side rate limiting use `governor` (0.10); for server middleware,
see `web-and-http.md`. For tower services, `tower::retry` and `tower::limit` are alternatives.

## NET-08: WebSockets

Default server: axum's built-in `ws` feature (`axum::extract::ws`). Default client, or a
server outside axum: `tokio-tungstenite` (0.30) — enable a TLS feature
(`rustls-tls-webpki-roots` or `rustls-tls-native-roots`) for `wss://`.
Use `async-tungstenite` only when you need runtime-agnostic code.

## NET-09: gRPC with tonic + prost

Default: `tonic` (0.14) with `prost` (0.14) messages. Since tonic 0.14 the prost
integration lives in separate crates: `tonic-prost` (runtime codec, a normal dependency) and
`tonic-prost-build` (codegen, a build-dependency used from `build.rs`); older examples that
call `tonic_build::compile_protos` from memory are out of date. Keep `.proto` files in the repo.

- Add `tower-http`/`tower` layers for tracing, timeouts and concurrency limits exactly
  as for axum.
- For browser clients, add gRPC-Web support or expose a JSON API alongside.
- `prost-types` provides well-known types (`Timestamp`, `Duration`, `Any`).

## NET-10: Lower-level networking

| Need | Default |
|---|---|
| TCP/UDP/Unix sockets | `tokio::net` |
| Socket options std/tokio don't expose (SO_REUSEPORT, keepalive tuning) | `socket2` (0.6) |
| QUIC / HTTP/3 transport | `quinn` (0.11) |
| Custom DNS resolution, DoH/DoT | `hickory-resolver` (0.26) — formerly `trust-dns-resolver` |
| TLS on raw streams | `tokio-rustls` (see `domain-specific.md` and `rust-security`) |

Never add `mio` directly unless you are writing a runtime; tokio wraps it.

## NET-11: Message brokers

| Broker | Default crate | Notes |
|---|---|---|
| NATS / JetStream | `async-nats` (0.50) | official client |
| Kafka | `rdkafka` (0.39) | wraps librdkafka (C); needs cmake or a system lib |
| RabbitMQ (AMQP 0.9.1) | `lapin` (4.12) | |
| Redis streams / pub-sub | `redis` (see `databases.md`) | |
| Cloud queues (SQS, Pub/Sub) | the official cloud SDKs (`aws-sdk-sqs`, …) | |

Wrap broker clients behind a small trait/port in your application so tests can use an
in-memory fake (see `rust-architecture`).

## NET-12: Pin projection

Default: `pin-project-lite` (0.2) when hand-writing futures or streams
that need structural pinning; it is a declarative macro with no proc-macro build cost.
Use `pin-project` (proc macro) only for features lite lacks (e.g. `PinnedDrop` on complex types).
Most application code never needs either — prefer `async` blocks and `tokio::pin!`/`std::pin::pin!`.
