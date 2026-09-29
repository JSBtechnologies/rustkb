---
id: architecture/modules-and-boundaries
title: Modules and boundaries
summary: >-
  How to shape the module tree inside a crate: organise by domain concept, private-by-default
  visibility with pub(crate), a re-exporting facade in lib.rs, no god-modules or utils dumping
  grounds, and a one-way dependency direction from infrastructure to domain.
area: architecture
tags: [modules, visibility, pub-crate, re-exports, facade, boundaries, dependency-direction, layout]
rust: "1.96"
edition: "2024"
crates: {}
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/reference/visibility-and-privacy.html
  - https://doc.rust-lang.org/book/ch07-00-managing-growing-projects-with-packages-crates-and-modules.html
  - https://rust-lang.github.io/api-guidelines/necessities.html
  - https://doc.rust-lang.org/rustc/lints/listing/allowed-by-default.html#unreachable-pub
---

# Modules and boundaries

The module tree is the first architecture most readers see. Crates are compile-enforced
boundaries (`workspace-layout.md`); modules are the cheaper, finer-grained ones. Get these right
before reaching for more crates or traits.

## Organising the tree

### MOD-01: Organise modules by domain concept, not by technical kind

Default: one module per concept or feature (`orders`, `inventory`, `billing`), each containing
its types, logic and errors. Technical folders (`models/`, `services/`, `controllers/`,
`helpers/`, `utils/`) are a port of MVC habits and scatter one feature across five files.

```text
❌ src/models/order.rs  src/services/order_service.rs  src/utils/money.rs  src/errors.rs
✅ src/orders.rs (+ src/orders/{pricing.rs, validation.rs} when it grows)  src/money.rs
```

Exception: a thin technical split is fine *at the edge* of a binary — `http/`, `cli/`,
`telemetry.rs`, `config.rs` — because those really are separate concerns wired in `main`.

### MOD-02: One file-layout convention: `foo.rs` + `foo/`

Default: for a module with children, use `src/foo.rs` next to `src/foo/bar.rs` (the 2018+
"non-mod-rs" style). Avoid mixing it with `foo/mod.rs` in the same crate; many editor tabs named
`mod.rs` are hard to navigate. To enforce, enable the restriction lint
`clippy::mod_module_files` in `[workspace.lints.clippy]`.

Exception: `tests/common/mod.rs` — in `tests/`, a `common.rs` would be compiled as its own test
binary, so the `mod.rs` form is the conventional way to share helpers between integration tests.

### MOD-03: Private by default; `pub(crate)` for internal sharing

Default visibility ladder — use the narrowest that compiles:

| Visibility | Use for |
|---|---|
| (private) | Everything, initially |
| `pub(super)` | Helpers shared by sibling modules under one parent (e.g. handlers → router) |
| `pub(crate)` | Items used across the crate but not part of its API |
| `pub` | The crate's public API, reachable through the facade (MOD-04) |

In **binary crates** nothing is public API, so use `pub(crate)` for cross-module items. With the
recommended `unreachable_pub = "warn"` lint, a `pub` item in a binary (or a `pub` item in a
private library module that is not re-exported) is flagged — that is the lint doing its job:

```rust,ignore
// crates/shop-server/src/config.rs  (binary crate)
#[derive(Debug, serde::Deserialize)]
pub(crate) struct Settings {
    pub(crate) http: HttpSettings,
    pub(crate) database: DatabaseSettings,
}
```

Never make fields `pub` just to construct a struct in tests: add a constructor, or write the test
inside the module (`#[cfg(test)] mod tests` can see private items).

### MOD-04: `lib.rs` is a facade: private modules, curated `pub use`

Default: library crates declare modules privately and re-export the public items, so the public
path is short and the file layout can change without breaking users.

```rust
//! Domain core: types, rules and ports. No IO, no framework types.
#![warn(missing_docs)]

mod order;
mod ports;
mod service;

pub use order::{Order, OrderId, OrderStatus, Sku};
pub use ports::{OrderRepository, RepoError};
pub use service::{OrderService, PlaceOrderError};
```

Users write `shop_domain::Order`, not `shop_domain::order::model::Order`. Use `pub mod` only when
the namespace carries meaning users should see (`http::header`, `serde::de`), and never both
re-export an item at the root **and** expose it via a public module path — two paths to one
item make docs and error messages confusing.

Avoid a glob `pub use submodule::*` in the facade: it silently exports whatever is added later,
which is how accidental semver breaks happen (`library-design.md` LIB-01).

## Keeping modules small and focused

### MOD-05: No god-modules, no `utils`

Split a module when it has more than one reason to change, or passes ~500–800 lines of
non-test code. Signs of a god-module: a 2 000-line `lib.rs`, a `types.rs` holding every struct, a
`handlers.rs` with every endpoint, an `error.rs` enum with 40 variants for unrelated operations.

❌ `utils.rs`, `helpers.rs`, `common.rs`, `misc.rs` — names that describe *nothing*, so everything
lands there. ✅ Name the concept: `money.rs`, `retry.rs`, `pagination.rs`, `time_window.rs`. If a
helper is used by one module, it belongs in that module.

### MOD-06: Errors live next to the operations that produce them

Default: each module (or operation family) defines its own error enum near its functions
(`orders::PlaceOrderError`), and the binary edge converts them (`ApiError`, exit codes). A single
crate-wide `Error` with every variant forces callers to match impossible cases. Error type design:
`idioms/error-handling`.

## Dependency direction

### MOD-07: Dependencies point inward: edge → application → domain

Default rule for any application, whether expressed as modules or crates:

```text
main / http / cli  ──►  application services  ──►  domain types + ports (traits)
        │                                              ▲
        └──────────►  adapters (postgres, http clients) ┘ (implement ports)
```

- Domain code never imports `axum`, `sqlx`, `reqwest`, `clap`, or `tracing_subscriber`.
- Adapters depend on the domain (to implement its ports), never the other way around.
- `main` is the only place that knows every concrete type (the *composition root*).

Inside one crate the compiler does not enforce this; code review does. When a violation would be
costly (domain accidentally depends on the DB driver), move the domain into its own crate — then
the missing `Cargo.toml` dependency makes the violation a compile error. See
`layered-hexagonal.md` for ports and adapters.

### MOD-08: Don't leak dependency types through your boundaries

Default: a module or crate boundary exposes its own types. Returning `sqlx::Error` from a domain
port, or taking `axum::http::HeaderMap` in a service function, couples every caller to that
dependency (and, for published crates, its semver — LIB-03). Convert at the adapter:

```rust,ignore
fn unavailable(e: sqlx::Error) -> RepoError {
    RepoError::Unavailable(Box::new(e)) // domain sees RepoError; the sqlx error stays as `source()`
}
```

Exception: deliberately shared vocabulary types (`bytes::Bytes`, `http::StatusCode`,
`uuid::Uuid`, `chrono`/`time` types) may cross boundaries when the whole codebase standardises
on them.

## Tests and module structure

### MOD-09: Unit tests inside the module; integration tests against the public API

- Unit tests: `#[cfg(test)] mod tests { use super::*; … }` at the bottom of the file they test;
  they may use private items.
- Integration tests: `tests/*.rs` compile as separate crates and see only `pub` items — they
  verify the facade (MOD-04) and are the reason binaries should have a `lib.rs` (ARCH-02).
- Shared test helpers: `tests/common/mod.rs`, or a `testing` module behind
  `#[cfg(any(test, feature = "test-util"))]` when other crates need your fakes.

Test style, fixtures and property tests: `idioms/testing`.

### MOD-10: Avoid prelude modules in applications

A `prelude` that glob-imports everything hides where names come from and causes ambiguity errors
as the crate grows. Use explicit imports. Libraries with a large, trait-heavy API may offer a
`prelude` for extension traits only.

## Checklist

### MOD-11: Module review checklist

- [ ] Module names are domain nouns; no `utils`/`helpers`/`common`/`misc`.
- [ ] One layout style (`foo.rs` + `foo/`); `mod.rs` only in `tests/common/`.
- [ ] Nothing is `pub` that is not reachable through the facade; binaries use `pub(crate)`.
- [ ] `lib.rs` re-exports the public API explicitly (no glob re-exports).
- [ ] Domain modules import no framework, driver or subscriber crates.
- [ ] Dependency-specific error types are converted at the adapter boundary.
- [ ] No file above ~800 lines of non-test code without a reason.

Related: crate-level boundaries → `workspace-layout.md`; trait boundaries → `layered-hexagonal.md`;
public API rules for published crates → `library-design.md`.
