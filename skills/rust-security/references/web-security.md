---
id: security/web-security
title: Web service security (axum / tower-http)
summary: >-
  Security defaults for HTTP services on axum 0.8 + tower-http 0.7: authentication as
  extractors, object-level authorization, body size and time limits, strict CORS, CSRF
  via tower-http's CsrfLayer, rate limiting with tower_governor, security headers,
  cookie sessions, safe error responses, log hygiene, SSRF and output encoding.
area: security
tags: [web, axum, tower-http, authn, authz, idor, csrf, cors, body-limit, timeout, rate-limiting, headers, sessions, cookies, ssrf, xss]
rust: "1.96"
edition: "2024"
crates:
  axum: "0.8"
  tower-http: "0.7"
  tower: "0.5"
  tower_governor: "0.8"
  tower-sessions: "0.15"
  reqwest: "0.13"
  url: "2.5"
  askama: "0.16"
  maud: "0.27"
  ammonia: "4.2"
  jsonwebtoken: "11.1"
verified: 2026-09-29
sources:
  - https://docs.rs/axum/0.8/axum/extract/struct.DefaultBodyLimit.html
  - https://docs.rs/tower-http/0.7/tower_http/csrf/index.html
  - https://docs.rs/tower-http/0.7/tower_http/cors/struct.CorsLayer.html
  - https://docs.rs/tower_governor/0.8/tower_governor/
  - https://docs.rs/tower-sessions/0.15/tower_sessions/
  - https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html
  - https://words.filippo.io/csrf/
---

# Web service security (axum / tower-http)

Examples are excerpts from code compiled against axum 0.8.9 and tower-http 0.7.1. Layer order in
axum: the **last** `.layer()` call is the **outermost** middleware (runs first on the
request).

## WEB-01: Authenticate with an extractor, so handlers can't forget

Default: authentication is a `FromRequestParts` extractor. A handler that needs a user
takes `AuthUser` as a parameter; if it's absent, the request is rejected before the
handler runs. Role checks are separate extractor types.

```rust
use axum::{extract::FromRequestParts, http::{StatusCode, header, request::Parts}};

pub struct AuthUser { pub user_id: String, pub roles: Vec<String> }

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = parts.headers.get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let claims = verify_jwt(token, &state.jwt_secret).map_err(|_| StatusCode::UNAUTHORIZED)?;
        Ok(AuthUser { user_id: claims.sub, roles: Vec::new() /* load from claims/DB */ })
    }
}

pub struct Admin(pub AuthUser);

impl FromRequestParts<AppState> for Admin {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.roles.iter().any(|r| r == "admin") { Ok(Admin(user)) } else { Err(StatusCode::FORBIDDEN) }
    }
}

async fn delete_user(Admin(_admin): Admin, Path(_id): Path<i64>) -> StatusCode {
    StatusCode::NO_CONTENT // only admins get here
}
```

- For a whole router subtree that must be authenticated, *also* add a
  `route_layer(middleware::from_fn_with_state(..))` guard, so a new route added there
  without the extractor still isn't public.
- Password verification: crypto.md CRYPTO-05; token verification: CRYPTO-08.
- Return the same error and similar timing for "unknown user" and "wrong password".

## WEB-02: Authorize every object access (IDOR)

Authentication says who the caller is; each handler must still check they may touch *this*
record. The LLM bug: `SELECT * FROM documents WHERE id = $1` with the ID from the path.

```rust
async fn get_document(user: AuthUser, Path(doc_id): Path<i64>, State(db): State<Db>)
    -> Result<Json<Document>, StatusCode>
{
    // Scope the query to the caller; "not yours" and "doesn't exist" look identical.
    sqlx::query_as::<_, Document>("SELECT * FROM documents WHERE id = $1 AND owner_id = $2")
        .bind(doc_id).bind(&user.user_id)
        .fetch_optional(&db.pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json).ok_or(StatusCode::NOT_FOUND)
}
```

- Put the authorization rule in the data-access layer (a function that *requires* the
  user) rather than repeating `WHERE owner_id` by hand in each handler.
- Use unguessable IDs (UUIDv4/v7) for public references, but never as the access check.
- Updates: never deserialize the request into the full DB entity (mass assignment of
  `owner_id`, `role`, `is_admin`); use a dedicated input struct.

## WEB-03: Limit request size and time

axum's body extractors (`Json`, `Bytes`, `String`, `Form`) stop at **2 MB** by default
(`DefaultBodyLimit`). Streaming the raw `Body` bypasses that, and the default applies to
every route. Set limits explicitly:

```rust
use std::time::Duration;
use axum::{extract::DefaultBodyLimit, http::StatusCode, routing::post, Router};
use tower_http::{limit::RequestBodyLimitLayer, timeout::{RequestBodyDeadlineLayer, TimeoutLayer}};

let app = Router::new()
    .route("/upload", post(upload).layer(DefaultBodyLimit::max(50 * 1024 * 1024))) // only here
    .layer(RequestBodyLimitLayer::new(1024 * 1024))                   // hard cap, incl. streaming
    .layer(RequestBodyDeadlineLayer::new(Duration::from_secs(30)))    // slow-upload (slowloris) cap
    .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)));
```

- Never `DefaultBodyLimit::disable()` globally; raise the limit per route.
- `TimeoutLayer::new` is deprecated in tower-http 0.7; use `with_status_code`.
- `RequestBodyTimeoutLayer` resets on every chunk (a byte every few seconds never trips
  it); `RequestBodyDeadlineLayer` caps the total transfer time.
- Also cap JSON element counts/lengths in validation (SEC-03) — 2 MB of `[1,1,1,...]` is a
  million-element `Vec`.

## WEB-04: CORS: explicit origins, never permissive with credentials

CORS relaxes the browser's same-origin policy; it is not access control. Default: no CORS
layer at all unless a browser app on another origin calls the API. When needed, list
exact origins, methods and headers:

```rust
use tower_http::cors::CorsLayer;
use axum::http::{HeaderValue, Method, header};

let cors = CorsLayer::new()
    .allow_origin([HeaderValue::from_static("https://app.example.com")])
    .allow_methods([Method::GET, Method::POST, Method::DELETE])
    .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
    .allow_credentials(true)
    .max_age(Duration::from_secs(600));
```

- ❌ `CorsLayer::permissive()` / `very_permissive()` or `Any` on an authenticated API.
  (tower-http panics when the layer is built if you combine `allow_credentials(true)` with `Any`.)
- ❌ Reflecting the request's `Origin` back (`AllowOrigin::mirror_request()`) with
  credentials — equivalent to allowing every site.
- Origins from config must be exact `scheme://host[:port]`; don't do suffix/regex matching
  that `https://example.com.evil.net` satisfies.

## WEB-05: CSRF protection for cookie-authenticated endpoints

Needed when the browser sends credentials automatically (cookies, HTTP auth). APIs
authenticated only by an `Authorization: Bearer` header set from JavaScript are not
CSRF-prone.

Default (tower-http 0.7): `CsrfLayer`, which rejects cross-origin state-changing requests
using `Sec-Fetch-Site` and `Origin` (the Go 1.25 `CrossOriginProtection` scheme) — no token
state. Combine with `SameSite=Lax` (or `Strict`) session cookies.

```rust
use tower_http::csrf::CsrfLayer;

let csrf = CsrfLayer::new().add_trusted_origin("https://app.example.com")?; // other origins you trust
let app = router.layer(csrf);
```

- GET/HEAD/OPTIONS always pass — so **never change state on GET**.
- Reverse proxies must forward `Origin` and `Host` unchanged, or the fallback check
  degrades to `Sec-Fetch-Site` only.
- Very old browsers send neither header; if you must support them, add a synchronizer
  token (random per-session value, compared in constant time — SEC-11).

## WEB-06: Rate-limit by a key an attacker can't forge

Default: `tower_governor` (GCRA, per-key). Stricter limits on login, password reset,
signup, and anything that sends email/SMS or does expensive work (Argon2, report
generation).

```rust
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};

let conf = GovernorConfigBuilder::default()
    .per_second(2)    // replenish one token every 2 s
    .burst_size(10)
    .finish()
    .expect("valid governor config");
let app = router.layer(GovernorLayer::new(conf));

// The default key (peer IP) needs connection info:
axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;
```

- `SmartIpKeyExtractor` trusts `X-Forwarded-For`/`X-Real-IP`/`Forwarded`. Use it **only**
  behind a proxy that overwrites those headers; otherwise every request can claim a new IP
  and bypass the limit.
- Behind a proxy with the default extractor, everyone shares the proxy's IP — one global
  limit. Pick the key deliberately (client IP from the trusted proxy, API key, user ID).
- IP limits don't stop credential stuffing from botnets: add per-account throttling and
  lockout/backoff on repeated failures.

## WEB-07: Send security headers

```rust
use tower_http::set_header::SetResponseHeaderLayer;

let app = router
    .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
    .layer(SetResponseHeaderLayer::overriding(header::STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=63072000; includeSubDomains")))
    .layer(SetResponseHeaderLayer::overriding(header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'")))
    .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin")));
```

- HSTS only on HTTPS origins; add `preload` only when every subdomain is HTTPS.
- Tailor the CSP to what the pages load; `frame-ancestors 'none'` replaces
  `X-Frame-Options` for clickjacking.
- JSON APIs: correct `Content-Type` (axum's `Json` sets it) plus `nosniff`.

## WEB-08: Cookie sessions: Secure, HttpOnly, SameSite, rotated on login

```rust
use tower_sessions::{Expiry, SessionManagerLayer, cookie::{SameSite, time::Duration}};

let sessions = SessionManagerLayer::new(store)
    .with_secure(true)
    .with_http_only(true)
    .with_same_site(SameSite::Lax)
    .with_expiry(Expiry::OnInactivity(Duration::hours(8)));

pub async fn on_login(session: tower_sessions::Session, user_id: i64)
    -> Result<(), tower_sessions::session::Error>
{
    session.cycle_id().await?;        // new ID on privilege change: prevents session fixation
    session.insert("user_id", user_id).await?;
    Ok(())
}
```

- `MemoryStore` is for development and single instances; production uses a persistent
  shared store so logout/expiry work across replicas.
- Logout deletes the server-side session (`session.flush()`), not just the cookie.
- If you must keep state in the cookie itself, use `with_private(key)` (encrypted +
  authenticated) — never an unsigned cookie holding a user ID or role.

## WEB-09: Don't leak internals in errors; catch panics

- Map internal errors (`sqlx::Error`, `io::Error`, upstream bodies) to a generic 500 with
  a request ID; log the detail server-side. Never return `err.to_string()` of DB errors to
  clients — it leaks schema and sometimes data.
- Add `tower_http::catch_panic::CatchPanicLayer` so a handler panic becomes a 500 instead
  of a dropped connection, and still treat every panic as a bug (SEC-02).
- Mark credentials as sensitive before any `TraceLayer`, so they're redacted in logs:

```rust
let app = router
    .layer(TraceLayer::new_for_http())
    .layer(SetSensitiveHeadersLayer::new([header::AUTHORIZATION, header::COOKIE, header::SET_COOKIE]))
    .layer(CatchPanicLayer::new());
```

## WEB-10: Server-side request forgery (SSRF)

Any feature that fetches a user-supplied URL (webhooks, link previews, "import from URL",
image proxies) can be pointed at `http://169.254.169.254/` (cloud metadata),
`localhost` admin ports or internal services.

- Allowlist scheme (`https`) and, where possible, hosts.
- Resolve the host, reject non-public addresses, then connect **to those vetted addresses**
  (otherwise DNS rebinding swaps the IP between check and use). Disable redirects.

```rust
use std::net::IpAddr;

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !(v4.is_private() || v4.is_loopback() || v4.is_link_local()
            || v4.is_unspecified() || v4.is_broadcast() || v4.is_documentation()
            || v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64), // 100.64/10 CGNAT
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public(IpAddr::V4(v4)),
            None => !(v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local()
                || v6.is_unicast_link_local()),
        },
    }
}

pub async fn vet_outbound(raw: &str) -> Result<(url::Url, Vec<std::net::SocketAddr>), &'static str> {
    let url = url::Url::parse(raw).map_err(|_| "invalid URL")?;
    if url.scheme() != "https" { return Err("https only"); }
    let host = url.host_str().ok_or("missing host")?;
    let port = url.port_or_known_default().ok_or("missing port")?;
    let addrs: Vec<_> = tokio::net::lookup_host((host, port)).await.map_err(|_| "DNS failure")?.collect();
    if addrs.is_empty() || !addrs.iter().all(|a| is_public(a.ip())) {
        return Err("destination not allowed");
    }
    Ok((url, addrs))
}

pub fn pinned_client(host: &str, addrs: &[std::net::SocketAddr]) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .resolve_to_addrs(host, addrs)                 // connect only to the vetted IPs
        .redirect(reqwest::redirect::Policy::none())   // a redirect could point anywhere
        .timeout(std::time::Duration::from_secs(10))
        .build()
}
```

Also cap the response size you read (SEC-03). For high-risk fetchers, run them in an
egress-restricted network segment — code checks are the second line of defense.

## WEB-11: Encode output; sanitize user HTML

- Render HTML with an autoescaping template engine (`askama`, `maud`); never build HTML
  with `format!` from user data. Don't mark user data `|safe`.
- User-supplied rich text: sanitize with `ammonia` on output (or on input *and* output).
- JSON responses via `Json(...)`, never hand-concatenated strings.
- Set `Content-Disposition: attachment` and a fixed `Content-Type` when serving user
  uploads, and serve them from a separate origin if they can be HTML/SVG.
