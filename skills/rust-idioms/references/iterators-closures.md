---
id: idioms/iterators-closures
title: Iterators and closures
summary: >-
  When to use iterator chains vs `for` loops, how to collect into `Result`/`Option`, avoid needless
  `collect`, return and accept iterators, write custom iterators, which std/itertools adaptors to reach
  for, and how to choose `Fn`/`FnMut`/`FnOnce` and `move` for closures.
area: idioms
tags: [iterators, closures, collect, itertools, fn-traits, move, loops, adaptors]
rust: "1.96"
edition: "2024"
crates:
  itertools: "0.15"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/std/iter/trait.Iterator.html
  - https://doc.rust-lang.org/std/iter/index.html
  - https://doc.rust-lang.org/book/ch13-00-functional-features.html
  - https://docs.rs/itertools/0.15
  - https://doc.rust-lang.org/edition-guide/rust-2021/disjoint-capture-in-closures.html
---

# Iterators and closures

Decides: loop vs iterator chain, how to move data through `collect`, what to accept and return, and
which closure trait a parameter should use. Read when writing data-transformation code, a function that
takes a callback, or a custom collection. Async closures (`async ||`, `AsyncFn*`) are in `async.md`.

## Loops vs iterator chains

### ITER-01: Iterate over elements, not indices

**Default:** `for x in &items`, `.iter().enumerate()` when you need the index, `.zip()` to walk two
sequences together, `.windows(2)` / `.array_windows()` (1.94+) for neighbours. **Never** write
`for i in 0..v.len() { v[i] }` (clippy `needless_range_loop`) — it is slower (bounds checks), and
panics when lengths disagree.

```rust
pub fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn increasing_steps(v: &[i32]) -> usize {
    v.windows(2).filter(|w| w[1] > w[0]).count()
}

pub fn label(items: &[&str]) -> Vec<String> {
    items.iter().enumerate().map(|(i, s)| format!("{i}: {s}")).collect()
}
```

### ITER-02: Use a `for` loop for side effects and complex control flow

**Default:** iterator chains for transformations that produce a value (`map`/`filter`/`collect`/`sum`).
**Use** a plain `for` loop when the body does I/O, mutates several things, uses `?`, `break` with
state, or `.await`. **Never** contort logic into `for_each` with captured mutable state, or nest
`fold` five levels deep to avoid a loop. Readability beats "functional style".

```rust
use std::io::{self, Write};

pub fn write_report(out: &mut impl Write, rows: &[(String, u32)]) -> io::Result<u32> {
    let mut total = 0;
    for (name, count) in rows {
        if *count == 0 {
            continue;
        }
        writeln!(out, "{name}: {count}")?; // `?` inside a loop: fine and clear
        total += count;
    }
    Ok(total)
}
```

### ITER-03: Know `iter()`, `iter_mut()`, `into_iter()`

| Call | Yields | Source after |
|---|---|---|
| `v.iter()` / `for x in &v` | `&T` | still usable |
| `v.iter_mut()` / `for x in &mut v` | `&mut T` | still usable, modified |
| `v.into_iter()` / `for x in v` | `T` | moved (consumed) |

**Default:** borrow (`&v`) unless you are done with `v` and need owned elements. Arrays (`[T; N]`,
edition 2021+) and `Box<[T]>` (edition 2024) iterate by value with `.into_iter()`. **Never** write
`v.clone().into_iter()` or `for x in v.clone()` to keep `v` — iterate `&v`. Use `.copied()` for `Copy`
elements and put `.cloned()` *after* filtering so you clone only what you keep.

```rust
pub fn long_names(names: &[String]) -> Vec<String> {
    names.iter().filter(|n| n.len() > 8).cloned().collect() // clone only survivors
}

pub fn max_score(scores: &[u32]) -> Option<u32> {
    scores.iter().copied().max()
}
```

## Collecting

### ITER-04: Collect fallible iterators into `Result<Vec<_>, E>` or `Option<Vec<_>>`

**Default:** `.map(parse).collect::<Result<Vec<_>, _>>()?` — stops at the first error. Same for `sum`
/ `product` over `Result`s, and for `Option`. **Use** `partition` / `filter_map(Result::ok)` only when
you deliberately want to keep going past failures (and then log or count the failures). **Never**
`unwrap` inside `map`, or build a `Vec<Result<T, E>>` and loop over it to find errors.

```rust
use std::num::ParseIntError;

pub fn parse_all(fields: &[&str]) -> Result<Vec<u32>, ParseIntError> {
    fields.iter().map(|f| f.trim().parse::<u32>()).collect()
}

pub fn total(fields: &[&str]) -> Result<u64, ParseIntError> {
    fields.iter().map(|f| f.parse::<u64>()).sum()
}

pub fn split_valid(fields: &[&str]) -> (Vec<u32>, usize) {
    let (ok, bad): (Vec<_>, Vec<_>) = fields.iter().map(|f| f.parse::<u32>()).partition(Result::is_ok);
    (ok.into_iter().flatten().collect(), bad.len())
}
```

### ITER-05: Don't `collect` just to iterate, count, or test again

**Default:** keep the chain lazy until the final consumer: `.count()`, `.any(..)`, `.next().is_none()`,
`.sum()`, `.last()`. **Use** an intermediate `collect` only when you need the data twice, need random
access, or must end a borrow. **Never** write `.collect::<Vec<_>>().len()` or
`.collect::<Vec<_>>().into_iter()` (clippy `needless_collect`).

```rust
// ❌ allocates a Vec only to throw it away
pub fn has_admin(users: &[(String, bool)]) -> bool {
    users.iter().filter(|(_, admin)| *admin).collect::<Vec<_>>().len() > 0
}
```

```rust
pub fn has_admin(users: &[(String, bool)]) -> bool {
    users.iter().any(|(_, admin)| *admin)
}
```

### ITER-06: Grow collections with `extend`, `collect`, or `with_capacity`

**Default:** `vec.extend(iter)` to append; `collect()` to build; `Vec::with_capacity(n)` when you
push in a loop with a known bound. **Never** push in a loop into an unsized `Vec::new()` when the final
length is known, or `for x in other { v.push(x) }` instead of `v.extend(other)`.

## Accepting and returning iterators

### ITER-07: Accept `impl IntoIterator`, return `impl Iterator`

**Default:** parameters that only iterate take `impl IntoIterator<Item = T>` (callers pass a `Vec`,
array, slice iterator, or chain). Functions that produce a sequence the caller will consume once return
`impl Iterator<Item = T>` instead of materialising a `Vec`. **Use** `Vec<T>` returns when the caller
needs indexing or multiple passes, or when the function must release a lock/borrow before returning.
**Never** return `Box<dyn Iterator>` when a single concrete chain is returned (see `traits-generics.md`,
TRAIT-13 for capture rules).

```rust
pub fn sum_lengths<I, S>(items: I) -> usize
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    items.into_iter().map(|s| s.as_ref().len()).sum()
}

pub fn evens(limit: u32) -> impl Iterator<Item = u32> {
    (0..limit).filter(|n| n % 2 == 0)
}

fn demo() -> usize {
    sum_lengths(["a", "bc"]) + sum_lengths(vec![String::from("def")]) + evens(10).count()
}
```

### ITER-08: Write custom iterators with `from_fn` / `successors` first

**Default:** for simple generators use `std::iter::from_fn` (stateful closure) or `std::iter::successors`
(each item derived from the previous). **Use** a named struct implementing `Iterator` when the iterator
is public API, needs extra methods, or must implement `DoubleEndedIterator` / `ExactSizeIterator`. When
you implement `Iterator`, override `size_hint` if you know the length. **Never** implement a custom
`next_item()` method on a collection instead of `Iterator`/`IntoIterator`.

```rust
use std::iter;

pub fn powers_of_two() -> impl Iterator<Item = u64> {
    iter::successors(Some(1u64), |&n| n.checked_mul(2)) // stops on overflow
}

pub fn fibonacci() -> impl Iterator<Item = u64> {
    let mut state = (0u64, 1u64);
    iter::from_fn(move || {
        let next = state.0;
        state = (state.1, state.0.checked_add(state.1)?);
        Some(next)
    })
}

#[test]
fn generators() {
    assert_eq!(powers_of_two().nth(10), Some(1024));
    assert_eq!(fibonacci().take(6).collect::<Vec<_>>(), [0, 1, 1, 2, 3, 5]);
}
```

### ITER-09: Implement `IntoIterator for &YourCollection`

**Default:** a collection type exposes `iter()` returning a named or `impl` iterator, and implements
`IntoIterator` for `&Coll` (and `&mut Coll` / `Coll` if meaningful) so `for x in &coll` works.

```rust
pub struct Playlist {
    tracks: Vec<String>,
}

impl Playlist {
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

## Adaptors worth knowing

### ITER-10: Reach for the specific std adaptor before a manual fold

| Intent | Use |
|---|---|
| filter + map in one step | `filter_map` |
| first match, transformed | `find_map` |
| index of first match | `position` |
| any / all | `any`, `all` (short-circuit) |
| max/min by field | `max_by_key`, `min_by_key`; `max_by(f64::total_cmp)` for floats |
| early-exit accumulation with errors | `try_fold`, `try_for_each` |
| keep a prefix while predicate holds / map until `None` | `take_while`, `map_while` |
| split into two collections | `partition`, `unzip` |
| group consecutive runs of a slice | `slice::chunk_by` |
| fixed-size neighbour arrays | `array_windows::<N>()` (1.94+), `windows(n)` |
| remove matching elements and get them | `Vec::extract_if` (1.87+), `HashMap::extract_if` (1.88+) |
| remove matching elements, discard them | `retain`, `retain_mut` |
| pop the last element if it matches | `Vec::pop_if` (1.86+) |
| repeat a value n times | `iter::repeat_n` (1.82+) |
| check ordering | `is_sorted`, `is_sorted_by_key` (1.82+) |
| look ahead one element | `peekable` + `next_if` / `next_if_eq` |

**Never** implement `max` of floats with `partial_cmp(..).unwrap()` — use `total_cmp`.

```rust
pub fn drain_expired(sessions: &mut Vec<(u64, String)>, now: u64) -> Vec<(u64, String)> {
    sessions.extract_if(.., |(expires, _)| *expires <= now).collect()
}

pub fn warmest(temps: &[f64]) -> Option<f64> {
    temps.iter().copied().max_by(f64::total_cmp)
}

pub fn runs(v: &[i32]) -> Vec<&[i32]> {
    v.chunk_by(|a, b| a == b).collect()
}

pub fn first_port(lines: &[&str]) -> Option<u16> {
    lines.iter().find_map(|l| l.strip_prefix("port=")?.parse().ok())
}
```

### ITER-11: Add `itertools` for what std lacks, not for what it has

**Default:** std first. **Use** `itertools` (0.15) for `join`, `sorted_by_key`, `tuple_windows`,
`interleave`, `unique`, `cartesian_product`, `multi_cartesian_product`, `kmerge`, `positions`,
`exactly_one`, `process_results`. **Never** add it for `chunk_by`/`windows`/`is_sorted`/`repeat_n`
which std now has, and avoid itertools method names that shadow newer std methods — call the std one.

```rust
use itertools::Itertools;

pub fn csv_line(fields: &[u32]) -> String {
    fields.iter().join(",")
}

pub fn pairs(v: &[i32]) -> Vec<(i32, i32)> {
    v.iter().copied().tuple_windows().collect()
}
```

### ITER-12: Iterators are lazy — consume them

**Default:** a chain does nothing until a consumer (`for`, `collect`, `sum`, `count`, `for_each`)
runs it. **Never** use `map` for side effects (`v.iter().map(|x| println!("{x}"));` does nothing and
triggers `unused_must_use`); use a `for` loop.

## Closures

### ITER-13: Take the least restrictive closure trait the caller can satisfy

| Called | Parameter bound | Caller may pass |
|---|---|---|
| exactly once | `impl FnOnce() -> T` | anything, including closures that move out captures |
| many times, sequentially | `impl FnMut(Item)` | closures that mutate captured state |
| many times, possibly shared/concurrently | `impl Fn(Item)` (+ `Send + Sync` for threads) | only closures that don't mutate |

**Default:** `FnOnce` for "run once" hooks (`unwrap_or_else`, `get_or_insert_with`), `FnMut` for
visitors and callbacks invoked in a loop, `Fn` for shared handlers. **Use** `Box<dyn Fn(..) + Send + Sync>`
to *store* heterogeneous callbacks. **Never** demand `Fn` when you call it once — it rejects valid
closures that move out of their captures.

```rust
pub fn retry<T, E>(mut attempts: u32, mut op: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) if attempts <= 1 => return Err(e),
            Err(_) => attempts -= 1,
        }
    }
}

type Handler = Box<dyn Fn(&str) + Send + Sync>;

pub struct EventBus {
    handlers: Vec<Handler>,
}

impl EventBus {
    pub fn subscribe(&mut self, f: impl Fn(&str) + Send + Sync + 'static) {
        self.handlers.push(Box::new(f));
    }

    pub fn publish(&self, event: &str) {
        for h in &self.handlers {
            h(event);
        }
    }
}
```

### ITER-14: Use `move` when the closure outlives the current frame; clone before moving

**Default:** closures passed to `thread::spawn`, `tokio::spawn`, or returned from functions need
`move`. To share an `Arc` with a moved closure, clone it into a new binding first. Since edition 2021
closures capture disjoint fields (`self.a` rather than all of `self`), so borrowing one field while
another is captured works. **Never** clone the whole struct to satisfy a `move` closure that needs one
field.

```rust
use std::sync::Arc;
use std::thread;

pub fn spawn_workers(config: &Arc<Vec<String>>) -> Vec<thread::JoinHandle<usize>> {
    (0..4)
        .map(|i| {
            let config = Arc::clone(config); // cheap refcount bump, then move it
            thread::spawn(move || config.iter().filter(|s| s.len() > i).count())
        })
        .collect()
}
```

### ITER-15: Pass functions and methods directly instead of wrapping them

**Default:** `.map(str::trim)`, `.map(String::as_str)`, `.map(ToString::to_string)`,
`.filter_map(Result::ok)`, `.for_each(drop)`. **Never** write `.map(|s| s.trim())` style redundant
closures in new code (clippy `redundant_closure_for_method_calls`, pedantic).

```rust
pub fn clean(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).map(str::trim).filter(|s| !s.is_empty()).collect()
}
```

## Review checklist

- [ ] No `for i in 0..v.len()` index loops; `zip`/`enumerate`/`windows` instead (ITER-01).
- [ ] Side-effecting or `?`/`await`-heavy logic uses `for` loops, not `for_each`/`fold` contortions (ITER-02).
- [ ] No `.clone().into_iter()`; `.cloned()` after `filter`; `.copied()` for `Copy` (ITER-03).
- [ ] Fallible maps collect into `Result<Vec<_>, _>`; no `unwrap` in `map` (ITER-04).
- [ ] No `collect` followed by `len`/`is_empty`/`into_iter` (ITER-05).
- [ ] Functions accept `impl IntoIterator` and return `impl Iterator` where streaming fits (ITER-07).
- [ ] Custom sequences implement `Iterator`/`IntoIterator for &T` (ITER-08, ITER-09).
- [ ] Specific adaptors (`find_map`, `extract_if`, `chunk_by`, `total_cmp`) over manual loops/folds (ITER-10).
- [ ] `itertools` only for what std lacks (ITER-11).
- [ ] No `map` used for side effects (ITER-12).
- [ ] Closure parameters use `FnOnce`/`FnMut`/`Fn` minimally; stored callbacks are `Box<dyn Fn + Send + Sync>` (ITER-13).
- [ ] `move` closures clone `Arc`s into fresh bindings first (ITER-14).
