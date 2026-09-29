---
id: idioms/concurrency
title: Concurrency with threads, locks and atomics
summary: >-
  Choose between rayon, scoped threads, channels, locks, arc-swap and atomics by workload; prefer
  ownership transfer over Arc<Mutex<_>>, keep critical sections short and ordered, use std LazyLock/OnceLock
  for globals, and test lock-free code with loom.
area: idioms
tags: [concurrency, threads, mutex, rwlock, atomics, channels, rayon, scoped-threads, lazylock, oncelock, send, sync, loom]
rust: "1.96"
edition: "2024"
crates:
  rayon: "1.12"
  crossbeam-channel: "0.5"
  parking_lot: "0.12"
  arc-swap: "1.9"
  dashmap: "6.2"
  loom: "0.7"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/std/sync/index.html
  - https://doc.rust-lang.org/std/thread/fn.scope.html
  - https://marabos.nl/atomics/
  - https://docs.rs/rayon/latest/rayon/
  - https://docs.rs/arc-swap/latest/arc_swap/
  - https://doc.rust-lang.org/edition-guide/rust-2024/temporary-if-let-scope.html
---

# Concurrency with threads, locks and atomics

Decides how threads share work and data: which primitive fits which workload, how to avoid the
reflexive `Arc<Mutex<_>>`, how to lock without deadlocks, when atomics are enough, and how to do
global/lazy state on modern std. Read before spawning threads, adding a lock or an atomic, or
writing a `static`. Async tasks, `.await` and Tokio channels are in `async.md`.

## Choosing a concurrency model

### CONC-01: Pick the primitive from the workload, not from habit
**Default:** use the least-shared option that fits. **Never** start from `Arc<Mutex<T>>` and work
backwards.

| Workload | Use |
|---|---|
| CPU-bound work over a collection | `rayon` `par_iter()` (CONC-04) |
| A few threads that need to borrow local data | `std::thread::scope` (CONC-03) |
| Pipeline / workers with independent state | threads + channels, each thread owns its state (CONC-05) |
| Small shared state, frequent short writes | `Mutex<T>` (in an `Arc` only if not scoped) (CONC-06) |
| Read-mostly snapshot (config, routing table) | `arc_swap::ArcSwap<T>` or `RwLock<Arc<T>>` (CONC-07) |
| Single counter / flag | `AtomicU64` / `AtomicBool` (CONC-10) |
| Concurrent map, many writers, many threads | sharded map (`dashmap`) — measure against `Mutex<HashMap>` first |
| Initialise once, read forever | `LazyLock` / `OnceLock` (CONC-12) |
| IO-bound concurrency (sockets, HTTP) | async tasks — see `async.md` |

### CONC-02: Transfer ownership instead of sharing mutable state
**Default:** give each thread the data it needs by value and get results back by value (join
handle return, channel, `collect`). **Never** collect results by pushing into a shared
`Arc<Mutex<Vec<_>>>` — it serialises the workers and needs clones and `unwrap`s for nothing.

```rust
// ❌ LLM reflex: Arc<Mutex<Vec>> + clone + 'static threads to gather results
use std::sync::{Arc, Mutex};
use std::thread;

pub fn lengths_bad(words: Vec<String>) -> Vec<usize> {
    let out = Arc::new(Mutex::new(Vec::new()));
    let handles: Vec<_> = words.into_iter().map(|w| {
        let out = Arc::clone(&out);
        thread::spawn(move || out.lock().unwrap().push(w.len()))
    }).collect();
    for h in handles { h.join().unwrap(); }
    Arc::try_unwrap(out).unwrap().into_inner().unwrap()
}
```

```rust
use rayon::prelude::*;

pub fn lengths(words: &[String]) -> Vec<usize> {
    words.par_iter().map(String::len).collect() // parallel, ordered, no locks, no clones
}
```

## Scoped threads and data parallelism

### CONC-03: Use `thread::scope` to borrow instead of `Arc` + `'static`
**Default:** when threads finish before the function returns, spawn them in `std::thread::scope`
(stable since 1.63): they may borrow locals, including `&mut` to disjoint parts. **Use**
`thread::spawn` + `Arc` only for threads that outlive the caller (background services).
**Never** clone a large `Vec` into an `Arc` just to satisfy `'static`.

```rust
pub fn sum_halves(data: &mut [u64]) -> u64 {
    let (left, right) = data.split_at_mut(data.len() / 2);
    std::thread::scope(|s| {
        let a = s.spawn(|| {
            for x in left.iter_mut() {
                *x *= 2; // &mut borrow of the caller's data
            }
            left.iter().sum::<u64>()
        });
        let b = s.spawn(|| right.iter().sum::<u64>());
        a.join().expect("left worker panicked") + b.join().expect("right worker panicked")
    }) // all scoped threads are joined here
}
```

### CONC-04: Use rayon for CPU-bound parallelism; don't hand-roll pools
**Default:** turn `iter()` into `par_iter()` (or `par_chunks`, `par_sort_unstable`, `rayon::join`)
when each item costs more than a few microseconds; measure first (`performance.md`). **Use**
`ThreadPoolBuilder` only to cap threads for a subsystem. **Never** do blocking IO or hold locks
inside rayon closures (it starves the pool), never call `par_iter` from an async worker without
bridging (ASYNC-04), and never spawn `num_cpus` OS threads by hand for a loop.

```rust
use rayon::prelude::*;

pub fn checksum(chunks: &[Vec<u8>]) -> u64 {
    chunks
        .par_iter()
        .map(|c| c.iter().map(|&b| u64::from(b)).sum::<u64>())
        .sum()
}
```

## Channels

### CONC-05: Choose the channel by topology; bound it
**Default:** `std::sync::mpsc::sync_channel(n)` (bounded) for many-producer/one-consumer between
threads — std's implementation has been crossbeam's since 1.67. **Use** `crossbeam-channel` when you
need multiple consumers (MPMC work queue) or `select!` over several channels. **Use** Tokio channels
when either end is async (`async.md`, ASYNC-13). **Never** use an unbounded channel between a fast
producer and a slow consumer — memory grows until the process dies. Shut down by dropping all
senders; receivers see the end as `recv()` returning `Err` / the `for` loop ending.

```rust
use std::thread;

pub fn pipeline(lines: Vec<String>) -> usize {
    let (tx, rx) = crossbeam_channel::bounded::<String>(64);
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let rx = rx.clone(); // MPMC: each worker pulls from the same queue
            thread::spawn(move || rx.iter().map(|l| l.len()).sum::<usize>())
        })
        .collect();
    for line in lines {
        tx.send(line).expect("all workers exited");
    }
    drop(tx); // closes the channel: workers' iterators end
    workers.into_iter().map(|w| w.join().expect("worker panicked")).sum()
}
```

## Locks

### CONC-06: Lock the data, briefly, and never call out while holding a guard
**Default:** `std::sync::Mutex<T>` wrapping exactly the data it protects (a field, not the whole
service struct), locked for a few statements. Take `&mut self` / `Mutex::get_mut` when you have
exclusive access — no locking needed. **Never** hold a guard while doing IO, sending on a bounded
channel, calling a user callback, or taking another lock unless in the global order (CONC-09).
Clone or `mem::take` what you need, drop the guard, then work.

```rust
use std::collections::HashMap;
use std::sync::Mutex;

pub struct Registry {
    names: Mutex<HashMap<u32, String>>, // only the map is shared-mutable
    capacity: usize,                    // immutable config needs no lock
}

impl Registry {
    pub fn new(capacity: usize) -> Self {
        Self { names: Mutex::new(HashMap::new()), capacity }
    }

    pub fn insert(&self, id: u32, name: String) -> bool {
        let mut names = self.names.lock().expect("registry lock poisoned");
        if names.len() >= self.capacity {
            return false;
        }
        names.insert(id, name);
        true
    }

    pub fn snapshot(&self) -> Vec<String> {
        let names = self.names.lock().expect("registry lock poisoned").clone(); // guard dropped at `;`
        let mut out: Vec<String> = names.into_values().collect(); // sort outside the lock
        out.sort();
        out
    }
}
```

### CONC-07: Use `RwLock` only for long reads; use `ArcSwap` for read-mostly snapshots
**Default:** `Mutex` — it is usually as fast as `RwLock` for short critical sections, and `std`'s
`RwLock` priority policy depends on the OS (readers can starve writers or vice versa). **Use**
`RwLock` when reads are long and truly concurrent. **Use** `arc_swap::ArcSwap<T>` (or
`RwLock<Arc<T>>` with clone-out) for config/routing tables replaced wholesale and read on every
request: readers get an `Arc<T>` snapshot without blocking writers.

```rust
use arc_swap::ArcSwap;
use std::sync::Arc;

pub struct Config {
    pub max_conns: usize,
}

pub struct Shared {
    config: ArcSwap<Config>,
}

impl Shared {
    pub fn new(cfg: Config) -> Self {
        Self { config: ArcSwap::from_pointee(cfg) }
    }
    pub fn max_conns(&self) -> usize {
        self.config.load().max_conns // lock-free read
    }
    pub fn reload(&self, cfg: Config) {
        self.config.store(Arc::new(cfg)); // readers holding the old Arc keep it alive
    }
}
```

### CONC-08: Treat poisoning as a bug signal; use `parking_lot` only for a reason
**Default:** `lock().expect("<what> lock poisoned")` — a poisoned std lock means another thread
panicked mid-update, and propagating the panic is correct. **Use** `PoisonError::into_inner` /
`Mutex::clear_poison` (1.77) only when every update leaves the data consistent. **Use**
`parking_lot` when you need its extras (fair unlocking, `const` constructors with timeouts,
upgradable reads, `ReentrantMutex`, no poisoning by design) — not for speed by default; std's
futex-based locks are competitive. **Never** `lock().unwrap_or_else(|e| e.into_inner())`
reflexively to "handle" errors.

### CONC-09: Avoid deadlocks: global lock order, no re-entrancy, watch temporaries
**Default:** at most one lock at a time; if two are unavoidable, always acquire in one documented
order. `std::sync::Mutex` is not re-entrant: locking it again on the same thread deadlocks or
panics. Temporaries in a `match` scrutinee live to the end of the `match`, so a guard created
there is still held in every arm. (Edition 2024 drops `if let` scrutinee temporaries before
`else`, but not before the `then` block.)

```rust
// ❌ guard from the scrutinee lives through the whole match: re-locking deadlocks
use std::sync::Mutex;

pub fn bump_bad(counts: &Mutex<Vec<u32>>) {
    match counts.lock().unwrap().last() {
        Some(&n) => counts.lock().unwrap().push(n + 1), // same thread, same mutex: deadlock
        None => counts.lock().unwrap().push(0),
    }
}
```

```rust
use std::sync::Mutex;

pub fn bump(counts: &Mutex<Vec<u32>>) {
    let mut counts = counts.lock().expect("counts lock poisoned"); // one guard, one scope
    let next = counts.last().map_or(0, |n| n + 1);
    counts.push(next);
}
```

## Atomics

### CONC-10: Use atomics for independent counters and flags; use the weakest correct ordering
**Default:** `Relaxed` for statistics counters and IDs where no other memory depends on the
value; `Release` on the store + `Acquire` on the load when a flag publishes other data;
`SeqCst` only when you need a single total order across several atomics and can say why.
**Use** a `Mutex` as soon as two values must change together — separate atomics cannot keep a
multi-field invariant. **Use** `fetch_add`/`fetch_update` (or `update`, 1.95) for read-modify-write;
**never** `load` then `store` (lost updates).

```rust
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct Metrics {
    requests: AtomicU64,
    shutting_down: AtomicBool,
}

impl Metrics {
    pub const fn new() -> Self {
        Self { requests: AtomicU64::new(0), shutting_down: AtomicBool::new(false) }
    }
    pub fn hit(&self) -> u64 {
        self.requests.fetch_add(1, Ordering::Relaxed) + 1 // counter: no data published
    }
    pub fn stop(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }
    pub fn is_stopping(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }
}

pub static METRICS: Metrics = Metrics::new(); // const-constructible: no LazyLock needed
```

### CONC-11: Don't write lock-free data structures; if you must, test them with loom
**Default:** use `std`, `crossbeam`, `arc-swap` or `dashmap` instead of hand-rolled lock-free
code. **Use** `loom` (model-checks every interleaving under `--cfg loom`) and Miri (detects data
races in `unsafe` code; see `unsafe-ffi.md`) for any custom atomic protocol. **Never** rely on
"it passed 1000 test runs" for memory-ordering correctness.

## Global and lazy state

### CONC-12: Use `LazyLock` / `OnceLock` from std; drop `lazy_static` and `once_cell`
**Default:** `static X: LazyLock<T> = LazyLock::new(|| ..)` (1.80) when initialisation needs no
runtime input; `OnceLock<T>` (1.70) + `get_or_init` / `set` when the value comes from runtime
config; `const`-constructible statics (`Mutex::new`, atomics) need neither. **Use** `LazyCell` /
`OnceCell` for single-threaded lazies. **Never** add `lazy_static` or `once_cell` to new code —
std covers them (see `editions-msrv.md`). Prefer passing state explicitly over globals; globals
make tests share state (tests run in parallel threads).

```rust
use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

static STATUS_TEXT: LazyLock<HashMap<u16, &'static str>> =
    LazyLock::new(|| HashMap::from([(200, "OK"), (404, "Not Found")]));

static DATA_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();

pub fn init(dir: std::path::PathBuf) -> Result<(), std::path::PathBuf> {
    DATA_DIR.set(dir) // Err(dir) if already initialised
}

pub fn status_text(code: u16) -> &'static str {
    STATUS_TEXT.get(&code).copied().unwrap_or("Unknown")
}
```

### CONC-13: Use `thread_local!` with `const` init for per-thread caches
**Default:** `thread_local! { static BUF: RefCell<String> = const { RefCell::new(String::new()) }; }`
and access via `with_borrow_mut` (1.73). **Use** it for scratch buffers and per-thread RNG/state that
must not be shared. **Never** use thread-locals for request context in async code — tasks migrate
between threads at every `.await` (use `tokio::task_local!` or pass the context).

```rust
use std::cell::RefCell;
use std::fmt::Write;

thread_local! {
    static SCRATCH: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn render(id: u64) -> usize {
    SCRATCH.with_borrow_mut(|buf| {
        buf.clear(); // reuse this thread's allocation
        write!(buf, "item-{id}").expect("writing to a String cannot fail");
        buf.len()
    })
}
```

## `Send` and `Sync`

### CONC-14: Let the compiler derive `Send`/`Sync`; fix the type, not the bound
**Default:** `Send`/`Sync` are auto traits — a type made of `Send + Sync` parts is itself. For
sharing across threads use `Arc` (not `Rc`), `Mutex`/`RwLock`/atomics (not `RefCell`/`Cell`).
`Arc<T>` is `Send + Sync` only if `T: Send + Sync`, so `Arc<RefCell<T>>` never crosses threads —
use `Arc<Mutex<T>>`. **Never** write `unsafe impl Send`/`Sync` to silence a compiler error; that
is an unsafe proof obligation reviewed like any other (`unsafe-ffi.md`, UNSAFE-01).

## Review checklist

- [ ] The primitive matches the workload table; no reflexive `Arc<Mutex<_>>` (CONC-01, CONC-02)
- [ ] Borrowing threads use `thread::scope`, not `Arc` + clones (CONC-03)
- [ ] CPU-parallel loops use rayon; no blocking or locks inside rayon closures (CONC-04)
- [ ] Channels are bounded, dropped to close, and MPMC uses crossbeam (CONC-05)
- [ ] Locks wrap only shared data; no IO/callbacks/other locks while guarded (CONC-06)
- [ ] Read-mostly snapshots use `ArcSwap`/`RwLock<Arc<_>>`; `RwLock` justified (CONC-07)
- [ ] Poisoned locks propagate via `expect`; `parking_lot` has a stated reason (CONC-08)
- [ ] No lock held across a `match` arm that locks again; lock order documented (CONC-09)
- [ ] Atomics only for independent values; orderings justified; no load-then-store (CONC-10)
- [ ] Custom lock-free code has loom tests (CONC-11)
- [ ] Globals use `LazyLock`/`OnceLock`/const statics; no `lazy_static`/`once_cell` (CONC-12)
- [ ] No `thread_local!` for async request context (CONC-13); no `unsafe impl Send/Sync` to silence errors (CONC-14)
