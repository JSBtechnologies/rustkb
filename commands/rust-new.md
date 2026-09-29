---
description: Design and scaffold a new best-practice Rust project (workspace layout, lints, CI, deps with verified versions)
argument-hint: <what you are building, e.g. "an HTTP API for invoices with Postgres">
---

Design and scaffold a new Rust project for: $ARGUMENTS

1. Use the `rust-architect` agent to produce the design (kickoff questions from the
   `rust-architecture` skill; ask me only the questions whose answers genuinely change the design —
   assume sensible defaults for the rest and state them).
2. After I confirm the design, scaffold it using the templates in the `rust-architecture` skill
   (`templates` reference): workspace `Cargo.toml` with `[workspace.lints]`, `rustfmt.toml`,
   `clippy.toml`, `deny.toml`, `.github/workflows/ci.yml`, `rust-toolchain.toml` only if justified.
3. Add dependencies with `cargo add` (never hand-typed versions); confirm each with the rustkb
   `crate_info` tool when available.
4. Write a minimal vertical slice that compiles, with one test, then run
   `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`. Fix until green.
5. Summarise the decisions with the rule ids they follow.
