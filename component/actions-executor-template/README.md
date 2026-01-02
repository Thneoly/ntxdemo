# actions-executor-template

A minimal **component template** showing how to build an actions-executor quickly using `ntx-action-sdk` in *framework mode* (traits + macros).

## What you copy

- `src/lib.rs`: structured `execute_action` wrapper + a tiny action module
- `Cargo.toml`: already wired to `ntx-action-sdk`

This template also includes a **self-describing action catalog** exported via WIT:

- `schema-version()`
- `list-actions()`
- `describe-action(action-id)`

## How to use

### Requirements

- Rust/Cargo >= 1.91.0 (template `Cargo.toml` sets `rust-version = 1.91`)
- Target: `wasm32-wasip2` (this template includes `.cargo/config.toml` to default builds to it)

Tools (recommended):

```bash
# Optional helper for installing CLI tools quickly
cargo install cargo-binstall

# Build wasm components
cargo binstall cargo-component -y

# Fetch WIT dependencies defined in wit/deps.toml
cargo binstall wit-deps-cli -y

# Target toolchain
rustup target add wasm32-wasip2
```

### Option A: generate with cargo-generate (recommended)

This folder is a `cargo-generate` template.

From the repo root, generate a new component folder under `component/`:

```bash
cargo generate --path component/actions-executor-template --destination component
```

### Option B: generate from a public template repo

If/when you publish this template as a standalone public repo, you can generate directly from Git.

```bash
cargo generate --git https://github.com/Thneoly/ntx-executor-template --name my-actions-executor --force
```

After generation:

- the folder name and crate name will match `project-name`
- `ntx-action-sdk` is fetched via Git tag (default: `v0.0.1`)

- WIT dependencies are fetched via `wit-deps` from `wit/deps.toml` (URLs point to your GitHub Release assets)

Notes:

- Before building, run `wit-deps update` in the generated crate root to fetch WIT packages into `wit/deps/*`.
- For reproducible CI, commit `wit/deps.lock` and run `wit-deps lock --check`.
- If you fork/mirror the repo, update the `ntx-action-sdk` git URL/tag in `Cargo.toml` accordingly.
- If you need fully offline builds inside this mono-repo, switch the SDK dependency back to a path dependency:
	`ntx-action-sdk = { path = "../core-libs/ntx-action-sdk", features = ["core-types-adapter"] }`

Quickstart:

```bash
wit-deps update

# quick sanity check
cargo check

# build the WASM component (recommended)
cargo component build --release
```

The built component typically lands under:

- `target/wasm32-wasip2/release/<crate-name>.wasm`

### Option B: manual copy

1. Copy this folder to a new component folder.
2. Rename the crate name + component metadata in `Cargo.toml`.
3. Implement handlers in the `MyActions` module.

This template keeps a strong, repeatable structure:

- parse JSON params once
- enforce unknown action failure semantics
- standardize event publishing via an `EventBusAdapter`
- standardize outcome exports as JSON string via `exports_json!`

To add your own actions, update both:

- routing + handlers in `MyActions`
- metadata in `ActionExecutorImpl::{list_actions, describe_action}`

After generation, if you want to change the component identity, edit:

- `wit_bindgen::generate!({ world: "..." })` in `src/lib.rs`
- `[package.metadata.component] package = "..."` in `Cargo.toml`