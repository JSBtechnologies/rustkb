---
id: security/incident-and-advisories
title: Responding to advisories and security incidents
summary: >-
  What to do when cargo-deny/cargo-audit reports a RustSec advisory (triage by kind,
  reachability, upgrade paths, justified ignores), how yanking works, where advisories
  come from (RustSec, OSV, GHSA), how to report a vulnerability in someone else's crate,
  how to handle and publish one in your own, and how to respond to a malicious crate.
area: security
tags: [advisories, rustsec, osv, ghsa, cve, cargo-audit, cargo-deny, yank, disclosure, incident-response, malicious-crate, unmaintained, unsound]
rust: "1.96"
edition: "2024"
crates:
  cargo-deny: "0.20"
  cargo-audit: "0.22"
  cargo-auditable: "0.7"
  rustsec: "0.33"
verified: 2026-09-29
sources:
  - https://rustsec.org/
  - https://github.com/rustsec/advisory-db/blob/main/CONTRIBUTING.md
  - https://osv.dev/list?ecosystem=crates.io
  - https://doc.rust-lang.org/cargo/commands/cargo-yank.html
  - https://www.rust-lang.org/policies/security
  - https://docs.github.com/en/code-security/security-advisories
---

# Responding to advisories and security incidents

Advisory data changes daily; never rely on memory for "is crate X affected". Query the
rustkb MCP `check_advisories` tool (RustSec + OSV) or run `cargo deny check advisories`.

## ADV-01: Triage an advisory by kind

| Kind (RustSec `informational`) | Meaning | Response |
|---|---|---|
| *(none)* — vulnerability | Exploitable bug with a CVE/GHSA-style impact | Fix now: upgrade, or mitigate and document (ADV-02) |
| `unsound` | Safe API can trigger UB; exploitability unknown | Fix soon; treat as a vulnerability if the API is reachable with untrusted input |
| `unmaintained` | No one will fix future bugs | Plan a replacement (the advisory usually names one); no emergency |
| `notice` | Informational (e.g. deprecation) | Read, decide |
| Category `malicious` | The crate itself is malware; crates.io has removed it | Incident: ADV-08 |
| yanked (not an advisory) | Author withdrew that version | Upgrade (ADV-04) |

Severity is contextual: read the advisory's `affected.functions` and description, and
check whether your code path reaches them. "The CVSS says medium" is not a triage.

## ADV-02: Resolve a vulnerable dependency

1. **Find who pulls it in**: `cargo tree -i <crate>@<version> -e normal,build` (also shows
   whether it's only a dev/build dependency).
2. **Compatible fix available** → `cargo update -p <crate>` (or
   `cargo update -p <crate> --precise <patched>`), commit the lockfile change, re-run
   `cargo deny check`.
3. **Parent pins an old major** → upgrade the parent; if the parent has no release, open an
   issue/PR upstream, and as a stopgap use `[patch.crates-io]` pointing at a fixed fork
   pinned by `rev` (SUP-06).
4. **No fix exists** → mitigate (disable the feature, avoid the affected function, add
   input limits), or replace the crate. Only if the affected code is provably unreachable,
   ignore it with a written justification:

```toml
[advisories]
ignore = [
    { id = "RUSTSEC-2023-0071", reason = "rsa used only for signature *verification* of public data; Marvin affects private-key ops. Owner: @alice. Re-check 2026-12." },
]
```

Keep `unused-ignored-advisory = "warn"` (the default) so stale ignores get noticed, and
review every ignore at each release.

Never "fix" an advisory by: deleting `Cargo.lock`, pinning to a vulnerable version with
`=`, downgrading the scanner's severity levels, or removing the CI job.

## ADV-03: Check deployed binaries, not just the repository

The lockfile on `main` says nothing about what's running. For a new advisory, answer "which
deployed artifacts contain the affected version?":

- binaries built with cargo-auditable: `cargo audit bin <binary>`, or scan container
  images with Trivy/Grype/osv-scanner (they read the embedded dependency list);
- otherwise, the `Cargo.lock` at each release tag.

Then rebuild and redeploy every affected artifact — merging the fix is not the end.

## ADV-04: Yanked versions: what yanking does and doesn't do

- `cargo yank --version 1.2.3 <crate>` prevents **new** lockfiles from selecting 1.2.3.
  Existing `Cargo.lock` files keep using it, and it remains downloadable. It is not a
  deletion and not a security fix by itself.
- Consumers: cargo-deny `yanked = "deny"` (supply-chain.md) flags yanked versions in
  your lockfile; move off with `cargo update -p <crate>`.
- Maintainers: yank *in addition to* publishing a fixed version and an advisory, not
  instead. Yanking every old version breaks users who can't upgrade yet; yank only what is
  broken or dangerous.

## ADV-05: Where advisories live

| Source | Covers | Consumed by |
|---|---|---|
| **RustSec advisory-db** (`RUSTSEC-YYYY-NNNN`) | crates.io crates + `rust/std`, `rust/cargo`, `rust/rustdoc` | cargo-deny, cargo-audit, `rustsec` crate |
| **OSV** (osv.dev, ecosystem `crates.io`) | RustSec exported in OSV format, plus GHSA | osv-scanner, rustkb `check_advisories` |
| **GitHub Advisory Database** (GHSA) | Imports RustSec; maintainers publish GHSAs | Dependabot alerts |
| Rust blog / security announcements list | rustc, std, cargo, crates.io incidents | Humans — subscribe |

Toolchain vulnerabilities (e.g. CVE-2024-24576 in `std::process::Command` on Windows) are
fixed by updating Rust, not a crate. Keep the pinned toolchain (`rust-toolchain.toml`)
current and don't pin an old patch release indefinitely.

## ADV-06: Report a vulnerability in someone else's crate

1. Report **privately** first: the repo's `SECURITY.md`, GitHub "Report a vulnerability"
   (private vulnerability reporting), or the maintainers' email. Never a public issue or
   PR that reveals the bug before a fix.
2. Include a minimal reproducer, affected versions, and impact. A Miri trace or fuzz
   crash input is ideal.
3. Agree on a disclosure timeline (commonly 90 days). If the maintainer is unreachable,
   RustSec can publish an `unmaintained`/vulnerability advisory.
4. Vulnerabilities in the Rust toolchain, std, crates.io or docs.rs go to
   `security@rust-lang.org` (see the Rust security policy), not RustSec.

## ADV-07: Handle a vulnerability in your own crate

1. Enable private vulnerability reporting and add a `SECURITY.md` *before* you need them.
2. Develop the fix in a GitHub security advisory's temporary private fork (or privately);
   request a CVE through the GHSA if appropriate.
3. Release patched versions for **every supported major/minor line** (backport), then
   publish the advisory.
4. File a RustSec advisory: PR to `rustsec/advisory-db` adding
   `crates/<crate>/RUSTSEC-0000-0000.md` (the ID is assigned on merge):

````markdown
```toml
[advisory]
id = "RUSTSEC-0000-0000"
package = "mycrate"
date = "2026-09-29"
url = "https://github.com/me/mycrate/security/advisories/GHSA-xxxx-xxxx-xxxx"
categories = ["denial-of-service"]
keywords = ["parser", "stack-overflow"]
aliases = ["GHSA-xxxx-xxxx-xxxx"]

[affected.functions]
"mycrate::parse" = [">= 1.0.0, < 1.4.2"]

[versions]
patched = [">= 1.4.2"]
```

# Stack overflow on deeply nested input in `mycrate::parse`

Describe the impact, the affected API, and the fix. Mention workarounds for users who
cannot upgrade.
````

- `affected.functions` lets scanners report only callers of the vulnerable function; fill
  it in whenever the bug is localised.
- Mark `unsound` issues with `informational = "unsound"`; they still matter to users of
  `unsafe`-free code.
- Optionally yank the vulnerable versions (ADV-04).

## ADV-08: Respond to a malicious or compromised crate

RustSec records removed malware with `categories = ["malicious"]` (e.g. RUSTSEC-2025-0147,
`evm-units`). Malicious crates typically act in `build.rs` or proc macros — at **build
time**, on developer machines and CI — so the incident scope is wider than "production".

1. Identify every lockfile, CI run and developer machine that built with the crate since
   it entered the graph (`git log -p Cargo.lock`, CI history, cargo-auditable data).
2. **Rotate every secret reachable from those environments**: CI secrets, cloud
   credentials, SSH/GPG keys, `~/.cargo/credentials.toml` crates.io tokens, `.env` files,
   browser/wallet data on dev machines.
3. Remove the dependency, rebuild from clean runners, redeploy.
4. If you published crates from an affected machine, audit their recent releases and
   crates.io owners; revoke and reissue tokens (prefer Trusted Publishing, SUP-10).
5. Report newly found malware to the Rust security team (`security@rust-lang.org`, per the
   Rust security policy) so crates.io removes it and RustSec can publish an advisory.

Prevention lives in supply-chain.md: vetting new crates (SUP-04), no secrets in build jobs
(SUP-07), cargo-vet for reviewed dependencies (SUP-08).
