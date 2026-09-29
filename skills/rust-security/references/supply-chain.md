---
id: security/supply-chain
title: Supply-chain security
summary: >-
  How a Rust project vets, pins and monitors its dependencies: cargo-deny as the CI gate
  (advisories, licenses, bans, sources) with a working deny.toml, lockfile policy, the
  checklist for adding a crate, build-script/proc-macro risk, cargo-vet for high assurance,
  auditable binaries, and trusted publishing to crates.io.
area: security
tags: [supply-chain, cargo-deny, cargo-audit, cargo-vet, cargo-auditable, lockfile, typosquatting, build-rs, proc-macro, trusted-publishing, sbom]
rust: "1.96"
edition: "2024"
crates:
  cargo-deny: "0.20"
  cargo-audit: "0.22"
  cargo-vet: "0.10"
  cargo-auditable: "0.7"
  cargo-cyclonedx: "0.5"
  cargo-machete: "0.9"
verified: 2026-09-29
sources:
  - https://embarkstudios.github.io/cargo-deny/
  - https://github.com/rustsec/rustsec/tree/main/cargo-audit
  - https://mozilla.github.io/cargo-vet/
  - https://github.com/rust-secure-code/cargo-auditable
  - https://crates.io/docs/trusted-publishing
  - https://doc.rust-lang.org/cargo/reference/resolver.html
---

# Supply-chain security

Every dependency is code you ship and, via `build.rs` and proc macros, code that runs on
your build machine with your credentials. The defaults below make that risk visible and
gated in CI instead of discovered in production.

## Default tooling

Default: **cargo-deny** in CI on every PR and on a daily schedule. It covers RustSec
advisories (vulnerable, unmaintained, unsound, yanked), licenses, banned crates, duplicate
versions and allowed sources in one tool with one config file.

| Need | Tool | When |
|---|---|---|
| Advisories + licenses + bans + sources gate | `cargo deny check` | Always (default) |
| Advisories only, or scanning a shipped binary | `cargo audit` / `cargo audit bin` | Quick local check; auditing binaries built with cargo-auditable |
| Human review record for every dependency | `cargo vet` | High-assurance projects (browsers, crypto, infra, regulated) |
| Dependency list embedded in the binary | `cargo auditable build` | Anything you ship as a binary/container |
| SBOM file (CycloneDX) | `cargo cyclonedx` | Customers/compliance ask for an SBOM |
| Unused dependencies (attack surface) | `cargo machete` | Periodically; before releases |

Install CLI tools with `cargo install --locked <tool>` (or a prebuilt-binary installer such
as `taiki-e/install-action` in CI). Without `--locked`, cargo re-resolves the tool's
dependencies to the newest semver-compatible versions — untested, and a supply-chain
exposure of its own.

For live advisory lookups while coding, use the rustkb MCP `check_advisories` tool
(RustSec/OSV) instead of recalling advisory IDs from memory.

## SUP-01: Gate CI on cargo-deny with a checked-in deny.toml

Default: commit this `deny.toml` at the workspace root and run `cargo deny check` in CI.
Validated with cargo-deny 0.20 (`advisories ok, bans ok, licenses ok, sources ok` on an
axum + sqlx + rustls + RustCrypto graph).

```toml
# deny.toml — cargo-deny 0.20. Run: cargo deny check
[graph]
# Only resolve the platforms you ship; avoids noise from e.g. windows-* crates on Linux-only services.
targets = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
]
all-features = true

[advisories]
# Vulnerabilities always fail. These control the other advisory kinds.
yanked = "deny"
unmaintained = "workspace"     # fail only when a *direct* dependency is unmaintained
unsound = "all"
ignore = [
    # Every ignore needs a reason and an owner; revisit on each release.
    # { id = "RUSTSEC-0000-0000", reason = "not reachable: we never call X (alice, 2026-09)" },
]

[licenses]
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "Zlib",
    "CDLA-Permissive-2.0",   # webpki-root-certs / webpki-roots
]
confidence-threshold = 0.9
unused-allowed-license = "allow"

[licenses.private]
ignore = true                 # don't license-check unpublished workspace crates

[bans]
multiple-versions = "warn"
wildcards = "deny"            # no `version = "*"` in any manifest
allow-wildcard-paths = true   # ...but allow path deps without versions in private workspaces
highlight = "all"
deny = [
    { crate = "openssl", use-instead = "rustls" },
    { crate = "openssl-sys", use-instead = "rustls" },
    { crate = "serde_yaml", use-instead = "serde_norway or toml" },
    { crate = "serde_yml", reason = "unmaintained; use serde_norway or toml" },
]

# Scan crates that run at build time (build.rs, proc-macros) for shipped binaries/scripts.
[bans.build]
executables = "deny"
include-archives = true        # count prebuilt .a/.lib as native code too
bypass = [
    # Import libraries shipped by windows-targets; reviewed, expected.
    { crate = "windows_x86_64_msvc", allow-globs = ["lib/*.lib"] },
    { crate = "windows_x86_64_gnu", allow-globs = ["lib/*.a"] },
]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
required-git-spec = "rev"     # git deps must pin a commit
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
allow-git = []
```

Notes on the choices:

- `unmaintained = "workspace"`: an unmaintained *transitive* crate is usually not yours to
  replace; an unmaintained *direct* dependency is. Use `"all"` for high-assurance projects.
- `wildcards = "deny"` also rejects dependencies without a version requirement; the
  `allow-wildcard-paths` escape hatch applies only to private (`publish = false`) crates.
- The `openssl` ban encodes a project decision (rustls everywhere). Remove it if you
  genuinely need OpenSSL (FIPS on a platform aws-lc-rs doesn't cover, HSM engines).
- Add a `bans.build.bypass` entry only after looking at what the reported file is.
- Adjust the license `allow` list with legal input; never add a license just to turn CI green.

GitHub Actions:

```yaml
cargo-deny:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v7
    - uses: EmbarkStudios/cargo-deny-action@v2
```

Also run the advisories check on a daily `schedule:` trigger — new advisories are published
against code that hasn't changed.

## SUP-02: Vulnerability ignores need a reason, an owner and a re-check date

Default: fix by upgrading (`cargo update -p <crate>`), or by upgrading the parent that pins
it. Ignore only when you have *verified* the vulnerable code path is unreachable, and write
why in the `reason` field.

Never silence an advisory by deleting `Cargo.lock`, by adding a blanket `ignore` without a
reason, or by setting `[advisories]` levels to `allow`. See
[incident-and-advisories.md](incident-and-advisories.md) for the triage procedure.

## SUP-03: Commit Cargo.lock and build with --locked

Default: commit `Cargo.lock` for every package — binaries *and* libraries (Cargo's guidance
since 2023). CI builds and tests use `--locked`, so CI fails instead of silently resolving
a different graph than the one that was reviewed.

```sh
cargo build --release --locked
cargo test --locked
```

- Update dependencies deliberately (Dependabot/Renovate PRs, or a scheduled
  `cargo update` PR) so every lockfile change goes through review and cargo-deny.
- `--frozen` additionally forbids network access; use it in hermetic/offline builds.
- Libraries: the lockfile doesn't affect downstream users, so also test against the latest
  compatible versions in a scheduled job (drop `--locked` there) to catch breakage early.

## SUP-04: Vet a crate before adding it

LLM agents invent plausible crate names and pick the first search hit. Before
`cargo add <name>`:

1. **Exact name.** Confirm it on crates.io. crates.io blocks names differing only by `-`/`_`
   or case, but not a swapped letter, an extra suffix or a plural (`serde_jsonn`,
   `tokio-rs`, `rust-crypto` — the last is a real, long-abandoned crate agents still suggest).
2. **Provenance.** `repository` field points to a real repo whose code matches the published
   crate; owners are known people/orgs (`cargo owner --list <crate>`).
3. **Health.** Recent releases or a clearly "done" small crate; no RustSec *unmaintained*
   advisory (`check_advisories`); reverse-dependency and download counts consistent with
   its claimed popularity.
4. **Weight.** How many new transitive dependencies does it pull (`cargo tree -e normal -i
   <crate>` after adding)? Does it have `build.rs`, proc macros, `-sys` native code?
5. **Prefer the ecosystem default** listed in the rust-ecosystem catalog over a lookalike.

Never copy a dependency line from an old blog post or model memory without checking the
current major version on crates.io.

## SUP-05: Enable only the features you use

Default: `default-features = false` for large crates, then add back what you need. Every
feature is more code, more transitive crates, and sometimes more `unsafe` or native code.

```toml
[dependencies]
reqwest = { version = "0.13", default-features = false, features = ["rustls", "json"] }
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "net"] }  # not "full"
```

Check what actually got enabled with `cargo tree -e features -i <crate>`. Use
`[[bans.features]]` in deny.toml to forbid features you've decided against (e.g. a
`native-tls` feature when the project standard is rustls).

## SUP-06: Pin git dependencies to a commit; prefer published crates

Default: depend on crates.io releases. A git dependency is acceptable only temporarily
(unreleased fix) and must pin `rev`:

```toml
# ❌ tracks whatever the default branch is at the next `cargo update`
foo = { git = "https://github.com/org/foo" }
# ✅ immutable
foo = { git = "https://github.com/org/foo", rev = "3f2c1a9e0b7d4c6a8e5f1b2d3c4e5f6a7b8c9d0e" }
```

`required-git-spec = "rev"` plus `unknown-git = "deny"` in deny.toml enforces this and
forces each git source to be listed in `allow-git`. Prefer `[patch.crates-io]` over
rewriting dependency lines, so removing the patch later is a one-line change.

## SUP-07: Treat build scripts and proc macros as code execution

`build.rs` and proc macros run with the invoking user's full privileges at `cargo build`,
`cargo check` and — through rust-analyzer — when an IDE opens the project. There is no
sandbox.

- Don't build or open untrusted repositories on machines holding credentials (SSH keys,
  cloud tokens, `~/.cargo/credentials.toml`). Use a container or throwaway VM.
- In CI, give build/test jobs no secrets; the publish/deploy job runs separately with
  minimal permissions.
- Count crates with build scripts or proc macros when vetting (SUP-04); prefer
  alternatives without them when otherwise equal.
- `[bans.build]` in cargo-deny flags prebuilt executables and archives in build-time
  crates. It does **not** detect malicious Rust code — only review (cargo-vet) does.

Your own `build.rs`: no network access, no writing outside `OUT_DIR`, and emit
`cargo::rerun-if-changed` so it doesn't re-run on every build.

## SUP-08: Use cargo-vet when you need a review record

Default for most projects: cargo-deny is enough. Adopt **cargo-vet** when you must show
that every third-party crate version was reviewed by someone you trust.

```sh
cargo vet init            # creates supply-chain/{config.toml,audits.toml,imports.lock}
cargo vet                 # fails listing unaudited crates
cargo vet suggest         # cheapest audits to do next (diffs from already-audited versions)
cargo vet certify <crate> <version>   # record your audit
```

Import audits from organisations that publish them (e.g. Mozilla, Google, Bytecode
Alliance) in `supply-chain/config.toml` to avoid auditing the world yourself; use
`safe-to-deploy` for runtime deps and `safe-to-run` for dev/build deps.

cargo-crev is the decentralised alternative (web-of-trust review proofs); its review
coverage is thin for most crates, so prefer cargo-vet for team policy.

## SUP-09: Ship auditable binaries

Default: build release binaries with cargo-auditable. It embeds the resolved dependency
list (a few KB) so scanners can audit the binary after deployment, long after the lockfile
is gone.

```sh
cargo install --locked cargo-auditable cargo-audit
cargo auditable build --release --locked
cargo audit bin target/release/my-service   # also understood by Trivy, Grype, Syft, osv-scanner
```

Need a standalone SBOM document? `cargo cyclonedx` writes CycloneDX files from the
manifest/lockfile.

## SUP-10: Publish with crates.io Trusted Publishing, not long-lived tokens

Default for crates published from CI: **Trusted Publishing** (OIDC). The CI job exchanges a
short-lived identity token for a crates.io token that expires after 30 minutes; there's no
`CARGO_REGISTRY_TOKEN` secret to leak. Supported on GitHub Actions; GitLab CI/CD in beta.
The first release of a crate still needs a normal API token.

```yaml
# .github/workflows/release.yml — configure the matching publisher in crate Settings → Trusted Publishing
on:
  push:
    tags: ['v*']
jobs:
  publish:
    runs-on: ubuntu-latest
    environment: release          # protect with required reviewers
    permissions:
      id-token: write
      contents: read
    steps:
      - uses: actions/checkout@v7
      - uses: rust-lang/crates-io-auth-action@v1
        id: auth
      - run: cargo publish --locked
        env:
          CARGO_REGISTRY_TOKEN: ${{ steps.auth.outputs.token }}
```

If you must use API tokens: scope them (`publish-update` for specific crates), give them an
expiry, keep them out of build jobs, and enable 2FA on the crates.io/GitHub accounts of all
owners. Pin third-party actions in release workflows to a commit SHA.

## SUP-11: Keep the dependency graph small and current

- Remove unused dependencies (`cargo machete`); each one is attack surface and advisory noise.
- `multiple-versions = "warn"` surfaces duplicate major versions; fix the ones that are
  security-relevant (two TLS stacks, two `rand_core`s feeding crypto).
- Prefer std where it now covers the need (`std::sync::LazyLock` over `lazy_static`/
  `once_cell`, `std::io::IsTerminal` over `atty`) — the old crates carry unmaintained
  advisories (`atty`: RUSTSEC-2024-0375).
- Replace crates flagged unmaintained before they get a vulnerability nobody will fix.
