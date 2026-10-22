---
name: rust-best-practices
description: >
  Guide for writing idiomatic, safe, and performant Rust, synthesized from
  Effective Rust and Rust for Rustaceans. Use this skill when:
  (1) writing new Rust code or functions,
  (2) reviewing or refactoring existing Rust code,
  (3) deciding between borrowing vs cloning or ownership patterns,
  (4) implementing error handling with Result types,
  (5) touching unsafe, FFI, untrusted input, or dependencies,
  (6) writing tests or documentation for Rust projects.
license: MIT
compatibility: Rust 1.85+ (edition 2024), Cargo
metadata:
  author: opencode-agent-harness
  version: "2.0.0"
allowed-tools: Bash(cargo:*) Bash(rustc:*) Bash(rustfmt:*) Bash(clippy:*) Read Write Edit Glob Grep
---

# Rust Best Practices

Apply these guidelines when writing or reviewing Rust code. Synthesized from
*Effective Rust* (Dry) and *Rust for Rustaceans* (Nelson); individual rules cite
their source item/chapter (e.g. Effective 18, RfR Ch. 9).

## How to use this skill

Read ONLY the reference files that match the task — never load all of them by
default. Cite the relevant rule when providing review feedback.

| Task | Read |
|------|------|
| Writing or reviewing any Rust code | `references/coding-style.md` + `references/patterns.md` |
| `unsafe`, FFI, untrusted input (PTY output, env vars), secrets, dependency changes | add `references/security.md` |
| Writing or updating tests | add `references/testing.md` |
| Performance work (only when performance is explicitly in scope) | add `references/performance.md` |

## Quick Reference

### Borrowing & Ownership
- Take the least powerful reference that works: `&T` > `&mut T` > `T`
- Use `&str` over `String`, `&[T]` over `Vec<T>` in function parameters; own only when storing or mutating
- Small `Copy` types can be passed by value
- Use `Cow<'_, T>` when a function sometimes returns borrowed, sometimes owned
- `clone()` to satisfy the borrow checker usually signals a design issue — restructure ownership; if genuinely needed, comment why

### Error Handling
- Return `Result<T, E>` for fallible operations; never panic to signal expected failure
- `unwrap()`/`expect()` only when the state is provably unreachable given type invariants, or in tests; prefer `expect("reason")` over `unwrap()`
- Use `thiserror` for library errors (`#[non_exhaustive]`, `#[from]`), `anyhow` for binaries only
- Prefer `?` and combinator methods (`map`, `and_then`, `ok_or`) over hand-written `match`

### Performance
- Measure before optimizing (`cargo flamegraph`, `samply`, `criterion`) — no performance claim without a benchmark
- Run `cargo clippy -- -D clippy::perf` for performance hints
- `.clone()` of `String`/`Vec`/`HashMap` in a loop is a hidden allocation; `Vec::with_capacity(n)` when size is predictable
- Prefer iterators over manual loops; keep pipelines lazy, `.collect()` once at the end

### Linting
Run regularly: `cargo clippy --all-targets --all-features --locked -- -D warnings`

Key lints to watch:
- `redundant_clone` - unnecessary cloning
- `large_enum_variant` - oversized variants (consider boxing)
- `needless_collect` - premature collection

Use `#[expect(clippy::lint)]` over `#[allow(...)]` with justification comment.

### Testing
- Name tests descriptively: `process_should_return_error_when_input_empty()`
- One logical assertion per test when possible
- Every public capability needs at least one integration or doc test
- Consider `cargo insta` for snapshot testing generated output

### Generics & Dispatch
- Prefer generics (static dispatch) for performance-critical code
- Use `dyn Trait` only when object safety or heterogeneous collections demand it
- For a closed set of behaviors, enum dispatch beats `Box<dyn Trait>` (see `references/patterns.md`)

### Type State Pattern
Encode valid states in the type system to catch invalid operations at compile time:
```rust
struct Connection<State> { /* ... */ _state: PhantomData<State> }
struct Disconnected;
struct Connected;

impl Connection<Connected> {
    fn send(&self, data: &[u8]) { /* only connected can send */ }
}
```

### Documentation
- `//` comments explain *why* (safety, workarounds, design rationale)
- `///` doc comments explain *what* and *how* for public APIs; document `# Panics`, `# Errors`, `# Safety` sections
- `// SAFETY:` comment is mandatory before every `unsafe` operation
- Every `TODO` needs a linked issue: `// TODO(#42): ...`
- Enable `#![deny(missing_docs)]` for libraries

## Project gates

Per `AGENTS.md`: `cargo clippy` and `cargo fmt --check` must pass before a
change counts as done; `cargo test` is the verification gate. Runtime behavior
is only testable on real COSMIC hardware — unit tests cover what can be tested
headless.
