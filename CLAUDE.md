# rustkb — working in this repo

This repo is both a Claude Code plugin (skills/agents/commands) and a Rust workspace (the
`rustkb` CLI + MCP server). See README.md for the architecture.

## Curated knowledge (skills/)
- `docs/AUTHORING.md` is the binding spec. Frontmatter is machine-checked; rule ids are stable
  public identifiers — never renumber, retire instead.
- Never write crate versions, APIs or stabilisation versions from memory: use `rustkb crate <name>`,
  `rustkb item <crate> <path>`, `rustkb release --since <ver>` (or the MCP tools).
- Every change: `cargo run -q -p rustkb -- validate` must pass. Compile non-trivial examples in a
  scratch crate outside the repo.
- Drift work: `cargo run -q -p rustkb -- stale` then follow `agents/rustkb-curator.md`.

## Rust code (crates/)
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` (pedantic is on),
  `cargo test --workspace`. Also check `--features rustkb/semantic` builds.
- stdout of `rustkb serve` is the MCP transport — log only via `tracing` (stderr).
- The index is shared by concurrent processes (one MCP server per session + CLI): keep writers
  short-lived (`Index::write`), rebuild via new generations, never delete a live index dir.
- rustdoc JSON's format is unstable: parse loosely from `serde_json::Value` (see `rustdoc.rs`).
- Be polite upstream: crates.io ≤ 1 req/s, cache everything with TTLs, descriptive User-Agent
  without personal data.
