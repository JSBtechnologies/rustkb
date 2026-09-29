---
description: Install the rustkb binary (MCP server + CLI) and build its knowledge index
---

Install and initialise the rustkb MCP server for this plugin.

1. Find the plugin root: the directory containing `.claude-plugin/plugin.json` whose `name` is
   `rustkb` (check `$CLAUDE_PLUGIN_ROOT` first; otherwise look under `~/.claude/plugins/`).
2. Check prerequisites: `cargo --version` (Rust ≥ 1.88). If missing, tell me to install Rust via
   https://rustup.rs and stop.
3. Install: `cargo install --path "<plugin root>/crates/rustkb" --locked`
   (or without the plugin checkout: `cargo install --git https://github.com/JSBtechnologies/rustkb rustkb --locked`)
   (add `--features semantic` if I want semantic search; it downloads a ~30 MB embedding model on
   first use).
4. Verify `rustkb --version` works from a new shell (the binary lands in `~/.cargo/bin`; tell me to
   add it to PATH if needed).
5. Build the index: `rustkb --root "<plugin root>" ingest` — this downloads clippy lint docs,
   RustSec/OSV advisories, Rust release notes and docs.rs API docs for the tracked crates (a few
   minutes; it is polite to docs.rs and crates.io). Show me `rustkb --root "<plugin root>" status`.
6. Tell me to restart Claude Code (or run `/mcp`) so the `rustkb` MCP server connects.
