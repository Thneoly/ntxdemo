# actions-executor-template

A minimal **component template** showing how to build an actions-executor quickly using `ntx-action-sdk` in *framework mode* (traits + macros).

## What you copy

- `src/lib.rs`: structured `execute_action` wrapper + a tiny action module
- `Cargo.toml`: already wired to `ntx-action-sdk`

## How to use

1. Copy this folder to a new component folder.
2. Rename the crate name + component metadata in `Cargo.toml`.
3. Implement handlers in the `MyActions` module.

This template keeps a strong, repeatable structure:

- parse JSON params once
- enforce unknown action failure semantics
- standardize event publishing via an `EventBusAdapter`
- standardize outcome exports as JSON string via `exports_json!`
