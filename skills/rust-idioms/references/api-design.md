---
id: idioms/api-design
title: API design
summary: >-
  Rust API Guidelines distilled into decisions: naming and conversion conventions, From/TryFrom/AsRef,
  constructors and builders, #[must_use], #[non_exhaustive], sealed traits, which traits to derive,
  and how to choose impl Trait vs generics vs dyn in public signatures.
area: idioms
tags: [api, naming, conversions, builder, must_use, non_exhaustive, sealed-traits, impl-trait, docs]
rust: "1.96"
edition: "2024"
crates:
  bon: "3.10"
  serde: "1.0"
verified: 2026-09-29
sources:
  - https://rust-lang.github.io/api-guidelines/
  - https://rust-lang.github.io/api-guidelines/checklist.html
  - https://doc.rust-lang.org/reference/attributes/type_system.html#the-non_exhaustive-attribute
  - https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-must_use-attribute
  - https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html
---

# API design

Decisions for the public surface of a crate or module: what to name things, which conversion traits
to implement, how to construct values, which attributes and derives to add, and how to shape
signatures. Read this when adding or reviewing any `pub` item. Semver policy and feature flags live
in the rust-architecture skill; newtypes and typestate live in `types-and-state.md`.

## Naming conventions

### API-01: Use the standard conversion prefixes by cost and ownership

**Default:** follow the `as_` / `to_` / `into_` convention exactly. **Never** name a cheap borrow
`to_*` or an expensive conversion `as_*`; callers read the prefix as a cost signal.

| Prefix | Cost | Ownership | Example |
|---|---|---|---|
| `as_` | free | borrowed → borrowed | `str::as_bytes`, `Path::as_os_str` |
| `to_` | expensive | borrowed → owned (or borrowed → borrowed with work) | `str::to_lowercase`, `Path::to_path_buf` |
| `into_` | variable | owned → owned, consumes `self` | `String::into_bytes`, `Vec::into_boxed_slice` |

Casing (RFC 430): `UpperCamelCase` for types/traits/variants, `snake_case` for functions/modules,
`SCREAMING_SNAKE_CASE` for consts/statics. Acronyms are one word: `HttpClient`, not `HTTPClient`.

### API-02: Getters have no `get_` prefix; iterators come as the `iter` trio

**Default:** a getter for field `name` is `fn name(&self) -> &str`; the mutable one is `name_mut`.
`get` is reserved for lookups that can miss (`HashMap::get`, `Vec::get`). Collections expose
`iter()`, `iter_mut()`, and `IntoIterator` for `T`, `&T`, `&mut T`. **Never** write `get_name()` —
no default lint catches it, so reviewers must.

```rust
pub struct Playlist {
    name: String,
    tracks: Vec<String>,
}

impl Playlist {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn iter(&self) -> std::slice::Iter<'_, String> {
        self.tracks.iter()
    }
}

impl<'a> IntoIterator for &'a Playlist {
    type Item = &'a String;
    type IntoIter = std::slice::Iter<'a, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.tracks.iter()
    }
}
```

## Conversion traits

### API-03: Implement `From`/`TryFrom`, never `Into`/`TryInto`

**Default:** implement `From<A> for B`; the blanket impl gives callers `a.into()` for free. Use
`TryFrom` when the conversion can fail, with a real error type. **Never** implement `Into` directly
(clippy `from_over_into`), and **never** implement `From` for a lossy or fallible conversion that
panics inside.

```rust
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Port(u16);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortError(u32);

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "port {} is out of range 1..=65535", self.0)
    }
}

impl std::error::Error for PortError {}

impl TryFrom<u32> for Port {
    type Error = PortError;
    fn try_from(v: u32) -> Result<Self, Self::Error> {
        match u16::try_from(v) {
            Ok(p) if p != 0 => Ok(Port(p)),
            _ => Err(PortError(v)),
        }
    }
}

impl From<Port> for u16 {
    fn from(p: Port) -> u16 {
        p.0
    }
}
```

### API-04: Parse strings with `FromStr`, not ad-hoc `from_string` functions

**Default:** if a type has a textual form, implement `FromStr` (so `"…".parse::<T>()` works and
clap/serde helpers can use it) and `Display` for the inverse. Keep `parse → display → parse`
round-trippable. **Never** add `fn from_str(s: &str) -> Self` as an inherent method (clippy
`should_implement_trait`).

### API-05: Take the cheapest parameter type that works; go generic only at the edges

**Default:** borrow: `&str`, `&[T]`, `&Path`, `&T`. Take owned (`String`, `Vec<T>`) only when the
function stores it. For public constructors that store a string, `impl Into<String>` is fine; for
path-taking public functions, `impl AsRef<Path>` matches std. **Never** take `&String`, `&Vec<T>`,
or `&PathBuf` (clippy `ptr_arg`), and don't sprinkle `impl Into<…>`/`AsRef` on internal helpers —
each generic parameter is monomorphised and slows compile times.

```rust
use std::path::{Path, PathBuf};

pub struct Config {
    name: String,
    root: PathBuf,
}

impl Config {
    // Stores both values: take ownership, accept anything convertible.
    pub fn new(name: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self { name: name.into(), root: root.into() }
    }
}

// Only reads: borrow. Mirrors std::fs::read_to_string's signature.
pub fn load(path: impl AsRef<Path>) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}
```

### API-06: Return borrowed views from getters, owned values from computations

**Default:** getters return `&str`/`&[T]`/`Option<&T>`, never `&String`/`&Vec<T>`/`&Option<T>`.
Functions that build something return it owned. Return `Cow<'_, str>` only when the common path
borrows and the rare path allocates (see `ownership-borrowing.md`). **Never** return `&mut Vec<T>`
if callers only need to push — expose `push_*` instead to keep invariants.

## Constructors and builders

### API-07: Provide `new` and `Default` together; `new` takes only required data

**Default:** `new(required…) -> Self` for infallible construction, `try_new(…) -> Result<Self, E>`
(or `TryFrom`) when validation can fail. If `new()` takes no arguments, also implement `Default`
(clippy `new_without_default`). Use `with_*` for alternative constructors (`Vec::with_capacity`).
**Never** make `new` return `Result` silently named `new` in a crate where other `new`s are infallible
— name it `try_new`/`parse`/`open`.

### API-08: Use a builder for 4+ optional settings; otherwise use a config struct or arguments

**Default:** 0–3 parameters → plain arguments. Many optional knobs → either a public config struct
with `Default` + struct-update syntax, or a builder. Hand-write the builder when it's small; reach for
`bon` (derive/fn builders with compile-time required-field checks) when there are many fields or many
functions needing builders. Builder methods take and return `self` by value; `build()` returns
`Result` only if it validates. **Never** generate a builder for a 2-field struct, and never make
required fields `Option` and `unwrap()` them in `build()`.

```rust
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    timeout: Duration,
    retries: u32,
}

#[derive(Debug, Clone)]
#[must_use = "a builder does nothing until .build() is called"]
pub struct ClientBuilder {
    base_url: String,
    timeout: Duration,
    retries: u32,
}

impl Client {
    // Required data goes into the builder constructor, not an Option.
    pub fn builder(base_url: impl Into<String>) -> ClientBuilder {
        ClientBuilder {
            base_url: base_url.into(),
            timeout: Duration::from_secs(30),
            retries: 3,
        }
    }
}

impl ClientBuilder {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }
    pub fn build(self) -> Client {
        let ClientBuilder { base_url, timeout, retries } = self;
        Client { base_url, timeout, retries }
    }
}

fn demo() -> Client {
    Client::builder("https://api.example.com").retries(5).build()
}
```

With `bon`, the same shape comes from a derive; required fields are enforced at compile time:

```rust
#[derive(Debug, bon::Builder)]
pub struct Job {
    name: String,
    #[builder(default = 3)]
    max_attempts: u32,
    queue: Option<String>,
}

fn make_job() -> Job {
    Job::builder().name("reindex".to_owned()).build()
}
```

## Attributes that protect callers

### API-09: Mark results that are useless to ignore with `#[must_use]`

**Default:** add `#[must_use]` to pure functions and methods returning a new value instead of
mutating (`fn with_x(self, …) -> Self`, `fn to_*`), to builder types, and to guard types whose drop
does something. Add a message when the mistake isn't obvious. `Result` and `Future` are already
`must_use`. **Never** put it on functions whose main effect is a side effect (`fn insert(&mut self…) -> bool`
is fine without it). clippy `must_use_candidate` (pedantic) finds candidates. On a method:
`#[must_use = "returns a new point; the original is unchanged"] pub fn translated(&self, …) -> Point`;
on a type: see `ClientBuilder` in API-08.

### API-10: Put `#[non_exhaustive]` on public types that will grow

**Default:** public error enums, config/options structs and "kind" enums that you expect to extend get
`#[non_exhaustive]`, so adding a variant or field is not a breaking change. Downstream code must then
use `_ =>` arms and cannot build the struct with a literal (provide a constructor or `Default`).
**Use exhaustive enums when** the set is closed by definition (`Ordering`, `Direction::{In, Out}`)
and callers benefit from exhaustiveness checking. **Never** add it inside a binary crate or a private
module — it only affects other crates and just adds noise.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Compression {
    None,
    Gzip { level: u32 },
    Zstd { level: i32 },
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Options {
    pub verbose: bool,
    pub compression: Option<Compression>,
}
// Downstream: `let mut o = Options::default(); o.verbose = true;` — literals won't compile.
```

### API-11: Seal traits that users may call but must not implement

**Default:** if a public trait exists only to abstract over your own types (and you want to add
methods later without a breaking change), seal it with a private supertrait. **Use an open trait
when** downstream implementations are the point (plugins, `Serialize`-like extension). Document
"This trait is sealed" in its docs.

```rust
mod private {
    pub trait Sealed {}
}

/// Types that can be used as a column key. This trait is sealed.
pub trait ColumnKey: private::Sealed {
    fn column_index(&self) -> usize;
}

impl private::Sealed for usize {}
impl ColumnKey for usize {
    fn column_index(&self) -> usize {
        *self
    }
}
```

## Standard trait implementations

### API-12: Derive the common traits eagerly on public types

**Default:** every public type implements `Debug` (enable `missing_debug_implementations`). Value
types also derive `Clone`, `PartialEq`, and — when semantically valid — `Eq`, `Hash`, `PartialOrd`,
`Ord`, `Default`, and `Copy` (small, plain data with no heap ownership). Put `serde::{Serialize,
Deserialize}` behind an optional `serde` feature in libraries (see rust-architecture for features).
**Never** derive `Default` for a type whose zero/empty value is invalid, derive `PartialOrd` on floats
expecting a total order, or hand-write `Debug` just to hide fields (use `finish_non_exhaustive()` when
you must redact secrets).

```rust
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

pub struct Credentials {
    user: String,
    password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("user", &self.user)
            .finish_non_exhaustive() // prints `Credentials { user: "..", .. }`
    }
}
```

### API-13: Keep `Send + Sync` guaranteed and tested

**Default:** public types should be `Send + Sync` unless they are deliberately thread-bound. Auto
traits are part of your API: adding an `Rc` or `Cell` field silently removes them and breaks users.
Lock it in with a compile-time assertion.

```rust
pub struct Engine {
    cache: std::sync::Mutex<Vec<u8>>,
}

const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Engine>();
};
```

### API-14: Implement `Deref` only for smart pointers

**Default:** `Deref`/`DerefMut` are for pointer-like wrappers (`Box`, `Arc`, guards). **Never** use
`Deref` on a newtype to "inherit" the inner type's methods — it leaks the representation and bypasses
invariants. Expose `as_str()`/`as_slice()`, `AsRef`, or forward the few methods you need
(see `types-and-state.md`, TYPE-03).

## Shaping public signatures

### API-15: Choose `impl Trait`, generics or `dyn` deliberately in public signatures

**Default:** generic or `impl Trait` arguments for hot or small functions; `&dyn Trait`/`Box<dyn Trait>`
when you store heterogeneous values, need a stable non-generic signature, or want to cut
monomorphisation. The mechanics of static vs dynamic dispatch are in `traits-generics.md`.

| Position | Choose | Why |
|---|---|---|
| Argument, used once | `impl Trait` | Short; but callers can't turbofish it |
| Argument, same type twice / callers may need `::<T>` / bound in `where` | named `<T: Trait>` | Nameable, relatable |
| Stored in a struct field | `T` param or `Box<dyn Trait>` | `impl Trait` isn't allowed in fields |
| Return, one concrete hidden type | `-> impl Trait` | Hides type; caller can't name it |
| Return, library iterator callers may store | named struct (`pub struct Iter<'a>`) | Nameable, can add trait impls later |
| Return, one of several types | `Box<dyn Trait>` or an enum | `impl Trait` needs one concrete type |

Replacing a named generic with `impl Trait` (breaks callers' turbofish), adding a bound, or losing an
auto trait (`Send`) on a returned `impl Trait` are breaking changes — choose once.

### API-16: Return data, don't fill out-parameters; don't take `bool` flags

**Default:** return a tuple or small struct instead of `&mut` out-parameters; replace `bool` params
with an enum (`types-and-state.md`, TYPE-05). **Use `&mut` params when** the caller reuses a buffer
(`read(&mut buf)`).

### API-17: Keep fields private unless the type is passive data

**Default:** private fields plus accessors, so you can add invariants and fields later. **Use `pub`
fields when** every combination of values is valid (a `Point`, a `#[non_exhaustive]` options struct).
**Never** mix `pub` and private invariant-carrying fields in one struct.

## Documentation

### API-18: Document errors, panics, safety and give an example for every public item

**Default:** every public function documents `# Errors` (when it returns `Result`), `# Panics` (when
it can panic), `# Safety` (every `unsafe fn`), and has at least one `# Examples` doctest that
compiles. Use intra-doc links (``[`Config::new`]``). Enable `missing_docs` and clippy
`missing_errors_doc`/`missing_panics_doc` for libraries (see `lints-and-tooling.md`). Doctests
should use `?`, not `unwrap()`:

~~~rust
/// Parses a `host:port` pair.
///
/// # Errors
///
/// Returns an error if the input has no `:` or the port is not a valid `u16`.
///
/// # Examples
///
/// ```
/// let (host, port) = netaddr::parse_addr("localhost:8080")?;
/// assert_eq!((host.as_str(), port), ("localhost", 8080));
/// # Ok::<(), String>(())
/// ```
pub fn parse_addr(s: &str) -> Result<(String, u16), String> {
    let (host, port) = s.rsplit_once(':').ok_or_else(|| format!("missing ':' in {s:?}"))?;
    let port = port.parse().map_err(|e| format!("bad port {port:?}: {e}"))?;
    Ok((host.to_owned(), port))
}
~~~

## Review checklist

- [ ] `as_`/`to_`/`into_` names match cost and ownership; getters have no `get_` (API-01, API-02)
- [ ] Conversions are `From`/`TryFrom`/`FromStr`, never hand-written `Into` (API-03, API-04)
- [ ] No `&String`/`&Vec<T>`/`&PathBuf` params; generic params only where they pay off (API-05)
- [ ] Getters return `&str`/`&[T]`, not `&String`/`&Vec` (API-06)
- [ ] Zero-arg `new` has a matching `Default`; fallible constructors are `try_new`/`TryFrom` (API-07)
- [ ] Builders only for many optional settings; required fields not `Option` + `unwrap` (API-08)
- [ ] `#[must_use]` on value-returning pure methods and builders (API-09)
- [ ] Growing public enums/structs are `#[non_exhaustive]` (API-10); internal-only traits sealed (API-11)
- [ ] Public types derive `Debug` + applicable common traits; `Send + Sync` asserted (API-12, API-13)
- [ ] No `Deref` on newtypes (API-14); `impl Trait` vs generic vs `dyn` chosen per table (API-15)
- [ ] No bool flag params or out-params (API-16); fields private unless passive data (API-17)
- [ ] `# Errors`/`# Panics`/`# Safety`/`# Examples` present on public items (API-18)
