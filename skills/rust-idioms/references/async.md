---
id: idioms/async
title: Async Rust
summary: >-
  Tokio as the default runtime, never blocking the executor or holding locks across .await,
  structured concurrency with JoinSet/TaskTracker/CancellationToken, cancellation-safe select!,
  bounded channels, and native async fn in traits (trait-variant for Send, async-trait only for dyn).
area: idioms
tags: [async, tokio, futures, send, cancellation, select, joinset, async-fn-in-trait, async-closures, backpressure]
rust: "1.96"
edition: "2024"
crates:
  tokio: "1.53"
  tokio-util: "0.7"
  futures: "0.3"
  trait-variant: "0.1"
  async-trait: "0.1"
  rayon: "1.12"
  anyhow: "1.0"
verified: 2026-09-29
sources:
  - https://docs.rs/tokio/latest/tokio/macro.select.html
  - https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/
  - https://ryhl.io/blog/async-what-is-blocking/
  - https://ryhl.io/blog/actors-with-tokio/
---

# Async Rust

Decides how to write async code that is correct under a multi-threaded work-stealing runtime:
what may run on the executor, how tasks are owned and shut down, how cancellation interacts with
`select!`, and how to express async traits and closures on Rust 1.96. Read before writing any
`async fn`, `tokio::spawn`, `select!` or async trait. Thread-based concurrency is in `concurrency.md`.

## Runtime and where async belongs

### ASYNC-01: Use one Tokio runtime per process, created at the binary edge
**Default:** `#[tokio::main]` in `main.rs` (multi-thread flavor); `#[tokio::main(flavor = "current_thread")]`
for small CLIs. **Use** `tokio::runtime::Builder` only when you need to tune worker threads or run
the runtime on a dedicated thread. **Never** create a runtime inside a library, mix runtimes
(`async-std` is discontinued; `smol` futures do not drive Tokio IO), or build a new runtime per request.
Libraries take `async fn` and let the caller's runtime drive them. Crate choice: rust-ecosystem.

### ASYNC-02: Keep pure and CPU-bound code synchronous
**Default:** only functions that await IO, timers or channels are `async`. Parsing, validation,
formatting and business rules stay plain `fn` and are called from async code. **Never** mark a
function `async` "for consistency" — it makes the function uncallable from sync code and tests, adds
a state machine, and hides no concurrency. An `async fn` with no `.await` is a smell
(`clippy::unused_async`, pedantic).

### ASYNC-03: Never call `block_on` from inside the runtime
**Default:** stay async all the way down. **Use** `Handle::block_on` / `Runtime::block_on` only on
threads the runtime does not own (e.g. a sync callback thread, `spawn_blocking` body). **Never** call
`futures::executor::block_on` or `Runtime::block_on` inside an async context: it either panics
("Cannot start a runtime from within a runtime") or deadlocks a worker thread.

## Blocking the executor

### ASYNC-04: Move blocking and CPU-heavy work off the async workers
**Default:** a task should reach an `.await` every ~10–100 µs. **Use** `tokio::fs` / async clients for
IO; `tokio::task::spawn_blocking` for blocking syscalls, sync libraries and short CPU bursts;
`rayon` (bridged with a `oneshot`) for parallel CPU work. **Never** call `std::thread::sleep`,
`std::fs`, `reqwest::blocking`, sync DB drivers, or long loops directly in an async fn: one blocked
worker stalls every task scheduled on it, and on `current_thread` it stalls the whole program.

```rust
use tokio::sync::oneshot;

/// Blocking library call or short CPU burst: dedicated blocking pool.
pub async fn digest(data: Vec<u8>) -> anyhow::Result<u64> {
    let sum = tokio::task::spawn_blocking(move || data.iter().map(|&b| u64::from(b)).sum()).await?;
    Ok(sum)
}

/// Data-parallel CPU work: rayon's pool, result sent back over a oneshot.
pub async fn parallel_sum(data: Vec<u64>) -> anyhow::Result<u64> {
    let (tx, rx) = oneshot::channel();
    rayon::spawn(move || {
        use rayon::prelude::*;
        let sum = data.par_iter().sum();
        let _ = tx.send(sum); // receiver dropped = caller was cancelled; nothing to do
    });
    Ok(rx.await?)
}
```

`spawn_blocking` tasks can't be cancelled; give work that runs forever a dedicated `std::thread`.

## Locks, `Send` and `.await`

### ASYNC-05: Never hold a std/parking_lot guard across `.await`
**Default:** `std::sync::Mutex` for shared state in async code, locked for a few statements and
released before the next `.await`. **Use** `tokio::sync::Mutex` only when the guard must be held
across an `.await` (exclusive use of an async resource) — and prefer an actor task owning the
resource (ASYNC-13) even then. **Never** keep a `MutexGuard` alive over an `.await`: the future
becomes `!Send` (won't spawn) and, with an async mutex, every caller serialises behind slow IO.
`clippy::await_holding_lock` (on by default) flags std/parking_lot guards held across `.await`.

```rust
// ❌ guard lives across .await: "future cannot be sent between threads safely"
async fn record_bad(state: &std::sync::Mutex<Vec<u32>>) {
    let mut hits = state.lock().expect("stats lock poisoned");
    let value = fetch().await;
    hits.push(value);
}
```

```rust
use std::sync::Mutex;

pub async fn record(hits: &Mutex<Vec<u32>>) {
    let value = fetch().await; // await first, with no lock held
    hits.lock().expect("stats lock poisoned").push(value); // guard dropped at end of statement
}

pub async fn drain(hits: &Mutex<Vec<u32>>) -> usize {
    let batch = std::mem::take(&mut *hits.lock().expect("stats lock poisoned")); // guard dropped here
    publish(&batch).await;
    batch.len()
}

async fn fetch() -> u32 { 7 }
async fn publish(_batch: &[u32]) {}
```

### ASYNC-06: Keep spawned futures `Send + 'static`
**Default:** everything alive across an `.await` in a spawned task is `Send`: `Arc` not `Rc`,
`Mutex` not `RefCell`, owned data (`String`, `Arc<str>`) not borrows of locals. **Use** `move`
blocks and clone the `Arc` *before* `tokio::spawn`. **Use** `tokio::task::LocalSet` / `spawn_local`
only for genuinely `!Send` state (e.g. some FFI handles). **Never** "fix" a `!Send` error with
`unsafe impl Send` or by switching to `current_thread` silently — drop or scope the value before the
`.await` instead. The compiler note "has type `Rc<..>` which is not `Send` ... value is used across
an await" names the offending variable.

```rust
use std::sync::Arc;

pub fn spawn_report(name: Arc<str>) -> tokio::task::JoinHandle<usize> {
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        name.len() // Arc<str> is Send + 'static; an Rc<str> or &str here would not compile
    })
}
```

## Task ownership and structured concurrency

### ASYNC-07: Every spawned task has an owner that joins it
**Default:** keep the `JoinHandle`, or spawn into a `tokio::task::JoinSet` (dynamic group; dropping
the set aborts its tasks) or a `tokio_util::task::TaskTracker` (long-lived server tasks you wait on
at shutdown). **Never** fire-and-forget `tokio::spawn(...)`: panics and errors vanish silently. Awaiting a `JoinHandle` yields
`Result<T, JoinError>`; handle the `JoinError` (panic or cancellation) separately from `T`'s error.

```rust
use tokio::task::JoinSet;

pub async fn fetch_all(ids: Vec<u64>) -> anyhow::Result<Vec<String>> {
    let mut set = JoinSet::new();
    for id in ids {
        set.spawn(fetch_one(id));
    }
    let mut out = Vec::with_capacity(set.len());
    while let Some(joined) = set.join_next().await {
        out.push(joined??); // outer ?: task panicked/cancelled; inner ?: fetch failed
    }
    Ok(out) // completion order, not input order
}

async fn fetch_one(id: u64) -> anyhow::Result<String> { Ok(format!("item-{id}")) }
```

### ASYNC-08: Bound fan-out; prefer in-task concurrency for IO
**Default:** a fixed number of futures → `tokio::join!` / `tokio::try_join!` (runs concurrently in
the current task, no `'static` needed). A collection → `futures::stream::iter(..).buffer_unordered(n)`
(or `buffered(n)` to keep order). **Use** `JoinSet` + `tokio::sync::Semaphore` when items need real
parallelism across workers. **Never** spawn one task per item of an unbounded input, and never
`join_all` 10 000 requests at once — you exhaust sockets and memory and get rate-limited.

```rust
use futures::stream::{self, StreamExt, TryStreamExt};

pub async fn fetch_bounded(ids: Vec<u64>) -> anyhow::Result<Vec<String>> {
    // at most 16 in flight, results in input order
    stream::iter(ids).map(fetch_one).buffered(16).try_collect().await
}

pub async fn profile_page(user: u64) -> anyhow::Result<(String, String)> {
    tokio::try_join!(fetch_one(user), fetch_one(user + 1))
}

async fn fetch_one(id: u64) -> anyhow::Result<String> { Ok(format!("item-{id}")) }
```

## Cancellation

### ASYNC-09: Treat every `.await` as a possible cancellation point
**Default:** assume any future can be dropped at any `.await` (timeouts, `select!`, aborted tasks,
a client disconnecting). Keep invariants valid across awaits: compute, then commit in one
synchronous step; write to a temp file and rename. **Never** leave shared state half-updated
between two awaits, and never rely on code after an `.await` to "clean up" — use a guard type with
`Drop` (drop runs on cancellation; async cleanup does not).

### ASYNC-10: Only put cancellation-safe futures in a looping `select!`
**Default:** in `loop { select! { .. } }`, each branch must be cancel-safe: `mpsc::Receiver::recv`,
`broadcast::Receiver::recv`, `watch::Receiver::changed`, `TcpListener::accept`,
`AsyncReadExt::read`, `StreamExt::next`, `interval.tick()`, `CancellationToken::cancelled`.
**Not** cancel-safe (data or progress lost when another branch wins): `read_exact`, `read_to_end`,
`read_to_string`, `write_all`, `tokio::sync::Mutex::lock` (loses queue position), most of your own
multi-step `async fn`s. **Use** a long-lived future created once *outside* the loop, pinned with
`std::pin::pin!`, and polled as `&mut fut` — or move the multi-step work into its own task.

```rust
use std::time::Duration;
use tokio::sync::mpsc;

pub async fn run(mut rx: mpsc::Receiver<String>, shutdown: impl Future<Output = ()>) -> Vec<String> {
    let mut shutdown = std::pin::pin!(shutdown); // created once, never restarted
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut buf = Vec::new();
    loop {
        tokio::select! {
            () = &mut shutdown => break,
            msg = rx.recv() => match msg {      // cancel-safe
                Some(m) => buf.push(m),
                None => break,                  // all senders gone
            },
            _ = tick.tick() => buf.clear(),     // cancel-safe
        }
    }
    buf
}
```

### ASYNC-11: Shut down cooperatively with `CancellationToken` + `TaskTracker`
**Default:** long-running tasks take a `tokio_util::sync::CancellationToken` (clone per task; child
tokens via `child_token()`) and exit their loop on `token.cancelled()`; the owner spawns them on a
`TaskTracker`, then `close()` + `wait()` at shutdown. **Use** `JoinHandle::abort` / dropping a
`JoinSet` only for tasks with no cleanup to do. **Never** implement shutdown with a global
`AtomicBool` polled in a sleep loop. Process-level signal handling and drain timeouts:
rust-architecture (services).

```rust
use std::time::Duration;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub async fn serve(shutdown: impl Future<Output = ()>) {
    let token = CancellationToken::new();
    let tracker = TaskTracker::new();
    for worker in 0..4 {
        let token = token.clone();
        tracker.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    () = token.cancelled() => break,
                    _ = tick.tick() => do_work(worker).await,
                }
            }
        });
    }
    tracker.close(); // no more spawns; wait() can now complete
    shutdown.await;
    token.cancel();
    tracker.wait().await;
}

async fn do_work(_worker: u32) {}
```

### ASYNC-12: Put a timeout on every await that crosses the network
**Default:** `tokio::time::timeout(d, fut)` around outbound calls (or the client's own timeout
setting), with the deadline chosen at the edge. **Never** await a remote peer, lock or channel
without an upper bound in server code — one stuck peer pins a task and its resources forever.

```rust
use anyhow::Context;
use std::time::Duration;

pub async fn fetch_with_deadline(id: u64) -> anyhow::Result<String> {
    tokio::time::timeout(Duration::from_secs(5), fetch_one(id))
        .await
        .with_context(|| format!("fetching {id} timed out"))?
}

async fn fetch_one(id: u64) -> anyhow::Result<String> { Ok(format!("item-{id}")) }
```

## Channels and backpressure

### ASYNC-13: Use bounded channels and pick the channel by message shape
**Default:** `tokio::sync::mpsc::channel(n)` — a full channel makes `send().await` wait, which is
your backpressure. **Use** `unbounded_channel` only when producers are naturally bounded (e.g. one
message per user action). Pick by shape:

| Need | Channel |
|---|---|
| many producers → one consumer (work queue, actor inbox) | `tokio::sync::mpsc` (bounded) |
| one reply to one request | `tokio::sync::oneshot` |
| every subscriber sees every event (lossy if slow) | `tokio::sync::broadcast` |
| subscribers only need the latest value (config, state) | `tokio::sync::watch` |
| sync thread ↔ async task | `mpsc` with `blocking_send` / `blocking_recv` on the sync side |

Prefer an actor (a task owning the state, fed by `mpsc`, answering via `oneshot`) over
`Arc<tokio::sync::Mutex<T>>` when the state is touched across awaits.

```rust
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

pub enum Command {
    Get { key: String, reply: oneshot::Sender<Option<String>> },
    Set { key: String, value: String },
}

pub fn spawn_store() -> (mpsc::Sender<Command>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel(64);
    let handle = tokio::spawn(async move {
        let mut map = HashMap::new();
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Get { key, reply } => {
                    let _ = reply.send(map.get(&key).cloned()); // requester may have given up
                }
                Command::Set { key, value } => {
                    map.insert(key, value);
                }
            }
        }
    }); // loop ends when every Sender is dropped: natural shutdown
    (tx, handle)
}
```

## Async traits and closures

### ASYNC-14: Use native `async fn` in traits; add `Send` bounds for public traits
**Default:** `async fn` / `-> impl Future` in traits (stable since 1.75) for static dispatch. For a
**public** trait whose futures callers will `tokio::spawn`, either declare
`fn f(&self) -> impl Future<Output = T> + Send` (impls may still write `async fn`) or generate a
`Send` variant with `#[trait_variant::make(Name: Send)]`; plain `async fn` in a `pub trait` triggers
the `async_fn_in_trait` warning because callers cannot add `Send` later (return-type notation is
not stable as of 1.98). **Never** reach for `#[async_trait]` just because a trait has async methods —
it boxes every call.

```rust
use std::sync::Arc;

#[trait_variant::make(Store: Send)]
pub trait LocalStore {
    async fn get(&self, key: &str) -> Option<String>;
}

pub struct MemStore;

impl Store for MemStore {
    async fn get(&self, key: &str) -> Option<String> {
        Some(key.to_uppercase())
    }
}

pub fn spawn_lookup<S: Store + Sync + 'static>(store: Arc<S>, key: String) -> tokio::task::JoinHandle<Option<String>> {
    tokio::spawn(async move { store.get(&key).await }) // compiles because Store's future is Send
}
```

### ASYNC-15: Use `async-trait` (or manual boxing) only for `dyn` async traits
**Default:** traits with `async fn` are not dyn-compatible on stable (as of 1.98). **Use**
`#[async_trait::async_trait]` when you need `Box<dyn Trait>` / `Arc<dyn Trait>` (plugin lists,
handler registries), or write `fn f(&self) -> Pin<Box<dyn Future<Output = T> + Send + '_>>` by hand.
Prefer an enum of implementations over `dyn` when the set is closed (see `traits-generics.md`).

```rust
use std::sync::Arc;

#[async_trait::async_trait]
pub trait Handler: Send + Sync {
    async fn handle(&self, req: String) -> String;
}

struct Echo;

#[async_trait::async_trait] // needed on every impl, too
impl Handler for Echo {
    async fn handle(&self, req: String) -> String { req }
}

pub fn registry() -> Vec<Arc<dyn Handler>> { vec![Arc::new(Echo)] }
```

### ASYNC-16: Use `AsyncFn*` when the callback borrows its argument; keep `Fn() -> Fut` otherwise
**Default:** a callback whose future borrows the argument you pass it ("run this with a borrowed
connection/transaction") takes `impl AsyncFnOnce(&mut Conn) -> R` (1.85, in the prelude) and callers
pass `async |c| ..` — `F: Fn(&mut Conn) -> Fut` cannot express that borrow. **Use** the classic
`F: FnMut() -> Fut, Fut: Future<Output = ..>` for retry/poll-style helpers with no borrowed
argument. **Check** that the caller's future can still be `tokio::spawn`ed: as of 1.96, zero-argument
`async ||` closures passed to a generic `AsyncFnMut` helper often fail with "implementation of `Send`
is not general enough"; the `Fn() -> Fut` form does not.

```rust
pub struct Conn;

impl Conn {
    async fn query(&mut self, sql: &str) -> usize { sql.len() }
}

/// The callback's future borrows `&mut Conn`: needs AsyncFnOnce.
pub async fn with_conn<R>(f: impl AsyncFnOnce(&mut Conn) -> R) -> R {
    f(&mut Conn).await
}

pub async fn count(table: &str) -> usize {
    with_conn(async |c| c.query(table).await + c.query("SELECT 1").await).await
}

/// No borrowed argument: plain FnMut -> Future stays spawn-friendly.
pub async fn retry<T, E, F, Fut>(attempts: u32, mut op: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut left = attempts.max(1);
    loop {
        match op().await {
            Err(_) if left > 1 => left -= 1,
            done => return done,
        }
    }
}

pub fn spawned() -> tokio::task::JoinHandle<(usize, Result<usize, std::io::Error>)> {
    tokio::spawn(async { (count("users").await, retry(3, || probe("https://example.com")).await) })
}

async fn probe(url: &str) -> Result<usize, std::io::Error> { Ok(url.len()) }
```

## Review checklist

- [ ] One runtime, created in `main`; no runtime or `block_on` inside library/async code (ASYNC-01, ASYNC-03)
- [ ] No `async fn` without IO; pure logic is sync (ASYNC-02)
- [ ] No `std::fs`, `thread::sleep`, blocking clients or long CPU loops on async workers (ASYNC-04)
- [ ] No `MutexGuard`/`RefCell` borrow/`Rc` alive across `.await` (ASYNC-05, ASYNC-06)
- [ ] Every `tokio::spawn` has an owner: `JoinHandle`, `JoinSet` or `TaskTracker` (ASYNC-07)
- [ ] Fan-out is bounded (`buffered(n)`, `Semaphore`) (ASYNC-08)
- [ ] State stays consistent if dropped at any `.await`; cleanup lives in `Drop` (ASYNC-09)
- [ ] Looping `select!` branches are cancel-safe; long futures pinned outside the loop (ASYNC-10)
- [ ] Shutdown via `CancellationToken`, tasks awaited (ASYNC-11); network awaits have timeouts (ASYNC-12)
- [ ] Channels bounded; `watch`/`broadcast`/`oneshot` used where they fit (ASYNC-13)
- [ ] Public async traits give `Send` futures; `async-trait` only for `dyn` (ASYNC-14, ASYNC-15)
- [ ] `AsyncFn*` only where the callback borrows its argument; spawned callers still compile (ASYNC-16)
