---
id: ecosystem/data-and-serialization
title: Serialization, data formats and configuration crates
summary: >-
  serde as the universal layer; which crate per format (JSON, TOML, YAML after serde_yaml's
  deprecation, binary formats after bincode's shutdown, CSV, XML, protobuf); config loading crates.
area: ecosystem
tags: [serde, json, toml, yaml, postcard, bincode, csv, xml, protobuf, config, dotenv, schema]
rust: "1.96"
edition: "2024"
crates:
  serde: "1.0"
  serde_json: "1.0"
  serde_with: "3.24"
  toml: "1.1"
  toml_edit: "0.25"
  serde-saphyr: "1.3"
  serde_norway: "0.9"
  postcard: "1.1"
  wincode: "0.6"
  bitcode: "0.6"
  rkyv: "0.8"
  ciborium: "0.2"
  rmp-serde: "1.3"
  csv: "1.4"
  quick-xml: "0.42"
  prost: "0.14"
  base64: "0.23"
  schemars: "1.2"
  sonic-rs: "0.5"
  serde_path_to_error: "0.1"
  config: "0.15"
  figment: "0.10"
  dotenvy: "0.15"
  saphyr: "0.1"
  yaml-rust2: "0.13"
  simd-json: "0.18"
verified: 2026-09-29
sources:
  - https://serde.rs/
  - https://rustsec.org/advisories/RUSTSEC-2025-0141.html
  - https://rustsec.org/advisories/RUSTSEC-2025-0068.html
  - https://github.com/dtolnay/serde-yaml
  - https://docs.rs/serde-saphyr/latest/serde_saphyr/
  - https://postcard.jamesmunns.com/
---

# Serialization, data formats and configuration crates

## SER-01: serde is the data-model layer for everything

Default: `serde` (1.0) with `features = ["derive"]`, plus one format crate per format.
Put `#[derive(Serialize, Deserialize)]` on DTOs at the boundary, not necessarily on domain types.

```toml
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

- Libraries: make serde support an optional feature (`serde = { version = "…", optional = true,
  features = ["derive"] }`) and gate derives with `#[cfg_attr(feature = "serde", derive(...))]`.
- Use serde attributes instead of hand-written impls: `rename_all`, `default`,
  `deny_unknown_fields` (for config), `skip_serializing_if = "Option::is_none"`, `with`.
- `serde_with` (3.24) covers the common custom cases (`DisplayFromStr`, durations,
  base64/hex bytes, maps as sequences) — prefer it to bespoke `serialize_with` modules.
- `serde_path_to_error` (0.1) turns "invalid type at line 1 column 900"
  into a field path; wrap deserializers of large user-supplied documents with it.
- `rustc-serialize` is dead (RUSTSEC-2025-0025); never add it.

## SER-02: Format choice table

| Format | Default crate | Notes |
|---|---|---|
| JSON | `serde_json` (1.0) | `Value` for dynamic data; `sonic-rs` (0.5) / `simd-json` only after profiling |
| TOML | `toml` (1.1) | 1.x implements TOML 1.1; `toml_edit` (0.25) to edit preserving comments |
| YAML | `serde-saphyr` (1.3) | pure Rust; see SER-03 — **not** `serde_yaml` |
| Compact binary (Rust ↔ Rust, embedded) | `postcard` (1.1) | stable wire format, `no_std`; see SER-04 |
| Zero-copy archives | `rkyv` (0.8) | large read-mostly data, mmap |
| CBOR | `ciborium` (0.2) | replaces unmaintained `serde_cbor` |
| MessagePack | `rmp-serde` (1.3) | interop with MessagePack systems |
| Protobuf | `prost` (0.14) | with `tonic` for gRPC |
| CSV | `csv` (1.4) | serde `Deserialize` per record |
| XML | `quick-xml` (0.42) | `serialize` feature for serde support |
| Base64 | `base64` (0.23) | `use base64::prelude::*; BASE64_STANDARD.encode(..)` |
| JSON Schema from types | `schemars` (1.2) | 1.x API differs from 0.8 |

✅ One set of derives, several formats:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ServerConfig {
    listen: String,
    #[serde(default = "default_workers")]
    workers: usize,
    #[serde(default)]
    tags: Vec<String>,
}

fn default_workers() -> usize {
    4
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let from_toml: ServerConfig = toml::from_str("listen = \"0.0.0.0:8080\"\n")?;
    let from_yaml: ServerConfig = serde_saphyr::from_str("listen: 127.0.0.1:9000\nworkers: 8\n")?;
    let json = serde_json::to_string(&from_toml)?;
    let yaml = serde_saphyr::to_string(&from_yaml)?;
    println!("{json}\n{yaml}");
    Ok(())
}
```

## SER-03: YAML — serde_yaml is deprecated; use serde-saphyr

Default for new code: `serde-saphyr` (1.3). Pure Rust (built on
`saphyr-parser`), panic-free parsing, good error messages, actively released; requires
Rust 1.89+.

Timeline agents don't know:
- `serde_yaml` was deprecated by its author in March 2024 (last release `0.9.34+deprecated`,
  repository archived). It still has huge download counts only because of old dependents.
- `serde_yml`, the most visible fork, is flagged **unsound and unmaintained**
  (RUSTSEC-2025-0068) and its repository is archived. Never use it.
- `serde_norway` (0.9) keeps the serde_yaml API (drop-in rename) on a maintained
  libyaml port, but has had no release since 2024-12. Use it only for a mechanical migration
  of existing serde_yaml code.
- `serde_yaml_ng` keeps the API too but depends on the archived `unsafe-libyaml` and has not
  released since 2024-05.
- `yaml-rust` is unmaintained (RUSTSEC-2024-0320); low-level YAML users move to `saphyr` or
  `yaml-rust2`.

Migration from serde_yaml: `serde_yaml::from_str` → `serde_saphyr::from_str`,
`serde_yaml::to_string` → `serde_saphyr::to_string`. Code that manipulated
`serde_yaml::Value` needs rework (deserialize into typed structs or `serde_json::Value`).
Consider whether the file format should be TOML instead — for app config it usually should.

## SER-04: Binary formats — bincode is gone

Default: `postcard` (1.1) for compact Rust-to-Rust messages, persisted blobs and
embedded links. It has a documented, stable wire format and works in `no_std`.

- **bincode is discontinued** (RUSTSEC-2025-0141, December 2025). The final `3.0.0`
  release is a tombstone whose `lib.rs` is only a `compile_error!`. An agent writing
  `bincode = "3"` gets a build failure; writing `"1.3"`/`"2.0"` pins a dead crate.
- Need to keep reading existing bincode-1 data? `wincode` (0.6) is a
  bincode-compatible alternative named by the bincode maintainers and RustSec.
- Need the smallest/fastest encoding where both ends ship together (games, internal RPC)?
  `bitcode` (0.6) — its format is not stable across major versions.
- Need zero-copy access to large archives? `rkyv` (0.8).
- Need cross-language interop? Protobuf (`prost`), CBOR (`ciborium`) or MessagePack
  (`rmp-serde`) instead of a Rust-specific format.

Non-self-describing formats (postcard, bincode-style) **cannot** handle
`#[serde(tag = "...")]`/`untagged` enums or `#[serde(flatten)]` (serialization fails at
runtime: `WontImplement` / `SerializeSeqLengthUnknown` in postcard), and
`skip_serializing_if` silently produces bytes that fail to deserialize
(`DeserializeBadOption`). Keep wire DTOs plain and round-trip-test them.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Reading {
    sensor: u16,
    millis: u64,
    value: f32,
}

fn roundtrip(r: &Reading) -> Result<Reading, postcard::Error> {
    let bytes: Vec<u8> = postcard::to_allocvec(r)?; // needs the "alloc" or "use-std" feature
    postcard::from_bytes(&bytes)
}
```

## SER-05: Enum representation in JSON

Default: pick the representation deliberately — serde's default "externally tagged"
(`{"Created":{"id":1}}`) is rarely what an HTTP API wants.

- `#[serde(tag = "type")]` (internally tagged) — typical REST/event payloads.
- `#[serde(tag = "t", content = "c")]` (adjacently tagged) — when variants hold non-struct data.
- `#[serde(untagged)]` — only for accepting legacy shapes; errors are poor and matching is
  order-dependent.
- Add `#[serde(rename_all = "snake_case")]` (or `camelCase`) at the type level rather than
  renaming each field.

## SER-06: Configuration loading

Default: a typed `Settings` struct deserialized with serde, loaded by `config`
(0.15) or `figment` (0.10), validated once at startup, then passed down
explicitly (not read from globals). Config architecture (layering order, secrets, reload)
lives in `rust-architecture`.

| Need | Crate |
|---|---|
| Layer files (TOML/YAML/JSON) + env vars | `config` — actively released |
| Same, with value provenance in errors ("from env APP_PORT") | `figment` — mature, infrequent releases |
| `.env` for local development | `dotenvy` (0.15) — `dotenv` is unmaintained (RUSTSEC-2021-0141) |
| Env vars only, into a struct | `envy`-style: `config::Environment` or `figment::providers::Env` |
| CLI flags that override config | `clap` with `env` feature (see `cli-and-tui.md`) |

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Settings {
    database_url: String,
    #[serde(default)]
    log_level: Option<String>,
}

fn load() -> Result<Settings, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::with_name("config/default").required(false))
        .add_source(config::File::with_name("config/local").required(false))
        .add_source(config::Environment::with_prefix("APP").separator("__"))
        .build()?
        .try_deserialize()
}
```

Never commit `.env` files with real secrets; wrap secret fields in `secrecy::SecretString`
so they don't appear in `Debug` output (see `rust-security`).

## SER-07: Performance notes

Default: `serde_json` is fast enough for almost every service; measure before switching.

- Deserialize borrowed data (`&'de str`, `Cow<'de, str>`) from a buffer you keep alive to
  avoid allocations in hot paths.
- Use `serde_json::from_slice` on bytes rather than converting to `String` first.
- For huge JSON documents, stream with `serde_json::Deserializer::from_reader(..).into_iter()`
  or `StreamDeserializer` instead of loading a `Value`.
- `sonic-rs` / `simd-json` give real speedups on large payloads but add SIMD/unsafe
  complexity; adopt them only with a benchmark.
- `Value`-heavy code is slow and stringly-typed; deserialize into structs.
