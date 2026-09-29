# rustkb — a Rust knowledge base for coding agents

rustkb gives AI coding agents what they lack when writing Rust: **current, versioned,
opinionated knowledge** of how to design, architect and build best-practice Rust projects, and a
way to **keep that knowledge current** as Rust and its ecosystem move.

It ships as:

- a **Claude Code plugin**: 4 skills, 2 agents, 4 commands and a session hook;
- an **MCP server** (`rustkb serve`) that any MCP-capable agent can use (Claude Code, Cursor, Codex, Zed, …);
- a **CLI** (`rustkb`) for humans and CI.

```
                ┌──────────────────────── curated (in git, reviewed) ────────────────────────┐
                │ skills/rust-idioms        skills/rust-architecture                          │
                │ skills/rust-ecosystem     skills/rust-security      catalog.toml (crates)   │
                │   SKILL.md → references/*.md  (YAML frontmatter: rust, crates, verified)    │
                └───────────────┬─────────────────────────────────────────┬──────────────────┘
      Claude Code reads skills  │                                         │ indexed by section
      directly (progressive     │                                         ▼
      disclosure)               │      ┌───────────── rustkb (one Rust binary) ─────────────┐
                                │      │ ingest:  docs.rs rustdoc JSON · clippy lint docs     │
                                │      │          RustSec/OSV advisories · RELEASES.md         │
                                │      │          crates.io metadata                           │
                                │      │ index:   tantivy BM25 (code-aware tokenizer)          │
                                │      │          + exact-path lookup + optional model2vec     │
                                │      │          embeddings, fused with RRF                   │
                                │      │ stale:   drift detection for curated guidance         │
                                │      │ serve:   MCP over stdio                               │
                                │      └───────────────────────┬──────────────────────────────┘
                                ▼                              ▼
                        Claude Code agent  ◄──── MCP tools ────► any MCP agent
```

## Why not only a vector database?

Rust questions are mostly about **exact identifiers** (`tokio::sync::Mutex`, `#[non_exhaustive]`,
`clippy::unwrap_used`, `ERR-03`), where lexical search with a code-aware tokenizer beats
embeddings. Most of the value is in **structured, versioned** data: which version, which crate
replaces which, which advisory applies. rustkb therefore combines:

1. **BM25** over everything, weighted so curated guidance wins conceptual queries and API items win
   identifier queries;
2. **exact structured lookups**: item path → signature/docs/members, crate → versions, lint → docs;
3. **optional semantic retrieval** (`--features semantic`) with a static
   [model2vec](https://github.com/MinishLab/model2vec-rs) model. This is pure Rust: no ONNX, no GPU,
   no service. It is fused with BM25 by reciprocal rank fusion.

Everything runs locally in one binary. There is no database server to operate.

## The MCP tools

| Tool | What it answers |
|---|---|
| `search` | Guidance, catalog, API docs, lints, advisories, release notes. Returns ranked ids and snippets. Filters by source and crate. |
| `get_doc` | Full text of any doc id, or a whole curated guide (`idioms/error-handling`). |
| `get_item` | **Version-accurate API docs** for a crate item (signature, docs, members). Fetched from docs.rs rustdoc JSON on demand and cached. |
| `crate_info` | Live crates.io facts (latest version, MSRV, license, last release, downloads), the catalog tier, replacements and advisories. |
| `recommend_crates` | "Which crate for X", with tiers, *use for / avoid when* notes, and crates to avoid. |
| `check_advisories` | RustSec/OSV advisories for `crate@version` (live) or a whole `Cargo.lock`. |
| `explain_lint` | Clippy lint docs and how to enable the lint. |
| `rust_release` | Release notes for one version, or highlights of everything *since* a version. This lets an agent catch up past its training cutoff. |
| `kb_status` | Index contents, freshness and corpus health. |

## Install

### Claude Code plugin

```sh
# 1. Add the marketplace and install the plugin
claude plugin marketplace add JSBtechnologies/rustkb
claude plugin install rustkb@rustkb

# 2. Install the binary (the skills work without it; the MCP tools need it)
cargo install --git https://github.com/JSBtechnologies/rustkb rustkb --locked   # add --features semantic for embeddings
rustkb --root <plugin root> ingest                                            # first data pull (a few minutes)
```

Or run `/rustkb-setup` inside Claude Code, which does step 2 for you. The plugin registers the
MCP server with `RUSTKB_ROOT=${CLAUDE_PLUGIN_ROOT}` and `RUSTKB_HOME=${CLAUDE_PLUGIN_DATA}`.

Plugin contents:

- **Skills** (load automatically when relevant): `rust-idioms`, `rust-architecture`, `rust-ecosystem`, `rust-security`.
- **Agents**:
  - `rust-architect` designs projects and features, citing the rules each decision follows.
  - `rustkb-curator` keeps the curated corpus true.
- **Commands**:
  - `/rust-new <idea>` designs and scaffolds a project.
  - `/rust-audit` audits a project against the knowledge base.
  - `/rustkb-update` refreshes the data and fixes drift.
  - `/rustkb-setup` installs the binary.

### Other agents (MCP)

```jsonc
// Cursor: .cursor/mcp.json · Claude Desktop: claude_desktop_config.json · most MCP clients use this shape
{
  "mcpServers": {
    "rustkb": { "command": "rustkb", "args": ["serve"], "env": { "RUSTKB_ROOT": "/path/to/rustkb" } }
  }
}
```

```toml
# Codex: ~/.codex/config.toml
[mcp_servers.rustkb]
command = "rustkb"
args = ["serve"]
env = { RUSTKB_ROOT = "/path/to/rustkb" }
```

The server's `instructions` teach the agent the workflow: search guidance first, verify crates
before adding them, look up exact APIs, and audit dependencies.

## CLI

```sh
rustkb ingest [--only clippy,advisories,releases,rustdoc] [--crate name[@ver]] [--force]
rustkb index                      # rebuild the index offline
rustkb search "holding a mutex across await" [-s curated,rustdoc] [--crate tokio]
rustkb doc idioms/error-handling  # whole curated guide
rustkb item tokio sync::Mutex::lock [--version 1.47.1]
rustkb crate reqwest
rustkb recommend "parse yaml config"
rustkb audit time@0.1.43 | rustkb audit --lockfile Cargo.lock
rustkb lint needless_collect
rustkb release --since 1.85
rustkb stale [--format markdown|json] [--offline] [--strict]
rustkb validate                   # CI: frontmatter, catalog and skill headers
rustkb status
```

Environment variables:

| Variable | Purpose |
|---|---|
| `RUSTKB_ROOT` | Knowledge root. Otherwise auto-discovered. |
| `RUSTKB_HOME` | Data directory. Defaults to the platform data dir. |
| `RUSTKB_LOG` | Tracing filter. |
| `RUSTKB_EMBED_MODEL` | Any model2vec model. Defaults to `minishlab/potion-base-8M`. |

## How it stays current

Two kinds of knowledge age differently, so they are updated differently.

**Mechanical knowledge updates itself.** This covers API docs, lints, advisories, release notes
and crate versions. `rustkb ingest` refreshes them from upstream with daily-TTL caches.
`get_item` fetches any crate version on demand. `check_advisories` queries OSV live. Nothing here
is hand-maintained.

**Curated knowledge is versioned and drift-checked.** Every reference file records what it was
verified against:

```yaml
rust: "1.96"
crates: { thiserror: "2.0", anyhow: "1.0" }
verified: 2026-09-29
```

`rustkb stale` compares that against reality and reports:

| Finding | Severity |
|---|---|
| A recommended crate had a semver-incompatible release | High |
| A recommended crate is flagged unmaintained or unsound by RustSec | High |
| The doc is 6 or more Rust releases behind | Medium |
| A catalog crate looks abandoned, or its newest release is yanked | Medium |
| The doc hasn't been re-verified in 180 days | Low |

**The loop is automated.**
[`.github/workflows/refresh.yml`](.github/workflows/refresh.yml) runs weekly: it ingests, runs
`stale`, and files or updates a *Knowledge drift report* issue. If you opt in (an
`ANTHROPIC_API_KEY` secret plus the `RUSTKB_AUTO_CURATE` variable or a manual trigger), it also
runs Claude with the `rustkb-curator` agent. The curator researches each finding, edits the
guidance, re-verifies the examples, bumps the frontmatter, and opens a PR for human review.

Rule ids (`ERR-01`, `WS-03`, …) are stable, so agents and reviews can cite them across versions.

## Repository layout

```
.claude-plugin/        plugin.json (declares the MCP server), marketplace.json
skills/<skill>/        SKILL.md + references/*.md; rust-ecosystem/catalog.toml
agents/  commands/  hooks/
docs/AUTHORING.md      the spec for curated knowledge (frontmatter, rule ids, style)
crates/
  rustkb-core          Doc model, frontmatter, catalog, paths
  rustkb-index         tantivy index (generational, multi-process safe) + model2vec embeddings
  rustkb-ingest        docs.rs rustdoc JSON, clippy, OSV, RELEASES.md, crates.io, drift checks
  rustkb               CLI + MCP server (rmcp)
```

## Contributing knowledge

Read [`docs/AUTHORING.md`](docs/AUTHORING.md). In short: decision-first sections, stable rule ids,
compiling examples, and no version claims from memory. `rustkb validate` must pass.

## License

[MIT](LICENSE) © Jeffrie Budde
