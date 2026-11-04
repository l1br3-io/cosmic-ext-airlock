//! Library target for dev tooling (the `overlay-preview` example). The
//! shipped artifact stays the `airlock` binary; only the pure modules —
//! the ones the example renders from, plus the system-preference read —
//! are exposed here.

pub mod icons;
pub mod ipc;
pub mod switcher;
pub mod theme;
pub mod ui;
