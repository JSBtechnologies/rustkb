---
id: ecosystem/testing-and-benchmarking
title: Testing, mocking, benchmarking and profiling crates
summary: >-
  Which crates and cargo tools to use for property tests, snapshots, parameterized tests,
  mocks, HTTP fakes, CLI tests, test runners, coverage, benchmarks and profiling.
area: ecosystem
tags: [testing, proptest, insta, rstest, mockall, wiremock, nextest, coverage, criterion, divan, benchmark, profiling]
rust: "1.96"
edition: "2024"
crates:
  proptest: "1.11"
  quickcheck: "1.1"
  insta: "1.48"
  cargo-insta: "1.48"
  rstest: "0.27"
  test-case: "3.4"
  mockall: "0.15"
  wiremock: "0.6"
  httpmock: "0.8"
  tempfile: "3.27"
  assert_cmd: "2.2"
  predicates: "3.1"
  pretty_assertions: "1.4"
  similar-asserts: "2.0"
  testcontainers: "0.28"
  serial_test: "4.0"
  loom: "0.7"
  criterion: "0.8"
  divan: "0.1"
  gungraun: "0.20"
  cargo-nextest: "0.9"
  cargo-llvm-cov: "0.9"
  cargo-mutants: "27.1"
  samply: "0.13"
  flamegraph: "0.6"
  tower: "0.5"
  http-body-util: "0.1"
  dhat: "0.3"
  expect-test: "1.5"
  mockito: "1.7"
  proptest-derive: "0.8"
  shuttle: "0.9"
  snapbox: "1.2"
  trycmd: "1.2"
  tokio-console: "0.1"
verified: 2026-09-29
sources:
  - https://proptest-rs.github.io/proptest/
  - https://insta.rs/
  - https://nexte.st/
  - https://github.com/criterion-rs/criterion.rs
  - https://github.com/nvzqz/divan
  - https://github.com/gungraun/gungraun
---

# Testing, mocking, benchmarking and profiling crates

Test strategy (unit vs integration layout, what to mock, doctests, test naming) is in the
`rust-idioms` testing reference; fuzzing and Miri are in `rust-security`. This file picks tools.
All libraries below go in `[dev-dependencies]`.

## TST-01: Crate choice by test need

| Need | Default | Alternatives |
|---|---|---|
| Property-based tests | `proptest` (1.11) | `quickcheck` (1.1) — simpler, weaker shrinking |
| Snapshot tests | `insta` (1.48) + `cargo-insta` (1.48) | `expect-test` |
| Parameterized tests / fixtures | `rstest` (0.27) | `test-case` (3.4) |
| Mock a trait | `mockall` (0.15) | hand-written fake struct (often better) |
| Fake HTTP server | `wiremock` (0.6) | `httpmock` (0.8), `mockito` |
| Temp files/dirs | `tempfile` (3.27) | — (`tempdir` is deprecated) |
| Run the binary | `assert_cmd` (2.2) + `predicates` (3.1) | `trycmd`, `snapbox` |
| Readable `assert_eq!` diffs | `pretty_assertions` (1.4) | `similar-asserts` (2.0) |
| Real DB/broker in tests | `testcontainers` (0.28) | CI service containers |
| Tests that share global state | `serial_test` (4.0) | better: remove the global state |
| Exhaustive concurrency testing | `loom` (0.7) | `shuttle` (randomized) |

## TST-02: proptest for invariants and round-trips

Default: add a property test whenever a function has an invariant (encode/decode round-trip,
sort idempotence, parser never panics).

```rust
use proptest::prelude::*;

fn encode(s: &str) -> String { s.chars().rev().collect() }
fn decode(s: &str) -> String { s.chars().rev().collect() }

proptest! {
    #[test]
    fn roundtrip(s in r"\PC*") {
        prop_assert_eq!(decode(&encode(&s)), s);
    }
}
```

Commit the `proptest-regressions/` files so found failures are re-run forever.
Derive strategies for your own types with `proptest-derive` or `prop_compose!`.

## TST-03: insta for large or structured outputs

Default: snapshot any output that is tedious to assert by hand (rendered text, CLI output,
serialized structs, error messages). Review changes with `cargo insta review`.

```rust
#[test]
fn renders_report() {
    let report = format!("total: {}\nfailed: {}", 10, 2);
    insta::assert_snapshot!(report, @r"
    total: 10
    failed: 2
    ");
}
```

Enable `yaml`/`json` features for `assert_yaml_snapshot!`/`assert_json_snapshot!` of serde
types; use `redactions` for timestamps and IDs. In CI run with `INSTA_UPDATE=no` (the default
when `CI` is set) so missing snapshots fail.

## TST-04: rstest for tables of cases

```rust
use rstest::rstest;

#[rstest]
#[case("1.2.3", Some(1))]
#[case("10.0.0", Some(10))]
#[case("x", None)]
fn parses_major(#[case] input: &str, #[case] expected: Option<u64>) {
    let got = input.split('.').next().and_then(|m| m.parse().ok());
    assert_eq!(got, expected);
}
```

`#[fixture]` functions give reusable setup; `#[rstest]` also works on `#[tokio::test]`
async tests.

## TST-05: Mocks — prefer fakes, use mockall at boundaries

Default: depend on a trait at the boundary (clock, repository, HTTP API) and pass a small
hand-written fake in tests. Use `mockall` when you need call expectations/counts.

```rust
#[cfg_attr(test, mockall::automock)]
trait Clock {
    fn now_secs(&self) -> u64;
}

fn is_expired(clock: &dyn Clock, deadline: u64) -> bool {
    clock.now_secs() > deadline
}

#[test]
fn expired_after_deadline() {
    let mut clock = MockClock::new();
    clock.expect_now_secs().return_const(100_u64);
    assert!(is_expired(&clock, 50));
}
```

Don't mock types you don't own (e.g. `reqwest::Client`, `sqlx::PgPool`): fake the HTTP server
(`wiremock`) or run the real database (`testcontainers`) instead.

## TST-06: Testing web handlers and HTTP clients

Handlers: call the axum `Router` as a tower `Service` — no socket, no port.

```rust
#[tokio::test]
async fn healthz_returns_ok() {
    use axum::{Router, body::Body, http::{Request, StatusCode}, routing::get};
    use http_body_util::BodyExt;
    use tower::ServiceExt; // for `oneshot`

    let app = Router::new().route("/healthz", get(|| async { "ok" }));
    let res = app
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}
```

Clients: point them at a `wiremock::MockServer` and assert on requests:

```rust
#[tokio::test]
async fn client_handles_404() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/42"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let res = reqwest::get(format!("{}/users/42", server.uri())).await.unwrap();
    assert_eq!(res.status(), 404);
}
```

## TST-07: Test runner and coverage tools

- `cargo-nextest` (0.9): default runner for CI and local use — per-test
  processes, parallelism, retries for flaky tests, JUnit output. It does **not** run doctests:
  also run `cargo test --doc`.
- `cargo-llvm-cov` (0.9): source-based coverage (`cargo llvm-cov nextest`),
  lcov/HTML/Codecov output. Prefer it over tarpaulin.
- `cargo-mutants` (27.1): mutation testing to find code whose tests don't
  notice changes — run periodically, not on every PR.
- Install tools in CI with `cargo binstall` or `taiki-e/install-action` rather than compiling them.

## TST-08: Benchmarks

Default: `divan` (0.1) for quick, low-boilerplate micro-benchmarks, or `criterion`
(0.8) for statistically rigorous comparisons against saved baselines. Both need
`harness = false` in `[[bench]]`. criterion is maintained again under the `criterion-rs`
organisation; divan's last release is 2025-04 (repository still active).

```rust
// benches/sum.rs  —  [[bench]] name = "sum", harness = false
fn main() {
    divan::main();
}

#[divan::bench(args = [10, 1_000, 100_000])]
fn sum_to(n: u64) -> u64 {
    (0..divan::black_box(n)).sum()
}
```

```rust
// benches/crit.rs  —  [[bench]] name = "crit", harness = false
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

fn bench_sum(c: &mut Criterion) {
    c.bench_function("sum 1000", |b| b.iter(|| (0..black_box(1_000u64)).sum::<u64>()));
}

criterion_group!(benches, bench_sum);
criterion_main!(benches);
```

- Use `std::hint::black_box` (stable since 1.66) rather than `criterion::black_box`.
- Noise-free instruction-count benchmarks for CI (Linux + Valgrind): `gungraun`
  (0.20) — the renamed successor of `iai-callgrind`.
- Nightly `#[bench]`/`test::Bencher` is not an option on stable; don't generate it.

## TST-09: Profiling

- CPU sampling: `samply` (0.13) (`samply record ./target/release/app`, opens the
  Firefox Profiler UI; Linux/macOS/Windows) or `cargo flamegraph` (0.6)
  (perf/dtrace).
- Build profiled binaries with `debug = "line-tables-only"` (or `true`) in `[profile.release]`
  so stacks are symbolised.
- Heap profiling: `dhat` (in-process, test-friendly) or platform tools (heaptrack, Instruments).
- Async runtime stalls: `tokio-console` (see `observability-crates.md`).
