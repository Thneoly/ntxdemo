use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Plugins to watch: map logical name -> relative path
    let plugins = [
        ("core", "plugins/core"),
        ("demo", "plugins/demo"),
        ("tcp-client", "plugins/tcp-client"),
        ("wac", "plugins/wac"),
    ];

    // Collect all files and print rerun-if-changed so cargo rebuilds when plugin files change
    for &(_, path) in &plugins {
        let p = Path::new(path);
        if p.exists() {
            for entry in walk_files(p).unwrap_or_default() {
                // Tell cargo to re-run build script if any plugin file changes
                println!("cargo:rerun-if-changed={}", entry.display());
            }
        }
    }

    // Locate state file inside OUT_DIR (written by cargo) to avoid polluting repo root
    let out_dir = env::var("OUT_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"));
    let state_file = out_dir.join("plugin_build_state");
    let old_state = read_state(&state_file).unwrap_or_default();

    // Environment switch to disable plugin builds when set (e.g., in CI)
    let disable_plugin_builds = env::var("DISABLE_PLUGIN_BUILDS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // Compute new hashes and determine which plugins changed
    let mut new_state = Vec::new();
    let mut changed = Vec::new();
    for &(name, relpath) in &plugins {
        let p = Path::new(relpath);
        let hash = if p.exists() {
            match compute_dir_hash(p) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("warning: failed to hash {}: {}", relpath, e);
                    0
                }
            }
        } else {
            0
        };
        new_state.push((name.to_string(), hash));
        let prev = old_state
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, h)| *h)
            .unwrap_or(0);
        if prev != hash {
            changed.push((name.to_string(), relpath.to_string()));
        }
    }

    // If changed, run appropriate commands
    for (name, rel) in &changed {
        if disable_plugin_builds {
            println!(
                "build.rs: DISABLE_PLUGIN_BUILDS set, skipping plugin build for {}",
                name
            );
            continue;
        }
        match name.as_str() {
            "core" | "demo" | "tcp-client" => {
                println!(
                    "build.rs: Detected changes in {} -> running cargo build (wasm32-wasip2)",
                    name
                );
                let status = Command::new("cargo")
                    .arg("build")
                    .arg("--target")
                    .arg("wasm32-wasip2")
                    .current_dir(rel)
                    .status();
                match status {
                    Ok(s) if s.success() => {
                        println!("build.rs: cargo build succeeded for {}", name);
                    }
                    Ok(s) => {
                        panic!("build.rs: cargo build for {} failed with exit {}", name, s);
                    }
                    Err(e) => {
                        panic!("build.rs: failed to run cargo in {}: {}", rel, e);
                    }
                }
            }
            "wac" => {
                println!("build.rs: Detected changes in wac -> running run.sh");
                // Prefer using sh to execute run.sh so it's not required to be executable
                let status = Command::new("sh").arg("build.sh").current_dir(rel).status();
                match status {
                    Ok(s) if s.success() => println!("build.rs: run.sh succeeded for wac"),
                    Ok(s) => panic!("build.rs: run.sh for wac failed with exit {}", s),
                    Err(e) => panic!("build.rs: failed to run run.sh in {}: {}", rel, e),
                }
            }
            _ => {}
        }
    }

    // Persist new state
    if let Err(e) = write_state(&state_file, &new_state) {
        eprintln!("warning: failed to write plugin state: {}", e);
    }
}

fn walk_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        for entry in fs::read_dir(&p)? {
            let e = entry?;
            let path = e.path();
            if should_ignore(&path) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn should_ignore(path: &Path) -> bool {
    // ignore .git and node_modules anywhere
    for c in path.components() {
        if let Component::Normal(os) = c {
            let s = os.to_string_lossy();
            if s == ".git" || s == "node_modules" {
                return true;
            }
        }
    }

    // Strictly ignore any 'target' directory that appears under a plugins/ tree
    // (covers cases with multiple nested target dirs). Also ignore 'target' anywhere
    // to avoid walking build outputs.
    let mut seen_plugins = false;
    for c in path.components() {
        if let Component::Normal(os) = c {
            let s = os.to_string_lossy();
            if s == "plugins" {
                seen_plugins = true;
                continue;
            }
            if s == "target" {
                if seen_plugins {
                    return true;
                }
            }
        }
    }
    // final fallback: ignore any component literally named "target"
    path.components().any(|c| {
        if let Component::Normal(os_str) = c {
            os_str == "target"
        } else {
            false
        }
    })
}

fn compute_dir_hash(dir: &Path) -> io::Result<u64> {
    let mut hasher = DefaultHasher::new();
    let files = walk_files(dir)?;
    // Sort to make deterministic
    let mut files_sorted = files;
    files_sorted.sort();
    for path in files_sorted {
        let meta = fs::metadata(&path)?;
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let dur = mtime.duration_since(UNIX_EPOCH).unwrap_or_default();
        let secs = dur.as_secs();
        let nanos = dur.subsec_nanos();
        path.to_string_lossy().hash(&mut hasher);
        meta.len().hash(&mut hasher);
        secs.hash(&mut hasher);
        nanos.hash(&mut hasher);
    }
    Ok(hasher.finish())
}

fn read_state(path: &Path) -> io::Result<Vec<(String, u64)>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let f = File::open(path)?;
    let r = BufReader::new(f);
    let mut out = Vec::new();
    for line in r.lines() {
        let ln = line?;
        if ln.trim().is_empty() {
            continue;
        }
        if let Some((name, val)) = ln.split_once(':') {
            if let Ok(h) = val.trim().parse::<u64>() {
                out.push((name.trim().to_string(), h));
            }
        }
    }
    Ok(out)
}

fn write_state(path: &Path, state: &[(String, u64)]) -> io::Result<()> {
    let mut f = File::create(path)?;
    for (name, h) in state {
        writeln!(f, "{}:{}", name, h)?;
    }
    Ok(())
}
