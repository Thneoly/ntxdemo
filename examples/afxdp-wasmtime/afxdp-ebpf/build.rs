use std::{env, path::PathBuf};

fn main() {
    // Mirror the minimal build script pattern from xdp-hello.
    // We rely on aya's build tooling resolution via `which` in build-dependencies.
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=src/main.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");

    // Emit a marker so users know where the final object ends up.
    // The actual eBPF artifact naming is handled by aya build integration in the host crate.
    let _ = out_dir;
}
