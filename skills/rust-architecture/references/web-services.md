---
id: architecture/web-services
title: Web service structure (axum)
summary: >-
  How to structure an axum/tokio HTTP service: crate and module layout, AppState, thin handlers,
  one error type mapped to responses, tower-http layers for timeouts/limits/tracing/request ids,
  graceful shutdown with background tasks, DB pools, health checks and router tests.
area: architecture
tags: [axum, tokio, tower, tower-http, web, http, service, graceful-shutdown, state, health-check, sqlx]
rust: "1.96"
edition: "2024"
crates:
  axum: "0.8"
  tokio: "1.53"
  tokio-util: "0.7"
  tower: "0.5"
  tower-http: "0.7"
  sqlx: "0.9"
  serde: "1.0"
  serde_json: "1.0"
  thiserror: "2.0"
  anyhow: "1.0"
  tracing: "0.1"
  metrics: "0.24"
  metrics-exporter-prometheus: "0.18"
  reqwest: "0.13"
  rayon: "1.12"
verified: 2026-09-29
sources:
  - https://docs.rs/axum/0.8/axum/
  - https://github.com/tokio-rs/axum/tree/main/examples/graceful-shutdown
  - https://docs.rs/tower-http/0.7/tower_http/
  - https://docs.rs/tokio-util/0.7/tokio_util/task/task_tracker/struct.TaskTracker.html
---

# Web service structure (axum)

Default stack: `tokio` + `axum` 0.8 + `tower-http` 0.7 layers + `sqlx` for Postgres + `tracing`.
All code below compiles against those versions (checked with `cargo clippy -D warnings`, pedantic).

## Layout

### SVC-01: Composition root in `main.rs`, HTTP in `http/`, domain elsewhere

```text
crates/shop-server/src/
├── main.rs        # config → telemetry → pools/adapters → services → serve → drain
├── config.rs      # typed Settings (configuration.md)
├── telemetry.rs   # subscriber init (observability.md)
├── http.rs        # AppState, router(), health endpoints, layers
└── http/
    ├── error.rs   # ApiError + IntoResponse
    └── orders.rs  # handlers for one resource
```

Business rules live in the domain crate/module (`layered-hexagonal.md`), never in handlers.
`main.rs` stays readable top-to-bottom as the startup sequence.

## State and handlers

### SVC-02: One `Clone` `AppState` of cheap handles; no reflexive `Mutex`

Default: `AppState` holds `Arc`s and pools (which are already `Arc` inside), is `#[derive(Clone)]`,
and is passed with `Router::with_state`. It is read-only; mutable shared state is the exception
and belongs to the component that owns it (a DB, a cache type with its own internal locking).

```rust,ignore
pub(crate) type Orders = OrderService<PgOrderRepository>; // generics stop here (HEX-05)

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) orders: Arc<Orders>,
    pub(crate) db: PgPool,
    pub(crate) metrics: PrometheusHandle,
}
```

Never `Arc<Mutex<AppState>>` or `Arc<Mutex<PgPool>>` — it serialises every request. If a handler
needs only part of the state, implement `axum::extract::FromRef` for sub-states rather than
cloning large structs.

### SVC-03: Handlers are thin adapters: extract → call → map

```rust,ignore
#[derive(Debug, Deserialize)]
pub(super) struct CreateOrder {
    sku: Sku,          // domain newtype validates during deserialisation
    quantity: u32,
}

#[tracing::instrument(skip(state, body), fields(sku = %body.sku.as_str()))]
pub(super) async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateOrder>,           // body-consuming extractor must be last
) -> Result<(StatusCode, Json<OrderId>), ApiError> {
    let id = state.orders.place(body.sku, body.quantity).await?;
    metrics::counter!("orders_placed_total").increment(1);
    Ok((StatusCode::CREATED, Json(id)))
}

#[tracing::instrument(skip(state))]
pub(super) async fn show(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Order>, ApiError> {
    state.orders.find(OrderId(id)).await?.map(Json).ok_or(ApiError::NotFound)
}
```

Rules: no SQL, no business branching, no `.unwrap()`. axum 0.8 path syntax is `/orders/{id}`
(the old `/:id` form panics at startup). If a handler "doesn't implement Handler", add axum's
`macros` feature and annotate it with `#[axum::debug_handler]` to get a readable error.

### SVC-04: One `ApiError` type implements `IntoResponse`

Default: every handler returns `Result<_, ApiError>`. Domain errors convert into it via `From`;
the status-code mapping lives in exactly one `match`. Server errors log the full chain and return a
generic body; client errors return a useful message.

```rust
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use shop_domain::{PlaceOrderError, RepoError};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error(transparent)]
    Unavailable(#[from] RepoError),
}

impl From<PlaceOrderError> for ApiError {
    fn from(e: PlaceOrderError) -> Self {
        match e {
            PlaceOrderError::InvalidQuantity(_) => Self::BadRequest(e.to_string()),
            PlaceOrderError::Repo(e) => Self::Unavailable(e),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        };
        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed"); // details to logs only
            return (status, Json(json!({ "error": "service unavailable" }))).into_response();
        }
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
```

Never implement `IntoResponse` for domain types or `anyhow::Error` wholesale — you lose the
mapping and leak internals (SQL text, file paths) to clients.

## Router and middleware

### SVC-05: Graceful shutdown on SIGTERM *and* Ctrl-C, then drain background work

Kubernetes, systemd and Docker send SIGTERM; developers press Ctrl-C. Handle both, stop accepting
connections, let in-flight requests finish, stop background tasks via a `CancellationToken`, wait
for them with a `TaskTracker` (bounded), then close pools.

```rust,ignore
let shutdown = CancellationToken::new();
let tasks = TaskTracker::new();
tasks.spawn(cleanup_loop(shutdown.clone()));

let token = shutdown.clone();
axum::serve(listener, app)
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        token.cancel(); // tell background tasks too
    })
    .await
    .context("server error")?;

tasks.close();
if tokio::time::timeout(Duration::from_secs(10), tasks.wait()).await.is_err() {
    tracing::warn!("background tasks did not finish within 10s");
}
db.close().await;
```

```rust
/// Resolves on Ctrl-C (all platforms) or SIGTERM (Unix).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("install Ctrl-C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
```

Background loops `select!` on `shutdown.cancelled()` so they exit promptly. Graceful shutdown
waits for in-flight requests indefinitely — the request timeout (SVC-06) is what bounds it.
Requires tokio features `signal`, and `tokio-util` feature `rt` for `TaskTracker`.

### SVC-06: Every request has a timeout and a body limit; every outbound call has a timeout

```rust,ignore
Router::new()
    .nest("/api/v1", api)
    .route("/healthz", get(|| async { StatusCode::OK }))
    .route("/readyz", get(readyz))
    .with_state(state)
    // `.layer` wraps everything added before it: the last layer added runs first.
    .layer(RequestBodyLimitLayer::new(1024 * 1024))
    .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, request_timeout))
    .layer(PropagateRequestIdLayer::x_request_id())
    .layer(TraceLayer::new_for_http())
    .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
```

- `TimeoutLayer::new` is deprecated in tower-http (since 0.6.7); use `with_status_code`.
- axum's body extractors already cap bodies at 2 MB (`DefaultBodyLimit`); set an explicit limit
  that matches your API instead of relying on the default.
- Request id is set outermost so `TraceLayer` spans and the response both carry it.
- Outbound: configure `reqwest::Client` with `.timeout(..)` and build it **once** (it pools
  connections); DB pools need `acquire_timeout` (SVC-08).

tower-http features used here: `trace`, `timeout`, `request-id`, `limit`.

### SVC-07: Separate liveness from readiness

- `/healthz` (liveness): returns 200 if the process can serve HTTP. Never touches dependencies —
  otherwise a DB outage restarts every pod.
- `/readyz` (readiness): checks dependencies (`SELECT 1`, broker ping) and returns 503 if they are
  down, so the load balancer drains traffic.

```rust,ignore
async fn readyz(State(s): State<AppState>) -> StatusCode {
    match sqlx::query("SELECT 1").execute(&s.db).await {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            tracing::warn!(error = %e, "readiness check failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}
```

## Resources

### SVC-08: Build pools and clients once in `main`; clone the handle

```rust
use std::time::Duration;
use sqlx::postgres::{PgPool, PgPoolOptions};

pub async fn connect(url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(3))
        .connect(url)
        .await
}
```

- `PgPool` and `reqwest::Client` are cheap `Clone` handles — store them in state, never create one
  per request.
- Size the pool from config; total connections = replicas × `max_connections` must fit the DB.
- Run migrations as an explicit step (`sqlx migrate run` in the deploy pipeline, or a
  `migrate` subcommand) rather than implicitly on every replica's startup, unless you run a single
  instance.
- Postgres TLS: pick a sqlx `tls-rustls*` feature; avoid OpenSSL for easy cross-compilation.

### SVC-09: Never block the runtime

CPU-heavy work (password hashing, image resizing, big JSON transforms) and blocking APIs
(`std::fs` on large files, sync DB drivers) go to `tokio::task::spawn_blocking` or a `rayon` pool.
Never hold a `std::sync::Mutex` guard across `.await`. Details: `idioms/async`.

## Testing

### SVC-10: Test the router in-process with `tower::ServiceExt::oneshot`

```rust,ignore
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt; // `oneshot`; dev-dependency tower with feature "util"

#[tokio::test]
async fn healthz_is_ok_without_touching_the_database() {
    let db = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@localhost/unused") // never connects
        .unwrap();
    let app = router(test_state(db), Duration::from_secs(5));
    let res = app
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
```

Domain logic is tested with fakes (`layered-hexagonal.md` HEX-08); handler tests check routing,
extraction and error mapping; a few end-to-end tests run against a real Postgres.

## Checklist

### SVC-11: Service readiness checklist

- [ ] Config loaded and validated before binding the port (`configuration.md`).
- [ ] Subscriber initialised once in `main`; `TraceLayer` + request ids (`observability.md`).
- [ ] `ApiError` is the only `IntoResponse` error; 5xx bodies are generic.
- [ ] Request timeout, body limit, outbound timeouts, pool `acquire_timeout`.
- [ ] SIGTERM + Ctrl-C shutdown; background tasks cancelled and awaited with a deadline.
- [ ] `/healthz` without dependencies, `/readyz` with them, `/metrics` if scraped.
- [ ] No `unwrap()`/`expect()` in request paths; startup `expect` only for impossible failures.
- [ ] Routes versioned under `/api/v1` via `Router::nest`.
