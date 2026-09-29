---
description: Audit a Rust project against rustkb best practices — dependencies, advisories, lints, architecture, unsafe
argument-hint: [path to the Rust project, default: current directory]
---

Audit the Rust project at `$ARGUMENTS` (the current directory if empty) against the rustkb knowledge base.
Report findings only — don't change code unless I ask.

1. **Supply chain**: run the rustkb `check_advisories` tool with `lockfile` = its `Cargo.lock`
   (or `rustkb audit --lockfile Cargo.lock`). For each direct dependency in `Cargo.toml`, check
   `crate_info`: flag tier `avoid` crates (with the replacement), semver-incompatible lag behind the
   latest release, and unmaintained crates. Follow the `rust-security` skill (supply-chain).
2. **Lints & tooling**: compare `[lints]`/`[workspace.lints]`, `clippy.toml`, `rustfmt.toml`, CI
   against the `rust-idioms` skill (lints-and-tooling) and `rust-architecture` (ci-cd-release). Run
   `cargo clippy --all-targets` and summarise the top lint categories.
3. **Code patterns**: sample the codebase for the high-signal anti-patterns in the skills — `unwrap`
   /`expect` on fallible input in library code, `Box<dyn Error>` in library APIs, blocking calls in
   async, locks held across `.await`, needless `clone`/`Arc<Mutex<_>>`, stringly-typed errors,
   `unsafe` without `// SAFETY:` comments.
4. **Architecture**: crate/module boundaries, dependency direction, feature-flag additivity, config
   & observability setup vs the `rust-architecture` skill.

Output a prioritised table: severity · location · finding · rule id · suggested fix. Then the top 3
changes with the best effort/impact ratio.
