---
id: security/testing-for-security
title: Security testing (fuzzing, property tests, Miri, sanitizers, Kani)
summary: >-
  Which security testing technique to apply where: cargo-fuzz/libFuzzer for every
  untrusted-input parser, `arbitrary` for structured fuzzing, proptest for invariants on
  stable, Miri for any crate with unsafe, sanitizers on nightly, Kani for bounded proofs,
  plus negative authorization tests and what to run in CI.
area: security
tags: [fuzzing, cargo-fuzz, libfuzzer, arbitrary, proptest, property-testing, miri, sanitizers, asan, tsan, kani, formal-verification, loom, ci]
rust: "1.96"
edition: "2024"
crates:
  cargo-fuzz: "0.13"
  libfuzzer-sys: "0.4"
  arbitrary: "1.4"
  proptest: "1.11"
  kani-verifier: "0.68"
  bolero: "0.13"
  cargo-careful: "0.4"
  loom: "0.7"
verified: 2026-09-29
sources:
  - https://rust-fuzz.github.io/book/
  - https://github.com/rust-fuzz/cargo-fuzz
  - https://docs.rs/arbitrary/latest/arbitrary/
  - https://proptest-rs.github.io/proptest/
  - https://github.com/rust-lang/miri
  - https://doc.rust-lang.org/beta/unstable-book/compiler-flags/sanitizer.html
  - https://model-checking.github.io/kani/
---

# Security testing

Unit tests check the inputs you thought of. Security bugs live in the inputs you didn't.

| Code under test | Technique | Toolchain |
|---|---|---|
| Parser/decoder of untrusted bytes (protocols, file formats, custom deserializers) | cargo-fuzz (FUZZ-01) | nightly, Linux/macOS |
| Stateful APIs, data structures | `arbitrary`-driven op sequences vs a model (FUZZ-02) | nightly |
| Invariants: roundtrip, "never panics", "stays in bounds" | proptest (FUZZ-03) | stable, in `cargo test` |
| Any `unsafe` | Miri (FUZZ-04) | nightly |
| `unsafe` + FFI, C deps, races at scale | Sanitizers (FUZZ-05) | nightly, Linux |
| Small critical functions (arithmetic, unsafe helpers) | Kani proofs (FUZZ-06) | Kani toolchain |
| Authn/authz | Negative integration tests (FUZZ-07) | stable |
| Custom lock-free/sync primitives | loom | stable |

## FUZZ-01: Fuzz every parser of untrusted input with cargo-fuzz

Default: a cargo-fuzz target for each entry point that takes attacker bytes. libFuzzer's
coverage-guided mutation with AddressSanitizer (on by default) finds panics, infinite loops,
OOMs and — in `unsafe` code — memory corruption.

```sh
cargo install --locked cargo-fuzz
cargo +nightly fuzz init              # creates fuzz/ (its own crate, not a workspace member)
cargo +nightly fuzz add parse_frame
cargo +nightly fuzz run parse_frame -- -max_total_time=300 -max_len=65536
```

```rust
// fuzz/fuzz_targets/parse_frame.rs
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Must never panic, hang or allocate unboundedly, whatever `data` is.
    let _ = my_crate::parse_frame(data);
});
```

- Requires nightly and a Unix-like OS (Linux/macOS; use WSL or a container on Windows).
- Crash inputs land in `fuzz/artifacts/<target>/`. Reproduce with
  `cargo +nightly fuzz run <target> <file>`, minimise with `cargo fuzz tmin`, then add the
  input as a regular `#[test]` so the fix stays fixed.
- Commit a seed corpus (`fuzz/corpus/<target>/`) of valid samples; fuzzing from real
  examples reaches deep code much faster.
- Differential fuzzing: when two implementations exist (yours vs a reference, old vs new),
  assert they agree.
- Roundtrip fuzzing: `decode(encode(x)) == x` catches encoder/decoder mismatches that
  become smuggling bugs.
- Code only for fuzz builds: `#[cfg(fuzzing)]` (cargo-fuzz sets it). Declare it so the
  `unexpected_cfgs` lint stays quiet:

```toml
[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ["cfg(fuzzing)", "cfg(kani)"] }
```

Long-running fuzzing belongs in OSS-Fuzz (open source) or ClusterFuzzLite/a nightly job;
PR CI runs each target for a short time (FUZZ-08).

## FUZZ-02: Structured fuzzing with arbitrary

For APIs that take typed input or sequences of calls, derive `Arbitrary` and let the fuzzer
build values instead of raw bytes. Compare against a simple model to find logic bugs, not
just crashes.

```rust
#[derive(Debug, arbitrary::Arbitrary)]
pub enum Op {
    Push(u8),
    Pop,
    Truncate(usize),
}

// fuzz/fuzz_targets/ring_buffer.rs
fuzz_target!(|ops: Vec<Op>| {
    let mut sut = my_crate::RingBuffer::with_capacity(16);
    let mut model = std::collections::VecDeque::new();
    for op in ops {
        match op {
            Op::Push(b) => { sut.push(b); model.push_back(b); if model.len() > 16 { model.pop_front(); } }
            Op::Pop => assert_eq!(sut.pop(), model.pop_front()),
            Op::Truncate(n) => { sut.truncate(n); model.truncate(n); }
        }
        assert_eq!(sut.len(), model.len());
    }
});
```

Put the `Arbitrary` derive behind a feature (`arbitrary = { version = "1.4", optional =
true, features = ["derive"] }`) so production builds don't carry it.

## FUZZ-03: Property tests for security invariants (stable, every CI run)

proptest runs in normal `cargo test` on stable, shrinks failures to a minimal case, and
persists failing seeds in `proptest-regressions/` (commit that directory).

```rust
#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parse_frame_never_panics(data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = crate::parse_frame(&data);
        }

        #[test]
        fn safe_join_stays_inside(s in "\\PC{0,64}") {
            let base = std::path::Path::new("/srv/files");
            if let Some(p) = crate::safe_join(base, &s) {
                prop_assert!(p.starts_with(base));
            }
        }
    }
}
```

Good security properties: never panics on any input; output length ≤ limit; roundtrip
identity; authorization function denies whenever owner ≠ caller; sanitizer output contains
no `<script`; checked arithmetic never returns a value smaller than an input.

`bolero` is an option when you want one harness to run as a property test, under
libFuzzer/AFL, and under Kani.

## FUZZ-04: Run Miri on every crate that contains unsafe

Miri interprets the test suite and reports undefined behaviour: out-of-bounds and
use-after-free, reads of uninitialized memory, invalid values, misaligned accesses,
aliasing-model violations, data races and leaks.

```sh
rustup toolchain install nightly --component miri
cargo +nightly miri setup
cargo +nightly miri test
MIRIFLAGS="-Zmiri-tree-borrows" cargo +nightly miri test     # also check the Tree Borrows model
MIRIFLAGS="-Zmiri-many-seeds=0..16" cargo +nightly miri test # explore more thread interleavings
cargo +nightly miri test --target s390x-unknown-linux-gnu    # big-endian
```

- Miri can't execute FFI calls into C; mark such tests `#[cfg_attr(miri, ignore)]` and cover
  that code with sanitizers instead.
- It is 10–1000× slower: shrink iteration counts under `cfg(miri)` rather than skipping
  tests.
- Filesystem/network/clock access needs `-Zmiri-disable-isolation`; prefer tests that don't
  need it.
- A Miri error in a dependency is a real finding: report it upstream and check
  `check_advisories` for an existing `unsound` advisory.

`cargo-careful` (nightly) runs tests with a std built with debug assertions — a cheap extra
layer that catches some UB precondition violations at full speed.

## FUZZ-05: Sanitizers for FFI, C dependencies and concurrency

Sanitizers are nightly-only (`-Zsanitizer`) and need an explicit `--target` so build
scripts and proc macros aren't instrumented.

```sh
# AddressSanitizer: heap/stack overflows, use-after-free (also the cargo-fuzz default)
RUSTFLAGS=-Zsanitizer=address RUSTDOCFLAGS=-Zsanitizer=address \
  cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu
# ThreadSanitizer: data races, including across FFI
RUSTFLAGS=-Zsanitizer=thread RUSTDOCFLAGS=-Zsanitizer=thread \
  cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu
```

- MemorySanitizer needs *all* code, including C dependencies, instrumented; ASan and TSan
  are the practical defaults.
- Use sanitizers for code Miri can't run (FFI, inline asm, huge tests); use Miri for
  precise aliasing/validity checks in pure-Rust `unsafe`.

## FUZZ-06: Kani for bounded proofs of small critical functions

Kani model-checks a function over *all* inputs within bounds — e.g. "this length
arithmetic never overflows and the index is always in bounds". Use it for small, critical
helpers (unsafe wrappers, parsers of fixed-size headers, arithmetic), not whole programs.

```rust
#[cfg(kani)]
mod proofs {
    #[kani::proof]
    fn frame_len_never_exceeds_input() {
        let input: [u8; 8] = kani::any();
        if let Ok(body) = crate::parse_frame(&input) {
            assert!(body.len() <= input.len() - 4);
        }
    }
}
```

```sh
cargo install --locked kani-verifier && cargo kani setup
cargo kani
```

Loops need `#[kani::unwind(N)]`. A passing proof covers exactly the bounded inputs; state
the bounds in the proof's doc comment.

## FUZZ-07: Test that access is denied, not only that it's granted

Agent-written test suites almost only test the happy path. For every protected route add
negative tests:

- no credentials → 401; malformed/expired/wrong-audience token → 401;
- valid user, someone else's object ID → 404 (WEB-02); non-admin on admin route → 403;
- cross-origin state-changing request with cookies → 403 (WEB-05);
- oversized body → 413; deeply nested JSON → 400/422, not a crash.

Drive the router in-process (`tower::ServiceExt::oneshot`) so these run in milliseconds
on every PR.

## FUZZ-08: What runs where in CI

| Job | Trigger | Command |
|---|---|---|
| Unit + property + negative authz tests | every PR | `cargo test --locked` |
| Miri (crates with `unsafe`) | every PR | `cargo +nightly miri test` |
| Fuzz smoke (each target, 1–5 min) | every PR touching parsers | `cargo +nightly fuzz run <t> -- -max_total_time=120` |
| Long fuzzing | nightly / OSS-Fuzz / ClusterFuzzLite | corpus persisted between runs |
| Sanitizers | nightly or pre-release | ASan/TSan commands above |
| Kani proofs | every PR (they're bounded) | `cargo kani` |
| Supply chain | every PR + daily | `cargo deny check` (supply-chain.md) |

Any crash found by a fuzzer or sanitizer is fixed with a regression test that reproduces it
on stable (`#[test]` with the minimised input).
