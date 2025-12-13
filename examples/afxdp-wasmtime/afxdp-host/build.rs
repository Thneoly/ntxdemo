fn main() {
    use std::{env, fs, path::PathBuf, process::Command};

    println!("cargo:rerun-if-changed=../afxdp-ebpf");

    // Build the eBPF crate with a dedicated target dir, and then copy the produced binary into
    // OUT_DIR with a stable filename. This avoids `aya-build`'s OUT_DIR naming collisions.

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let scratch = out_dir.join("_afxdp_ebpf_target");
    let _ = fs::create_dir_all(&scratch);

    // `-Z build-std=core` requires nightly. We explicitly run the eBPF build on nightly.
    // (The host binary itself can remain on stable.)
    unsafe {
        env::set_var("RUSTUP_TOOLCHAIN", "nightly");
    }

    // Optional: enable eBPF crate features (comma-separated list) via env var.
    // Example: `AFXDP_EBPF_FEATURES=xdp-abort` to force the XDP program to return XDP_ABORTED.
    println!("cargo:rerun-if-env-changed=AFXDP_EBPF_FEATURES");
    let ebpf_features = env::var("AFXDP_EBPF_FEATURES").ok();

    // `-Z build-std=core` requires nightly, so we explicitly run the eBPF build on nightly.
    // (The host crate itself can remain on stable.)
    let mut cmd = Command::new("rustup");
    cmd.args([
        "run",
        "nightly",
        "cargo",
        "build",
        "--package",
        "afxdp-ebpf",
        "-Z",
        "build-std=core",
        "--bins",
        "--release",
        "--target",
        "bpfel-unknown-none",
        "--target-dir",
        scratch.to_str().unwrap(),
    ]);

    if let Some(features) = ebpf_features.as_deref().filter(|s| !s.trim().is_empty()) {
        cmd.args(["--features", features]);
    }

    let status = cmd
        // Cargo injects the parent `RUSTC` into build-scripts; ensure the child build really
        // uses nightly by overriding the `RUSTC` it sees.
        .env("RUSTUP_TOOLCHAIN", "nightly")
        .env_remove("RUSTC")
        .current_dir("..")
        .status()
        .expect("failed to spawn ebpf cargo build");
    assert!(status.success(), "ebpf cargo build failed: {status:?}");

    let built = scratch
        .join("bpfel-unknown-none")
        .join("release")
        .join("afxdp-ebpf");
    let dst = out_dir.join("afxdp-ebpf.o");
    if dst.is_dir() {
        fs::remove_dir_all(&dst).expect("failed to remove stale OUT_DIR/afxdp-ebpf.o directory");
    }
    fs::copy(&built, &dst).expect("failed to copy ebpf artifact into OUT_DIR");
}
