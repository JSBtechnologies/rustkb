---
id: idioms/unsafe-ffi
title: Unsafe code and FFI
summary: >-
  Avoid unsafe unless no safe API exists; when needed, wrap it in a sound safe abstraction with
  SAFETY comments and # Safety docs, use edition-2024 syntax (unsafe extern, safe fn, #[unsafe(no_mangle)]),
  prefer bytemuck/zerocopy over transmute, and verify with Miri. Covers C FFI strings, layout, panics and ownership.
area: idioms
tags: [unsafe, ffi, safety, miri, edition-2024, unsafe-extern, no_mangle, transmute, bytemuck, zerocopy, cstr, repr-c, provenance]
rust: "1.96"
edition: "2024"
crates:
  bytemuck: "1.25"
  zerocopy: "0.8"
  bindgen: "0.73"
  cbindgen: "0.29"
  cxx: "1.0"
  libc: "0.2"
verified: 2026-09-29
sources:
  - https://doc.rust-lang.org/nomicon/
  - https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-extern.html
  - https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-attributes.html
  - https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-op-in-unsafe-fn.html
  - https://doc.rust-lang.org/edition-guide/rust-2024/static-mut-references.html
  - https://doc.rust-lang.org/edition-guide/rust-2024/newly-unsafe-functions.html
  - https://github.com/rust-lang/miri/blob/master/README.md
  - https://std-dev-guide.rust-lang.org/policy/safety-comments.html
---

# Unsafe code and FFI

Decides when `unsafe` is justified, how to write and document it so it stays sound, which
edition-2024 syntax to use, and how to cross the C boundary (strings, layout, panics, ownership).
Read before writing `unsafe`, `extern`, `#[no_mangle]`, `transmute`, `static mut` or raw pointers.
The review process for unsafe code, fuzzing and supply-chain concerns live in rust-security.

## When to write `unsafe`

### UNSAFE-01: Exhaust safe alternatives first; forbid unsafe where it isn't needed
**Default:** no `unsafe`. Set `unsafe_code = "forbid"` in `[lints.rust]` for crates that don't
need it (see `lints-and-tooling.md`). Before writing `unsafe`, check for the safe tool:

| You want | Use instead |
|---|---|
| two `&mut` into one slice / map | `split_at_mut`, `get_disjoint_mut` (1.86), `chunks_mut` |
| skip bounds checks | iterators, `zip`, slice patterns, an up-front `assert!` (see `performance.md`) |
| reinterpret bytes as a struct | `bytemuck` / `zerocopy` (UNSAFE-09) |
| `u32` ⇄ `[u8; 4]`, `f32` ⇄ bits | `to_le_bytes`/`from_le_bytes`, `to_bits`/`from_bits` |
| global mutable state | atomics, `Mutex`, `OnceLock`, `LazyLock` (UNSAFE-07) |
| silence a `Send`/`Sync` error | fix the type (`Arc`, `Mutex`); see `concurrency.md` (CONC-14) |

**Never** add `unsafe` for unmeasured speed (`get_unchecked` in a loop the optimiser already
cleans up) or to "get past the borrow checker" — that is almost always UB waiting to happen.

### UNSAFE-02: Encapsulate unsafe behind a sound safe API
**Default:** keep each `unsafe` block tiny and inside a module whose private fields enforce the
invariant the block relies on. The safe public API must be impossible to misuse into UB for
*any* input, including from safe code in the same crate — that is what "sound" means.
**Never** expose a safe `fn` whose correctness depends on the caller (make it `unsafe fn` with a
`# Safety` section instead), and never let a public field break an invariant unsafe code relies on.

```rust
mod ascii {
    /// A byte string that is guaranteed to be ASCII (and therefore valid UTF-8).
    pub struct AsciiBuf(Vec<u8>); // private field: only this module can construct it

    impl AsciiBuf {
        pub fn new(bytes: Vec<u8>) -> Option<Self> {
            bytes.is_ascii().then_some(Self(bytes))
        }

        pub fn as_str(&self) -> &str {
            // SAFETY: `new` is the only constructor and rejects non-ASCII input;
            // ASCII is always valid UTF-8, and the Vec is never mutated afterwards.
            unsafe { std::str::from_utf8_unchecked(&self.0) }
        }
    }
}
```

### UNSAFE-03: Write a `SAFETY:` comment for every block and `# Safety` for every unsafe fn
**Default:** each `unsafe { }` is preceded by `// SAFETY:` explaining why every precondition of
every unsafe operation inside holds *here*. Each `unsafe fn` and `unsafe trait` documents its
contract in a `# Safety` doc section. Enable `clippy::undocumented_unsafe_blocks` and keep
`clippy::missing_safety_doc` (on by default); consider `clippy::multiple_unsafe_ops_per_block`.
**Never** write `// SAFETY: this is safe` — state the facts (lengths, alignment, liveness,
aliasing, initialisation) that make it so.

### UNSAFE-04: Treat an `unsafe fn` body as safe code (edition 2024)
**Default:** in edition 2024 `unsafe_op_in_unsafe_fn` warns by default: the body of an `unsafe fn`
is *not* an unsafe block. Wrap each unsafe operation in its own `unsafe { }` with a `SAFETY:`
comment tying it to the function's `# Safety` contract; deny the lint in `[lints.rust]`.

```rust
/// Returns the element at `index` without bounds checking.
///
/// # Safety
///
/// `index` must be less than `values.len()`.
pub unsafe fn get_fast(values: &[u32], index: usize) -> u32 {
    // SAFETY: the caller guarantees `index < values.len()` (see # Safety).
    unsafe { *values.get_unchecked(index) }
}
```

## Edition 2024 unsafe syntax

### UNSAFE-05: Declare foreign functions in `unsafe extern` blocks and mark the safe ones `safe`
**Default:** edition 2024 requires `unsafe extern "C" { .. }` — writing the block is the unsafe
promise that the signatures are correct. Items are `unsafe fn` by default; declare
`pub safe fn` for functions with no preconditions (pure value arguments) so callers need no
`unsafe`. Use `core::ffi` types (`c_int`, `c_char`, `c_void`), never guess `i32`/`i8`.

```rust
use std::ffi::{CString, NulError, c_char, c_int};

unsafe extern "C" {
    /// No preconditions: callable from safe code.
    pub safe fn abs(x: c_int) -> c_int;
    /// Reads until NUL: the caller must pass a valid C string.
    pub unsafe fn strlen(s: *const c_char) -> usize;
}

pub fn distance(a: c_int, b: c_int) -> c_int {
    abs(a - b) // no unsafe block needed
}

pub fn c_len(s: &str) -> Result<usize, NulError> {
    let owned = CString::new(s)?; // fails on interior NUL
    // SAFETY: `owned` is NUL-terminated and outlives the call; strlen only reads.
    Ok(unsafe { strlen(owned.as_ptr()) })
}
```

### UNSAFE-06: Use the unsafe attribute forms: `#[unsafe(no_mangle)]`, `#[unsafe(export_name)]`, `#[unsafe(link_section)]`
**Default:** in edition 2024 these attributes must be written `#[unsafe(..)]` — an unmangled symbol
can collide with or override any other symbol in the final binary. Choose globally unique names
(prefix with your crate), and mark exported functions that take raw pointers `unsafe extern "C" fn`
(`clippy::not_unsafe_ptr_arg_deref` flags safe ones).

```rust
use std::ffi::c_int;

#[unsafe(no_mangle)]
pub extern "C" fn rustkb_add(a: c_int, b: c_int) -> c_int {
    a.wrapping_add(b) // never panic across FFI (UNSAFE-13)
}
```

### UNSAFE-07: Don't take references to `static mut`; don't call `set_var` after threads start
**Default:** `static` + atomics / `Mutex` / `OnceLock` instead of `static mut`. Edition 2024 makes
`static_mut_refs` deny-by-default: `&STATIC_MUT`, `&mut STATIC_MUT` and method calls that borrow
it are errors (plain `X += 1` still compiles — and is still a data race across threads). **Use** `&raw const`
/ `&raw mut STATIC` (1.82) only when C code needs a pointer to a Rust-owned static.
`std::env::set_var` / `remove_var` are `unsafe` in edition 2024 because other threads (including C
code reading `getenv`) may race. **Use** `Command::env` for child processes and pass configuration
explicitly; call `set_var` only at the very start of `main`, before any thread exists.

```rust
// ❌ edition 2024: error "creating a mutable reference to mutable static"; and a data race
static mut SEEN: Vec<String> = Vec::new();
pub fn remember_bad(name: String) {
    unsafe { SEEN.push(name) } // method call = implicit &mut SEEN
}
```

```rust
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new()); // const-constructible, no unsafe
static REQUESTS: AtomicU64 = AtomicU64::new(0);

pub fn remember(name: String) -> u64 {
    SEEN.lock().expect("SEEN lock poisoned").push(name);
    REQUESTS.fetch_add(1, Ordering::Relaxed) + 1
}

pub fn git_status() -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("git").arg("status").env("GIT_PAGER", "cat").status() // no set_var
}
```

## Raw memory

### UNSAFE-08: Use `&raw`, `MaybeUninit` and strict-provenance APIs, not the old idioms
**Default:** `&raw const place` / `&raw mut place` (1.82) to get a pointer without creating an
intermediate reference (packed fields, uninitialised memory, statics). `MaybeUninit<T>` for
uninitialised memory — never `mem::uninitialized()` and never `mem::zeroed()` for types where all-zero
is invalid (references, `NonZero*`, enums, `bool` from arbitrary bytes). Pointer ⇄ integer: use
`ptr.addr()`, `ptr.with_addr(a)`, `ptr.map_addr(f)`, `ptr::without_provenance` (strict provenance,
1.84); use `expose_provenance` / `with_exposed_provenance` only for integers that round-trip through
C. **Never** `transmute` an integer into a pointer.

```rust
#[repr(C, packed)]
pub struct WireHeader {
    pub tag: u8,
    pub len: u32, // unaligned: `&self.len` would be UB, and is rejected by the compiler
}

pub fn len_ptr(h: &WireHeader) -> *const u32 {
    &raw const h.len // pointer without an intermediate (misaligned) reference
}

pub fn len(h: &WireHeader) -> u32 {
    h.len // reading a Copy field by value is safe; prefer this when you just need the value
}
```

### UNSAFE-09: Replace `transmute` and pointer casts on bytes with `bytemuck` or `zerocopy`
**Default:** `bytemuck` (`Pod`/`Zeroable` derives, `cast_slice`, `pod_read_unaligned`) for
plain-old-data casts; `zerocopy` (`FromBytes`, `IntoBytes`, `KnownLayout`, `Immutable` derives,
`ref_from_bytes`) when parsing untrusted buffers with layout checks and endian-aware types. Both
verify at compile time that every bit pattern is valid and there is no padding. **Never**
`transmute::<&[u8], &Header>` or `ptr as *const Header` on network/file bytes — alignment, padding
and invalid bit patterns are all UB.

```rust
use bytemuck::{Pod, Zeroable};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct Header {
    pub magic: u32,
    pub len: u32,
}

pub fn parse_header(buf: &[u8]) -> Option<Header> {
    let bytes = buf.get(..size_of::<Header>())?;
    bytemuck::try_pod_read_unaligned(bytes).ok() // copies; works for any alignment
}

pub fn as_bytes(headers: &[Header]) -> &[u8] {
    bytemuck::cast_slice(headers) // zero-copy, checked at compile time
}
```

## FFI boundaries

### UNSAFE-10: Generate bindings and wrap them in a safe layer
**Default:** `bindgen` for C headers (in `build.rs` or pre-generated and committed), `cbindgen` to
emit a C header for a Rust library, `cxx` for C++ interop. Put raw bindings in a `foo-sys` crate
and the safe wrapper in `foo` (layout: rust-architecture). **Never** hand-transcribe large C
headers — one wrong integer width or missing field is silent UB.

### UNSAFE-11: Pass strings as `CStr`/`CString`, and keep the owner alive
**Default:** Rust → C: `CString::new(s)?` then `.as_ptr()` while the `CString` is alive; constants
as `c"literal"` (`&CStr`, 1.77). C → Rust: `unsafe { CStr::from_ptr(p) }` then `to_str()` (fallible)
or `to_string_lossy()`. **Never** pass `str.as_ptr()` to C (no NUL terminator), and never write
`CString::new(s).unwrap().as_ptr()` in an argument position you store — the temporary is freed at
the end of the statement (`dangling_pointers_from_temporaries` warns).

### UNSAFE-12: Give FFI types a defined layout
**Default:** `#[repr(C)]` on every struct/enum passed by value or pointed to by C;
fieldless enums as `#[repr(C)]`/`#[repr(u32)]` **only** if C can never send an unknown value —
otherwise receive a plain integer and convert with `TryFrom`. Nullable pointers as
`Option<&T>`, `Option<NonNull<T>>`, `Option<extern "C" fn(..)>` (guaranteed to be pointer-sized with
`None` = null). Opaque Rust types cross the boundary only behind a pointer (UNSAFE-14).

### UNSAFE-13: Never let a panic cross an `extern "C"` boundary
**Default:** a panic escaping an `extern "C" fn` aborts the process (since 1.81). At every exported
entry point that can panic, wrap the body in `std::panic::catch_unwind` and map a panic to an
error code. **Use** `extern "C-unwind"` only when you deliberately want Rust panics and C++
exceptions to unwind through each other's frames.

```rust
use std::ffi::{CStr, c_char, c_int};
use std::panic::catch_unwind;

/// Parses a TCP port; returns -1 on invalid input, -2 on internal panic.
///
/// # Safety
///
/// `input` must be null or point to a NUL-terminated string valid for reads.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustkb_parse_port(input: *const c_char) -> c_int {
    if input.is_null() {
        return -1;
    }
    // SAFETY: non-null and NUL-terminated per the # Safety contract.
    let text = unsafe { CStr::from_ptr(input) };
    catch_unwind(|| text.to_str().ok().and_then(|s| s.parse::<u16>().ok()).map_or(-1, c_int::from))
        .unwrap_or(-2)
}
```

### UNSAFE-14: Free memory with the allocator that allocated it
**Default:** a Rust object handed to C is `Box::into_raw` in a `*_new` function and released by a
matching exported `*_free` that calls `Box::from_raw` exactly once; C-allocated memory is freed by
the C library's own free function. **Never** call `libc::free` on Rust memory or drop C memory as a
Rust `Box`/`Vec`/`CString`, and never free twice (null the handle on the C side).

```rust
pub struct Counter {
    hits: u64,
}

#[unsafe(no_mangle)]
pub extern "C" fn rustkb_counter_new() -> *mut Counter {
    Box::into_raw(Box::new(Counter { hits: 0 }))
}

/// # Safety
///
/// `counter` must come from `rustkb_counter_new`, not be freed, and not be used concurrently.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustkb_counter_hit(counter: *mut Counter) -> u64 {
    // SAFETY: live, exclusively-owned pointer per the # Safety contract.
    let counter = unsafe { &mut *counter };
    counter.hits += 1;
    counter.hits
}

/// # Safety
///
/// `counter` must be null or come from `rustkb_counter_new`; it must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustkb_counter_free(counter: *mut Counter) {
    if !counter.is_null() {
        // SAFETY: produced by Box::into_raw in rustkb_counter_new and freed exactly once.
        drop(unsafe { Box::from_raw(counter) });
    }
}
```

## Verifying unsafe code

### UNSAFE-15: Run the tests that exercise unsafe code under Miri
**Default:** every crate with `unsafe` runs its tests under Miri in CI (nightly only):

```sh
rustup toolchain install nightly --component miri
cargo +nightly miri setup
cargo +nightly miri test
MIRIFLAGS="-Zmiri-tree-borrows" cargo +nightly miri test   # alternative aliasing model
MIRIFLAGS="-Zmiri-many-seeds" cargo +nightly miri test     # explore more thread interleavings
```

Miri detects out-of-bounds and use-after-free accesses, invalid values, misalignment, aliasing
violations and data races — but only on paths the tests execute, and it cannot call most foreign
functions: mark FFI-calling tests `#[cfg_attr(miri, ignore)]` and test the pure-Rust layers.
**Use** sanitizers (nightly `-Zsanitizer=address`) for code that calls C; fuzzing and review
process: rust-security. `unsafe impl Send/Sync` needs the same `SAFETY:` justification as a block.

## Review checklist

- [ ] Each `unsafe` has no safe equivalent; unaffected crates `forbid(unsafe_code)` (UNSAFE-01)
- [ ] Public safe APIs are sound for every input; invariants guarded by private fields (UNSAFE-02)
- [ ] Every `unsafe {}` has a factual `// SAFETY:`; every `unsafe fn` has `# Safety` (UNSAFE-03)
- [ ] Unsafe ops inside `unsafe fn` are wrapped in their own blocks (UNSAFE-04)
- [ ] `unsafe extern` blocks with `safe fn` where applicable; `core::ffi` types (UNSAFE-05)
- [ ] `#[unsafe(no_mangle)]` names are crate-prefixed; pointer-taking exports are `unsafe fn` (UNSAFE-06)
- [ ] No `static mut` references, no `set_var` after threads start (UNSAFE-07)
- [ ] `&raw`/`MaybeUninit`/strict-provenance APIs; no int→ptr `transmute` (UNSAFE-08)
- [ ] Byte reinterpretation via `bytemuck`/`zerocopy`, never `transmute` (UNSAFE-09)
- [ ] Bindings generated, strings via `CStr`/`CString`, `repr(C)` layouts (UNSAFE-10..12)
- [ ] Exports catch panics; memory freed by its own allocator (UNSAFE-13, UNSAFE-14)
- [ ] Tests run under Miri in CI; FFI tests `cfg_attr(miri, ignore)` (UNSAFE-15)
