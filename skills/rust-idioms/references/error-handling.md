---
id: idioms/error-handling
title: Error handling
summary: >-
  Typed thiserror enums in libraries, anyhow (or eyre) with context at application edges; when to
  panic, how to use unwrap/expect, Display and source-chain conventions, and how main should exit.
area: idioms
tags: [errors, thiserror, anyhow, eyre, panic, result, option, unwrap, context, exit-code]
rust: "1.96"
edition: "2024"
crates:
  thiserror: "2.0"
  anyhow: "1.0"
  eyre: "0.6"
  color-eyre: "0.6"
  miette: "7.6"
  snafu: "0.9"
  serde_json: "1.0"
  regex: "1.13"
verified: 2026-09-29
sources:
  - https://rust-lang.github.io/api-guidelines/interoperability.html#c-good-err
  - https://docs.rs/thiserror/2
  - https://docs.rs/anyhow/1
  - https://doc.rust-lang.org/std/error/trait.Error.html
  - https://doc.rust-lang.org/std/process/struct.ExitCode.html
---

# Error handling

Decides: which error crate goes where, how to shape error types, when a panic is acceptable, and how
errors reach the user. Read when defining an error type, writing `?`-heavy code, touching `main`, or
reviewing `.unwrap()` usage.

## Library vs application errors

### ERR-01: Libraries expose typed, matchable errors (thiserror)

**Default:** a library crate (or any module boundary other crates depend on) returns
`Result<T, MyError>` where `MyError` is an enum deriving `thiserror::Error`. **Use** one error enum per
module or per operation family, not one crate-wide "god enum" with 40 variants. **Never** return
`Box<dyn Error>`, `anyhow::Error` or `String` from a public library API — callers can't match on them,
and `Box<dyn Error>` is not even `Send + Sync`.

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid config syntax")]
    Parse(#[from] serde_json::Error),
    #[error("missing required key `{0}`")]
    MissingKey(&'static str),
}

pub fn load(path: &Path) -> Result<serde_json::Value, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|source| ConfigError::Read { path: path.to_path_buf(), source })?;
    let value: serde_json::Value = serde_json::from_str(&text)?; // #[from] conversion
    if value.get("name").is_none() {
        return Err(ConfigError::MissingKey("name"));
    }
    Ok(value)
}
```

```rust
// ❌ unmatchable, not Send + Sync, loses structure
pub fn load(path: &str) -> Result<String, Box<dyn std::error::Error>> { todo!() }
pub fn parse(s: &str) -> Result<u32, String> { s.parse().map_err(|e| format!("{e}")) }
```

### ERR-02: Applications use `anyhow::Result` with context at the edges

**Default:** binaries, CLIs, services' top-level handlers, build scripts and tests use
`anyhow::Result<T>` and add `.context(..)` / `.with_context(|| ..)` at every I/O or boundary call.
**Use** `eyre` + `color-eyre` instead only if you want its colourful report/span-trace output, and
`miette` for compiler-like diagnostics with source snippets. **Never** mix `anyhow` into the domain
layer that other code needs to match on — keep typed errors there and convert at the edge.

```rust
use anyhow::{Context, Result, bail};

fn read_port(path: &str) -> Result<u16> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading port file {path}"))?;
    let port: u16 = text.trim().parse().context("port file is not a number")?;
    if port == 0 {
        bail!("port must be non-zero");
    }
    Ok(port)
}
```

| Situation | Choose |
|---|---|
| Library / reusable module | `thiserror` enum |
| Binary, CLI, service main, tests | `anyhow` (or `eyre`/`color-eyre`) |
| User-facing diagnostics with source spans (parsers, config linters) | `miette` |
| Large codebase wanting context selectors per variant | `snafu` (situational) |
| `no_std` library | hand-written enum implementing `core::error::Error` (stable since 1.81) |

## Designing error types

### ERR-03: Model variants by what the caller can do, not by where it failed

**Default:** one variant per distinct recovery path (`NotFound`, `PermissionDenied`, `Invalid { .. }`,
`Io(..)`). Carry data the caller needs (the key, the path, the limit). **Use** `#[non_exhaustive]` on
public error enums so you can add variants without a breaking change (see `api-design.md`). **Never**
create a variant per call site or a catch-all `Other(String)` that everything funnels into.

### ERR-04: Keep `Display` short, lowercase, and without the source

**Default:** messages are lowercase, no trailing period, describe *this* layer only
(`"failed to read config file {path}"`). The underlying cause goes in `#[source]`/`#[from]`, and
reporters (`anyhow`'s `{:#}`/`{:?}`) print the chain. **Never** interpolate the source into the message
(`#[error("read failed: {0}")]` on a `#[from]` field) — reporters then print the cause twice.

```rust
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    // ✅ cause is reachable via source(), not duplicated in the text
    #[error("request to {url} failed")]
    Http {
        url: String,
        #[source]
        source: std::io::Error,
    },
    // ✅ transparent: forward Display and source() entirely to the inner error
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

```rust
// ❌ prints "request failed: connection refused: connection refused" in a chain report
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("request failed: {0}")]
    Io(#[from] std::io::Error),
}
```

### ERR-05: Errors crossing threads or tasks must be `Send + Sync + 'static`

**Default:** error types own their data (no borrowed `&str` fields) and contain only `Send + Sync`
types, so they work with `tokio::spawn`, `anyhow`, and `?` into `Box<dyn Error + Send + Sync>`.
**Never** put `Rc`, `RefCell`, or references in an error type that leaves the function.

### ERR-06: Use `#[from]` only for unambiguous conversions

**Default:** `#[from]` when a source type maps to exactly one variant and no extra context is needed.
**Use** `.map_err(|source| MyError::Variant { ctx, source })` when the same source type (e.g.
`io::Error`) can arise in several operations that the caller must distinguish. **Never** add a blanket
`From<io::Error>` and lose which file/operation failed.

## Propagation

### ERR-07: Propagate with `?`; never `match` just to re-wrap

**Default:** `?` with `From` conversions or `map_err`/`context`. **Use** `Option::ok_or_else` to turn
a missing value into an error, and `?` on `Option` inside functions returning `Option`. **Never** write
`match r { Ok(v) => v, Err(e) => return Err(e.into()) }` — that is `?`.

```rust
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("unknown user {0}")]
    UnknownUser(u64),
    #[error("user {0} has no email")]
    NoEmail(u64),
}

pub fn email_domain(users: &HashMap<u64, Option<String>>, id: u64) -> Result<&str, LookupError> {
    let email = users
        .get(&id)
        .ok_or(LookupError::UnknownUser(id))?
        .as_deref()
        .ok_or(LookupError::NoEmail(id))?;
    Ok(email.rsplit_once('@').map_or(email, |(_, domain)| domain))
}
```

### ERR-08: Don't log and return the same error

**Default:** either handle an error (log it, retry, fall back) or propagate it — not both. Log once,
at the top where it is finally handled, with the full chain (`{:#}` for anyhow, or a `tracing` field).
**Never** `error!(..)` then `return Err(e)`; the same failure appears N times in the logs.

## Panics, unwrap and expect

### ERR-09: Panic only for bugs, never for expected failures

**Default:** return `Result` for anything caused by input, the environment, the network, or the
filesystem. **Use** `panic!`/`unreachable!`/`assert!` only for violated invariants that indicate a bug
in *this* program. Document any panicking public function under `# Panics`. **Never** panic in a
library on bad caller input that could have been an `Err`.

### ERR-10: `expect` with an invariant message over `unwrap`; neither in library paths

**Default:** in non-test code, `.unwrap()` is a review flag. When a value truly cannot be absent, use
`.expect("why this cannot fail")` — phrase the message as the invariant ("regex literal is valid"),
not as a complaint ("failed to parse"). **Use** `unwrap` freely in tests, examples, and on constants
proven at compile time. **Never** `unwrap` on I/O, parsing of external input, locks in async code paths
without thinking about poisoning, or `env::var`. Enable `clippy::unwrap_used` for library crates
(see `lints-and-tooling.md`).

```rust
use std::sync::LazyLock;

static VERSION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^\d+\.\d+\.\d+$").expect("VERSION_RE literal is a valid regex")
});

pub fn is_version(s: &str) -> bool {
    VERSION_RE.is_match(s)
}

pub fn first_char_upper(s: &str) -> Option<char> {
    s.chars().next().map(|c| c.to_ascii_uppercase()) // no unwrap: absence is a normal outcome
}
```

### ERR-11: Prefer non-panicking std APIs

**Default:** `slice.get(i)` over `slice[i]` when `i` comes from outside; `checked_*`/`saturating_*`
arithmetic on untrusted numbers; `str::get(a..b)` / `split_at_checked` (1.80+) over byte slicing that
can split a UTF-8 char; `TryFrom` over `as` for narrowing casts. **Never** use `as` to truncate a
`u64`/`usize` that came from input (`clippy::cast_possible_truncation`).

```rust
pub fn item(items: &[String], index: usize) -> Option<&str> {
    items.get(index).map(String::as_str) // no panic on an out-of-range index from a request
}

pub fn to_u16(n: u64) -> Result<u16, std::num::TryFromIntError> {
    u16::try_from(n)
}
```

## Option vs Result

### ERR-12: `Option` for "absent is normal", `Result` for "something went wrong"

**Default:** lookups, `find`, first/last return `Option`. Anything with a reason the caller may want
to report returns `Result`. **Never** return `Result<T, ()>` (use `Option<T>` or a real error type), or
`Option<T>` where the caller needs to know *why* it failed.

## main and exit codes

### ERR-13: `main` returns `anyhow::Result<()>` or `ExitCode`; don't `process::exit` deep inside

**Default:** `fn main() -> anyhow::Result<()>` for simple binaries — a returned error prints its
`Debug` form (anyhow's `Debug` shows the full cause chain) and exits with code 1. **Use**
`fn main() -> ExitCode` when you need specific exit codes or custom error rendering. **Never** call
`std::process::exit` from library code or deep helpers — it skips destructors (unflushed buffers,
temp files).

```rust
use std::process::ExitCode;

fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(!args.is_empty(), "usage: tool <file>...");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}"); // "{:#}" prints the chain on one line: outer: inner: root
            ExitCode::from(2)
        }
    }
}
```

## Inspecting errors

### ERR-14: Match on typed errors; `downcast_ref` only at the application edge

**Default:** callers `match` on library error enums (`Err(ConfigError::MissingKey(k)) => ..`).
**Use** `anyhow::Error::downcast_ref::<T>()` or walking `.chain()` only in application code that must
special-case a specific root cause (e.g. map `io::ErrorKind::NotFound` to exit code 3). **Never**
compare error messages as strings.

```rust
use std::io;

fn exit_code_for(err: &anyhow::Error) -> u8 {
    let not_found = err
        .chain()
        .filter_map(|e| e.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::NotFound);
    if not_found { 3 } else { 1 }
}
```

## Review checklist

- [ ] Public library functions return a typed `thiserror` enum, not `Box<dyn Error>`/`anyhow`/`String` (ERR-01).
- [ ] Application boundary code adds `.context(..)` to I/O and parsing failures (ERR-02).
- [ ] Error variants map to recovery paths; public enums are `#[non_exhaustive]` (ERR-03).
- [ ] `Display` messages are lowercase, no trailing period, don't embed the source (ERR-04).
- [ ] Error types are `Send + Sync + 'static` (ERR-05).
- [ ] `#[from]` only where the conversion is unambiguous (ERR-06).
- [ ] `?` everywhere; no manual `match`-and-rewrap; no log-and-return (ERR-07, ERR-08).
- [ ] No `unwrap()` in non-test code; `expect` messages state the invariant (ERR-09, ERR-10).
- [ ] `get`/`checked_*`/`try_from` on untrusted indices and numbers (ERR-11).
- [ ] `main` returns `Result`/`ExitCode`; no `process::exit` in helpers (ERR-13).
- [ ] No string comparison of error messages (ERR-14).
