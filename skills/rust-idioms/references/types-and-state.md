---
id: idioms/types-and-state
title: Types and state modelling
summary: >-
  Make illegal states unrepresentable: newtypes for domain values, parse-don't-validate constructors,
  enums instead of bools/sentinels/Option soup, typestate for compile-time protocols, typed IDs with
  PhantomData, and exhaustive matching.
area: idioms
tags: [newtype, parse-dont-validate, typestate, enums, nonzero, phantomdata, const-generics, serde]
rust: "1.96"
edition: "2024"
crates:
  serde: "1.0"
  nutype: "0.8"
  derive_more: "2.1"
verified: 2026-09-29
sources:
  - https://rust-lang.github.io/api-guidelines/type-safety.html
  - https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/
  - https://cliffle.com/blog/rust-typestate/
  - https://serde.rs/container-attrs.html#try_from
  - https://doc.rust-lang.org/std/num/struct.NonZero.html
---

# Types and state modelling

How to encode domain rules in types so the compiler enforces them: newtypes, validated constructors,
enums for states and choices, typestate, typed IDs. Read this when designing data structures,
function parameters that are "just a `String`/`u64`/`bool`", or a type whose methods are only valid in
some states. Naming, builders and trait derives are in `api-design.md`.

## Newtypes for domain values

### TYPE-01: Wrap domain primitives in newtypes

**Default:** any value with meaning beyond its representation — IDs, money, emails, paths inside a
sandbox, units — gets a tuple-struct newtype. It is zero-cost and turns argument mix-ups into compile
errors. **Use the raw primitive when** it is genuinely just a number/string with no rules (a loop
counter, free-form display text). **Never** pass two same-typed IDs side by side as `u64, u64`.

```rust
// ❌ Which u64 is which? Swapping them compiles and silently corrupts data.
fn transfer(from: u64, to: u64, cents: i64) {}
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cents(i64);

pub fn transfer(from: AccountId, to: AccountId, amount: Cents) {
    let _ = (from, to, amount);
}
```

### TYPE-02: Parse, don't validate — check once in the constructor, then trust the type

**Default:** a validated type has a private field and exactly one way in: `TryFrom`/`FromStr`/`try_new`
returning `Result`. Everything downstream takes the validated type, never `&str` + a re-check.
Expose the inner value read-only (`as_str`, `get`, `into_inner`). **Never** write `fn is_valid_email(&str) -> bool`
and hope callers call it, and **never** make the field `pub` (that re-opens the invalid state).

```rust
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid email address: {0:?}")]
pub struct InvalidEmail(String);

impl FromStr for Email {
    type Err = InvalidEmail;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        match s.split_once('@') {
            Some((user, domain)) if !user.is_empty() && domain.contains('.') => {
                Ok(Email(s.to_ascii_lowercase()))
            }
            _ => Err(InvalidEmail(s.to_owned())),
        }
    }
}

impl Email {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn domain(&self) -> &str {
        // Invariant from from_str: exactly one '@' exists.
        self.0.split_once('@').map_or("", |(_, d)| d)
    }
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// Downstream code cannot receive an unchecked string.
pub fn send_welcome(to: &Email) -> String {
    format!("Welcome mail queued for {to}")
}
```

### TYPE-03: Give newtypes explicit accessors, not `Deref`, and only the traits that hold

**Default:** expose `as_str()`/`as_inner()`, `AsRef<str>` if useful, `Display`, `FromStr`. Derive only
traits that are true for the domain: `Ord` for `Cents` yes, `Default` for `Email` no (the empty
string is invalid), arithmetic only via explicit `impl Add` where it makes sense (adding two
`AccountId`s must not compile). **Never** `impl Deref<Target = String>` to get all `String` methods
for free — `email.push_str("x")`-style mutations via `DerefMut` or `.clone()` into raw `String`
bypass the invariant. (See `api-design.md`, API-14.)

### TYPE-04: Keep validation when deserializing

**Default:** route serde through the validating constructor with `#[serde(try_from = "…", into = "…")]`.
A derived `Deserialize` on a newtype with a private field would bypass `FromStr`. For many simple
validated newtypes, `nutype` generates the constructor, error type and serde glue; `derive_more` cuts
boilerplate for `Display`/`From`/arithmetic. Both are situational — hand-written code is fine for a
handful of types.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Username(String);

impl TryFrom<String> for Username {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        let ok = (3..=32).contains(&s.len())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if ok { Ok(Username(s)) } else { Err(format!("invalid username {s:?}")) }
    }
}

impl From<Username> for String {
    fn from(u: Username) -> String {
        u.0
    }
}

#[test]
fn rejects_invalid_on_deserialize() {
    assert!(serde_json::from_str::<Username>(r#""ab""#).is_err());
    let u: Username = serde_json::from_str(r#""ferris_42""#).unwrap();
    assert_eq!(String::from(u), "ferris_42");
}
```

## Enums instead of flags, sentinels and Option soup

### TYPE-05: Replace `bool` parameters with two-variant enums

**Default:** a boolean argument whose meaning isn't obvious at the call site becomes an enum.
`open(path, Mode::Append)` reads; `open(path, true)` doesn't. **Use `bool` when** the function name
states the meaning (`set_visible(true)`) or it's a field on a plain options struct. **Never** add a
second bool param — two bools are four states, usually only three of which are valid.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overwrite {
    Allow,
    Deny,
}

pub fn copy_file(src: &std::path::Path, dst: &std::path::Path, overwrite: Overwrite) -> std::io::Result<u64> {
    if overwrite == Overwrite::Deny && dst.exists() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "destination exists"));
    }
    std::fs::copy(src, dst)
}
```

### TYPE-06: Use `Option`/`NonZero`, never sentinel values

**Default:** "no value" is `Option<T>`, never `-1`, `0`, `""`, or `u64::MAX`. For IDs/counts that are
never zero, `std::num::NonZero<u32>` (generic `NonZero<T>` since 1.79) makes `Option<NonZero<u32>>` the
same size as `u32`. **Never** return `-1` for "not found"; return `Option<usize>`.

```rust
use std::num::NonZero;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowId(NonZero<u64>);

const _: () = assert!(size_of::<Option<RowId>>() == size_of::<u64>());

pub fn find(haystack: &[&str], needle: &str) -> Option<usize> {
    haystack.iter().position(|s| *s == needle)
}
```

### TYPE-07: Model mutually exclusive states as an enum with data

**Default:** when fields are only meaningful in some states, make each state a variant carrying its
own data. **Never** write a struct of `Option`s plus a `status` field that must be kept in sync — LLM
code does this constantly and every reader has to re-derive which combinations are legal.

```rust
// ❌ 2^3 × status combinations; most are nonsense (Failed with a response body?)
struct Request {
    status: String,
    started_at: Option<std::time::Instant>,
    response: Option<Vec<u8>>,
    error: Option<String>,
}
```

```rust
use std::time::Instant;

#[derive(Debug)]
pub enum Request {
    Queued,
    InFlight { started_at: Instant },
    Done { started_at: Instant, body: Vec<u8> },
    Failed { error: String, attempts: u32 },
}

impl Request {
    pub fn body(&self) -> Option<&[u8]> {
        match self {
            Request::Done { body, .. } => Some(body),
            Request::Queued | Request::InFlight { .. } | Request::Failed { .. } => None,
        }
    }
}
```

### TYPE-08: Match your own enums exhaustively — no `_` arm

**Default:** list every variant (or group them with `|`) when matching an enum your crate defines, so
adding a variant produces compile errors at every decision point. **Use `_` when** matching foreign
`#[non_exhaustive]` enums (required) or large foreign enums like `io::ErrorKind`. Enable clippy
`wildcard_enum_match_arm` (restriction) or at least `match_wildcard_for_single_variants` (pedantic) in
domain-heavy crates. `matches!` and, since 1.96, `assert_matches!` (import it: `use std::assert_matches;`) are fine
for single-variant checks.

## Typestate

### TYPE-09: Use typestate when call order is known at compile time

**Default:** if an object has a fixed protocol (unconfigured → connected → authenticated; builder with
required steps) and the sequence is decided in code, encode each state as a type and make transitions
consume `self`. Invalid calls then fail to compile. **Use a runtime enum (TYPE-07) when** the state
depends on runtime events (network messages, user input) or must be stored in a collection. **Never**
build typestate with more than ~4 states or generic state explosions; the API becomes unreadable.

```rust
use std::marker::PhantomData;

pub struct Disconnected;
pub struct Connected;
pub struct Authenticated;

pub struct Session<S> {
    addr: String,
    token: Option<String>,
    _state: PhantomData<S>,
}

impl Session<Disconnected> {
    pub fn new(addr: impl Into<String>) -> Self {
        Session { addr: addr.into(), token: None, _state: PhantomData }
    }
    pub fn connect(self) -> std::io::Result<Session<Connected>> {
        if self.addr.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty address"));
        }
        Ok(Session { addr: self.addr, token: None, _state: PhantomData })
    }
}

impl Session<Connected> {
    pub fn login(self, token: impl Into<String>) -> Session<Authenticated> {
        Session { addr: self.addr, token: Some(token.into()), _state: PhantomData }
    }
}

impl Session<Authenticated> {
    // Only exists once authenticated: `Session::new(..).query(..)` does not compile.
    pub fn query(&self, q: &str) -> String {
        format!("{}@{}: {q}", self.token.as_deref().unwrap_or_default(), self.addr)
    }
}

fn run() -> std::io::Result<String> {
    let s = Session::new("db:5432").connect()?.login("secret");
    Ok(s.query("select 1"))
}
```

## Typed IDs, units and const generics

### TYPE-10: Build typed IDs with `PhantomData`, implementing traits by hand

**Default:** one generic `Id<T>` instead of N hand-written ID newtypes, using `PhantomData<fn() -> T>`
(keeps `Id<T>` `Send + Sync` and covariant regardless of `T`). **Never** `#[derive(Clone, Copy,
PartialEq, …)]` on it — derives add `T: Clone`/`T: PartialEq` bounds, so `Id<User>` isn't `Copy`
unless `User` is. Implement the traits manually.

```rust
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

pub struct Id<T> {
    raw: u64,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    pub const fn new(raw: u64) -> Self {
        Id { raw, _marker: PhantomData }
    }
    pub const fn get(self) -> u64 {
        self.raw
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Id<T> {}
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl<T> Eq for Id<T> {}
impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}
impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.raw)
    }
}

pub struct User; // not Clone, not Copy — Id<User> still is
pub struct Order;

fn owner_of(order: Id<Order>) -> Id<User> {
    Id::new(order.get()) // explicit conversion; Id<Order> is not an Id<User>
}
```

### TYPE-11: Carry units in types

**Default:** time is `std::time::Duration`/`Instant`, never `u64` seconds or `f64` millis. For other
units (bytes, meters, cents) use a newtype per unit with explicit conversion methods
(`Bytes::from_kib`). **Never** name a parameter `timeout_ms: u64` in new code — take `Duration`.

### TYPE-12: Use const generics for sizes that are part of the type

**Default:** const generics for fixed-size buffers, matrices, and arrays (`[T; N]`,
`fn checksum<const N: usize>(block: &[u8; N])`). Since 1.89 the `_` placeholder can infer const
arguments (`let buf: [u8; 4] = [0; _];`). **Never** use const generics for runtime configuration values
(retry counts, limits) — each distinct value monomorphises a new copy.

```rust
pub struct RingBuffer<T, const N: usize> {
    items: [Option<T>; N],
    head: usize,
}

impl<T, const N: usize> RingBuffer<T, N> {
    pub fn new() -> Self {
        RingBuffer { items: [const { None }; N], head: 0 }
    }
    pub fn push(&mut self, item: T) -> Option<T> {
        let old = self.items[self.head].replace(item);
        self.head = (self.head + 1) % N;
        old
    }
}

impl<T, const N: usize> Default for RingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}
```

## Review checklist

- [ ] IDs, money, emails, units are newtypes, not bare `u64`/`String`/`f64` (TYPE-01, TYPE-11)
- [ ] Validated types have private fields and a single fallible constructor (TYPE-02)
- [ ] No `Deref` on newtypes; no `Default` where the empty value is invalid (TYPE-03)
- [ ] Serde goes through `try_from` for validated types (TYPE-04)
- [ ] No unexplained `bool` params; no sentinel `-1`/`""` values (TYPE-05, TYPE-06)
- [ ] No struct-of-`Option`s + status field; states are enum variants (TYPE-07)
- [ ] No `_` arm when matching own enums (TYPE-08)
- [ ] Compile-time protocols use typestate; runtime states use enums (TYPE-09)
- [ ] Generic `Id<T>` impls are hand-written, marker is `PhantomData<fn() -> T>` (TYPE-10)
- [ ] Const generics only for type-level sizes (TYPE-12)
