# Authoring curated knowledge

The curated corpus is the part of rustkb written by people (or agents) rather than
ingested from upstream. It lives in `skills/<skill>/references/*.md` so that:

- **Claude Code** reads it directly through skills (progressive disclosure: `SKILL.md` → reference file), and
- **every other agent** reaches the same text through the `rustkb` MCP server (the indexer walks these files).

One source of truth, two access paths. Anything that can be ingested mechanically
(API docs, lint docs, advisories, release notes) must NOT be duplicated here — link to it
or let the MCP tools surface it.

## Skills

| Skill | Scope |
|---|---|
| `rust-idioms` | Language, std, idioms: ownership, errors, traits/generics, API design, async, concurrency, unsafe/FFI, performance, testing, macros, editions/MSRV, lints |
| `rust-architecture` | Project & system design: workspaces, crate boundaries, layering/hexagonal, config, observability, services, CLIs, libraries & semver, feature flags, CI/CD, release, templates |
| `rust-ecosystem` | Which crate for what, with versions: the crate catalog (`catalog.toml`) + per-domain guides |
| `rust-security` | Supply chain, unsafe review, secure coding, crypto choices, fuzzing/Miri, advisories |

## Reference file format

Every `references/*.md` file starts with YAML frontmatter:

```yaml
---
id: idioms/error-handling            # stable, unique, "<area>/<slug>"; never change once published
title: Error handling
summary: >-                          # 1–2 sentences: what decisions this doc makes
  Typed errors with thiserror in libraries, anyhow/eyre at application edges; when to panic.
area: idioms                         # idioms | architecture | ecosystem | security
tags: [errors, thiserror, anyhow, panic, result]
rust: "1.96"                         # stable Rust the content was verified against
edition: "2024"
crates:                              # every crate the doc recommends → major.minor verified against
  thiserror: "2.0"
  anyhow: "1.0"
verified: 2026-09-29                 # date content was last checked against reality
sources:
  - https://rust-lang.github.io/api-guidelines/
---
```

`rustkb stale` uses `crates`, `rust` and `verified` to flag docs whose guidance may have
drifted: a crate had a semver-incompatible release, Rust moved many minors ahead, or the
doc hasn't been re-verified in 180 days. **List every recommended crate in `crates`** or
drift detection can't see it.

## Body rules

1. **Decision first.** Open each section with the default, then the exceptions:
   "Default: X. Use Y when Z. Never W." Agents need decisions, not surveys.
2. **H2 (`##`) sections are retrieval chunks.** Each must make sense on its own (the MCP
   server returns single sections). Don't write "as mentioned above".
3. **Rules get stable IDs** so agents and reviewers can cite them:
   `### ERR-01: Libraries expose typed, matchable errors`. Prefix per file (ERR, OWN, TRAIT,
   API, ASYNC, CONC, UNSAFE, PERF, TEST, ARCH, WS, CFG, OBS, SEC, SUP, …). Never renumber;
   retire rules by marking them `(retired)`.
4. **Show, briefly.** One minimal ✅ example beats three paragraphs. Show ❌ only when the
   anti-pattern is common in LLM-written Rust.
5. **Examples must compile** on the `rust` version in frontmatter (use `edition = "2024"`).
   Check non-trivial ones with `cargo check`/`cargo clippy` in a scratch project.
6. **Target LLM failure modes.** Prioritise what coding agents actually get wrong:
   outdated crates/APIs (e.g. `lazy_static`, `structopt`, `failure`), `.unwrap()` everywhere,
   `Arc<Mutex<_>>` by reflex, needless `clone()`, `Box<dyn Error>` in library APIs, blocking
   in async, holding locks across `.await`, stringly-typed errors, over-generic code.
7. **No version claims from memory.** Check crates.io
   (`https://crates.io/api/v1/crates/<name>` → `crate.max_stable_version`) before writing a version.
8. **Length.** 150–450 lines per reference file. Split rather than grow.

## SKILL.md format

```yaml
---
name: rust-idioms
description: <what it covers + when to use it; this is what triggers the skill — be specific>
---
```

Body: ≤150 lines. A short "core rules" list (the 10–20 most important rule IDs, one line
each) followed by a routing table: *situation → reference file to read*. Mention that the
`rustkb` MCP tools (`search`, `get_item`, `crate_info`, `check_advisories`, `explain_lint`,
`rust_release`) provide versioned API docs, advisories and lint docs when available.

## Crate catalog (`skills/rust-ecosystem/catalog.toml`)

Structured, machine-read. Drives `recommend_crates`, drift detection and which crates get
rustdoc ingested.

```toml
[[crate]]
name = "tokio"
category = "async-runtime"         # see categories in catalog header
tier = "default"                   # default | recommended | situational | avoid
version = "1.53"                   # major.minor verified against crates.io
summary = "Multi-threaded async runtime; the ecosystem default."
use_for = "Any async application: servers, clients, CLIs doing concurrent IO."
avoid_when = "Pure CPU-bound work (use rayon) or embedded/no_std (use embassy)."
alternatives = ["smol", "embassy"]
replaces = []                      # crates this one supersedes, e.g. ["async-std"]
track_docs = true                  # ingest docs.rs API docs for this crate
```

When `rustkb stale` reports "no release since …" for a crate that is in fact maintained or
finished, record the check in the top-level `[maintained_checked]` table at the end of the file
(`walkdir = "2026-09-30"  # evidence`). The finding stays quiet for 180 days after that date.
It is a separate table, not an entry field, because older `rustkb` binaries reject unknown entry
fields but ignore unknown top-level tables. New entry fields break them the same way, so add
catalog data as new top-level tables.

`tier = "avoid"` entries document crates agents still reach for but shouldn't
(`lazy_static`, `structopt`, `failure`, `async-std`, `serde_yaml`…) with `replaces`/
`alternatives` pointing to the modern choice.
