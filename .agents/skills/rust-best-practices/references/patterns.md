# Rust Patterns Rule

Design patterns that make Rust code idiomatic, synthesized from Effective Rust
Items 1, 6, 7, 11 and RfR Chapters 2, 3, 13. Each pattern includes when to use
it and a code sketch.

## 1. Make Invalid States Unrepresentable  *(highest-leverage pattern)*

Encode invariants in the type system so illegal combinations don't compile.

```rust
// BAD: fg_color validity depends on monochrome
struct DisplayProps { monochrome: bool, fg_color: RgbColor }

// GOOD
enum Color { Monochrome, Foreground(RgbColor) }
struct DisplayProps { color: Color }
```

Apply whenever you find a comment like "// X must be None if Y is true".

## 2. Newtype (Effective Item 6)

Wrap a primitive to give it domain meaning and prevent mix-ups.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(pub u64);
pub struct AccountId(pub u64);
// Now UserId and AccountId can't be passed interchangeably.
```
- Cheap (zero-cost at runtime, often `Copy`).
- Add conversions: `From<u64>`, `Into<u64>`, `Deref` only if truly value-like.
- Implement the standard traits (Debug/Eq/Hash/serde) so it behaves like a value.

## 3. Builder (Effective Item 7)

For types with many optional/incompatible fields or multi-step construction.

```rust
pub struct Server { host: String, port: u16, tls: bool /* ... */ }

#[derive(Default)]
pub struct ServerBuilder { host: Option<String>, port: Option<u16>, tls: bool }

impl ServerBuilder {
    pub fn host(mut self, h: impl Into<String>) -> Self { self.host = Some(h.into()); self }
    pub fn port(mut self, p: u16) -> Self { self.port = Some(p); self }
    pub fn tls(mut self) -> Self { self.tls = true; self }
    pub fn build(self) -> Result<Server, BuildError> {
        Ok(Server {
            host: self.host.ok_or(BuildError::MissingHost)?,
            port: self.port.unwrap_or(80),
            tls: self.tls,
        })
    }
}
impl Server {
    pub fn builder() -> ServerBuilder { ServerBuilder::default() }
}
```
- Use `#[derive(Default)]` on the builder to reduce boilerplate, or
  `derive_builder`/`typed-builder` crates for compile-time required-field checks.
- Builders make the illegal intermediate states (half-built) unconstructable by
  the caller.

## 4. Type-State

Use phantom type parameters to encode configuration into the type, moving
runtime checks to compile time.

```rust
use std::marker::PhantomData;

pub struct Sealed;
pub struct Unsealed;
pub struct Channel<State = Unsealed> { buf: Vec<u8>, _state: PhantomData<State> }

impl Channel<Unsealed> {
    pub fn seal(self) -> Channel<Sealed> { Channel { buf: self.buf, _state: PhantomData } }
}
impl Channel<Sealed> {
    pub fn checksum(&self) -> u32 { /* ... */ }
}
```
Use sparingly—powerful but increases type complexity.

## 5. RAII / Drop Guard (Effective Item 11; RfR Ch. 13)

Tie resource acquisition to object lifetime; release in `Drop`.

```rust
pub struct TempFile { path: PathBuf }
impl Drop for TempFile { fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); } }
```
- Implement `Drop` for anything owning an OS handle, lock, allocation, or
  transaction. Never require callers to remember to "close".
- Pair acquisition with a guard type so release is automatic even on early
  return/panic. (`MutexGuard`, `BufReader`, scoped threads.)

## 6. Error-as-Enum with `thiserror` (RfR Ch. 4)

```rust
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("io error")]
    Io(#[from] std::io::Error),
    #[error("invalid format")]
    Invalid(#[from] serde_json::Error),
}
```
- One variant per user-actionable failure mode.
- `#[non_exhaustive]` so adding variants is non-breaking (Effective 21).
- Use `#[from]` to auto-convert and source-chain.
- Application top-level: `anyhow::Result` + `anyhow::Context` for ergonomics.

## 7. Combinator / Pipeline over Match (Effective Items 3, 9)

```rust
// Prefer:
let total: u64 = ids.iter().map(|i| i.0).sum();

// Over:
let mut total = 0u64;
for i in &ids { total += i.0; }
```
And for `Option`/`Result`:
```rust
let name = user.and_then(|u| u.name).ok_or_else(|| Error::NoName)?;
```

## 8. Extension Trait (RfR Ch. 13)

Add convenience methods to foreign types via a trait.

```rust
pub trait JsonExt: Sized {
    fn to_json(&self) -> anyhow::Result<String> where Self: Serialize;
}
impl<T: Serialize> JsonExt for T {
    fn to_json(&self) -> anyhow::Result<String> { Ok(serde_json::to_string(self)?) }
}
```

## 9. SlotMap / Indexed Storage (RfR Ch. 13 "Index Pointers")

For graphs/ASTs with arbitrary sharing, store nodes in a `Vec`/`SlotMap` and
reference them by `u32` keys — avoids `Rc<RefCell<>>` borrow-ordering problems
and enables simple `Clone` of references.

## 10. Enum Dispatch over Dynamic Dispatch

When you have a closed set of behaviors, an enum with a `match` is often faster,
simpler, and exhaustiveness-checked versus `Box<dyn Trait>`.

```rust
enum Shape { Circle(f64), Rect(f64, f64) }
impl Shape {
    fn area(&self) -> f64 {
        match self { Shape::Circle(r) => std::f64::consts::PI*r*r, Shape::Rect(w,h)=>w*h }
    }
}
```

## 11. PhantomData for Lifetimes/Types without Fields

When a struct has a lifetime/type parameter but no field of that type, add
`PhantomData<X>` to express the intended variance and drop behavior.

## 12. Crate Prelude (RfR Ch. 13)

For libraries with commonly-needed traits, provide a `prelude` module users can
glob-import: `pub mod prelude { pub use crate::{Foo, BarTrait}; }`. Document that
`use yourcrate::prelude::*;` is the one acceptable glob import.

## Anti-patterns to refuse

- `Rc<RefCell<Node>>` graphs for tree-like data — use indices/SlotMap.
- `Box<dyn Trait>` when a generic `impl Trait` parameter would do.
- God-struct with `Option<T>` fields representing mutually-exclusive states.
- Builder that panics on `build()` for missing fields — return `Result` or use
  type-state/typed-builder.
- Hand-rolling what a std method/iterator does (`for`+`push` instead of `collect`).
