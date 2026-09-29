---
id: ecosystem/web-and-http
title: Web frameworks, middleware and HTTP clients
summary: >-
  axum + tower-http as the default server stack, when actix-web or others fit, OpenAPI and
  auth helpers, and reqwest (async) / ureq (blocking) as HTTP clients with rustls.
area: ecosystem
tags: [web, http, axum, tower, tower-http, hyper, actix-web, reqwest, ureq, openapi, rustls]
rust: "1.96"
edition: "2024"
crates:
  axum: "0.8"
  axum-extra: "0.12"
  tower: "0.5"
  tower-http: "0.7"
  hyper: "1.11"
  http: "1.5"
  actix-web: "4.15"
  utoipa: "6.0"
  reqwest: "0.13"
  ureq: "3.4"
  url: "2.5"
  tokio: "1.53"
  serde: "1.0"
  tracing: "0.1"
  tower-sessions: "0.15"
  governor: "0.10"
  tower_governor: "0.8"
  jsonwebtoken: "11.1"
  lettre: "0.11"
  aide: "0.15"
  axum-login: "0.18"
  axum-server: "0.8"
  oauth2: "5.0"
  openidconnect: "4.0"
  poem: "3.1"
  salvo: "1.0"
  reqwest-middleware: "0.5"
  reqwest-retry: "0.9"
  utoipa-axum: "0.3"
  utoipa-scalar: "0.4"
  utoipa-swagger-ui: "10.0"
  headers: "0.4"
  mime: "0.3"
  hyper-util: "0.1"
  http-body-util: "0.1"
  tokio-rustls: "0.26"
  rustls-pki-types: "1.15"
  argon2: "0.6"
  backon: "1.6"
verified: 2026-09-29
sources:
  - https://docs.rs/axum/latest/axum/
  - https://docs.rs/tower-http/latest/tower_http/
  - https://seanmonstar.com/blog/reqwest-v013-rustls-default/
  - https://docs.rs/ureq/latest/ureq/
---

# Web frameworks, middleware and HTTP clients

Service architecture (layering, handlers vs domain, config, shutdown orchestration) is in
`rust-architecture`; security hardening of endpoints in `rust-security`. This file picks crates.

## WEB-01: Default server stack is axum + tower-http

Default: `axum` (0.8) on `tokio`, with `tower-http` (0.7) middleware and
`tracing` for logs. axum has no macros-required routing, uses plain async functions as
handlers, and shares the tower `Service`/`Layer` ecosystem with tonic and hyper.

```toml
[dependencies]
axum = "0.8"
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "net", "signal"] }
tower-http = { version = "0.7", features = ["trace", "timeout", "cors", "compression-gzip"] }
serde = { version = "1.0", features = ["derive"] }
tracing = "0.1"
```

✅ Minimal production-shaped server (state, JSON, middleware, graceful shutdown):

```rust
use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::Serialize;
use tower_http::{compression::CompressionLayer, timeout::TimeoutLayer, trace::TraceLayer};

#[derive(Clone)]
struct AppState {
    greeting: Arc<str>,
}

#[derive(Serialize)]
struct Hello {
    message: String,
}

async fn hello(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Hello>, StatusCode> {
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(Json(Hello { message: format!("{}, {name}!", state.greeting) }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info,tower_http=debug").init();

    let state = AppState { greeting: Arc::from("Hello") };
    let app = Router::new()
        .route("/hello/{name}", get(hello))
        .route("/healthz", get(|| async { "ok" }))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(10)))
        .layer(CompressionLayer::new())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
```

Agent pitfalls with axum:
- axum 0.8 path syntax is `/{param}` and `/{*rest}`; the old `/:param` form panics at startup.
- `#[async_trait]` is no longer needed for custom extractors (`FromRequestParts` uses native
  async fn in traits since 0.8).
- Put shared state in `State<T>` with `T: Clone` (wrap non-Clone parts in `Arc`); don't use
  global statics or `Extension` for app state.
- Extractor order matters: the body-consuming extractor (`Json`, `Form`, `String`, `Bytes`)
  must be the last handler argument.
- `axum::serve` + `with_graceful_shutdown` replaces hand-rolled hyper server loops.
- Use `axum-extra` (0.12) for cookies, `TypedHeader`, `Query` with repeated keys,
  and typed routing — not a hand-written header parser.

## WEB-02: Middleware comes from tower-http and tower, not hand-written layers

Default: reach for an existing `tower-http` layer before writing middleware.

| Need | Layer |
|---|---|
| Request/response logging with spans | `tower_http::trace::TraceLayer` |
| Timeouts | `tower_http::timeout::TimeoutLayer` |
| CORS | `tower_http::cors::CorsLayer` (never `CorsLayer::permissive()` in production) |
| gzip/br/zstd | `CompressionLayer` / `RequestDecompressionLayer` |
| Request IDs | `SetRequestIdLayer` + `PropagateRequestIdLayer` |
| Body size limit | `RequestBodyLimitLayer` (axum also has `DefaultBodyLimit`) |
| Static files / SPA | `tower_http::services::ServeDir` |
| Hide secrets in logs | `SetSensitiveRequestHeadersLayer` |
| Catch panics → 500 | `CatchPanicLayer` |
| Concurrency/rate limit, load shed | `tower::limit`, `tower::load_shed` (feature-gated in `tower`) |
| Per-client rate limiting | `tower_governor` (0.8) on top of `governor` (0.10) |

Write custom middleware with `axum::middleware::from_fn` for app-specific logic (auth
checks, tenant resolution); implement a raw `tower::Layer` only for reusable, generic middleware.

## WEB-03: When to choose something other than axum

- **`actix-web`** (4.15): actively maintained, very fast, mature. Choose it for
  existing actix codebases or teams already fluent in it. It uses its own runtime wrapper
  (`actix-rt`) and its own middleware traits, so tower/tower-http layers don't plug in.
- **`poem` / `salvo`**: viable, smaller communities; pick only for a specific feature (e.g.
  poem-openapi's spec-first style).
- **`rocket`**: last release 0.5.1 (2024-05); don't start new projects on it.
- **`warp`**: still receives maintenance, but its filter-combinator types make errors and
  compile times painful; new services should use axum.
- **`tide`, `iron`, `nickel`, `gotham`**: unmaintained or effectively dead — never.
- **`hyper`** (1.11) directly: only for proxies, frameworks or protocol-level control.
  hyper 1.x needs `hyper-util` for the runtime glue that hyper 0.14 bundled.
- **Full-stack Rust UI** (SSR + WASM): Leptos or Dioxus (see `domain-specific.md`).

## WEB-04: OpenAPI with utoipa

Default: `utoipa` (6.0) for code-first OpenAPI generation from handler and type
annotations, with `utoipa-axum` for router integration and `utoipa-swagger-ui`/`utoipa-scalar`
for docs UIs. Keep the spec in CI (generate + diff) so API changes are reviewed.
Alternative: `aide` when you want the spec derived from axum extractors with less annotation.

## WEB-05: Sessions, auth and tokens

| Need | Default |
|---|---|
| Server-side sessions | `tower-sessions` (0.15) with a DB/Redis store |
| Login/session-based auth on axum | `axum-login` (built on tower-sessions) |
| JWT validate/issue | `jsonwebtoken` (11.1) — always pin allowed algorithms in `Validation` |
| OAuth2 / OIDC client | `oauth2`, `openidconnect` |
| Password hashing | `argon2` (see `domain-specific.md` → crypto) |

Security review of these flows (CSRF, cookie flags, token lifetimes) belongs to `rust-security`.

## WEB-06: Default HTTP client is reqwest (async)

Default: `reqwest` (0.13). Since 0.13, **rustls is the default TLS backend**
(aws-lc-rs provider), the `rustls-tls` feature is renamed `rustls`, and `json`, `query` and
`form` are opt-in features — enable what you call.

```toml
[dependencies]
reqwest = { version = "0.13", features = ["json"] }
```

✅ Build one `Client` and reuse it (it pools connections); set timeouts explicitly:

```rust
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
}

async fn latest_release(client: &reqwest::Client, repo: &str) -> reqwest::Result<Release> {
    client
        .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header(reqwest::header::USER_AGENT, "my-app/1.0")
        .send()
        .await?
        .error_for_status()?   // turn 4xx/5xx into Err
        .json::<Release>()
        .await
}

fn build_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
}
```

❌ Common agent mistakes:
- `reqwest::get(url)` or `Client::new()` inside a loop/handler — creates a new pool each time.
- Forgetting `.error_for_status()` so a 500 page is parsed as JSON and fails confusingly.
- Writing `features = ["rustls-tls"]` (pre-0.13 name) or `reqwest = "0.11"`/`"0.12"` from memory.
- Using `reqwest::blocking` inside an async runtime (panics) — use the async client there.

Add retries with `backon` (see `async-and-networking.md`) or `reqwest-middleware` +
`reqwest-retry` when you want middleware-style clients. For tower-based clients (e.g. a
hyper client with tower layers) use `hyper-util`'s client.

## WEB-07: Blocking HTTP client: ureq

Default for sync code (CLIs, build scripts, simple tools): `ureq` (3.4). No async
runtime, small dependency tree, rustls by default. ureq 3.x changed the API from 2.x
(`ureq::get(url).call()?.body_mut().read_json::<T>()?`); don't write 2.x code
(`.into_json()`) from memory.

```rust
fn fetch_ip() -> Result<String, ureq::Error> {
    let body = ureq::get("https://api.ipify.org")
        .header("User-Agent", "my-tool/1.0")
        .call()?
        .body_mut()
        .read_to_string()?;
    Ok(body)
}
```

Never pull in tokio + reqwest just to make one request from a synchronous CLI.

## WEB-08: HTTP building blocks

- `http` (1.5): shared `Request`/`Response`/`HeaderMap`/`StatusCode` types; use them
  in library APIs instead of framework types.
- `url` (2.5): parse and join URLs; never build URLs with `format!` over untrusted
  input without encoding (`Url::parse_with_params`, `url.query_pairs_mut()`).
- `mime` / `headers`: typed MIME and typed header values.
- `http-body-util`: `BodyExt::collect()` to read bodies in tests and low-level hyper code.
- Testing handlers: build the `Router` and call it with `tower::ServiceExt::oneshot` — no
  network needed (see `testing-and-benchmarking.md`).

## WEB-09: Email

Default: `lettre` (0.11) with its rustls feature for SMTP. For transactional email
providers (SES, Postmark, SendGrid) prefer their HTTP APIs via reqwest or the official SDK.

## WEB-10: TLS for servers

Default: terminate TLS at the load balancer/ingress. When the Rust process must terminate TLS,
use rustls: `axum-server` with its rustls feature, or `tokio-rustls` with a custom accept loop.
Load PEM files with `rustls-pki-types`' `PemObject` (the old `rustls-pemfile` crate is
archived — RUSTSEC-2025-0134). Details: `domain-specific.md` → TLS and `rust-security`.
