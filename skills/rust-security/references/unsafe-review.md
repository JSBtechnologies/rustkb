---
id: security/unsafe-review
title: Reviewing unsafe code
summary: >-
  Security review procedure for `unsafe`: when it is allowed at all (forbid by default),
  the soundness checklist every unsafe block must pass (validity, aliasing, alignment,
  initialization, lifetimes, panic safety, Send/Sync), FFI boundary rules, safe
  replacements for transmute/set_len/static mut, and the tools that find UB (Miri,
  cargo-geiger).
area: security
tags: [unsafe, soundness, undefined-behavior, ffi, send, sync, maybeuninit, transmute, miri, cargo-geiger, forbid-unsafe]
rust: "1.96"
edition: "2024"
crates:
  bytemuck: "1.25"
  zerocopy: "0.8"
  cargo-geiger: "0.13"
  bindgen: "0.73"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/nomicon/
  - https://doc.rust-lang.org/reference/behavior-considered-undefined.html
  - https://doc.rust-lang.org/std/mem/union.MaybeUninit.html
  - https://github.com/rust-lang/miri
  - https://rust-lang.github.io/rust-clippy/master/index.html#undocumented_unsafe_blocks
---

# Reviewing unsafe code

General `unsafe`/FFI mechanics live in the rust-idioms skill. This file is the *security
review* view: how to decide whether an `unsafe` block is allowed, and how to prove it sound.
Memory-safety bugs in Rust come almost entirely from `unsafe` that was written to silence the
compiler rather than to express a proven invariant.

## UNS-01: Forbid unsafe unless the crate's purpose requires it

Default: every crate that doesn't wrap FFI or implement a low-level data structure forbids
`unsafe` via the manifest, so the policy is visible and inherited by new files.

```toml
# Cargo.toml (or [workspace.lints.rust] + `lints.workspace = true` in members)
[lints.rust]
unsafe_code = "forbid"
```

Crates that genuinely need `unsafe`: use `unsafe_code = "deny"` and put
`#[allow(unsafe_code)]` on the one module that holds it (`forbid` cannot be overridden).
Keep that module small, private, and wrapped in a safe API. Prefer moving it into a
dedicated `*-sys` or `*-raw` crate so reviewers know exactly where to look.

## UNS-02: Never use unsafe to get past the borrow checker

The most common LLM-written UB: a borrow error "fixed" with raw pointers, `transmute`, or
`static mut`. If safe Rust rejects it, the aliasing or lifetime problem is real.

```rust
// ❌ UB: creates &mut while & borrows exist; mutating through a shared reference
let this = unsafe { &mut *(self as *const Self as *mut Self) };
// ❌ UB waiting to happen: lies about a lifetime
let s: &'static str = unsafe { std::mem::transmute(local_string.as_str()) };
```

Fix the design instead: split borrows (`split_at_mut`, destructure the struct), clone or
`Arc` the data, use indices instead of references, `Cell`/`RefCell`/`Mutex` for genuine
shared mutation, or restructure ownership. Reject any PR where `unsafe` appears in the same
diff as a borrow-checker error message.

## UNS-03: Every unsafe block states its proof

Default: each `unsafe` block has a `// SAFETY:` comment naming *which* precondition holds
and *why*; each `unsafe fn` and `unsafe trait` has a `# Safety` doc section listing the
caller's obligations; each `unsafe impl` gets its own `// SAFETY:` comment.

```toml
[lints.clippy]
undocumented_unsafe_blocks = "deny"       # also covers unsafe impls
multiple_unsafe_ops_per_block = "warn"    # one proof per operation
missing_safety_doc = "deny"
```

In edition 2024, `unsafe_op_in_unsafe_fn` warns by default: the body of an `unsafe fn` is
no longer implicitly an unsafe block, so each operation gets its own block and proof. "It
compiled and the tests pass" is not a safety argument.

## UNS-04: The soundness checklist

Run every `unsafe` block through these questions. One "don't know" is a finding.

| Area | Question |
|---|---|
| Validity of pointers | Non-null? Points into a live allocation for the *entire* access? In bounds (`ptr.add(n)` stays within the allocation or one past)? |
| Alignment | Is the pointer aligned for `T`? Bytes from the network/disk are align 1 → `read_unaligned` or a zerocopy type. |
| Initialization | Is every byte read initialized? Is the value *valid* for its type (`bool` is 0/1, enums have a valid discriminant, `char` is a scalar value, `&T` non-null and aligned, `str` is UTF-8)? |
| Aliasing | While a `&mut T` exists, is it the only access path? Is anything mutated through a `&T` without `UnsafeCell`? |
| Lifetimes | Can the returned reference outlive the data (e.g. a `&[u8]` into a buffer that is later reallocated)? |
| Panic safety | If code between "invariant broken" and "invariant restored" panics, does `Drop` observe a broken state (double free, reading uninit)? |
| Threads | Can this run concurrently? Does a `Send`/`Sync` impl let it? (UNS-06) |
| Size limits | Is `len * size_of::<T>() <= isize::MAX` for `slice::from_raw_parts`? Can a length from input overflow the multiplication? |
| FFI contract | Does the C side's documented contract (ownership, nullability, thread-safety, lifetime) match what the Rust signature claims? (UNS-09) |

## UNS-05: The safe API must be sound for every input

A function without `unsafe` in its signature must not cause UB for *any* argument values or
call sequence — including adversarial ones. "Callers never pass that" makes the function
unsound; either validate, or mark it `unsafe fn` with a `# Safety` section.

- The soundness boundary is the **module**: private fields that unsafe code relies on (a
  `len` that must be ≤ capacity, an index known in-bounds) can be broken by *safe* code
  elsewhere in the same module. Review the whole module, not just the `unsafe` lines.
- Don't rely on `Ord`, `Hash`, `Eq` or `Iterator::size_hint` of a generic `T` for memory
  safety; safe trait impls may be wrong or malicious.
- Replace `get_unchecked` with checked indexing or iterators; the optimizer usually
  removes the bounds check. Keep `get_unchecked` only with a benchmark proving it matters
  and an assert-based invariant proof.

## UNS-06: Send and Sync impls need bounds and a thread-safety proof

`unsafe impl Send/Sync` asserts something the compiler could not prove. A missing
bound on such an impl is one of the most frequent causes of `unsound` RustSec advisories.

```rust
pub struct MyBox<T> {
    ptr: std::ptr::NonNull<T>,
    _owns: std::marker::PhantomData<T>,
}

// SAFETY: MyBox<T> uniquely owns its T, exactly like Box<T>, so it can move
// between threads exactly when T can.
unsafe impl<T: Send> Send for MyBox<T> {}
// SAFETY: &MyBox<T> only hands out &T, so sharing it is sound exactly when T: Sync.
unsafe impl<T: Sync> Sync for MyBox<T> {}
```

Reject:

- `unsafe impl<T> Send for X<T> {}` with no `T: Send` bound — lets `Rc` or `RefCell` cross
  threads.
- `Sync` on anything containing `Cell`, `RefCell`, a raw pointer used for mutation, or a
  non-atomic counter.
- `Send` for a wrapper around a C handle whose library isn't documented as thread-safe
  (many C libraries use thread-local state). When the docs are silent, don't impl it.
- `Send`/`Sync` added "because tokio::spawn required it". Use a `Mutex`, a dedicated
  thread with channels, or `spawn_local` instead.

## UNS-07: Uninitialized memory: MaybeUninit, never set_len-then-read

```rust
// ❌ UB: exposes uninitialized bytes as initialized u8 (and any read is UB)
let mut v: Vec<u8> = Vec::with_capacity(n);
unsafe { v.set_len(n) };
reader.read(&mut v)?;                  // the callee may read the buffer: UB
// ❌ UB for almost every T (even integers: uninitialized is not "some value")
let x: T = unsafe { std::mem::MaybeUninit::uninit().assume_init() };
```

Defaults, in order:

1. `vec![0u8; n]` — zeroed allocation is cheap (calloc); this is right 99% of the time.
2. `Read::read_to_end` / `take(limit).read_to_end` — std manages the uninit tail safely.
3. `Vec::spare_capacity_mut()` + write every slot + `set_len`, for hot paths:

```rust
use std::mem::MaybeUninit;

pub fn read_into_spare(v: &mut Vec<u8>, src: &[u8]) {
    v.reserve(src.len());
    let spare: &mut [MaybeUninit<u8>] = v.spare_capacity_mut();
    for (dst, &b) in spare.iter_mut().zip(src) {
        dst.write(b);
    }
    // SAFETY: the loop initialized exactly `src.len()` elements past the old length;
    // `reserve` guaranteed that many spare slots exist.
    unsafe { v.set_len(v.len() + src.len()) };
}
```

`mem::uninitialized()` (deprecated) is UB for nearly every type; `mem::zeroed()` is UB for
types where all-zero isn't valid (references, `Box`, `NonNull`, `NonZero*`, many enums).

## UNS-08: Use safe conversions instead of transmute

| Instead of `transmute` for… | Use |
|---|---|
| bytes ↔ integers | `u32::from_le_bytes`, `to_be_bytes`, … |
| float bits | `f32::to_bits` / `from_bits` |
| `&[u8]` ↔ `&[T]` of plain-old-data | `bytemuck::cast_slice` / `try_cast_slice` (checks size and alignment) |
| parsing a struct from untrusted bytes | `zerocopy` (`FromBytes`, `KnownLayout`, `Immutable` derives): validates layout at compile time, length at runtime |
| pointer type change | `ptr.cast::<U>()` |
| lifetime extension | Nothing. Redesign (UNS-02). |
| `&T` → `&mut T` | Nothing. Always UB. |
| `Vec<T>` → `Vec<U>` | `into_iter().map(..).collect()` (can reuse the allocation for same-layout types) |

For packed structs, never create a reference to a field: copy the field (`let v = p.value;`)
or take `&raw const p.value` and `read_unaligned`.

## UNS-09: FFI boundary rules

- **Validate everything from C** as untrusted input: null checks, lengths, `CStr::from_ptr`
  only when NUL termination is part of the contract (otherwise take pointer + length).
- A Rust function that dereferences a raw-pointer argument must itself be `unsafe`
  (`clippy::not_unsafe_ptr_arg_deref` is deny-by-default and catches this).
- **Panics must not unwind into C.** Since Rust 1.81 a panic escaping an `extern "C"` fn
  aborts the process — memory-safe but a DoS. Catch and convert to an error code; use
  `extern "C-unwind"` only when the foreign side is built to propagate unwinding.

```rust
/// # Safety
/// `ptr` must be valid for reads of `len` bytes for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mylib_parse(ptr: *const u8, len: usize) -> i32 {
    if ptr.is_null() {
        return -1;
    }
    // SAFETY: caller contract above; null was excluded.
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    match std::panic::catch_unwind(|| parse(input)) {
        Ok(Ok(n)) => n,
        Ok(Err(())) => -2,
        Err(_) => -3, // never let a panic cross the boundary
    }
}

fn parse(input: &[u8]) -> Result<i32, ()> {
    i32::try_from(input.len()).map_err(|_| ())
}
```

- Memory allocated by Rust is freed by Rust (export a `mylib_free`), memory from C's
  `malloc` by `free`. Mixing allocators is UB.
- Edition 2024 requires `unsafe extern "C" { ... }` blocks and `#[unsafe(no_mangle)]` /
  `#[unsafe(export_name)]`. Mark an item `safe fn` inside an extern block only if the C
  function is sound for *every* argument; otherwise the UB moves to safe callers.
  A wrong foreign signature (types, arity, ABI) is UB — generate bindings with bindgen.

## UNS-10: Global and environment state

- `static mut`: references to it are denied in edition 2024 (`static_mut_refs`). Use
  atomics, `Mutex`, `OnceLock`/`LazyLock`, or `thread_local!`.
- `std::env::set_var`/`remove_var` are `unsafe` in edition 2024 because other threads
  (including C code calling `getenv`) may read the environment concurrently. Don't call
  them after spawning threads or starting a tokio runtime; pass configuration explicitly.

## UNS-11: Verify with tools, and inventory unsafe in dependencies

- **Miri** runs the test suite in an interpreter that detects most UB (aliasing violations,
  uninit reads, out-of-bounds, misalignment, data races, leaks): `cargo +nightly miri test`.
  Every crate with `unsafe` runs it in CI. Setup and flags: testing-for-security.md.
- **Sanitizers** and **fuzzing** reach code paths Miri is too slow for (FUZZ rules).
- **cargo-geiger** counts `unsafe` usage per dependency. Use the output to prioritise
  review (crates that both contain `unsafe` *and* parse untrusted input first), not as a
  pass/fail gate — std-backed safe abstractions contain `unsafe` too.
- An `unsound` RustSec advisory on a dependency (`check_advisories`) is a real bug even if
  "no exploit is known"; cargo-deny's `unsound = "all"` fails on it.
