// Re-export the top-level `network/` folder as a module of the `Ntx` crate.
//
// NOTE: The implementation lives in `/network/*` at the repository root.
// We include it from here so we can iterate without turning it into a separate crate yet.

#[path = "../../network/mod.rs"]
mod root;

#[allow(unused_imports)]
pub use root::*;
