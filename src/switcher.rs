//! Pure switcher types shared by the wayland layer and the switcher core.
//!
//! No wayland or cctk imports ever live in this file: protocol data is
//! translated into these plain types at the `wayland` boundary so all
//! switching logic stays unit-testable without a compositor.

use std::collections::HashSet;

/// Opaque identifier for a toplevel, assigned by the wayland layer and stable
/// for the daemon's connection lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Handle(pub u64);

/// Opaque identifier for a workspace, same lifetime rules as [`Handle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct WorkspaceId(pub u64);

/// Toplevel state flags mirroring the `zcosmic_toplevel_handle_v1` states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct State(u8);

impl State {
    pub const MAXIMIZED: Self = Self(1 << 0);
    pub const MINIMIZED: Self = Self(1 << 1);
    pub const ACTIVATED: Self = Self(1 << 2);
    pub const FULLSCREEN: Self = Self(1 << 3);
    pub const STICKY: Self = Self(1 << 4);

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A toplevel as plain data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toplevel {
    pub handle: Handle,
    pub title: String,
    pub app_id: String,
    pub state: State,
    /// Workspaces this toplevel is visible on; sticky toplevels are visible
    /// on all of them.
    pub workspaces: HashSet<WorkspaceId>,
    /// Daemon-maintained MRU tick; 0 until an activation is observed.
    pub last_activated: u64,
}

/// A workspace as plain data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub coordinates: Vec<u32>,
}

/// Point-in-time view of compositor state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub toplevels: Vec<Toplevel>,
    pub workspaces: Vec<Workspace>,
    pub active: Option<WorkspaceId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_union_combines_flags() {
        let combined = State::MINIMIZED.union(State::ACTIVATED);
        assert!(combined.contains(State::MINIMIZED));
        assert!(combined.contains(State::ACTIVATED));
        assert!(!combined.contains(State::MAXIMIZED));
    }

    #[test]
    fn state_union_is_idempotent() {
        let once = State::FULLSCREEN.union(State::STICKY);
        let twice = once.union(State::FULLSCREEN);
        assert_eq!(once, twice);
    }

    #[test]
    fn empty_state_contains_nothing() {
        assert!(!State::default().contains(State::MINIMIZED));
        assert!(!State::default().contains(State::MAXIMIZED));
    }
}
