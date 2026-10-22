# Rust Security Rule

Authority for the security agent and any Rust author handling `unsafe`, FFI,
untrusted input, or supply-chain concerns. Synthesized from Effective Rust
Items 16, 33–35 and RfR Chapters 9, 11. **Rust eliminates memory-safety and
data-race bugs *in safe code*; everything below addresses the residual risk.**

## 1. `unsafe` policy (Effective 16; RfR Ch. 9)

`unsafe` is not forbidden but is held to a high bar.

### Before writing `unsafe`, exhaust alternatives
- FFI? Use `bindgen` (Effective 35) instead of hand-written bindings.
- Performance? Measure first (Effective 20); the safe version is often as fast.
- Bit-pattern conversion? `try_from`/`bytemuck::Pod` over `transmute`.
- Sharing across threads? Channels / `Arc<Mutex<_>>` / atomics over raw sync.

### Mandatory when `unsafe` is used
1. **Minimal scope**: confine to the smallest block possible, never a whole fn
   just to call one unsafe op.
2. **`// SAFETY:` comment** immediately above explaining the invariant relied on
   and why it holds. The comment must name the specific unsafe precondition.
3. **Encapsulation**: keep unsafe behind a safe API whose types/visibility make
   violating the invariant impossible from safe code. Private fields, not pub.
4. **No `unsafe fn` exposed as safe without a `# Safety` doc** explaining the
   caller's obligations. Prefer wrapping so callers never touch `unsafe`.
5. **Audit inputs**: any data crossing into `unsafe` from outside the type must
   be validated (lengths, nullness, validity of bit patterns).

### High-risk unsafe operations (extra scrutiny)
- `std::mem::transmute` — almost always wrong; use specific conversion APIs.
- `slice::get_unchecked` / pointer indexing — verify bounds by construction.
- `str::from_utf8_unchecked` — validate UTF-8 first; or just use `from_utf8`.
- Manual `Send`/`Sync` impls — justify why cross-thread sharing is sound.
- Calling C functions — see FFI section.
- `union` reads — every access must validate the active variant.

### Validity vs. safety (RfR Ch. 9)
Unsafe code may rely on **validity** (bit pattern is legal for the type) and
**safety** (logical invariants). Document both. A field the type assumes valid
must be private with no `&mut` escape route.

## 2. FFI boundaries (Effective 34, 35; RfR Ch. 11)

Treat the FFI boundary as a security perimeter. Validate **everything**.

- Use `bindgen` for C headers (Effective 35); regenerate in CI via `build.rs`.
- Validate on entry: null pointers, lengths against buffer bounds, enum
  discriminants, UTF-8, integer ranges. Fail closed (return error, don't abort).
- **Opaque handles**: expose foreign code a pointer to a boxed Rust type via a
  `Box::into_raw`/`from_raw` pair; provide an explicit free function. Never let
  foreign code see Rust generics/Drop types directly.
- **Ownership**: document who allocates and who frees, and with which allocator.
  Mismatching `malloc`/free with Rust's allocator is UB.
- **Callbacks**: functions passed to C must be `extern "C"`. A Rust panic
  unwinding into C is UB — either `catch_unwind` at the boundary or compile with
  `panic = "abort"`.
- **String types**: C strings are not guaranteed UTF-8; use `CString`/`OsStr`
  conversions explicitly; never `from_utf8_unchecked` foreign input.
- **Repr**: use `#[repr(C)]` for structs passed across FFI; otherwise layout is
  unspecified.

## 3. Panics as denial-of-service (Effective 18)

Any panic on attacker-influenced input is a DoS vector. In servers a panic kills
a thread (or, with `panic=abort`, the process).

Forbidden on untrusted input:
- `.unwrap()` / `.expect()` / direct indexing `v[i]`
- integer division `/` `%` (use `checked_*`)
- `as` truncating casts of sizes/offsets (use `try_into`)
- `str::from_utf8`, `char::from_u32` unchecked variants
- array/slice slicing with computed bounds

Replace with `?` / `.get(i)?` / checked arithmetic / `try_from`.

## 4. Arithmetic & casts

- Use `usize` for sizes/indices; never `as` between signed/unsigned on sizes.
- Sizes and offsets → checked or saturating arithmetic.
- Money/quantities → `i64`/`u128` with checked math, or a decimal crate.
- Prefer `u32::try_from(x)?` over `x as u32`.

## 5. Cryptography & secrets

- **Never invent cryptography.** Use `rustls`/`ring`/`RustCrypto`/`aws-lc`.
- Disable TLS cert verification only behind an explicit `danger_accept_invalid`
  flag that no production default enables; lint for it.
- Secrets live in types that:
  - zeroize on drop (`zeroize::Zeroize` / `Zeroizing<T>`),
  - derive `Debug`/`Display` that redact (e.g. `"***"`),
  - are `Clone` only deliberately (consider `Zeroizing<Box<[u8]>>`).
- Constant-time comparisons for tokens/MACs (`subtle::ConstantTimeEq`).
- RNG: `rand::rngs::OsRng` / `thread_rng()` (backed by OS) for crypto; never a
  seeded `SmallRng` for security decisions.
- Password hashing: `argon2` / `bcrypt` / `scrypt` via dedicated crates.

## 6. Deserialization & untrusted input

- `#[serde(deny_unknown_fields)]` where strict schemas are wanted.
- Bound input size before parsing; stream unbounded sources.
- Prefer typed structs over `serde_json::Value`; `Value` admits unbounded
  nesting/size and is a common memory-blowup vector.
- Validate after parse: ranges, enum discriminants, string lengths, UTF-8.
- Path traversal: canonicalize and check prefix for any filename from input.
- Format strings (`format!`, `println!`) never take untrusted format strings —
  that's a format-string injection. Use the data as an argument, not the fmt.

## 7. Supply chain (Effective 21, 25, 26)

- `cargo audit` for known CVEs; run in CI on every PR and nightly.
- `cargo deny` for advisories, license policy, banned crates, duplicate deps.
- Check `Cargo.lock` into VCS for binaries; for libraries decide deliberately.
- Before adding a dependency, review:
  - maintainer trust + activity, last release date,
  - download/dependents count, open security issues,
  - transitive dependency footprint (`cargo tree -d`),
  - whether a feature flag can scope it (Effective 26).
- **Build scripts and proc-macros run arbitrary code at build time** with the
  developer's privileges. Audit them like application code.
- Pin build-time toolchain with `rust-toolchain.toml` for reproducibility.

## 8. Concurrency safety (Effective 17; RfR Ch. 10)

- Safe Rust prevents data races, but not all concurrency bugs (deadlocks, logic
  races, torn reads at the API level).
- Manual `unsafe impl Send/Sync` is a red flag — justify each.
- Memory ordering for atomics: default `SeqCst` unless you can prove weaker is
  correct; `Relaxed` only for statistics/flags with no ordering dependency.
- Prefer message passing (`crossbeam`, `tokio::sync`, std `mpsc`) over shared
  `Mutex<state>`. Channels localize the critical section.
- Stress-test concurrent code (`loom`, long-running harnesses) (RfR Ch. 10).

## 9. Logging & error leakage

- Never log secrets, tokens, credentials, full auth headers, or PII.
- `Display` impls of errors may surface to users — keep them generic; push detail
  into structured fields or server-only logs.
- Redact before tracing: wrap secret values in a redacting `Debug`.
- Scrub `Debug` of request/response loggers for sensitive fields.

## 10. Required CI checks

```bash
cargo audit
cargo deny check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```
Gate merges on all of the above. Re-run `cargo audit`/`cargo deny` after any
dependency change.
