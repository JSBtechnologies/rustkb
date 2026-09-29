---
id: idioms/traits-generics
title: Traits and generics
summary: >-
  When to use a concrete type, a generic, `impl Trait`, an enum or `dyn Trait`; how to design small,
  coherent traits (associated types, supertraits, blanket and extension impls, dyn compatibility);
  and how to avoid over-generic code.
area: idioms
tags: [traits, generics, dyn, impl-trait, dispatch, object-safety, orphan-rule, upcasting, gat]
rust: "1.96"
edition: "2024"
crates:
  thiserror: "2.0"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility
  - https://doc.rust-lang.org/book/ch18-02-trait-objects.html
  - https://doc.rust-lang.org/edition-guide/rust-2024/rpit-lifetime-capture.html
  - https://blog.rust-lang.org/2025/04/03/Rust-1.86.0/
  - https://rust-lang.github.io/api-guidelines/future-proofing.html
---

# Traits and generics

Decides: concrete vs generic vs `impl Trait` vs enum vs `dyn Trait`, and how to shape traits so they
compose. Read when introducing a trait or type parameter, choosing dispatch, or hitting orphan-rule /
dyn-compatibility errors. Public-signature consequences (turbofish, semver, sealed traits) are in
`api-design.md`; `async fn` in traits is in `async.md`.

## Choosing an abstraction

### TRAIT-01: Start concrete; add a trait when a second real implementation exists

**Default:** write the function against a concrete type. Introduce a trait or type parameter when
there are two real implementations *today* (including a test fake you actually use), or when the code
is a library whose callers supply their own types. **Never** add `trait FooService` + `FooServiceImpl`
+ generics "for flexibility" with a single implementation — it is the most common over-engineering in
LLM-written Rust: slower builds, worse errors, no benefit.

```rust
// ❌ one implementation, three layers of indirection
trait UserRepository { fn find(&self, id: u64) -> Option<String>; }
struct UserRepositoryImpl;
impl UserRepository for UserRepositoryImpl { fn find(&self, _id: u64) -> Option<String> { None } }
struct UserService<R: UserRepository> { repo: R }
```

### TRAIT-02: Pick dispatch with this table

| Situation | Use |
|---|---|
| One type known at compile time | Concrete type |
| Caller picks the type; hot path; you want inlining | Generic `T: Trait` or arg-position `impl Trait` |
| Return one unnameable type (closure, iterator chain, future) | Return-position `impl Trait` |
| Closed set of variants you control | `enum` + `match` |
| Open set chosen at runtime, heterogeneous collections, plugins | `Box<dyn Trait>` / `&dyn Trait` / `Arc<dyn Trait>` |
| Generic code bloating compile times / binary size across many instantiations | `&dyn Trait` inside, generic shim outside |

**Default:** static dispatch (generics / `impl Trait`). **Use** an `enum` when you own every variant —
it is faster than `dyn`, exhaustively matched, and needs no allocation. **Use** `dyn` when the set is
open or chosen at runtime. **Never** use `Box<dyn Trait>` for a fixed set of 2–5 known types.

```rust
// Closed set you own: enum, not Box<dyn Shape>.
pub enum Shape {
    Circle { r: f64 },
    Rect { w: f64, h: f64 },
}

impl Shape {
    pub fn area(&self) -> f64 {
        match self {
            Shape::Circle { r } => std::f64::consts::PI * r * r,
            Shape::Rect { w, h } => w * h,
        }
    }
}

// Open set chosen at runtime: trait object.
pub trait Exporter {
    fn export(&self, rows: &[String]) -> String;
}

pub fn exporters() -> Vec<Box<dyn Exporter + Send + Sync>> {
    Vec::new() // populated from config / plugins
}
```

### TRAIT-03: Reduce monomorphisation with an inner non-generic function

**Default:** generic public functions that do substantial work convert their argument and delegate to
a private non-generic `inner` function, so only the tiny shim is instantiated per type. std does this
for `fs::read_to_string` and friends.

```rust
use std::path::Path;

pub fn load(path: impl AsRef<Path>) -> std::io::Result<Vec<String>> {
    fn inner(path: &Path) -> std::io::Result<Vec<String>> {
        let text = std::fs::read_to_string(path)?;
        Ok(text.lines().map(str::to_owned).collect())
    }
    inner(path.as_ref())
}
```

## Designing traits

### TRAIT-04: Keep traits small and focused on one capability

**Default:** one capability per trait (`Read`, `Write`, `Display`), few required methods, extra
behaviour as provided (default) methods built on the required ones. **Use** supertraits to compose
(`trait Store: Read + Write`). **Never** create a 15-method "interface" trait mirroring a class; callers
and fakes then have to implement everything.

```rust
pub trait Clock {
    fn now_millis(&self) -> u64;

    /// Provided method: implementors get it for free.
    fn elapsed_since(&self, start: u64) -> u64 {
        self.now_millis().saturating_sub(start)
    }
}

pub struct FixedClock(pub u64);

impl Clock for FixedClock {
    fn now_millis(&self) -> u64 {
        self.0
    }
}
```

### TRAIT-05: Associated types when there is one answer per implementor

**Default:** use an associated type when each implementing type has exactly one natural choice
(`Iterator::Item`, `Deref::Target`, a parser's `Output`). **Use** a generic parameter when one type can
implement the trait many times (`From<T>`, `Add<Rhs>`). **Never** use a generic parameter where an
associated type fits — it forces turbofish/annotations on every call.

```rust
pub trait Parser {
    type Output;
    fn parse(&self, input: &str) -> Option<Self::Output>;
}

pub struct IntParser;

impl Parser for IntParser {
    type Output = i64;
    fn parse(&self, input: &str) -> Option<i64> {
        input.trim().parse().ok()
    }
}
```

### TRAIT-06: Put bounds on impls and functions, not on struct definitions

**Default:** `struct Cache<K, V> { .. }` with no bounds; add `K: Hash + Eq` on the `impl` blocks or
methods that need them. **Use** bounds on the struct only when the struct's layout requires them
(e.g. it stores `K::Assoc`) or for `Drop` impls. **Never** put `T: Clone + Debug + Send` on a struct
"just in case" — every user, including `#[derive]`, must repeat the bounds.

```rust
use std::collections::HashMap;
use std::hash::Hash;

pub struct Cache<K, V> {
    map: HashMap<K, V>,
}

impl<K, V> Cache<K, V> {
    pub fn new() -> Self {
        Self { map: HashMap::new() }
    }
}

impl<K, V> Default for Cache<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Hash + Eq, V> Cache<K, V> {
    pub fn get_or_insert_with(&mut self, key: K, f: impl FnOnce() -> V) -> &V {
        self.map.entry(key).or_insert_with(f)
    }
}
```

### TRAIT-07: Use `where` clauses once bounds stop fitting on one line

**Default:** inline bounds for one short bound (`T: Display`); `where` clauses for several parameters
or multi-trait bounds. Prefer `impl Trait` in argument position for simple one-off parameters in
private code.

### TRAIT-08: Extension traits to add methods to foreign types; blanket impls sparingly

**Default:** to add methods to a type you don't own (`str`, `Vec<T>`, `Result`), define a local trait
(`StrExt`) and implement it for that type. **Use** a blanket impl (`impl<T: Display> MyTrait for T`)
when the behaviour is fully derivable from another trait. **Never** add a blanket impl to a published
trait after release — it is a breaking change (it conflicts with downstream impls).

```rust
pub trait StrExt {
    fn truncate_chars(&self, max: usize) -> &str;
}

impl StrExt for str {
    fn truncate_chars(&self, max: usize) -> &str {
        match self.char_indices().nth(max) {
            Some((idx, _)) => &self[..idx],
            None => self,
        }
    }
}

#[test]
fn truncates_on_char_boundary() {
    assert_eq!("héllo".truncate_chars(2), "hé");
}
```

### TRAIT-09: Hit the orphan rule? Wrap in a newtype

**Default:** you can implement a trait for a type only if the trait or the type is local. To implement
a foreign trait (`Display`, `serde::Serialize`) for a foreign type (`Vec<u8>`), wrap it:
`struct Hex(Vec<u8>)`. **Never** fork the crate or define a duplicate trait to get around coherence.

```rust
use std::fmt;

pub struct Hex<'a>(pub &'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
```

## Trait objects

### TRAIT-10: Keep traits you'll use as `dyn` dyn-compatible

**Default:** a trait is usable as `dyn Trait` only if its methods take a receiver (`&self`,
`&mut self`, `Box<Self>`, …), have no type parameters, and don't return `Self`. Mark methods that
break this with `where Self: Sized` so the rest of the trait stays usable as `dyn`. **Never** add a
generic method to a trait that is used as `dyn` elsewhere — it silently stops being dyn-compatible
and the error appears far away. (Native `async fn` in traits is not dyn-compatible; see `async.md`.)

```rust
pub trait Plugin {
    fn name(&self) -> &str;
    fn run(&self, input: &str) -> String;

    // Generic helper excluded from the vtable so `dyn Plugin` still works.
    fn run_all<I: IntoIterator<Item = String>>(&self, inputs: I) -> Vec<String>
    where
        Self: Sized,
    {
        inputs.into_iter().map(|s| self.run(&s)).collect()
    }
}

pub fn run_plugins(plugins: &[Box<dyn Plugin>], input: &str) -> Vec<String> {
    plugins.iter().map(|p| p.run(input)).collect()
}
```

### TRAIT-11: Spell out `Send + Sync + 'static` on trait objects that cross threads

**Default:** `Box<dyn Trait + Send + Sync>` or `Arc<dyn Trait + Send + Sync>` for anything stored in
shared state or moved into `tokio::spawn`/`std::thread::spawn`; `Box<dyn Fn(Event) + Send + 'static>`
for stored callbacks. **Use** a supertrait (`trait Handler: Send + Sync`) when every implementor must
be thread-safe anyway, so the bound isn't repeated everywhere.

### TRAIT-12: Upcast `dyn Sub` to `dyn Super` directly (Rust 1.86+)

**Default:** since Rust 1.86 a `&dyn Sub` / `Box<dyn Sub>` coerces to `&dyn Super` when
`trait Sub: Super`. **Never** hand-write `fn as_super(&self) -> &dyn Super` or `fn as_any(&self) ->
&dyn Any` boilerplate on new code targeting 1.86+; make `Any` a supertrait and coerce.

```rust
use std::any::Any;

pub trait Component: Any {
    fn tick(&mut self);
}

pub struct Health(pub u32);

impl Component for Health {
    fn tick(&mut self) {
        self.0 = self.0.saturating_sub(1);
    }
}

pub fn health_of(c: &dyn Component) -> Option<u32> {
    let any: &dyn Any = c; // trait upcasting coercion
    any.downcast_ref::<Health>().map(|h| h.0)
}
```

## `impl Trait` details

### TRAIT-13: Return `impl Trait` for unnameable types; mind edition-2024 capture rules

**Default:** return `impl Iterator<Item = T>` / `impl Fn(..)` / `impl Future` instead of boxing. In
edition 2024, return-position `impl Trait` captures **all** in-scope generic and lifetime parameters,
so the returned value is assumed to borrow from every reference argument. **Use** `+ use<..>` (1.82+;
in traits since 1.87) to capture less when the return value doesn't borrow an argument. **Never** box
(`Box<dyn Iterator>`) just to satisfy the borrow checker before trying `use<..>`.

```rust
// Returned iterator borrows `words` but not `prefix`: say so with `use<'a>`.
pub fn with_prefix<'a>(words: &'a [String], prefix: &str) -> impl Iterator<Item = &'a String> + use<'a> {
    let prefix = prefix.to_owned();
    words.iter().filter(move |w| w.starts_with(&prefix))
}

fn caller(words: &[String]) -> Vec<&String> {
    let p = String::from("ab");
    let it = with_prefix(words, &p);
    drop(p); // allowed: the iterator does not capture `p`'s lifetime
    it.collect()
}
```

### TRAIT-14: Use GATs only for lending/borrowing abstractions

**Default:** plain associated types. **Use** generic associated types (`type Item<'a> where Self: 'a`)
when an implementor must return items that borrow from `&'a self` or `&'a mut self` (lending
iterators, borrowed views over collections). **Never** reach for GATs to model ordinary ownership;
they complicate every bound that mentions the trait.

## Prefer std traits

### TRAIT-15: Implement standard traits instead of inventing equivalents

**Default:** `From`/`TryFrom` for conversions, `FromStr` for parsing, `Display` for user text,
`Default` for "empty" values, `Iterator`/`IntoIterator` for sequences, `AsRef` for cheap borrows,
`Extend`/`FromIterator` for collections. **Never** write `fn to_string_repr(&self)`, `fn from_str_custom`,
`fn empty()` or `trait Convertible` when a std trait already expresses it — std traits plug into
`?`, `.parse()`, `.collect()`, `format!` and generic code for free.

```rust
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Low,
    High,
}

#[derive(Debug, thiserror::Error)]
#[error("unknown level `{0}`")]
pub struct ParseLevelError(String);

impl FromStr for Level {
    type Err = ParseLevelError; // a real error type, not `String` (see error-handling.md)
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Level::Low),
            "high" => Ok(Level::High),
            other => Err(ParseLevelError(other.to_owned())),
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Level::Low => "low",
            Level::High => "high",
        })
    }
}

#[test]
fn round_trips() {
    let l: Level = "high".parse().unwrap();
    assert_eq!(l.to_string(), "high");
}
```

## Review checklist

- [ ] Every trait has ≥2 real implementors or is a genuine extension point (TRAIT-01).
- [ ] Closed sets use enums; `dyn` only for open/runtime sets (TRAIT-02).
- [ ] Heavy generic functions delegate to a non-generic inner fn (TRAIT-03).
- [ ] Traits are small with provided methods; associated types where one answer per type (TRAIT-04, TRAIT-05).
- [ ] No trait bounds on struct definitions unless layout needs them (TRAIT-06).
- [ ] Orphan-rule problems solved with newtypes; extension traits for foreign types (TRAIT-08, TRAIT-09).
- [ ] Traits used as `dyn` stay dyn-compatible (`where Self: Sized` on generic methods) (TRAIT-10).
- [ ] Trait objects crossing threads carry `Send + Sync` (+ `'static`) (TRAIT-11).
- [ ] No `as_any`/`as_super` boilerplate on 1.86+ code (TRAIT-12).
- [ ] `impl Trait` returns use `use<..>` instead of boxing to fix capture errors (TRAIT-13).
- [ ] Std traits (`FromStr`, `Display`, `From`, `Default`, `Iterator`) used instead of ad-hoc methods (TRAIT-15).
