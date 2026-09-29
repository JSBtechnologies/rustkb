---
name: rust-security
description: >-
  Security guidance for Rust code and projects. Use when reviewing Rust code for security;
  adding, updating or vetting dependencies (cargo add, Cargo.toml/Cargo.lock changes,
  cargo-deny/cargo-audit/cargo-vet setup, deny.toml, supply-chain or SBOM work, publishing to
  crates.io); writing or reviewing `unsafe`, FFI, `Send`/`Sync` impls, transmute or
  MaybeUninit; handling untrusted input (parsers, network protocols, file uploads, paths,
  shell commands, SQL, deserialization, regex); choosing or using crypto (TLS/rustls,
  password hashing, random tokens, encryption, HMAC, JWT); managing secrets and keeping them
  out of logs; building authn/authz, CORS, CSRF, sessions, rate limits or SSRF-safe
  fetchers in axum/tower-http services; fuzzing, Miri or sanitizers; and responding to
  RustSec advisories, yanked or malicious crates.
---

# rust-security

Security depth for Rust: supply chain, unsafe review, secure coding against untrusted
input, crypto choices, web-service hardening, security testing, and advisory response.
General language idioms (including basic unsafe/FFI mechanics) live in `rust-idioms`;
crate selection in `rust-ecosystem`; project/CI structure in `rust-architecture`.

**Live data beats memory.** When the `rustkb` MCP server is available, use
`check_advisories` for RustSec/OSV lookups on any crate+version you add or audit (never
recall advisory status from memory), `crate_info` for current versions, and `search` /
`get_item` / `explain_lint` / `rust_release` for versioned API docs, lint docs and release
notes.

## Core rules

Most frequent security mistakes in agent-written Rust, with the rule to apply:

| ID | Rule |
|---|---|
| SUP-01 | Gate CI on `cargo deny check` with a committed `deny.toml` (advisories, licenses, bans, sources). |
| SUP-03 | Commit `Cargo.lock`; build and test with `--locked`. |
| SUP-04 | Vet every new crate: exact name (typosquats), repo, owners, maintenance, advisories, transitive weight. |
| SUP-07 | `build.rs` and proc macros execute at build time: no secrets in build jobs, don't build untrusted repos with credentials. |
| UNS-01 | `unsafe_code = "forbid"` unless the crate exists to wrap FFI/low-level code. |
| UNS-02 | Never use `unsafe` (raw pointers, transmute, `static mut`) to get past a borrow-checker error. |
| UNS-06 | `unsafe impl Send/Sync` needs `T: Send`/`T: Sync` bounds and a written thread-safety proof. |
| UNS-07 | No `set_len` over uninitialized memory, no `MaybeUninit::uninit().assume_init()`; use `vec![0; n]` or `spare_capacity_mut`. |
| SEC-02 | No `unwrap`/`expect`/indexing/unchecked arithmetic on untrusted input — panics are DoS. |
| SEC-03 | Bound every read, allocation and decompression by a byte/element limit (`Read::take`). |
| SEC-05 | Checked arithmetic and `try_from` for sizes/money/indices; consider `overflow-checks = true` in release. |
| SEC-07 | No `Path::join` with user input; cap-std or `Component::Normal`-only joins. |
| SEC-08 | `Command::new(prog).arg(x)`, never `sh -c` with interpolation; reject leading `-`. |
| SEC-09 | Bind SQL parameters; never `AssertSqlSafe(format!(..))` in sqlx; allowlist identifiers. |
| SEC-11 | Compare tokens/MACs in constant time (`subtle`, `Mac::verify_slice`). |
| SEC-13 | Secrets in `secrecy` types; no derived `Debug` on `String` secrets; `#[instrument(skip(..))]`. |
| CRYPTO-01 | Never implement crypto primitives, MACs, RNGs or protocols; use vetted high-level APIs. |
| CRYPTO-04 | Never disable TLS certificate verification (`danger_accept_invalid_certs`, always-OK verifiers). |
| CRYPTO-05 | Passwords: Argon2id via `argon2` (PHC string), in `spawn_blocking`. |
| CRYPTO-06 | Secrets from a CSPRNG (`rand::rng()`, `getrandom::fill`), ≥128 bits; never seeded/small RNGs. |
| WEB-02 | Authorize every object access (scope queries by owner); 404 for others' objects. |
| WEB-04 | CORS with explicit origins; never permissive/mirrored origins with credentials. |

## Routing

| Situation | Read |
|---|---|
| Setting up cargo-deny / deny.toml, cargo-audit, cargo-vet, SBOM, auditable binaries | [references/supply-chain.md](references/supply-chain.md) |
| Adding or upgrading a dependency; choosing features; git dependencies | [references/supply-chain.md](references/supply-chain.md) (SUP-04..06) |
| Publishing a crate from CI (trusted publishing, tokens) | [references/supply-chain.md](references/supply-chain.md) (SUP-10) |
| Writing or reviewing `unsafe`, FFI exports/imports, `Send`/`Sync` impls, `transmute`, `MaybeUninit`, packed structs | [references/unsafe-review.md](references/unsafe-review.md) |
| Parsing untrusted bytes/JSON/binary formats; limits, recursion, integer overflow | [references/secure-coding.md](references/secure-coding.md) (SEC-01..06) |
| File paths from users, spawning processes, SQL (sqlx/diesel), deserialization | [references/secure-coding.md](references/secure-coding.md) (SEC-07..10) |
| Secrets: comparison, storage in memory, logging, Debug impls; temp files and fs races | [references/secure-coding.md](references/secure-coding.md) (SEC-11..14) |
| TLS clients/servers, rustls provider, certificates, reqwest config | [references/crypto.md](references/crypto.md) (CRYPTO-02..04) |
| Password hashing, random tokens/keys, encryption, HMAC, JWT | [references/crypto.md](references/crypto.md) (CRYPTO-05..08) |
| axum/tower-http service: auth extractors, IDOR, body limits, timeouts, CORS, CSRF | [references/web-security.md](references/web-security.md) (WEB-01..05) |
| Rate limiting, security headers, sessions/cookies, error leakage, SSRF, XSS | [references/web-security.md](references/web-security.md) (WEB-06..11) |
| Fuzzing, property tests for invariants, Miri, sanitizers, Kani, negative authz tests | [references/testing-for-security.md](references/testing-for-security.md) |
| cargo-deny/audit reports an advisory; yanked crate; reporting or publishing a vulnerability; malicious crate | [references/incident-and-advisories.md](references/incident-and-advisories.md) |

## Security review checklist (quick pass)

1. `cargo deny check` passes; new dependencies vetted (SUP-04); lockfile committed.
2. `grep -rn "unsafe\|transmute\|static mut\|set_len\|from_raw_parts"` — each hit has a
   `// SAFETY:` proof that survives the UNS-04 checklist.
3. Every input boundary: size limit, depth limit, no panics, validated types.
4. `grep -rn "AssertSqlSafe\|format!(.*SELECT\|sh\", \"-c\|danger_\|permissive()"` — each
   hit is justified or fixed.
5. Secrets: `secrecy` types, no derived `Debug` over plain `String` secrets, `instrument`
   skips, constant-time comparisons.
6. Crypto: no hand-rolled primitives, Argon2id for passwords, CSPRNG tokens, TLS verified.
7. Web: auth extractor on every protected route, object-level authz, CORS/CSRF/rate
   limits/body limits configured, generic error bodies.
8. Tests: fuzz target per parser, Miri for `unsafe`, negative authz tests.
