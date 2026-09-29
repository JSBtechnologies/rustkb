---
id: idioms/macros
title: Macros
summary: >-
  When a macro is justified at all (usually it isn't), how to write hygienic, exportable macro_rules!,
  cfg_select! over cfg-if, and how to build, structure and test derive/attribute proc macros with
  syn 3, quote and proc-macro2.
area: idioms
tags: [macros, macro_rules, proc-macro, derive, syn, quote, trybuild, cfg_select, hygiene]
rust: "1.96"
edition: "2024"
crates:
  syn: "3.0"
  quote: "1.0"
  proc-macro2: "1.0"
  trybuild: "1.0"
  cargo-expand: "1.0"
  pastey: "0.2"
  cfg-if: "1.0"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/reference/macros-by-example.html
  - https://doc.rust-lang.org/reference/procedural-macros.html
  - https://doc.rust-lang.org/edition-guide/rust-2024/macro-fragment-specifiers.html
  - https://github.com/dtolnay/syn/releases/tag/3.0.0
  - https://docs.rs/trybuild/1
  - https://rustsec.org/advisories/RUSTSEC-2024-0436.html
---

# Macros

Macros are the last resort, not a shortcut: they hurt compile times, IDE support, error messages and
readability. This file decides when a macro is warranted, how to write `macro_rules!` that work from
other crates, and how to structure proc macros. Read it before writing any `macro_rules!` or adding a
`proc-macro = true` crate.

## Should this be a macro?

### MAC-01: Reach for functions, generics, traits and `const fn` first

**Default:** a function. Generic over a trait if it must work for many types; `const fn` if it must run
at compile time; a closure parameter for "pass in behaviour". **Use a macro only when** you need
something functions cannot do: variadic or format-string arguments, early `return`/`?`/`break` in the
caller's scope, generating items (impls for many types, test cases), or a custom derive/attribute.
**Never** write a macro to save typing a generic bound, to fake overloading, or to build a collection
literal that std already supports.

```rust
// ❌ Both of these should be plain code.
macro_rules! square { ($x:expr) => { $x * $x }; }        // evaluates $x twice; use a fn
macro_rules! map { ($($k:expr => $v:expr),*) => {{       // std has HashMap::from([(k, v), ..])
    let mut m = ::std::collections::HashMap::new(); $(m.insert($k, $v);)* m }}; }
```

```rust
use std::collections::HashMap;
use std::ops::Mul;

fn square<T: Mul<Output = T> + Copy>(x: T) -> T {
    x * x
}

fn defaults() -> HashMap<&'static str, u32> {
    HashMap::from([("retries", 3), ("timeout_s", 30)])
}
```

## Declarative macros (`macro_rules!`)

### MAC-02: Use absolute paths inside macro bodies

**Default:** every path in an expansion is absolute: `$crate::…` for items of the defining crate,
`::core::…`/`::std::…` for std (e.g. `::core::result::Result::Err`). Macro hygiene only covers local
variables and labels, not paths; a bare `Result`, `HashMap` or `helper()` resolves at the call site
and breaks when the user has a different item with that name or hasn't imported it.
**Never** rely on the caller having `use`d anything.

### MAC-03: Scope macros like items; export public ones deliberately

**Default:** for crate-internal macros, define with `macro_rules!` and re-export with
`pub(crate) use name;` so they're path-addressable like any item (no `#[macro_use]`, no ordering
games). For public macros, `#[macro_export]` (which places the macro at the crate root) and route
all helper calls through a `#[doc(hidden)] pub mod __private` so you can change them without a
semver break. **Never** use `#[macro_use] extern crate` in edition 2018+ code.

```rust
macro_rules! bail_if {
    ($cond:expr, $($fmt:tt)+) => {
        if $cond {
            return ::core::result::Result::Err(::std::format!($($fmt)+));
        }
    };
}
pub(crate) use bail_if; // now usable as `crate::path::bail_if!` anywhere in the crate

fn withdraw(balance: u64, amount: u64) -> Result<u64, String> {
    bail_if!(amount > balance, "insufficient funds: {amount} > {balance}");
    Ok(balance - amount)
}
```

A public macro with a private support module (from a library's `lib.rs`):

~~~rust
#[doc(hidden)]
pub mod __private {
    pub use std::format; // re-export everything the expansion needs
    pub fn emit(line: &str) { eprintln!("{line}"); }
}

#[macro_export]
macro_rules! log_kv {
    ($($key:ident = $value:expr),+ $(,)?) => {
        $crate::__private::emit(&$crate::__private::format!(
            ::core::concat!($(::core::stringify!($key), "={:?} "),+), $($value),+
        ))
    };
}
~~~

### MAC-04: Evaluate each input exactly once and expand to a single expression or block

**Default:** bind each `$e:expr` to a local (`let value = $e;`) before using it more than once; wrap
multi-statement expansions in `{ … }` (or `{{ … }}` for an expression macro) so they work in any
position; accept an optional trailing comma with `$(,)?`. **Never** splice `$e` twice — side effects
and expensive calls run twice.

### MAC-05: Use the narrowest fragment specifier and know the edition-2024 `expr` change

**Default:** `ident`, `ty`, `path`, `lifetime`, `literal` when that's what you accept; `expr` for
values; `tt` only for pass-through (`$($fmt:tt)+` into `format!`). In edition 2024, `$e:expr` also
matches `const { … }` blocks and `_`; if an earlier arm must not swallow those (e.g. you have a later
`(const $e:expr)` arm), use `expr_2021` (available in all editions since 1.83). `cargo fix --edition`
rewrites every `expr` to `expr_2021` — revert to `expr` wherever no other arm conflicts (most macros).
**Never** write recursive `tt` munchers when a single repetition `$( … ),*` works; they are slow to
compile and hit the recursion limit.

### MAC-06: Generate repetitive impls with a repetition, not copy-paste

**Default:** when the same impl is needed for many concrete types (numeric primitives, tuple arities),
one `macro_rules!` with `$($t:ty),*` is the idiomatic tool. Prefer a blanket impl over a trait bound if
one exists (`impl<T: Into<u64>> …`) — see `traits-generics.md`.

```rust
pub trait Bits {
    const BITS: u32;
}

macro_rules! impl_bits {
    ($($t:ty),+ $(,)?) => {
        $( impl Bits for $t { const BITS: u32 = <$t>::BITS; } )+
    };
}

impl_bits!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

#[test]
fn bits() {
    assert_eq!(<u16 as Bits>::BITS, 16);
}
```

## Conditional compilation

### MAC-07: Use `cfg_select!` instead of `cfg-if` on Rust ≥ 1.95

**Default:** when `rust-version` is 1.95 or later, use std's `cfg_select!` for multi-way platform
selection; it works in item and expression position and needs no dependency. Keep `cfg-if` only for
crates with an older MSRV. For a single condition, a plain `#[cfg(…)]` attribute is clearer than either.

```rust
cfg_select! {
    windows => {
        pub fn config_dir_env() -> &'static str { "APPDATA" }
    }
    _ => {
        pub fn config_dir_env() -> &'static str { "XDG_CONFIG_HOME" }
    }
}

pub fn path_list_separator() -> char {
    cfg_select! {
        windows => ';',
        _ => ':',
    }
}
```

## Procedural macros

### MAC-08: Write a proc macro only for derives and attributes that need the item's syntax

**Default:** a custom derive when users would otherwise hand-write a mechanical impl for every type
(field lists, builders, conversions). Before writing one, check whether an existing derive does it
(serde, thiserror, strum, bon, derive_more — see rust-ecosystem). **Never** write a function-like proc
macro when `macro_rules!` can express it, and never write an attribute macro that silently rewrites a
function body in surprising ways.

Crate layout: proc macros must live in their own `proc-macro = true` crate. Ship it as
`foo-derive` (or `foo-macros`) and re-export it from the facade crate `foo`, which also owns the trait
the derive implements, so users add one dependency and generated code can refer to `::foo::Trait`.

```toml
# foo-derive/Cargo.toml
[lib]
proc-macro = true

[dependencies]
proc-macro2 = "1.0"
quote = "1.0"
syn = "3.0"            # add features = ["full"] only if you parse fn bodies/arbitrary items
```

### MAC-09: Use syn 3, quote and proc-macro2 — and mind syn's compile cost

**Default:** new proc-macro crates use `syn` 3.0 (released 2026-07), `quote` 1 and `proc-macro2` 1.
Derive-only macros work with syn's default features; enable `"full"` only when parsing statements,
expressions or whole items (e.g. an attribute on a `fn`). syn 3 renamed and restructured several
syntax-tree nodes (`Type::BareFn` → `Type::FnPtr`, new `…Modifiers` structs, `Safety` replacing
`unsafety`, match guards moved into `Pat::Guard`); code matching those variants must be updated,
while typical `DeriveInput`/`Data`/`Fields` code compiles unchanged. syn 2 remains everywhere in the
ecosystem; both majors in one dependency tree is expected for now. **Never** hand-parse `TokenStream`
strings with `to_string()` + string munging.

### MAC-10: Report errors as spanned `syn::Error`, never panic

**Default:** the `#[proc_macro_derive]` entry point parses with `parse_macro_input!`, calls an inner
`fn expand(&DeriveInput) -> syn::Result<proc_macro2::TokenStream>`, and converts errors with
`into_compile_error()`. Errors are created with `syn::Error::new_spanned(node, "msg")` so the compiler
points at the offending tokens. Combine multiple errors with `Error::combine`. **Never** `panic!`,
`unwrap()` or `expect()` in a proc macro — users get "proc macro panicked" with no location.

~~~rust
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

#[proc_macro_derive(FieldNames)]
pub fn derive_field_names(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

// proc_macro2 in, proc_macro2 out: unit-testable without a compiler round-trip.
fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(&input.ident, "FieldNames can only be derived for structs"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(&data.fields, "FieldNames requires named fields"));
    };
    let names = fields.named.iter().filter_map(|f| f.ident.as_ref()).map(ToString::to_string);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::field_names::FieldNames for #name #ty_generics #where_clause {
            const FIELD_NAMES: &'static [&'static str] = &[#(#names),*];
        }
    })
}
~~~

### MAC-11: Make generated code robust to the call site

**Default:** generated code uses fully qualified paths (`::field_names::FieldNames`,
`::core::option::Option`), forwards generics with `split_for_impl()`, and never assumes the user
imported a trait. Use `quote_spanned!` when the error for a bad field type should point at that field.
**Never** emit `use` statements into the caller's scope or generate items with fixed names that can
collide (wrap helpers in `const _: () = { … };`).

### MAC-12: Test proc macros at three levels

**Default:** (1) unit-test `expand` directly with `syn::parse_quote!` inputs; (2) use `trybuild` for
pass and compile-fail cases, committing the `.stderr` snapshots (`TRYBUILD=overwrite cargo test`
regenerates them after an intentional change); (3) debug expansions with `cargo expand`.
Put trybuild tests in the facade crate's `tests/` so they exercise the public re-export.

~~~rust
// field-names/tests/ui.rs  (dev-dependency: trybuild = "1.0")
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass_struct.rs");
    t.compile_fail("tests/ui/fail_enum.rs"); // expected error in tests/ui/fail_enum.stderr
}
~~~

## Helper crates

### MAC-13: Don't add `paste`; prefer not concatenating identifiers at all

**Default:** design macros so callers pass full identifiers. If you truly need to build identifiers
(`get_$field`), use `pastey`, the maintained drop-in fork. **Never** add `paste` to new code: it is
archived and flagged unmaintained by RUSTSEC-2024-0436. Likewise replace `lazy_static!` with
`std::sync::LazyLock` (see `concurrency.md`) — a macro is no longer needed for lazy statics.

## Review checklist

- [ ] Could this macro be a function, generic, trait or `const fn`? (MAC-01)
- [ ] Every path in the expansion is `$crate::`/`::core::`/`::std::` (MAC-02, MAC-11)
- [ ] Internal macros re-exported with `pub(crate) use`; public ones use a `#[doc(hidden)] __private` module (MAC-03)
- [ ] Each `$e:expr` evaluated once; expansion wrapped in a block; trailing comma accepted (MAC-04)
- [ ] Narrow fragment specifiers; no needless tt-munchers; `expr` vs `expr_2021` considered (MAC-05)
- [ ] `cfg_select!` instead of `cfg-if` when MSRV ≥ 1.95 (MAC-07)
- [ ] Proc macro lives in a `-derive` crate re-exported by the facade (MAC-08)
- [ ] syn 3 with minimal features; no string-munging of tokens (MAC-09)
- [ ] No panics/unwraps in proc macros; errors are spanned `syn::Error` (MAC-10)
- [ ] Unit tests on `expand` + trybuild pass/fail cases (MAC-12)
- [ ] No `paste`/`lazy_static` in new code (MAC-13)
