---
id: idioms/ownership-borrowing
title: Ownership and borrowing
summary: >-
  How to choose parameter and return types (borrow by default, own when storing), how to satisfy the
  borrow checker by restructuring instead of cloning, when lifetimes belong in structs, and how to pick
  between Box, Rc, RefCell, Cell and Cow.
area: idioms
tags: [ownership, borrowing, lifetimes, clone, cow, rc, refcell, box, borrow-checker]
rust: "1.96"
edition: "2024"
crates: {}
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html
  - https://rust-lang.github.io/api-guidelines/flexibility.html
  - https://doc.rust-lang.org/std/borrow/enum.Cow.html
  - https://doc.rust-lang.org/nomicon/lifetime-elision.html
  - https://doc.rust-lang.org/std/primitive.slice.html#method.get_disjoint_mut
---

# Ownership and borrowing

Decides: what a function should take and return, how to fix borrow-checker errors without `.clone()`,
when a struct may hold references, and which single-threaded smart pointer (if any) to use.
Read when writing function signatures, fighting E0499/E0502/E0505/E0506, or reaching for `Rc<RefCell<_>>`.
Thread-shared ownership (`Arc`, `Mutex`) is in `concurrency.md`.

## Parameter types

### OWN-01: Borrow the most general slice type in parameters

**Default:** take `&str`, `&[T]`, `&Path`, `&T`. **Use** `impl AsRef<Path>` / `impl AsRef<str>` only for
public convenience APIs that are called with many argument types. **Never** take `&String`, `&Vec<T>`,
`&PathBuf` or `&Box<T>` — they force callers to allocate and give you nothing extra (clippy `ptr_arg`).

```rust
use std::path::Path;

pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

pub fn total(values: &[u64]) -> u64 {
    values.iter().sum()
}

pub fn read_config(path: impl AsRef<Path>) -> std::io::Result<String> {
    std::fs::read_to_string(path.as_ref())
}

fn demo() -> (usize, u64) {
    let (owned, v) = (String::from("a b"), (1..4).collect::<Vec<u64>>());
    // &String / &Vec<u64> deref-coerce to &str / &[u64]; literals and arrays work too
    (count_words(&owned) + count_words("lit"), total(&v) + total(&[1, 2]))
}
```

```rust
// ❌ forces callers to own a String / Vec; clippy::ptr_arg
fn count_words(text: &String) -> usize { text.split_whitespace().count() }
fn total(values: &Vec<u64>) -> u64 { values.iter().sum() }
```

### OWN-02: Take ownership only when you store or consume the value

**Default:** if the function keeps the value (pushes it into a struct, sends it on a channel, returns
it transformed), take it by value (`String`, `Vec<T>`) so the caller can move in without a copy.
**Use** `impl Into<String>` on constructors where callers commonly pass literals. **Never** take `&str`
and immediately `.to_string()` it inside when you store it — that hides a forced allocation and
prevents moving an existing `String` in.

```rust
pub struct User {
    name: String,
    tags: Vec<String>,
}

impl User {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), tags: Vec::new() }
    }

    pub fn add_tag(&mut self, tag: String) {
        self.tags.push(tag); // stored: take by value, caller moves in
    }

    pub fn name(&self) -> &str {
        &self.name // getters return borrows, not clones
    }
}
```

### OWN-03: Return owned values from constructors and transforms, borrows from getters

**Default:** getters return `&T` / `&str` / `&[T]`; functions that build something return an owned value.
**Use** `Option<&T>` for fallible lookups. **Never** return `String`/`Vec` clones from getters "to avoid
lifetime issues" — the caller can call `.to_owned()` if they need ownership.

## Fixing borrow-checker errors without cloning

### OWN-04: Restructure before you clone

**Default:** when the borrow checker complains, shrink the borrow (compute what you need, end the borrow,
then mutate), split the struct borrow, or use `std::mem::take`. **Use** `.clone()` only when you truly
need two independent owners, or the type is cheap (`Copy`, `Arc`, small IDs). **Never** sprinkle
`.clone()` until it compiles — it is the most common LLM Rust smell and often hides a logic bug.

```rust
// ❌ clone to escape "cannot borrow `*self` as mutable because it is also borrowed as immutable"
struct Inventory { items: Vec<String>, log: Vec<String> }
impl Inventory {
    fn audit(&mut self) { for item in self.items.clone() { self.record(&item); } }
    fn record(&mut self, s: &str) { self.log.push(s.to_owned()); }
}
```

```rust
struct Inventory {
    items: Vec<String>,
    log: Vec<String>,
}

impl Inventory {
    // ✅ borrow disjoint fields directly: the compiler tracks fields separately inside one fn body
    fn audit(&mut self) {
        for item in &self.items {
            self.log.push(format!("seen {item}"));
        }
    }
}
```

### OWN-05: Borrow disjoint fields directly; don't route through `&mut self` methods

**Default:** inside a method, access `self.a` and `self.b` directly — the borrow checker understands
disjoint fields within one function body, but not across method calls (`self.get_a()` borrows all of
`self`). **Use** a free function or an associated fn taking the individual fields (`fn step(a: &mut A,
b: &B)`) when the logic must be shared. **Never** make a helper take `&mut self` if it only needs one field.

```rust
struct Sim {
    particles: Vec<f32>,
    gravity: f32,
}

impl Sim {
    fn tick(&mut self) {
        // helper takes only what it needs, so `self.particles` and `self.gravity` can be borrowed together
        apply(&mut self.particles, self.gravity);
    }
}

fn apply(particles: &mut [f32], gravity: f32) {
    for p in particles {
        *p += gravity;
    }
}
```

### OWN-06: Move values out of `&mut` with `mem::take` / `mem::replace` / `Option::take`

**Default:** to move a field out of `&mut self` (state machines, draining buffers), use
`std::mem::take` (type is `Default`), `std::mem::replace`, or `Option::take`. **Never** clone the field
and then clear it.

```rust
use std::mem;

enum State {
    Idle,
    Running { buffer: Vec<u8> },
    Done(Vec<u8>),
}

struct Machine {
    state: State,
    pending: Vec<u8>,
}

impl Machine {
    fn flush(&mut self) -> Vec<u8> {
        mem::take(&mut self.pending) // leaves an empty Vec, no allocation
    }

    fn finish(&mut self) {
        // take ownership of the old state to build the new one
        self.state = match mem::replace(&mut self.state, State::Idle) {
            State::Running { buffer } => State::Done(buffer),
            other => other,
        };
    }
}
```

### OWN-07: Use `get_disjoint_mut`, `split_at_mut` or `swap` for two `&mut` into one collection

**Default:** `slice::get_disjoint_mut([i, j])` (Rust 1.86+) or `split_at_mut` for two mutable elements;
`slice::swap` to exchange. `HashMap::get_disjoint_mut` (1.86+) for maps. **Never** use `unsafe` raw
pointers or index-then-clone to work around E0499.

```rust
use std::slice::GetDisjointMutError;

/// Errors if either index is out of bounds or `from == to` (overlapping borrows).
fn transfer(balances: &mut [i64], from: usize, to: usize, amount: i64) -> Result<(), GetDisjointMutError> {
    let [a, b] = balances.get_disjoint_mut([from, to])?;
    *a -= amount;
    *b += amount;
    Ok(())
}
```

### OWN-08: Use the entry API instead of lookup-then-insert

**Default:** `map.entry(k).or_default()` / `.or_insert_with(..)` / `.and_modify(..)`. **Never** do
`if !map.contains_key(&k) { map.insert(k, v) }` followed by `map.get_mut(&k).unwrap()` — two lookups
and an `unwrap` (clippy `map_entry`).

```rust
use std::collections::HashMap;

fn index_words(text: &str) -> HashMap<&str, Vec<usize>> {
    let mut index: HashMap<&str, Vec<usize>> = HashMap::new();
    for (pos, word) in text.split_whitespace().enumerate() {
        index.entry(word).or_default().push(pos);
    }
    index
}
```

## Lifetimes

### OWN-09: Let elision work; annotate only when the compiler asks

**Default:** write no lifetimes on functions; elision covers "one input reference" and "`&self`
methods". **Use** a named lifetime when a returned reference is tied to one of several inputs.
Write `'_` for elided lifetimes in type paths (`Formatter<'_>`, `Iter<'_, T>`) — since Rust 1.89 the
warn-by-default `mismatched_lifetime_syntaxes` lint flags hiding a lifetime in one place and naming it
in another. **Never** add `'a` everywhere "to be safe", and never reach for `'static` to silence an error.

```rust
// Two inputs: say which one the output borrows from.
fn longest<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.len() >= b.len() { a } else { b }
}

// Output borrows only from `haystack`; `needle` gets its own anonymous lifetime.
fn after<'h>(haystack: &'h str, needle: &str) -> Option<&'h str> {
    haystack.find(needle).map(|i| &haystack[i + needle.len()..])
}

struct Wrapper(Vec<u8>);

impl std::fmt::Display for Wrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes", self.0.len())
    }
}
```

### OWN-10: Put lifetimes in structs only for short-lived views

**Default:** structs own their data (`String`, `Vec<T>`). **Use** a lifetime-parameterised struct
(`Parser<'a> { input: &'a str }`) for short-lived views, iterators, and zero-copy parsers whose lifetime
is obviously scoped to a caller's buffer. **Never** put references into long-lived structs (app state,
config, anything sent to another thread or task) — the lifetime infects every user. Own the data, or
share it with `Arc<str>` / `Arc<T>`.

```rust
/// Zero-copy tokenizer: a view over the caller's buffer.
pub struct Tokens<'a> {
    rest: &'a str,
}

impl<'a> Tokens<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { rest: input }
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let s = self.rest.trim_start();
        if s.is_empty() {
            return None;
        }
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        let (tok, rest) = s.split_at(end);
        self.rest = rest;
        Some(tok)
    }
}
```

### OWN-11: Replace self-referential structs with indices or owned handles

**Default:** store data in a `Vec` and refer to it by index (or a newtype ID); for graphs and trees,
use an arena of nodes with `usize`/ID edges. **Use** `Rc`/`Weak` for genuinely shared, cyclic ownership
only when indices do not fit. **Never** try to store a value and a reference into it in the same struct,
and never reach for `unsafe` or `Pin` to force it (outside async internals).

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NodeId(usize);

pub struct Tree {
    nodes: Vec<Node>, // the arena; edges are NodeIds, not references
}

struct Node { value: String, parent: Option<NodeId>, children: Vec<NodeId> }

impl Tree {
    pub fn add(&mut self, value: impl Into<String>, parent: Option<NodeId>) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node { value: value.into(), parent, children: Vec::new() });
        if let Some(p) = parent {
            self.nodes[p.0].children.push(id);
        }
        id
    }

    pub fn parent_value(&self, id: NodeId) -> Option<&str> {
        let parent = self.nodes.get(id.0)?.parent?;
        Some(&self.nodes[parent.0].value)
    }
}
```

## Smart pointers and interior mutability

### OWN-12: Pick the smallest pointer that expresses the ownership

| Need | Use |
|---|---|
| Single owner, value on stack | `T` |
| Single owner, recursive type / large value / trait object | `Box<T>`, `Box<dyn Trait>` |
| Immutable shared string/slice, cheap clone | `Rc<str>` / `Arc<str>`, `Rc<[T]>` / `Arc<[T]>` |
| Shared ownership, one thread | `Rc<T>` |
| Shared ownership, across threads | `Arc<T>` (see `concurrency.md`) |
| Mutate a `Copy` value through `&self` | `Cell<T>` |
| Mutate a non-`Copy` value through `&self`, one thread | `RefCell<T>` |
| Shared + mutable, one thread | `Rc<RefCell<T>>` — last resort |

**Default:** plain owned values and `&`/`&mut` borrows. **Never** use `Rc<RefCell<T>>` to model a
design that `&mut` plus restructuring (OWN-04..OWN-06) or indices (OWN-11) can express. It trades
compile-time checks for runtime panics (`BorrowMutError`) and makes code harder to reason about.

### OWN-13: Keep `RefCell` borrows short and never across calls that may re-borrow

**Default:** borrow, do the work, drop the guard in the same statement or block. **Use**
`try_borrow_mut` only when re-entrancy is expected and handled. **Never** hold a `Ref`/`RefMut` while
calling callbacks or methods on the same object — that is how `already borrowed: BorrowMutError`
panics happen.

```rust
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct Cache {
    hits: Cell<u64>,
    entries: RefCell<Vec<String>>,
}

impl Cache {
    fn record(&self, entry: &str) {
        self.hits.set(self.hits.get() + 1); // Cell: no guard at all
        self.entries.borrow_mut().push(entry.to_owned()); // guard dropped at end of statement
    }
}
```

### OWN-14: `Box<T>` is for indirection, not for "heap is faster"

**Default:** `Box` for recursive enums, trait objects, and moving very large values cheaply. **Never**
`Box` small structs, `Vec`s or `String`s (already heap-backed); `Box<Vec<T>>` is a double indirection
(clippy `box_collection`).

## Cow: owned-or-borrowed values

### OWN-15: Return `Cow<'_, str>` when you usually borrow but sometimes must allocate

**Default:** return `&str` if you never modify, `String` if you always build. **Use** `Cow<'a, str>` (or
`Cow<'a, [T]>`) when most inputs pass through unchanged and only some need an owned, modified copy —
escaping, normalisation, trimming with replacement. **Never** use `Cow` in struct fields of long-lived
types just to avoid deciding ownership; own the data there.

```rust
use std::borrow::Cow;

/// Escapes `<` and `>`; allocates only when the input contains them.
pub fn escape_html(input: &str) -> Cow<'_, str> {
    if !input.contains(['<', '>']) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len() + 8);
    for c in input.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    Cow::Owned(out)
}

#[test]
fn escape_borrows_when_clean() {
    assert!(matches!(escape_html("plain"), Cow::Borrowed(_)));
    assert_eq!(escape_html("<b>"), "&lt;b&gt;");
}
```

## Copy, Clone and moves

### OWN-16: Derive `Copy` for small plain-data types; make clones visible elsewhere

**Default:** derive `Copy` (with `Clone`) for small, plain-data types — IDs, coordinates, flags,
fieldless enums — so they move freely without `.clone()` noise. **Never** derive `Copy` on types that
own resources or are large (> ~32 bytes is a smell), and never implement `Clone` with side effects.
When a clone is intentional and cheap (an `Arc`), write `Arc::clone(&x)` to make that explicit.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

pub fn manhattan(a: Point, b: Point) -> u32 {
    a.x.abs_diff(b.x) + a.y.abs_diff(b.y) // a and b are copied, still usable by caller
}
```

## Review checklist

- [ ] No `&String`, `&Vec<T>`, `&PathBuf`, `&Box<T>` parameters (OWN-01).
- [ ] Values that are stored are taken by value; getters return borrows (OWN-02, OWN-03).
- [ ] Every `.clone()` has a reason: two owners needed or the type is cheap (OWN-04, OWN-16).
- [ ] Borrow conflicts solved by field splitting, `mem::take`, `get_disjoint_mut`, or the entry API (OWN-05..OWN-08).
- [ ] No gratuitous lifetime annotations or `'static` to silence errors; `'_` in elided type paths (OWN-09).
- [ ] Long-lived structs own their data; references only in short-lived views (OWN-10).
- [ ] No self-referential structs; graphs use indices/arenas (OWN-11).
- [ ] `Rc<RefCell<_>>` only with a written justification; `RefCell` guards are short (OWN-12, OWN-13).
- [ ] No `Box<Vec<_>>` / `Box<String>` (OWN-14).
- [ ] `Cow` used for mostly-borrowed returns, not as a struct-field escape hatch (OWN-15).
