//! Placeholder module.
//!
//! We initially attempted a direct `libbpf-sys` based AF_XDP wrapper here.
//! The `libbpf-sys` crate published on crates.io does not expose the AF_XDP (XSK)
//! symbols we need (`xsk_socket__create`, `xsk_umem__create`, etc), so the host
//! moved to the `xsk-rs` crate.
//!
//! This file remains only to avoid confusing diffs during the migration.

#[allow(dead_code)]
pub fn _deprecated() {}
