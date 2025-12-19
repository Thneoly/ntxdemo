//! Compatibility wrapper.
//!
//! We keep `scheduler/src/guestnet/` in-tree so older paths and editor navigation still work,
//! but the real implementation has moved to the standalone crate `ntx-guestnet`.
//!
//! Public API: `scheduler::guestnet::*`.

pub use ntx_guestnet::*;
