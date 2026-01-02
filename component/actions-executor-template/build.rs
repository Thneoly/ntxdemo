use std::{path::PathBuf, process::exit, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=wit/deps.toml");
    println!("cargo:rerun-if-changed=wit/deps.lock");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    // Expected output layout after running `wit-deps update`:
    //   wit/deps/<package>/...
    let required = [
        "wit/deps/actions-executor",
        "wit/deps/core-types",
        "wit/deps/eventbus",
    ];

    let missing: Vec<String> = required
        .into_iter()
        .filter(|rel| !manifest_dir.join(rel).exists())
        .map(|s| s.to_string())
        .collect();

    if !missing.is_empty() {
        eprintln!("\nMissing WIT deps: {missing:?}\n");

        // Don't auto-download in build scripts. Just guide the user.
        let wit_deps_installed = Command::new("wit-deps")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok();

        eprintln!("This component template expects WIT packages to be fetched via `wit-deps`.\n");

        if !wit_deps_installed {
            eprintln!("`wit-deps` not found on PATH.");
            eprintln!("Install it (pick one):");
            eprintln!("  cargo binstall wit-deps-cli -y");
            eprintln!("  cargo install wit-deps-cli\n");
        }

        eprintln!("Then, from the crate root:");
        eprintln!("  wit-deps update\n");
        eprintln!("(it reads wit/deps.toml and writes to wit/deps/*)");

        eprintln!("\nFor reproducible CI builds: commit wit/deps.lock and run:");
        eprintln!("  wit-deps lock --check\n");
        exit(1);
    }
}
