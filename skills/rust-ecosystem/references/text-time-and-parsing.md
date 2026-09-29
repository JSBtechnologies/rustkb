---
id: ecosystem/text-time-and-parsing
title: Date/time, regex, Unicode, parsing and templating crates
summary: >-
  jiff vs chrono vs time for dates and time zones; regex and when fancy-regex is justified;
  winnow/nom/chumsky/pest/logos for parsing; Unicode, Markdown and template engines.
area: ecosystem
tags: [time, datetime, timezone, jiff, chrono, regex, parsing, winnow, nom, unicode, markdown, templates]
rust: "1.96"
edition: "2024"
crates:
  jiff: "0.2"
  chrono: "0.4"
  chrono-tz: "0.10"
  time: "0.3"
  humantime: "2.4"
  regex: "1.13"
  fancy-regex: "0.19"
  aho-corasick: "1.1"
  memchr: "2.8"
  bstr: "1.13"
  winnow: "1.0"
  nom: "8.0"
  chumsky: "0.13"
  pest: "2.9"
  logos: "0.16"
  unicode-segmentation: "1.13"
  unicode-width: "0.2"
  pulldown-cmark: "0.13"
  comrak: "0.55"
  minijinja: "2.24"
  askama: "0.16"
  ammonia: "4.2"
  ariadne: "0.6"
  encoding_rs: "0.8"
  lalrpop: "0.23"
  lol_html: "3.0"
  scraper: "0.27"
  tree-sitter: "0.27"
  unicase: "2.9"
  unicode-normalization: "0.1"
  miette: "7.6"
  semver: "1.0"
  url: "2.5"
  toml_edit: "0.25"
verified: 2026-09-29
sources:
  - https://docs.rs/jiff/latest/jiff/_documentation/comparison/index.html
  - https://docs.rs/regex/latest/regex/
  - https://docs.rs/winnow/latest/winnow/
  - https://github.com/rust-lang/cfg-if
---

# Date/time, regex, Unicode, parsing and templating crates

## TXT-01: Default date/time crate is jiff

Default for new code: `jiff` (0.2). It has IANA time zones built in (system tzdb on
Unix, bundled on Windows/wasm), DST-correct arithmetic, a `Span` type that distinguishes
calendar units from absolute durations, and RFC 9557 (`2026-09-29T10:00+02:00[Europe/Berlin]`)
round-tripping. It is pre-1.0 (0.2.x); its author has committed to a stable 1.0 but, as of
this writing, has not shipped it — pin `"0.2"`.

Use **`chrono`** (0.4) when:
- your public API or dependencies exchange chrono types (sqlx/diesel/sea-orm columns,
  many SDKs); converting at every boundary is worse than staying on chrono;
- you maintain an existing chrono codebase.
With chrono, named time zones need `chrono-tz` (0.10), which compiles the tz
database into your binary (tzdb updates require new crate releases).

Use **`time`** (0.3) for UTC/fixed-offset timestamps with compile-time format
descriptions (`format_description!`), e.g. log/record timestamps. It has **no** IANA time
zone support.

✅ jiff: zone-aware arithmetic that stays correct across DST changes:

```rust
use jiff::{Timestamp, ToSpan, Zoned, civil::date, tz::TimeZone};

fn jiff_demo() -> Result<(), jiff::Error> {
    let now: Timestamp = Timestamp::now();
    let _in_ny: Zoned = now.in_tz("America/New_York")?;

    // "Same wall-clock time tomorrow", across the 2026-03-08 US DST change
    let start = date(2026, 3, 7).at(9, 0, 0, 0).in_tz("America/New_York")?;
    let next = start.checked_add(1.day())?;
    assert_eq!(next.hour(), 9);

    let parsed: Zoned = "2026-09-29T10:00:00+02:00[Europe/Berlin]".parse()?;
    let _utc = parsed.with_time_zone(TimeZone::UTC);
    Ok(())
}
```

Rules regardless of crate:
- Store instants as UTC timestamps (`Timestamp`, `DateTime<Utc>`, `OffsetDateTime`); keep the
  user's IANA zone name separately when you need local-time semantics.
- Use `std::time::Instant` for measuring elapsed time, never wall-clock types.
- `std::time::Duration` for timeouts; calendar spans ("1 month") need the date library.
- Parse human durations in CLIs/config with `humantime` (2.4) or `jiff::Span`
  (`"5m 30s"` / ISO 8601 `PT5M30S`).
- `chrono::Local::now()` in servers is almost always a bug; servers run in UTC.

## TXT-02: Regular expressions — regex, compiled once

Default: `regex` (1.13). It guarantees linear-time matching (no catastrophic
backtracking), so it is safe on untrusted input. It deliberately lacks look-around and
backreferences.

✅ Compile once with `LazyLock` (std, 1.80+), not per call and not with `lazy_static`:

```rust
use std::sync::LazyLock;

use regex::Regex;

static SEMVER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+)\.(\d+)\.(\d+)$").expect("valid regex"));

fn major(v: &str) -> Option<u64> {
    SEMVER.captures(v)?.get(1)?.as_str().parse().ok()
}
```

- `fancy-regex` (0.19) only when you truly need look-around/backreferences
  and the patterns are trusted; it can backtrack exponentially.
- Many literal needles at once: `aho-corasick` (1.1). Single byte/substring
  search in hot loops: `memchr` (2.8).
- Don't use regex for things with real parsers: URLs (`url`), emails (validate by sending),
  semver (`semver`), JSON/HTML (parsers), dates (the date crate).
- `regex::bytes::Regex` for non-UTF-8 input; `bstr` (1.13) for byte strings that are
  conventionally UTF-8 (file contents, process output).

## TXT-03: Parsing — winnow for hand-written parsers

Default: `winnow` (1.0) — parser combinators in the nom lineage, now 1.0, with good
error reporting and performance (it powers `toml_edit`).

```rust
use winnow::{ModalResult, Parser, ascii::digit1, combinator::separated};

fn number(input: &mut &str) -> ModalResult<u32> {
    digit1.parse_to().parse_next(input)
}

fn csv_numbers(input: &mut &str) -> ModalResult<Vec<u32>> {
    separated(1.., number, ',').parse_next(input)
}

// csv_numbers.parse("1,2,3") == Ok(vec![1, 2, 3])
```

| Situation | Choice |
|---|---|
| Binary protocols, config/DSL parsers, existing nom code | `nom` (8.0) is fine; nom 8 changed APIs from 7 — don't write nom 7 code |
| Language front ends needing error recovery and rich diagnostics | `chumsky` (0.13) |
| Grammar-first DSL where a `.pest` file is the spec | `pest` (2.9) |
| Fast tokenizer/lexer | `logos` (0.16), feeding winnow/chumsky or a hand-written parser |
| Real programming languages, LR grammars | `lalrpop`, or a hand-written recursive-descent parser |
| Parsing Rust code | `syn` (proc macros) or `tree-sitter` |
| Well-known formats | the format crate (serde_json, toml, csv, quick-xml), never a custom parser |

For error display with source spans in any of these, render with `miette` or `ariadne`.

## TXT-04: Unicode-correct text handling

Default: remember that `str::len()` is bytes and `chars()` are scalar values, not
user-perceived characters.

- Grapheme clusters (cursor movement, truncation for display): `unicode-segmentation`
  (1.13).
- Terminal column width (CJK, emoji): `unicode-width` (0.2).
- Normalization (NFC/NFKC) before comparing user input: `unicode-normalization`.
- Case-insensitive comparisons: `str::eq_ignore_ascii_case` for ASCII protocols; `unicase`
  for Unicode keys.
- Legacy encodings (Shift-JIS, Windows-1252): `encoding_rs`.
- The whole `unic-*` family is unmaintained (RustSec advisories, October 2025) — use the
  unicode-rs crates above.

## TXT-05: Markdown and HTML

| Need | Crate |
|---|---|
| Markdown → HTML, fast, CommonMark | `pulldown-cmark` (0.13) |
| Exact GitHub-Flavored Markdown output, AST editing | `comrak` (0.55) |
| Sanitize untrusted HTML (e.g. rendered user Markdown) | `ammonia` (4.2) — always, before serving |
| Parse/scrape HTML | `scraper` (html5ever-based) |
| Rewrite streaming HTML | `lol_html` |

## TXT-06: Templates

Default: `minijinja` (2.24) for runtime-loaded templates (emails, reports, prompts,
user-editable templates): Jinja2 syntax, few dependencies, good errors, auto-escaping per
file extension.

Use `askama` (0.16) for server-rendered HTML where templates ship with the binary and
you want compile-time checking of variables and types.

`tera` (2.x) and `handlebars` are maintained; choose them only to match existing templates.
Never build HTML with `format!` over user data — escaping bugs become XSS.

## TXT-07: std has absorbed several text/macro helpers

- `cfg_select!` (stable since Rust 1.95) replaces the `cfg-if` crate for new code; `cfg-if`'s
  repository was archived in September 2026 pointing to it. Keep `cfg-if` only for MSRV < 1.95.
- `str::split_once`, `str::trim_ascii`, `char::is_ascii_*`, `str::char_indices` cover many
  cases people reach for regex for.
- `std::fmt::Write` + `write!` into a `String` avoids intermediate allocations from `format!`.
