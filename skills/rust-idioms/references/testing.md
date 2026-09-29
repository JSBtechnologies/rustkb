---
id: idioms/testing
title: Testing
summary: >-
  Where tests go (unit in-module, one integration-test binary, doctests), how to write them
  (expect/unwrap with context, assert on error variants, deterministic), and which tools to use:
  proptest for invariants, insta for snapshots, fakes over mocks, tokio paused time, cargo-nextest,
  cargo-llvm-cov and cargo-mutants.
area: idioms
tags: [testing, unit-tests, integration-tests, doctests, proptest, insta, nextest, mocking, tokio-test, coverage]
rust: "1.96"
edition: "2024"
crates:
  proptest: "1.11"
  insta: "1.48"
  cargo-insta: "1.48"
  rstest: "0.27"
  mockall: "0.15"
  tempfile: "3.27"
  tokio: "1.53"
  pretty_assertions: "1.4"
  cargo-nextest: "0.9"
  cargo-llvm-cov: "0.9"
  cargo-mutants: "27.1"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/book/ch11-03-test-organization.html
  - https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html
  - https://doc.rust-lang.org/edition-guide/rust-2024/rustdoc-doctests.html
  - https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html
  - https://nexte.st/
  - https://insta.rs/docs/
  - https://proptest-rs.github.io/proptest/
  - https://docs.rs/tokio/latest/tokio/attr.test.html
---

# Testing

Decides how Rust tests are organised, written and run. Read this when adding tests, setting up a
test layout, or reviewing tests an agent wrote. Benchmarks are in `performance.md`, Miri in
`unsafe-ffi.md`, loom in `concurrency.md`, fuzzing in the rust-security skill.

## Test layout

### TEST-01: Put unit tests in a `#[cfg(test)] mod tests` at the bottom of the file they test

**Default:** unit tests live next to the code, in `mod tests` with `use super::*;`, and may test
private functions. **Never** create a `src/tests/` tree mirroring modules, or make items `pub` just to
test them.

```rust
pub fn slugify(title: &str) -> String {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_words_with_dashes() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
    }

    #[test]
    fn empty_title_gives_empty_slug() {
        assert_eq!(slugify("  --  "), "");
    }
}
```

### TEST-02: Use one integration-test binary, not one per file

**Default:** integration tests exercise only the public API, from `tests/`. Every `tests/*.rs` file is
compiled and linked as a separate crate, which gets slow fast; put them in a single binary with
modules. **Never** put shared helpers in `tests/common.rs` (it becomes its own empty test binary);
use a module inside the single binary instead.

```text
tests/
  it/
    main.rs        // mod api; mod cli; mod helpers;
    helpers.rs     // shared fixtures, builders, fake impls
    api.rs
    cli.rs
```

Binary crates: keep logic in `src/lib.rs` (a thin `main.rs` calls it) so integration tests can reach
it; test the actual binary with `env!("CARGO_BIN_EXE_<name>")` + `std::process::Command`.

### TEST-03: Doctest every public example; hide setup lines, use `?`

**Default:** examples in `///` docs are tests — keep them compiling and asserting something. Hide
boilerplate with `# ` lines and end fallible examples with `# Ok::<(), MyError>(())` so they can use
`?` (see `api-design.md`, API-18). In edition 2024, doctests are merged into one binary, so they run
much faster; mark a doctest `standalone_crate` only if it relies on being its own process/crate.
Use `no_run` for examples that touch the network/filesystem, `compile_fail` to document misuse.
**Never** mark examples `ignore` to silence a failure.

## Writing assertions

### TEST-04: In tests, prefer `expect("what")`/`unwrap()` over `?`

**Default:** `unwrap()`/`expect()` in tests is correct: a panic reports file and line. A test that
returns `Result` and fails via `?` prints only `Error: ParseIntError { … }` with no location. **Use
`-> Result<(), E>` when** the test mirrors user-facing example code or every step would otherwise need
`expect`. If you enable clippy `unwrap_used`/`expect_used` for production code, set
`allow-unwrap-in-tests = true` / `allow-expect-in-tests = true` in `clippy.toml` (see
`lints-and-tooling.md`).

### TEST-05: Assert on the exact error variant, not on "is_err" or `#[should_panic]`

**Default:** check which error came back with `assert_eq!` (if the error is `PartialEq`) or
`assert_matches!` (stable since 1.96; not in the prelude — `use std::assert_matches;`), so the test
fails if the code errors for the wrong reason. **Use `#[should_panic(expected = "…")]` only** for
functions whose documented contract is to panic. **Never** write `assert!(result.is_err())` as the
only check.

```rust
use std::assert_matches;

#[derive(Debug, PartialEq)]
pub enum NameError {
    Empty,
    TooLong { len: usize, max: usize },
}

pub fn validate_name(name: &str) -> Result<&str, NameError> {
    const MAX: usize = 16;
    match name.chars().count() {
        0 => Err(NameError::Empty),
        len if len > MAX => Err(NameError::TooLong { len, max: MAX }),
        _ => Ok(name),
    }
}

#[test]
fn rejects_empty_name() {
    assert_eq!(validate_name(""), Err(NameError::Empty));
}

#[test]
fn rejects_long_name_with_length() {
    assert_matches!(validate_name(&"x".repeat(20)), Err(NameError::TooLong { len: 20, .. }));
}
```

### TEST-06: Keep tests deterministic and isolated

**Default:** tests run in parallel threads of one process (or separate processes under nextest), in
any order. So: no `thread::sleep` for synchronisation, no real network, no fixed ports (bind
`127.0.0.1:0`), no shared files (use `tempfile::tempdir()`), fixed RNG seeds, and no mutating process
state. **Never** call `std::env::set_var` in tests — it is `unsafe` in edition 2024 because it races
with other threads reading the environment; pass configuration explicitly instead.

```rust
use std::io::Write;

fn count_lines(path: &std::path::Path) -> std::io::Result<usize> {
    Ok(std::fs::read_to_string(path)?.lines().count())
}

#[test]
fn counts_lines_in_file() {
    let dir = tempfile::tempdir().expect("create temp dir"); // deleted on drop
    let path = dir.path().join("input.txt");
    let mut f = std::fs::File::create(&path).expect("create file");
    writeln!(f, "a\nb\nc").expect("write");
    assert_eq!(count_lines(&path).expect("read"), 3);
}
```

## Property and snapshot testing

### TEST-07: Use proptest for invariants over many inputs

**Default:** when a property holds for all inputs — round-trips (`parse(display(x)) == x`),
idempotence, "never panics", ordering/length invariants — write a `proptest!` test instead of
hand-picking five examples. Commit the `proptest-regressions/` files it creates on failure.
**Never** reimplement the function under test as the oracle; assert properties instead.

```rust
use proptest::prelude::*;
use std::fmt::Write;

pub fn encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
        let _ = write!(out, "{b:02x}"); // writing to a String cannot fail
        out
    })
}

pub fn decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

proptest! {
    #[test]
    fn hex_round_trips(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        prop_assert_eq!(decode(&encode(&bytes)), Some(bytes));
    }

    #[test]
    fn decode_never_panics(s in "\\PC*") {
        let _ = decode(&s);
    }
}
```

### TEST-08: Use insta snapshots for large or structured outputs

**Default:** when the expected value is big (rendered text, `Debug` of a tree, JSON, CLI output),
use `insta` instead of a giant string literal. Prefer inline snapshots (`@"…"`) for short output;
review changes with `cargo insta review` (install `cargo-insta`), never by blindly accepting.
Redact nondeterministic fields (timestamps, UUIDs) with insta's `redactions` feature. **Never**
snapshot output that contains `HashMap` iteration order — sort or use `BTreeMap` first.

```rust
#[derive(Debug)]
struct Token<'a> {
    kind: &'static str,
    text: &'a str,
}

fn tokenize(src: &str) -> Vec<Token<'_>> {
    src.split_whitespace()
        .map(|w| Token {
            kind: if w.chars().all(|c| c.is_ascii_digit()) { "num" } else { "word" },
            text: w,
        })
        .collect()
}

#[test]
fn tokenizes_mixed_input() {
    let rendered: Vec<String> = tokenize("add 1 2")
        .iter()
        .map(|t| format!("{}:{}", t.kind, t.text))
        .collect();
    insta::assert_snapshot!(rendered.join("\n"), @r"
    word:add
    num:1
    num:2
    ");
}
```

## Test doubles and parametrisation

### TEST-09: Prefer hand-written fakes behind a trait over mocking frameworks

**Default:** abstract the side-effecting dependency (clock, HTTP, storage) behind a small trait and
write an in-memory fake for tests; assert on resulting state, not on call sequences. **Use `mockall`
when** you genuinely need interaction checks (called exactly once with these args) on a large trait.
**Never** mock types you own that are cheap to construct for real, and never introduce a trait only
for tests where passing a value (e.g. `now: SystemTime`) would do.

```rust
use std::collections::HashMap;

pub trait KeyValue {
    fn get(&self, key: &str) -> Option<String>;
    fn put(&mut self, key: &str, value: String);
}

pub fn bump_counter(store: &mut impl KeyValue, key: &str) -> u64 {
    let next = store.get(key).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + 1;
    store.put(key, next.to_string());
    next
}

#[derive(Default)]
struct InMemory(HashMap<String, String>);

impl KeyValue for InMemory {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn put(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
}

#[test]
fn counter_starts_at_one_and_increments() {
    let mut store = InMemory::default();
    assert_eq!(bump_counter(&mut store, "hits"), 1);
    assert_eq!(bump_counter(&mut store, "hits"), 2);
    assert_eq!(store.get("hits").as_deref(), Some("2"));
}
```

### TEST-10: Table-driven tests: a plain array first, `rstest` when fixtures multiply

**Default:** a `for (input, expected) in [...]` loop with a message naming the case is enough for
most tables. **Use `rstest`** when you want each case reported as its own test or need reusable
fixtures. `pretty_assertions` gives readable diffs for large `assert_eq!` values.

```rust
fn is_leap(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

#[test]
fn leap_years() {
    for (year, expected) in [(2024, true), (1900, false), (2000, true), (2023, false)] {
        assert_eq!(is_leap(year), expected, "year {year}");
    }
}

#[rstest::rstest]
#[case(2024, true)]
#[case(1900, false)]
fn leap_years_rstest(#[case] year: u32, #[case] expected: bool) {
    assert_eq!(is_leap(year), expected);
}
```

## Async tests

### TEST-11: Use `#[tokio::test]` and paused time — never real sleeps

**Default:** async tests use `#[tokio::test]` (current-thread runtime). For timeouts, retries and
intervals use `#[tokio::test(start_paused = true)]` (requires tokio's `test-util` feature in
`[dev-dependencies]`): time only advances when all tasks are idle, so a 30-second timeout test runs
instantly and deterministically. Use `flavor = "multi_thread"` only to test real parallelism.

```rust
use std::time::Duration;

async fn with_timeout<T>(fut: impl Future<Output = T>, limit: Duration) -> Option<T> {
    tokio::time::timeout(limit, fut).await.ok()
}

#[tokio::test(start_paused = true)]
async fn slow_call_times_out() {
    let slow = tokio::time::sleep(Duration::from_secs(30));
    assert!(with_timeout(slow, Duration::from_secs(5)).await.is_none()); // runs instantly
}
```

## Running tests in CI

### TEST-12: Run tests with cargo-nextest, plus a separate doctest step

**Default:** in CI, `cargo nextest run --workspace --all-features` (process-per-test isolation,
parallelism, retries for known-flaky tests, JUnit output), followed by `cargo test --doc --workspace`
because nextest does not run doctests. Locally `cargo test` is fine. See the rust-architecture skill
for the full CI pipeline.

### TEST-13: Measure coverage with cargo-llvm-cov; use cargo-mutants on critical logic

**Default:** `cargo llvm-cov nextest` (or `cargo llvm-cov --doctests` on nightly) for line/region
coverage reports; treat the number as a gap-finder, not a target. For parsers, money/permission logic
and other critical code, run `cargo mutants` to find code whose behaviour no test checks.
**Never** chase 100% coverage with assertion-free tests.

## Review checklist

- [ ] Unit tests in `#[cfg(test)] mod tests`; nothing made `pub` just for tests (TEST-01)
- [ ] Integration tests compiled as one binary; helpers not in `tests/common.rs` (TEST-02)
- [ ] Public items have compiling doctests using `?`; none `ignore`d to hide failures (TEST-03)
- [ ] Tests use `expect`/`unwrap` for location info; no silent `?` chains without context (TEST-04)
- [ ] Error tests check the variant (`assert_eq!`/`assert_matches!`), not just `is_err()` (TEST-05)
- [ ] No sleeps, real network, fixed ports, shared files or `set_var` (TEST-06, TEST-11)
- [ ] Invariants covered by proptest; large outputs by insta snapshots (TEST-07, TEST-08)
- [ ] Fakes behind small traits rather than mock-heavy tests (TEST-09)
- [ ] Async timing tests use `start_paused = true` (TEST-11)
- [ ] CI runs nextest + `cargo test --doc` (TEST-12)
