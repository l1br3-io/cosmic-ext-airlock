# Rust Testing Rule

Testing standards synthesized from Effective Rust Item 30 and RfR Chapter 6.
Enforced by the rust-test-engineer agent and the developer workflow.

## 1. Test pyramid — use all layers

| Layer         | Location                        | Access       | Purpose                              |
|---------------|---------------------------------|--------------|--------------------------------------|
| Unit          | `#[cfg(test)] mod tests` in src | private      | fast, fine-grained, edge cases       |
| Integration   | `tests/*.rs`                    | public API   | consumer-perspective, wiring bugs    |
| Doc           | `///` blocks                    | public API   | examples that must stay correct      |
| Property      | `proptest`/`quickcheck`         | any          | invariants over input domains        |
| Fuzz          | `cargo-fuzz` targets            | input funcs  | untrusted-bytes robustness           |
| Bench         | `benches/` (`criterion`)        | hot paths    | performance regressions              |

"Write more than unit tests" (Effective 30): unit tests alone miss integration
and API-ergonomics issues. Every public capability needs at least one
integration or doc test exercising it end-to-end.

## 2. Unit tests — `#[cfg(test)]`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty() { assert!(parse("").unwrap().is_empty()); }

    #[test]
    fn rejects_non_utf8() {
        assert_matches!(parse(&[0xFF, 0xFE]), Err(ParseError::InvalidUtf8));
    }
}
```
- Co-located with the code, in the same file, under `#[cfg(test)]`.
- One logical assertion per `#[test]`; descriptive name = the scenario.
- Prefer specific macros: `assert_eq!`, `assert_ne!`, `assert_matches!`.
- No real time / network / filesystem / RNG — inject them.
- Fast: each test <100ms. Slow setup → `OnceLock`/`OnceCell` shared fixtures.

## 3. Integration tests — `tests/`

- `tests/foo.rs` exercises the public API via `use your_crate::...`.
- One file per concern/feature; shared helpers in `tests/common/mod.rs`.
- These catch: wrong visibility, broken re-exports, API ergonomics, multiple
  modules interacting.
- Use realistic but hermetic fixtures: temp dirs (`tempfile`), in-process
  servers, mock transports.

## 4. Doc tests — compile & run

- Every public function/type with non-trivial behavior gets a runnable example.
- ```` ```no_run ```` for examples with side effects you don't want executed;
  ```` ```ignore ```` for illustrative-only; ```` ```compile_fail ```` to assert
  an API misuses won't compile.
- `#` lines hide setup while keeping the example compilable.
- Run `cargo test --doc` — broken examples are test failures.

## 5. Property tests — `proptest` (preferred) / `quickcheck`

For any function with a clear input domain and invariant:
- **Round-trips**: `parse(format(x)) == x`.
- **Laws**: associativity, identity, monotonicity.
- **Invariants**: parser never panics, output normalizes, etc.
- Let `proptest` shrink to find minimal failing cases.
- Seed deterministically; property tests must be reproducible in CI.

```rust
proptest! {
    #[test]
    fn roundtrip(n in 0u64..) {
        prop_assert_eq!(parse(&format!("{n}")).unwrap(), n);
    }
}
```

## 6. Fuzz tests — `cargo-fuzz` (libFuzzer)

For parsers/decoders/deserializers/anything accepting bytes from outside:
```rust
#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| { let _ = my_crate::parse(data); });
```
- Long-running (minutes to hours); separate from the fast unit-test step.
- Run in nightly CI or a dedicated fuzz job. File crash repros as regression
  tests.

## 7. Concurrency & async testing (RfR Ch. 10)

- Async: `#[tokio::test]` (or your runtime's equivalent). Prefer `.await` over
  `block_on`.
- Time: `tokio::time::pause()` + `advance`, or an injected `Clock` — never
  `std::thread::sleep` in tests.
- Stress tests: loop N iterations spawning threads/tasks to surface races.
- Validated concurrency models: `loom` for lock-free structures.

## 8. Coverage & quality gates

- Coverage (`cargo-llvm-cov`/`cargo-tarpaulin`): track trend; aim for new code
  covered, not a fixed project-wide %.
- Mutation testing (`cargo-mutants`): detect tests that pass against wrong impls.
- Snapshot tests (`insta`) for serializable/stable output; review snapshots in PRs.

## 9. What to test (beyond happy path)

- Every `Err` arm reachable by a test.
- Boundaries: empty, single, `usize::MAX`-ish, overflow (`+1`, `*2`).
- Unicode: ASCII, multibyte, combining chars, invalid UTF-8.
- Malformed input for parsers.
- Concurrency: interleavings, timeouts, cancellation.
- Regression tests for every fixed bug (cite the issue).

## 10. Determinism & hygiene

- Tests must pass on any machine, any timezone, any order (`cargo test -- --test-threads=1` should also pass).
- No reliance on `now()`, `random()` without a seed, env vars, host filesystem
  layout, or network.
- Clean up: temp files, ports, env var mutation (save/restore or use `tempfile`).
- `#[should_panic(expected = "...")]` only for genuinely unreachable invariants,
  never to paper over a real failure mode.

## 11. CI workflow (Effective 32)

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test --doc
cargo bench --no-run            # compile benches
# nightly/periodic:
cargo +nightly fuzz run <target> -- -max_total_time=60
cargo llvm-cov --workspace      # coverage report
cargo audit && cargo deny check
```

Run on every PR plus a scheduled nightly for fuzz/coverage/audit. Use a matrix
across target platforms (linux/macos/windows) and stable + MSRV toolchains.
