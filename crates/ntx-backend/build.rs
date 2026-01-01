use std::process::Command;

fn main() {
    // Best-effort: embed git commit info into the binary.
    // If git isn't available (or not a git checkout), we just omit it.
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
    {
        if output.status.success() {
            let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !sha.is_empty() {
                println!("cargo:rustc-env=NTX_BACKEND_GIT_SHA={}", sha);
            }
        }
    }

    if let Ok(output) = Command::new("git").args(["status", "--porcelain"]).output() {
        if output.status.success() {
            let dirty = !String::from_utf8_lossy(&output.stdout).trim().is_empty();
            println!(
                "cargo:rustc-env=NTX_BACKEND_GIT_DIRTY={}",
                if dirty { "1" } else { "0" }
            );
        }
    }

    // Re-run if HEAD or index changes.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
