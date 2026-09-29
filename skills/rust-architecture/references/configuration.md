---
id: architecture/configuration
title: Configuration and secrets
summary: >-
  One typed, validated Settings struct loaded once at startup from layered sources (defaults <
  files < environment < flags), secrets wrapped in secrecy::SecretString, fail-fast validation,
  no env reads deep in the code, and libraries that take config from their caller.
area: architecture
tags: [configuration, config, env-vars, secrets, secrecy, validation, twelve-factor, dotenv, settings]
rust: "1.96"
edition: "2024"
crates:
  config: "0.15"
  figment: "0.10"
  serde: "1.0"
  secrecy: "0.10"
  anyhow: "1.0"
  humantime-serde: "1.1"
  dotenvy: "0.15"
  clap: "4.6"
verified: 2026-09-29
sources:
  - https://12factor.net/config
  - https://docs.rs/config/0.15/config/
  - https://docs.rs/secrecy/0.10/secrecy/
  - https://serde.rs/container-attrs.html#deny_unknown_fields
---

# Configuration and secrets

Configuration is an architectural boundary: parse it once at the edge into types, validate it,
then pass plain values inward. Every rule here fixes a failure mode seen in agent-written
services: `std::env::var("DATABASE_URL").unwrap()` in a handler, secrets printed by `{:?}`,
typos in config keys silently ignored, a global `CONFIG` static.

## Shape

### CFG-01: One typed `Settings` tree, loaded once in `main`, passed down explicitly

Default: nested `serde::Deserialize` structs mirroring config sections; `main` loads them before
doing anything else and hands each component the section it needs (`&settings.database`), not the
whole tree and never through a global.

```rust
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, ensure};
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub(crate) http: HttpSettings,
    pub(crate) database: DatabaseSettings,
    #[serde(default)]
    pub(crate) log: LogSettings,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HttpSettings {
    pub(crate) addr: SocketAddr,                 // parsed and validated by serde
    #[serde(default = "default_request_timeout_secs")]
    pub(crate) request_timeout_secs: u64,        // unit in the name
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DatabaseSettings {
    pub(crate) url: SecretString,                // Debug prints [REDACTED]
    #[serde(default = "default_max_connections")]
    pub(crate) max_connections: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogSettings {
    #[serde(default)]
    pub(crate) json: bool,
}

fn default_request_timeout_secs() -> u64 { 10 }
fn default_max_connections() -> u32 { 10 }

impl HttpSettings {
    pub(crate) fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
    }
}
```

Use real types in the struct (`SocketAddr`, `Url`, enums for modes, domain newtypes) so invalid
values fail at deserialisation with a precise message instead of later at use.

### CFG-02: Layer sources: defaults < files < environment < flags

Default for services with the `config` crate:

```rust,ignore
impl Settings {
    /// Precedence (low → high): built-in defaults, `config/default.toml`,
    /// `config/{APP_ENV}.toml`, `APP__*` environment variables.
    pub(crate) fn load() -> anyhow::Result<Self> {
        let env = std::env::var("APP_ENV").unwrap_or_else(|_| "local".into());
        let settings: Settings = config::Config::builder()
            .set_default("http.addr", "0.0.0.0:8080")?
            .add_source(config::File::with_name("config/default").required(false))
            .add_source(config::File::with_name(&format!("config/{env}")).required(false))
            .add_source(config::Environment::with_prefix("APP").separator("__"))
            .build()?
            .try_deserialize()
            .context("invalid configuration")?;
        settings.validate()?;
        Ok(settings)
    }
}
```

With `with_prefix("APP").separator("__")`, `APP__DATABASE__MAX_CONNECTIONS=20` sets
`database.max_connections` (the prefix separator defaults to the separator; keys are lowercased;
numeric strings deserialise into integer fields). Use `__` so single underscores inside key names
(`max_connections`) survive. Enable only the file formats you use
(`config = { version = "0.15", default-features = false, features = ["toml"] }`).

Alternatives: `figment` (similar layering, used by Rocket) is fine if already in use. CLIs merge
clap flags/env with a file by hand (`cli-apps.md` CLI-06). Tiny services can deserialise only from
environment with the same `config::Environment` source.

### CFG-03: Validate everything at startup and fail fast

After deserialising, check cross-field and range rules and exit **before** binding ports or
opening pools, with a message naming the key:

```rust,ignore
impl Settings {
    fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.http.request_timeout_secs > 0, "http.request_timeout_secs must be > 0");
        ensure!(
            (1..=200).contains(&self.database.max_connections),
            "database.max_connections must be in 1..=200"
        );
        Ok(())
    }
}
```

- `#[serde(deny_unknown_fields)]` turns typos (`max_conections`) into startup errors instead of
  silently using the default. With the `config` crate this includes any unknown `APP__*` env var —
  a feature, not a bug. (It cannot be combined with `#[serde(flatten)]`.)
- Log the effective config at startup (`tracing::info!(?settings, "starting")`) — safe only
  because secrets are `SecretString` (CFG-04).
- Crash-looping with a clear error beats running with a half-valid config.

### CFG-04: Secrets are `SecretString`, come from the environment or a secret store, never from git

- Wrap every credential (DB URLs with passwords, API keys, signing keys) in
  `secrecy::SecretString` (needs `secrecy`'s `serde` feature to deserialise). Its `Debug` prints
  `SecretBox<str>([REDACTED])`, and the value is zeroised on drop.
- Call `.expose_secret()` only at the point of use (building the pool, signing a request) — never
  store the exposed `&str`/`String` in another struct.
- Source secrets from env vars injected by the platform (Kubernetes Secrets, systemd
  `LoadCredential`, CI secrets) or a mounted file; config files in the repo hold only non-secret
  defaults.
- Don't derive `Serialize` on settings containing secrets, and never return config over an HTTP
  debug endpoint.

Broader secret-handling guidance (rotation, memory hygiene, logging): the `rust-security` skill.

### CFG-05: Durations and sizes carry their unit

Either put the unit in the key (`request_timeout_secs: u64`, `max_body_bytes: usize`) or accept
human strings with `humantime-serde` (`#[serde(with = "humantime_serde")] timeout: Duration`
accepting `"250ms"`, `"5s"`). Never a bare `timeout: u64` — nobody knows if it is ms or s.

## Where configuration is read

### CFG-06: No `std::env::var` outside the config module

Reading env vars in handlers, repositories or library code creates hidden inputs that tests
cannot control and ops cannot discover. The config module is the **only** place that reads the
environment (plus `RUST_LOG`, which the tracing `EnvFilter` reads by design).

Never call `std::env::set_var`/`remove_var` to pass configuration around: they are `unsafe` in
edition 2024 because they race with other threads reading the environment. For tests, build
`Settings` directly or feed the source a map:

```rust,ignore
let env: HashMap<String, String> =
    [("APP__DATABASE__URL".to_owned(), "postgres://u:p@db/shop".to_owned())].into();
let s: Settings = config::Config::builder()
    .add_source(config::Environment::with_prefix("APP").separator("__").source(Some(env)))
    .build()?
    .try_deserialize()?;
```

### CFG-07: `.env` files are a local-development convenience only

Use `dotenvy::dotenv().ok()` (ignore a missing file) at the top of `main` in development if the
team wants it; `.env` is in `.gitignore`, and a committed `.env.example` documents the variables.
Production gets real environment variables. Never commit `.env` and never require it to exist.

### CFG-08: Libraries take configuration; they never load it

A library exposes a config struct or builder (`Client::builder().timeout(..).build()`) with
sensible defaults and lets the application decide where values come from. It never reads files,
env vars or CLI args itself (LIB-05), and never pulls in `config`/`figment` as a dependency.

## Anti-patterns

### CFG-09: Configuration anti-patterns to remove

- `static CONFIG: LazyLock<Settings>` / `OnceLock` global read from anywhere → pass it down.
- `env::var("X").unwrap()` scattered through the code → one `Settings` with a validated field.
- `HashMap<String, String>` config with stringly lookups → typed structs.
- Secrets as `String` in a `#[derive(Debug)]` struct that gets logged → `SecretString`.
- Defaults duplicated in code, README and Helm chart → one `#[serde(default)]` (or
  `set_default`) source of truth, documented in `config/default.toml`.
- Hot reload of everything via file watchers → restart the process; reload only specific,
  designed-for-it values behind an explicit handle.

Related: `web-services.md` (where `Settings` is consumed), `cli-apps.md` CLI-06 (CLI precedence),
`observability.md` OBS-06 (not logging secrets).
