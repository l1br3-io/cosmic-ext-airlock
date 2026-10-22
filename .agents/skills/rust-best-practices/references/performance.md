# Rust Performance Rule

Performance guidance synthesized from Effective Rust Items 9, 12, 17, 20 and
RfR Chapters 1, 2, 10. **Governing principle: don't optimize prematurely —
measure, then fix the measured bottleneck (Effective 20).**

## 0. Process first (Effective Item 20)

1. **Write the clear, idiomatic version.** Rust's safe abstractions are fast by
   default; optimizer + borrow checker do the heavy lifting.
2. **Measure** with a profiler (`perf`, `flamegraph`, `cargo flamegraph`,
   `samply`) and benchmarks (`criterion`). Don't optimize on intuition.
3. **Fix the measured hotspot.** Re-measure to confirm. Keep the change only if
   it produces a real, reproducible improvement.
4. **Document** any non-obvious optimization with a comment + benchmark reference.

A 2x speedup in cold code is worse than readable code; a 2% speedup in a hot
loop is rarely worth the complexity.

## 1. Heap allocation is the usual suspect

- `String`, `Vec`, `VecDeque`, `HashMap`, `Box`, `Rc` allocate. Each allocation
  in a hot loop is a candidate for elimination.
- **Borrow, don't own**: `&str`, `&[T]`, `Cow<'_, str>` in function signatures
  and loop bodies.
- **Capacity hints**: `Vec::with_capacity(n)`, `String::with_capacity(n)`,
  `HashMap::with_capacity(n)` when size is predictable. Avoids reallocation+copy.
- **Reuse buffers**: pass a `&mut Vec<_>` / `&mut String` into a function rather
  than returning a fresh allocation, for code called in a tight loop.
- **Small-array optimization**: `smallvec::SmallVec`, `arrayvec::ArrayVec`, or
  plain arrays `[T; N]` when N is small and bounded.
- `bool`/`char`/small enums are free; prefer them over `String`-tagged variants.

## 2. Avoid unnecessary clones

- `.clone()` of `String`/`Vec`/`HashMap` in a loop is a hidden allocation.
- Move borrows through the call graph: `fn f(s: &str)`, not `fn f(s: String)`.
- If the borrow checker demands a clone, the data shape is usually wrong —
  restructure ownership (lifetimes, indices, `Rc`/`Arc` for shared read).
- `#[derive(Clone)]` is convenient; audit whether each call site needs it.

## 3. Iterators (Effective 9)

- Iterators compile to tight loops; prefer them over index-based `for`.
- Lazy combinators (`map`, `filter`, `take`) fuse into one pass — better than
  `collect`-then-re-iterate.
- `.collect()` allocates; do it once, with an explicit type, at the end.
- `&[T]` slices and `Iterator::copied`/`cloned` over `&T` avoid indirect reads
  when `T: Copy`.
- `chunks`/`windows`/`split` for slice processing; `par_iter` (rayon) for
  CPU-bound parallelism across large collections.
- Avoid `.collect::<Vec<_>>().iter()` patterns — keep the pipeline lazy.

## 4. String handling

- `&str` for reads; `String` only when you need to own/modify.
- `format!` allocates; in hot paths build with `write!(buf, ...)` into a
  reusable `String`/`String::with_capacity`.
- For ASCII operations, `as_bytes()` + byte iteration beats Unicode-aware
  `chars()` when you've validated ASCII.
- `Cow<str>` to defer/avoid allocation when most inputs are already owned.

## 5. Hashing

- `std::collections::HashMap` uses SipHash (DoS-resistant, not fastest). For
  non-attacker-controlled keys in hot paths, `ahash`/`fxhash`/`rustc-hash` are
  much faster — swap via the hasher type parameter.
- For small maps, linear scan of a `Vec<(K,V)>` beats `HashMap` (cache locality).
- `BTreeMap` when you need ordering or range queries; its locality is also good.

## 6. Branches & layout

- Hot enum dispatch: a `match` on a field-less enum is branch-prediction-
  friendly; `Box<dyn Trait>` adds indirection. Prefer enum dispatch for closed
  sets (see `patterns.md`).
- `#[inline]` on small hot functions; let the optimizer decide for the rest.
  Cross-crate hot functions benefit from explicit `#[inline]`.
- Struct field ordering: `#[repr(C)]` only if you need a stable ABI/FFI; the
  default reorders fields to minimize padding — usually leave it alone.
- `Vec<bool>` is 8x larger than needed; a bitset (`bitvec`, `fixedbitset`) or
  packed representation for large boolean collections.

## 7. Concurrency & parallelism (Effective 17; RfR Ch. 10)

- **Identify the bottleneck first**: CPU-bound → parallelism; I/O-bound →
  concurrency/async.
- CPU-bound over a collection: `rayon`'s `par_iter()` is the default choice.
- I/O-bound many connections: `tokio`/`async-std` async with `await` points.
- Avoid shared mutable state; if unavoidable, prefer:
  - `arc-swap`/`ArcSwap` for read-heavy shared values,
  - `crossbeam`/`dashmap` concurrent collections over `Mutex<HashMap>`,
  - sharded locks over one giant lock.
- Channel throughput: bounded channels apply backpressure; unbounded can OOM.
- Thread/task spawn has cost — pool workers (`rayon`, `tokio` runtime) rather
  than spawn-per-item.

## 8. Async performance (RfR Ch. 8)

- `async fn` is zero-cost per suspension point but creates a state-machine
  allocation if boxed (`Box::pin`) or if the future is large and lives across
  await. Keep futures small; don't hold large locals across `.await`.
- Don't `.await` while holding a `Mutex` guard across threads (long critical
  section); use `tokio::sync::Mutex` only when you must await under the lock.
- Avoid `block_on` inside async contexts (deadlocks, runtime stalls).
- Batch I/O: many small `await`s serialize latency — batch with `join_all`/
  `FuturesUnordered` for independent operations.

## 9. Generic vs. dynamic dispatch (Effective 12)

- Generics (`impl Trait` / `<T: Trait>`) monomorphize → no dispatch cost, larger
  binary. Best for hot single-type code.
- `dyn Trait` → one vtable indirection per call, smaller binary, heterogeneous.
- For a hot loop calling a trait method on one concrete type, generics win.
- Excessive monomorphization bloats binary and compile time; consider a generic
  front-end calling a `dyn` inner ("sealed monomorphization" pattern).

## 10. Compile-time & build perf

- Split crates so incremental rebuilds are cheap (workspace layout).
- Use `cargo nextest run` for faster test execution (parallel, finer-grained).
- Reduce proc-macro usage where a derive/trait/function would do.
- `sccache`/`mold`/`lld` linkers speed up dev iteration.
- `Cargo.toml`: set `[profile.dev]` split debuginfo, `[profile.release]`
  `lto = "thin"`, `codegen-units = 1` for release perf (slower build).

## 11. Measurement tooling (default set)

```bash
cargo install cargo-flamegraph samply criterion  # one-time
cargo flamegraph --bin <name>                    # CPU profile
samply record -- target/release/<name>           # sampling profiler
# in benches/:
criterion::Bencher via `cargo bench`
```
Always benchmark before/after with `criterion`'s statistical comparison; report
the change in PR descriptions. **No performance claim without a benchmark.**

## Anti-patterns

- Optimizing before measuring (Effective 20).
- `Box<dyn Trait>` "for flexibility" in a hot loop where generics would do.
- `.clone()` to appease the borrow checker without investigating ownership.
- `Vec` grown without `with_capacity` in a known-size loop.
- Unbounded channels / unbounded `Vec` accumulation under load (memory blowup).
- Premature `unsafe` for speed (Effective 16) — safe code is usually as fast.
- Blocking I/O inside async tasks (stalls the runtime).
