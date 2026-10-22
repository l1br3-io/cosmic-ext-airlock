# Rust Coding Style Rule

Authoritative style rule, synthesized from *Effective Rust* Items 1–15, 18, 22,
23, 27 and *Rust for Rustaceans* Ch. 1–4. Enforced by `cargo fmt` and clippy,
plus the conventions below.

## 1. Formatting & tooling (non-negotiable)

- `cargo fmt --all` governs layout. Never hand-format; configure `.rustfmt.toml`
  for project-wide rules (e.g. `edition`, `max_width`).
- `cargo clippy --all-targets --all-features -- -D warnings` must be clean.
  Allow a lint only with an in-source `#[allow(...)]` and a justifying comment.
- Run `cargo test --all-targets` and `cargo test --doc` before declaring done.

## 2. Naming (matches std convention)

| Item                     | Style        | Example            |
|--------------------------|--------------|--------------------|
| Types, traits, enums     | `UpperCamel` | `HttpClient`       |
| Variants, constants      | `UpperCamel` / `SCREAMING_SNAKE` (consts) | `Color::Red`, `MAX_LEN` |
| Functions, methods, vars | `snake_case` | `parse_request`    |
| Modules, crates          | `snake_case` | `http_client`      |
| Lifetimes                | short, `'a`/`'src` | lifetimes meaningful when >1 |
| Generic types            | `T`, `U`, or `UpperCamel` if semantic (`Key`, `Value`) | |

- Convert `bool` parameters to enums when meaning is unclear at the call site
  (Effective Item 1). `print_page(Sides::Both, Output::Color)` beats
  `print_page(true, false)`.
- Avoid `get_` prefix on accessors (`fn name(&self)`, not `fn get_name`).
- Iterator/bool accessors may use `is_`/`has_` (`is_empty`, `has_pending`).

## 3. Module & visibility

- Default to **private**; widen only when external callers need it (Effective 22).
- Prefer `pub(crate)` for internal cross-module use.
- Group `use` statements: std, then external crates, then `crate::`/`super::`,
  each block blank-line separated.
- **No glob imports** in library code except the prelude pattern (Effective 23).
  `use std::io::Read;` not `use std::io::*;`.
- Re-export crate types used in your public API from `lib.rs`.

## 4. Ownership & borrowing (Effective 8, 14, 15; RfR Ch. 1)

- Take the **least powerful reference** that works: `&T` > `&mut T` > `T`.
- Function inputs: prefer `&str`, `&[T]`, `impl AsRef<str>`; own (`String`,
  `Vec<T>`) only when you will store or mutate the buffer.
- Return borrows tied to inputs when possible (`fn name(&self) -> &str`).
- Avoid `clone()` to satisfy the borrow checker — it usually signals a design
  issue. If a clone is genuinely needed, comment why.
- Long-lived self-referential or graph data → use indices into a `Vec`/`SlotMap`
  rather than `Rc<RefCell<...>>` webs.
- `Cow<'a, T>` when a function sometimes returns borrowed, sometimes owned.

## 5. Lifetimes

- Let elision do its job; annotate only when the compiler demands or there are
  multiple input lifetimes with non-obvious output ties.
- Name lifetimes when multiple exist and the relationship matters: `'src`,
  `'input`. Single anonymous `'a` is fine.
- Beware lifetime extension via `unsafe`; prefer restructuring ownership.

## 6. Types & enums (Effective 1, 6, 7)

- **Make invalid states unrepresentable** — see `patterns.md`.
- Use `enum` with fields to model state machines and sum types.
- Prefer field-less enums as `#[non_exhaustive]` in public APIs so adding a
  variant is not a breaking change (Effective 21).
- `bool` flags → enum. Parallel "valid when X" fields → enum with data.
- Don't use `String` for identifiers/enums; newtype or enum.

## 7. Error handling (Effective 3, 4, 18; RfR Ch. 4)

- Return `Result<T, E>` from fallible APIs. Never panic to signal expected
  failure.
- Libraries: `thiserror`-derived error enum, `#[non_exhaustive]`, meaningful
  variants, carry source via `#[from]`.
- Applications/binaries: `anyhow::Result` at the top; `?` to propagate.
- Prefer `Option::map`/`and_then`/`ok_or` and `Result::map`/`?` over hand-written
  `match` (Effective 3).
- `?` is the default error-propagation mechanism — use it liberally.
- Document every `# Errors` and `# Panics` section on public functions.

### Panics (Effective 18)
Permitted only when:
1. The state is **provably unreachable** given the type invariants, OR
2. In test/bench code.
`unwrap`/`expect`/indexing/`from_utf8`/division on external input is forbidden.
Use `expect("reason")` over `unwrap()` when you must — it documents the assumption.

## 8. Iterators (Effective 9)

- Prefer iterator combinators over `for` with manual accumulation.
- `.iter()`/`.into_iter()`/`.iter_mut()` consciously; `into_iter()` consumes.
- Chain `map`/`filter`/`take`/`skip`; `collect` once at the end with an explicit
  target type (`: Vec<_>`).
- Avoid `collect`-then-`iter` again — fuse the pipelines.
- `?` works inside `try_for_each` and `Iterator::collect::<Result<_>>()`.

## 9. Traits (Effective 2, 10–13; RfR Ch. 2)

- Implement standard traits (`Debug`, `Clone`, `Eq`, `Hash`, `Display`,
  `FromStr`, `Default`) where they have natural meaning. Derive where possible.
- Use trait **bounds** to express requirements; keep `where` clauses tidy.
- Default trait methods (Item 13) to minimize required impls.
- Prefer generic `impl Trait` over `Box<dyn Trait>` unless object safety /
  heterogeneous storage demands dynamic dispatch (Effective 12).
- Keep traits cohesive and small; composable traits beat fat traits.

## 10. Documentation (Effective 27; RfR Ch. 3)

- Every public item gets `///` (not `//`). First line is a summary sentence.
- Include sections as relevant: `# Examples`, `# Panics`, `# Errors`, `# Safety`.
- Examples compile as doc tests — keep them runnable.
- Document invariants on types, especially those with `unsafe` or non-obvious
  contracts.

## 11. Comments

- Don't restate the code. Comments exist for *why* and for invariants.
- `// SAFETY:` blocks are mandatory before every `unsafe` operation.
- `// TODO:`/`// FIXME:` with a name/ticket; never leave context-free TODOs.

## 12. Edition & language features

- Target the latest stable **edition** (2024; this project uses it with
  `rust-version = "1.93"`). Set `edition` and `rust-version` (MSRV) in `Cargo.toml`.
- Use modern syntax: `let-else`, `let-chains`, `if let` chains where available,
  inline `format!("{x}")`, `dyn`/`impl` explicit, `?` for `Option`.

## 13. Macros (Effective 28; RfR Ch. 7)

- Prefer a function/trait/derive over a macro whenever possible.
- Declarative `macro_rules!` for internal repetition; proc-macros (`#[derive]`)
  for cross-cutting concerns — but treat them as costly (compile time, debuggability).
- Never use a macro where a generic function with a trait bound would do.
