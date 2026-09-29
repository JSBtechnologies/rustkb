---
id: ecosystem/databases
title: Database, cache and storage crates
summary: >-
  sqlx as the default async SQL toolkit, when to use diesel or sea-orm instead, SQLite via
  rusqlite, Redis/Mongo clients, embedded key-value stores, pooling, migrations and caches.
area: ecosystem
tags: [database, sql, sqlx, diesel, sea-orm, sqlite, rusqlite, postgres, redis, mongodb, redb, cache, moka, migrations]
rust: "1.96"
edition: "2024"
crates:
  sqlx: "0.9"
  sqlx-cli: "0.9"
  diesel: "2.3"
  diesel-async: "0.9"
  sea-orm: "2.0"
  rusqlite: "0.40"
  tokio-postgres: "0.7"
  deadpool-postgres: "0.14"
  redis: "1.7"
  mongodb: "3.9"
  redb: "4.3"
  heed: "0.22"
  duckdb: "1.10505"
  moka: "0.12"
  lru: "0.18"
  uuid: "1.26"
  testcontainers: "0.28"
  bb8: "0.9"
  deadpool: "0.13"
  clickhouse: "0.15"
  scylla: "1.9"
  rocksdb: "0.25"
  libsql: "0.9"
  turso: "0.8"
  opensearch: "2.4"
  deadpool-redis: "0.23"
  deadpool-diesel: "0.7"
verified: 2026-09-29
sources:
  - https://github.com/launchbadge/sqlx
  - https://diesel.rs/
  - https://www.sea-ql.org/SeaORM/
  - https://github.com/rusqlite/rusqlite
  - https://github.com/redis-rs/redis-rs
---

# Database, cache and storage crates

Repository/port design (where queries live, transactions across a use case) belongs to
`rust-architecture`. This file picks the crates.

## DB-01: Default SQL toolkit is sqlx

Default: `sqlx` (0.9) for async services on Postgres, MySQL/MariaDB or SQLite. It is
not an ORM: you write SQL, and the `query!`/`query_as!` macros check it against a real
database (or cached `.sqlx` metadata) at compile time.

```toml
sqlx = { version = "0.9", features = ["runtime-tokio", "tls-rustls", "postgres", "macros", "migrate", "uuid", "chrono"] }
```

```rust
use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Debug, sqlx::FromRow)]
struct User {
    id: uuid::Uuid,
    email: String,
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
}

async fn find_user(pool: &PgPool, email: &str) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as::<_, User>("SELECT id, email FROM users WHERE email = $1")
        .bind(email)            // always bind; never format! values into SQL
        .fetch_optional(pool)
        .await
}
```

- Prefer `sqlx::query_as!(User, "...", email)` in application code for compile-time
  checking; commit the `.sqlx/` directory from `cargo sqlx prepare` (`sqlx-cli`
  0.9) and set `SQLX_OFFLINE=true` in CI so builds don't need a database.
- Migrations: `sqlx::migrate!()` embedding `migrations/*.sql`, run at startup or as a
  separate deploy step.
- `PgPool` is an `Arc` internally — clone it into state; don't wrap it in `Arc<Mutex<_>>`.
- sqlx 0.9 is the current line; code written for 0.7/0.8 from memory may not compile.
  Check the CHANGELOG when upgrading.
- Choose TLS explicitly: `tls-rustls` (aws-lc-rs), `tls-rustls-ring-webpki`, `tls-native-tls`
  or `tls-none`.

## DB-02: When to choose diesel or sea-orm

| Situation | Choice |
|---|---|
| Raw SQL, compile-time checked, async | `sqlx` (default) |
| Strongly typed query DSL; complex dynamic queries checked by the type system; sync code | `diesel` (2.3) |
| diesel from async services | `diesel-async` (0.9) — pools via deadpool/bb8 |
| ActiveRecord-style ORM, entities/relations, admin-like CRUD, async | `sea-orm` (2.0) — 2.x runs on sqlx 0.9 |
| Postgres-only, max control (COPY, LISTEN/NOTIFY, pipelining) without macros | `tokio-postgres` (0.7) + `deadpool-postgres` (0.14) |

Don't mix two of these in one service without a reason; pick one data-access style.
Never run diesel's sync `Connection` directly inside async handlers — use `diesel-async` or
`tokio::task::spawn_blocking`.

## DB-03: SQLite

Default: `rusqlite` (0.40) with the `bundled` feature (compiles SQLite in; no system
library needed) for CLIs, desktop apps and embedded storage.

```rust
use rusqlite::{Connection, params};

#[derive(Debug)]
struct Note {
    id: i64,
    body: String,
}

fn run() -> rusqlite::Result<Vec<Note>> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
    )?;
    conn.execute("INSERT INTO notes (body) VALUES (?1)", params!["hello"])?;
    let mut stmt = conn.prepare("SELECT id, body FROM notes ORDER BY id")?;
    let notes = stmt
        .query_map([], |row| Ok(Note { id: row.get(0)?, body: row.get(1)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(notes)
}
```

- In async services that already use sqlx, use sqlx's `sqlite` feature instead.
- **Don't put `sqlx` and `rusqlite` in the same dependency graph without checking their
  `libsqlite3-sys` requirements.** Only one crate may link `sqlite3`; sqlx 0.9 accepts
  `libsqlite3-sys` 0.30–0.37 while rusqlite 0.40 requires 0.38, so the pair fails to
  resolve — even if sqlx's `sqlite` feature is off (optional deps still take part in `links`
  resolution).
- Set `PRAGMA journal_mode = WAL`, `busy_timeout`, and `foreign_keys = ON` explicitly.
- Libsql/Turso clients (`libsql`, `turso`) exist for their hosted/edge variants; pick them
  only when targeting that service.

## DB-04: Redis / Valkey

Default: `redis` (1.7) — maintained by the redis-rs organisation, now 1.x. Use the
async `ConnectionManager` (auto-reconnect, cheap to clone) with `tokio-comp`; enable
`tls-rustls` for TLS and `cluster-async` for cluster mode.

```rust
use redis::AsyncCommands;

async fn redis_demo(url: &str) -> redis::RedisResult<()> {
    let client = redis::Client::open(url)?;
    let mut conn = redis::aio::ConnectionManager::new(client).await?;
    let _: () = conn.set_ex("session:1", "data", 3600).await?;
    let _v: Option<String> = conn.get("session:1").await?;
    Ok(())
}
```

Don't open a new connection per request; don't use the sync API inside async code.

## DB-05: Other databases

| Database | Crate |
|---|---|
| MongoDB | `mongodb` (3.9) — official driver |
| DuckDB (embedded OLAP) | `duckdb` (1.10505) — unusual numbering (e.g. `1.10505.0`) that encodes the bundled DuckDB release |
| ClickHouse | `clickhouse` (official client) |
| ScyllaDB/Cassandra | `scylla` (official driver) |
| OpenSearch / Elasticsearch | `opensearch` client; the official `elasticsearch` crate publishes only pre-releases — or call the REST API with reqwest |
| DynamoDB / other AWS stores | `aws-sdk-*` crates |

## DB-06: Embedded key-value stores

Default: `redb` (4.3) — pure-Rust, ACID, stable file format, actively maintained.

- `heed` (0.22) — typed LMDB bindings; excellent read performance, C dependency.
- `rocksdb` — when you need an LSM tree for write-heavy workloads; heavy C++ build.
- **Not `sled`**: still a 0.34 beta with an unstable on-disk format and years without a
  stable release. Agents recommend it by reflex; don't.
- For most apps "embedded KV" is better served by SQLite (rusqlite) — queryable and debuggable.

## DB-07: Connection pooling

Default: use the pool your driver provides (`sqlx::Pool`, sea-orm's, `redis`'
`ConnectionManager`). For drivers without one: `deadpool` (`deadpool-postgres`,
`deadpool-redis`, `deadpool-diesel`) or `bb8`. Size pools from measurements, set acquire
timeouts, and never hold a pooled connection across unrelated `.await`s.

## DB-08: In-memory caches

Default: `moka` (0.12) — concurrent cache with size bounds, TTL/TTI, and an async
API (`future` feature) that coalesces concurrent loads of the same key.

```rust
use std::time::Duration;

async fn cache_demo() {
    let cache: moka::future::Cache<String, u64> = moka::future::Cache::builder()
        .max_capacity(10_000)
        .time_to_live(Duration::from_secs(300))
        .build();
    let v = cache.get_with("key".to_owned(), async { 42 }).await;
    assert_eq!(v, 42);
}
```

- A small cache owned by one task/thread: `lru` (0.18).
- Don't build caches from `Arc<Mutex<HashMap>>` without eviction — that's a memory leak
  with extra steps.
- Distributed cache: Redis, behind the same trait as the local cache.

## DB-09: IDs and testing

- IDs: `uuid` (1.26) with the `v7` feature for time-ordered primary keys (better index
  locality than v4); enable `serde` and the driver's uuid feature (`sqlx/uuid`).
- Integration tests: run the real database with `testcontainers` (0.28) or a CI
  service container; use `#[sqlx::test]` for per-test databases with sqlx. Mock the
  repository trait, not the SQL driver.
