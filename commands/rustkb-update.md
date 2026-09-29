---
description: Refresh the rustkb knowledge base (upstream docs, advisories, releases) and fix drifted curated guidance
argument-hint: [--offline]
---

Refresh rustkb and bring the curated guidance up to date.

1. Run `rustkb ingest` (refreshes clippy lints, RustSec/OSV advisories, Rust release notes and
   docs.rs API docs for tracked crates, then rebuilds the index). Report any failed sources.
2. Run `rustkb stale --format markdown` $ARGUMENTS and show me the findings table.
3. If I'm working in the rustkb repository (it has `docs/AUTHORING.md`), delegate the High and
   Medium findings to the `rustkb-curator` agent, then show me the resulting diff summary.
   Otherwise, just report the findings — the curated corpus is updated upstream; tell me to update
   the plugin (`/plugin update rustkb`).
