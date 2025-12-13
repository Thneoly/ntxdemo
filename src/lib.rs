//! Library crate for `Ntx`.
//!
//! The repository currently has a binary entrypoint in `src/main.rs`, but we also
//! want examples (e.g. userspace UDP echo) to reuse the `network/` module tree.
//! Providing a small `lib.rs` makes `cargo run --example ...` able to import
//! `ntx::network::*`.

pub mod network;
