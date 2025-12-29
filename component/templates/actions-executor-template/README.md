# actions-executor-template

A minimal **component template** showing how to build an actions-executor quickly using `ntx-action-sdk` in *framework mode* (traits + macros).

## What you copy

- `src/lib.rs`: structured `execute_action` wrapper + a tiny action module
- `Cargo.toml`: already wired to `ntx-action-sdk`

## How to use

### Option A: generate with cargo-generate (recommended)

This folder is a `cargo-generate` template.

From the repo root, generate a new component folder under `component/`:

```bash
cargo generate --path component/templates/actions-executor-template --destination component
```

You'll be prompted for:

- `project-name`: the new crate name (kebab-case)
- `component-namespace`: used in `package.metadata.component.package`
- `component-version`: used in `package.metadata.component.package`

After generation:

- the folder name and crate name will match `project-name`
- `ntx-action-sdk` remains wired via a relative path in the repo

### Option B: manual copy

1. Copy this folder to a new component folder.
2. Rename the crate name + component metadata in `Cargo.toml`.
3. Implement handlers in the `MyActions` module.

This template keeps a strong, repeatable structure:

- parse JSON params once
- enforce unknown action failure semantics
- standardize event publishing via an `EventBusAdapter`
- standardize outcome exports as JSON string via `exports_json!`

```shell
cargo generate --path component/templates/actions-executor-template --destination component
# or
cargo generate --path component/templates/actions-executor-template --destination component --name my-actions-executor --force --define component-namespace=scenario-actions-executor --define component-version=0.1.0
```